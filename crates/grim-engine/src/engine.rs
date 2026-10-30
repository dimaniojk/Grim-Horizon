// Grim-Horizon: a Rust port of Grim Anticheat (https://github.com/GrimAnticheat/Grim).
// Copyright (C) GrimAC contributors (original Java), Grim-Horizon contributors (port).
// Licensed under the GNU General Public License, version 3 or (at your option) any later version; see LICENSE.

//! The anti-cheat engine: per-player state, packet handling in client order,
//! transactions, setbacks, and the checks. It knows nothing about the server
//! it runs in: the adapter feeds it [`Serverbound`] and [`Clientbound`]
//! events and applies the [`Action`]s it returns.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::checks::{CheckId, Flag, Violations};
use crate::collisions;
use crate::config::Config;
use crate::entities::EntityHistory;
use crate::math::{Aabb, Vec3, look_vector, wrap_degrees};
use crate::player::{Attr, Effect, GameMode, KnownInput, MOVEMENT_THRESHOLD, Modifier, Movement};
use crate::prediction::{self, Context, PositionUpdate, PredictionComplete};
use crate::velocity::{ExplosionHandler, KnockbackHandler, VelocityVerdict};
use crate::world::{BlockView, PendingBlocks};

/// The adapter's id for a connected player (Horizon's connection id).
pub type PlayerKey = u32;

/// A packet from the client, in the order the client sent it.
#[derive(Clone, Debug, PartialEq)]
pub enum Serverbound {
    /// `ServerboundMovePlayerPacket` (`Pos`, `PosRot`, `Rot`, `StatusOnly`).
    Move {
        pos: Option<Vec3>,
        rot: Option<(f32, f32)>,
        on_ground: bool,
        horizontal_collision: bool,
    },
    /// `ServerboundAcceptTeleportationPacket` (26.3 carries the position).
    AcceptTeleport {
        id: i32,
        pos: Vec3,
        rot: (f32, f32),
    },
    /// `ServerboundClientTickEndPacket`.
    TickEnd,
    /// `ServerboundPlayerInputPacket` flags.
    Input {
        flags: u8,
    },
    /// `ServerboundPlayerCommandPacket` action ordinal (1 start sprint, 2 stop sprint).
    PlayerCommand {
        action: i32,
    },
    /// `ServerboundPlayerAbilitiesPacket`.
    Abilities {
        flying: bool,
    },
    Attack {
        entity: i32,
    },
    Interact {
        entity: i32,
    },
    /// `ServerboundSwingPacket`.
    Swing {
        main_hand: bool,
    },
    HeldSlot {
        slot: i32,
    },
    /// `ServerboundPongPacket`.
    Pong {
        id: i32,
    },
    UseItemOn {
        pos: [i32; 3],
        face: i32,
    },
    /// `ServerboundPlayerActionPacket` (0 start, 1 abort, 2 finish digging...).
    Dig {
        action: i32,
        pos: [i32; 3],
    },
    UseItem,
    /// Anything else, which only resets per-tick packet order state.
    Other,
}

/// A packet the server sent this player, in send order.
#[derive(Clone, Debug, PartialEq)]
pub enum Clientbound {
    /// `ClientboundPingPacket` with this id (the engine's own transactions
    /// come back through the adapter's send hook).
    Ping { id: i32 },
    /// `ClientboundSetEntityMotionPacket` for the player's own entity.
    Velocity(Vec3),
    /// `ClientboundExplodePacket.playerKnockback`.
    Explosion(Vec3),
    /// `ClientboundPlayerPositionPacket`.
    Teleport {
        id: i32,
        pos: Vec3,
        delta: Vec3,
        relatives: i32,
    },
    /// `ClientboundUpdateAttributesPacket` for the player.
    Attribute {
        attr: Attr,
        base: f64,
        modifiers: Vec<Modifier>,
    },
    /// `ClientboundUpdateMobEffectPacket` / `RemoveMobEffect` (None).
    Effect {
        effect: Effect,
        amplifier: Option<i32>,
    },
    /// `ClientboundPlayerAbilitiesPacket`.
    Abilities {
        flying: bool,
        may_fly: bool,
        fly_speed: f32,
    },
    /// `ClientboundGameEventPacket` CHANGE_GAME_MODE, or the login/respawn game mode.
    GameMode(i32),
    /// `ClientboundRespawnPacket`: a new level or a respawn.
    Respawn,
    /// The player died or came back to life (`SetHealth` at 0).
    Dead(bool),
    /// A block the client will change when it handles this packet.
    BlockChanged([i32; 3]),
    /// The player started (`Some(vehicle)`) or stopped riding.
    Riding(Option<i32>),
    /// Sleeping in a bed or not.
    Sleeping(bool),
    /// The player's own entity id (from the login packet).
    EntityId(i32),
}

/// What the engine asks the server to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Teleport the player back (a setback).
    Setback { key: PlayerKey, pos: Vec3 },
    /// Disconnect the player.
    Kick { key: PlayerKey, reason: String },
    /// A chat line for one player.
    Message { key: PlayerKey, text: String },
    /// A line for the server log.
    Log(String),
    /// Send `ClientboundPingPacket` with this id.
    SendPing { key: PlayerKey, id: i32 },
}

/// What to do with one incoming packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Keep,
    /// Drop it before the server handles it.
    Cancel,
    /// Let it through with this on-ground flag (NoFall/GroundSpoof correction).
    SetOnGround(bool),
}

/// Our transaction ids carry this bit, so pongs for anyone else's pings are
/// told apart.
pub const PING_MARK: i32 = 1 << 30;

#[derive(Debug, Default)]
struct Transactions {
    /// The last transaction sent (0: none yet).
    last_sent: u32,
    /// The last transaction answered.
    last_received: u32,
    /// (id, send time, server tick) of every unanswered transaction.
    in_flight: VecDeque<(u32, u64, u64)>,
    /// Send time of the last answered transaction (`playerClockAtLeast`).
    player_clock_at_least: Option<u64>,
    /// Server tick at the end of which the last answered transaction went out.
    acked_tick: u64,
}

#[derive(Debug)]
struct PendingTeleport {
    id: i32,
    pos: Vec3,
    delta: Vec3,
    relatives: i32,
    /// Sent by the engine as a setback.
    setback: bool,
}

#[derive(Debug, Default)]
struct TimerState {
    balance: i64,
    last_movement_player_clock: i64,
    known_player_clock: i64,
    has_movement_after_transaction: bool,
    initialised: bool,
}

#[derive(Debug, Default)]
struct PacketState {
    last_slot: Option<i32>,
    last_sprint_command: Option<bool>,
    sprint_exempt_next: bool,
    no_reminder_ticks: u32,
    received_tick_end: bool,
    flying_packets: u32,
    last_delta_yaw: f32,
    aim_exempt: bool,
    swung_since_attack: bool,
    attacked: bool,
}

#[derive(Debug, Clone, Copy)]
struct QueuedAttack {
    entity: i32,
    pos: Vec3,
    from_tick: u64,
    to_tick: u64,
}

/// Everything the engine tracks about one player.
pub struct PlayerData {
    pub key: PlayerKey,
    pub uuid: u128,
    pub name: String,
    pub entity_id: i32,
    /// Operator level.
    pub permission: u8,
    pub dimension: String,
    pub movement: Movement,
    pub violations: Violations,
    kb: KnockbackHandler,
    explosion: ExplosionHandler,
    tx: Transactions,
    teleports: VecDeque<PendingTeleport>,
    /// A setback the engine asked for and the client has not accepted yet.
    awaiting_setback: Option<Vec3>,
    /// Setback ticks complete: 1 right after a setback was accepted.
    setback_ticks_complete: u32,
    last_good: Option<Vec3>,
    /// The first teleport (join, respawn) was accepted: movement can be judged.
    spawned: bool,
    pending_blocks: PendingBlocks,
    timer: TimerState,
    packets: PacketState,
    attacks: Vec<QueuedAttack>,
    advantage_gained: f64,
    /// Staff toggles.
    pub alerts: bool,
    pub verbose: bool,
    /// Players whose prediction details this staff member receives.
    pub debug_targets: HashSet<PlayerKey>,
    /// Exempted at runtime (`/grim exempt`).
    pub exempt: bool,
    /// Counters for `/grim profile`.
    pub predictions: u64,
    pub skipped: u64,
    pub last_offset: f64,
    pub last_skip_reason: Option<&'static str>,
    last_timeout_warning: u64,
}

impl PlayerData {
    fn new(key: PlayerKey, uuid: u128, name: String, permission: u8, config: &Config) -> Self {
        Self {
            key,
            uuid,
            name,
            entity_id: -1,
            permission,
            dimension: "minecraft:overworld".into(),
            movement: Movement::default(),
            violations: Violations::default(),
            kb: KnockbackHandler::default(),
            explosion: ExplosionHandler::default(),
            tx: Transactions::default(),
            teleports: VecDeque::new(),
            awaiting_setback: None,
            setback_ticks_complete: 0,
            last_good: None,
            spawned: false,
            pending_blocks: PendingBlocks::default(),
            timer: TimerState::default(),
            packets: PacketState {
                received_tick_end: true,
                sprint_exempt_next: true,
                ..Default::default()
            },
            attacks: Vec::new(),
            advantage_gained: 0.0,
            alerts: permission >= config.permission_level && config.alerts.enabled_on_join,
            verbose: false,
            debug_targets: HashSet::new(),
            exempt: false,
            predictions: 0,
            skipped: 0,
            last_offset: 0.0,
            last_skip_reason: None,
            last_timeout_warning: 0,
        }
    }

    /// Round-trip time of the oldest answered transaction window, in ms.
    pub fn transaction_ping_ms(&self, now: u64) -> Option<f64> {
        let oldest = self.tx.in_flight.front().map(|f| f.1);
        let clock = self.tx.player_clock_at_least?;
        Some((oldest.unwrap_or(now).max(clock).saturating_sub(clock)) as f64 / 1e6)
    }
}

/// Engine-wide statistics.
#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    pub predictions: u64,
    pub prediction_nanos: u64,
    pub packets: u64,
}

pub struct Engine {
    pub config: Config,
    players: HashMap<PlayerKey, PlayerData>,
    /// Entity positions by level.
    entities: HashMap<String, EntityHistory>,
    /// Server ticks so far.
    tick: u64,
    actions: Vec<Action>,
    /// Entity sizes registered by the adapter, by type id.
    entity_sizes: HashMap<String, (f32, f32)>,
    pub stats: Stats,
}

impl Engine {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            players: HashMap::new(),
            entities: HashMap::new(),
            tick: 0,
            actions: Vec::new(),
            entity_sizes: HashMap::new(),
            stats: Stats::default(),
        }
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn player(&self, key: PlayerKey) -> Option<&PlayerData> {
        self.players.get(&key)
    }
    pub fn player_mut(&mut self, key: PlayerKey) -> Option<&mut PlayerData> {
        self.players.get_mut(&key)
    }
    pub fn players(&self) -> impl Iterator<Item = &PlayerData> {
        self.players.values()
    }
    pub fn find_player(&self, name: &str) -> Option<PlayerKey> {
        self.players
            .values()
            .find(|p| p.name.eq_ignore_ascii_case(name))
            .map(|p| p.key)
    }
    /// Takes the actions produced since the last call.
    pub fn drain_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    pub fn register_entity_size(&mut self, kind: &str, width: f32, height: f32) {
        self.entity_sizes.insert(kind.to_owned(), (width, height));
    }
    /// The width and height of an entity type, from the registered table or
    /// the built-in one.
    pub fn entity_size(&self, kind: &str) -> Option<(f32, f32)> {
        self.entity_sizes
            .get(kind)
            .copied()
            .or_else(|| crate::entities::builtin_size(kind))
    }

    pub fn add_player(&mut self, key: PlayerKey, uuid: u128, name: &str, permission: u8) {
        let p = PlayerData::new(key, uuid, name.to_owned(), permission, &self.config);
        self.players.insert(key, p);
    }
    pub fn remove_player(&mut self, key: PlayerKey) {
        self.players.remove(&key);
        for p in self.players.values_mut() {
            p.debug_targets.remove(&key);
        }
    }
    pub fn set_permission(&mut self, key: PlayerKey, permission: u8) {
        let level = self.config.permission_level;
        if let Some(p) = self.players.get_mut(&key) {
            if p.permission < level && permission >= level && self.config.alerts.enabled_on_join {
                p.alerts = true;
            }
            if permission < level {
                p.alerts = false;
                p.verbose = false;
                p.debug_targets.clear();
            }
            p.permission = permission;
        }
    }
    pub fn set_dimension(&mut self, key: PlayerKey, dimension: &str) {
        if let Some(p) = self.players.get_mut(&key)
            && p.dimension != dimension
        {
            p.dimension = dimension.to_owned();
        }
    }

    /// Records the entities of a level at the start of the current tick.
    pub fn record_entities(
        &mut self,
        dimension: &str,
        entities: impl IntoIterator<Item = crate::entities::EntitySnapshot>,
    ) {
        let tick = self.tick;
        self.entities
            .entry(dimension.to_owned())
            .or_default()
            .record(tick, entities);
    }

    /// The end of a server tick: one transaction for every player, timeouts
    /// and violation housekeeping.
    pub fn end_tick(&mut self, now: u64) {
        let tick = self.tick;
        let timeout = self.config.max_transaction_time_seconds;
        let punish = self.config.punishments_enabled();
        for p in self.players.values_mut() {
            let id = p.tx.last_sent.wrapping_add(1) & 0x3FFF_FFFF;
            p.tx.last_sent = id;
            if p.tx.in_flight.len() >= 1200 {
                p.tx.in_flight.pop_front();
            }
            p.tx.in_flight.push_back((id, now, tick));
            self.actions.push(Action::SendPing {
                key: p.key,
                id: (id as i32) | PING_MARK,
            });
            if timeout > 0
                && let Some(&(_, sent, _)) = p.tx.in_flight.front()
                && now.saturating_sub(sent) > timeout * 1_000_000_000
            {
                if punish {
                    self.actions.push(Action::Kick {
                        key: p.key,
                        reason: "Timed out".into(),
                    });
                } else if now.saturating_sub(p.last_timeout_warning) > 30_000_000_000 {
                    p.last_timeout_warning = now;
                    self.actions.push(Action::Log(format!(
                        "[Grim] {} has not answered transactions for {}s; their movement is not being judged",
                        p.name,
                        now.saturating_sub(sent) / 1_000_000_000
                    )));
                }
            }
        }
        self.tick += 1;
    }

    /// A packet the server sent `key`, in send order.
    pub fn on_outgoing(&mut self, key: PlayerKey, packet: Clientbound) {
        let Some(p) = self.players.get_mut(&key) else {
            return;
        };
        let tx = p.tx.last_sent;
        match packet {
            Clientbound::Ping { .. } => {}
            Clientbound::EntityId(id) => p.entity_id = id,
            Clientbound::Velocity(v) => p.kb.add(v, tx, p.awaiting_setback.is_some()),
            Clientbound::Explosion(v) => {
                if v != Vec3::ZERO {
                    p.explosion.add(v, tx);
                }
            }
            Clientbound::Teleport {
                id,
                pos,
                delta,
                relatives,
            } => {
                let setback = p
                    .awaiting_setback
                    .is_some_and(|s| s.distance_squared(pos) < 1e-6);
                if p.teleports.len() >= 64 {
                    p.teleports.pop_front();
                }
                p.teleports.push_back(PendingTeleport {
                    id,
                    pos,
                    delta,
                    relatives,
                    setback,
                });
            }
            Clientbound::Attribute {
                attr,
                base,
                modifiers,
            } => {
                if attr == Attr::Gravity {
                    p.movement.point_three.gravity_changed = true;
                }
                p.movement.attributes.set(attr, base, modifiers);
            }
            Clientbound::Effect { effect, amplifier } => {
                if effect == Effect::Levitation {
                    p.movement
                        .point_three
                        .update_levitation(amplifier.or(p.movement.effects.level(effect)));
                }
                p.movement.effects.set(effect, amplifier);
            }
            Clientbound::Abilities {
                flying,
                may_fly,
                fly_speed,
            } => {
                p.movement.is_flying = flying;
                p.movement.may_fly = may_fly;
                p.movement.fly_speed = fly_speed;
            }
            Clientbound::GameMode(m) => p.movement.game_mode = GameMode::from_id(m),
            Clientbound::Respawn => {
                // A new level or respawn: judge nothing until the following
                // teleport has been accepted.
                p.spawned = false;
                p.movement.effects.clear();
                p.movement.is_dead = false;
                p.movement.client_velocity = Vec3::ZERO;
                p.pending_blocks.clear();
                p.kb.clear();
                p.explosion.clear();
                p.packets.no_reminder_ticks = 0;
            }
            Clientbound::Dead(dead) => p.movement.is_dead = dead,
            Clientbound::BlockChanged(pos) => p.pending_blocks.add(pos, tx.wrapping_add(1)),
            Clientbound::Riding(v) => {
                p.movement.in_vehicle = v.is_some();
                p.movement.uncertainty.last_flying_status_change.reset();
            }
            Clientbound::Sleeping(s) => p.movement.in_bed = s,
        }
    }

    /// Handles the client's packets in order against `world` (the level the
    /// player is in). `now` is a monotonic clock in nanoseconds. Returns one
    /// verdict per packet.
    pub fn on_incoming(
        &mut self,
        key: PlayerKey,
        packets: &[Serverbound],
        world: &dyn BlockView,
        now: u64,
    ) -> Vec<Verdict> {
        let mut verdicts = Vec::with_capacity(packets.len());
        let Some(mut p) = self.players.remove(&key) else {
            verdicts.resize(packets.len(), Verdict::Keep);
            return verdicts;
        };
        for packet in packets {
            self.stats.packets += 1;
            let v = self.handle(&mut p, packet, world, now);
            verdicts.push(v);
        }
        self.players.insert(key, p);
        verdicts
    }

    fn checks_off(&self, p: &PlayerData) -> bool {
        !self.config.enabled || p.exempt || self.config.is_exempt(&p.name)
    }

    fn handle(
        &mut self,
        p: &mut PlayerData,
        packet: &Serverbound,
        world: &dyn BlockView,
        now: u64,
    ) -> Verdict {
        let modify = self.config.setbacks_enabled() && !self.checks_off(p);
        match packet {
            Serverbound::Pong { id } => {
                self.on_pong(p, *id, now);
                Verdict::Keep
            }
            Serverbound::AcceptTeleport { id, pos, rot } => {
                self.on_accept_teleport(p, *id, *pos, *rot, world);
                Verdict::Keep
            }
            Serverbound::Move {
                pos,
                rot,
                on_ground,
                ..
            } => self.on_move(p, *pos, *rot, *on_ground, world, now, modify),
            Serverbound::TickEnd => {
                self.on_tick_end(p, world, now);
                Verdict::Keep
            }
            Serverbound::Input { flags } => {
                let input = KnownInput::from_flags(*flags);
                p.movement.known_input = input;
                p.movement.is_sneaking = input.shift;
                Verdict::Keep
            }
            Serverbound::PlayerCommand { action } => {
                match action {
                    1 | 2 => {
                        let start = *action == 1;
                        // BadPacketsF.
                        if p.packets.last_sprint_command == Some(start) {
                            if p.packets.sprint_exempt_next {
                                p.packets.sprint_exempt_next = false;
                            } else {
                                self.flag(p, CheckId::BadPacketsF, format!("state={start}"));
                            }
                        }
                        p.packets.last_sprint_command = Some(start);
                        p.movement.is_sprinting = start;
                    }
                    6 => p.movement.uncertainty.last_flying_status_change.reset(),
                    _ => {}
                }
                Verdict::Keep
            }
            Serverbound::Abilities { flying } => {
                if *flying && !p.movement.may_fly {
                    self.flag(p, CheckId::BadPacketsI, String::new());
                    return if modify {
                        Verdict::Cancel
                    } else {
                        Verdict::Keep
                    };
                }
                p.movement.is_flying = *flying && p.movement.may_fly;
                Verdict::Keep
            }
            Serverbound::Attack { entity } => self.on_attack(p, *entity, modify),
            Serverbound::Interact { entity } => {
                if *entity == p.entity_id && p.entity_id >= 0 {
                    self.flag(p, CheckId::SelfInteract, String::new());
                    return if modify {
                        Verdict::Cancel
                    } else {
                        Verdict::Keep
                    };
                }
                Verdict::Keep
            }
            Serverbound::Swing { main_hand } => {
                if *main_hand {
                    p.packets.swung_since_attack = true;
                    p.packets.attacked = false;
                }
                Verdict::Keep
            }
            Serverbound::HeldSlot { slot } => {
                if !(0..=8).contains(slot) {
                    self.flag(p, CheckId::BadPacketsY, format!("slot={slot}"));
                    return if modify {
                        Verdict::Cancel
                    } else {
                        Verdict::Keep
                    };
                }
                if p.packets.last_slot == Some(*slot) {
                    self.flag(p, CheckId::BadPacketsA, format!("slot={slot}"));
                }
                p.packets.last_slot = Some(*slot);
                self.packet_order_reset(p);
                Verdict::Keep
            }
            Serverbound::UseItemOn { pos, face } => {
                let d = face_offset(*face);
                let tx = p.tx.last_sent.wrapping_add(1);
                p.pending_blocks.add(*pos, tx);
                p.pending_blocks
                    .add([pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]], tx);
                self.packet_order_reset(p);
                Verdict::Keep
            }
            Serverbound::Dig { action, pos } => {
                if (0..=2).contains(action) {
                    p.pending_blocks.add(*pos, p.tx.last_sent.wrapping_add(1));
                }
                self.packet_order_reset(p);
                Verdict::Keep
            }
            Serverbound::UseItem | Serverbound::Other => {
                self.packet_order_reset(p);
                Verdict::Keep
            }
        }
    }

    /// PacketOrderB's "a non-async packet came between the attack and its swing".
    fn packet_order_reset(&mut self, p: &mut PlayerData) {
        if p.packets.attacked {
            p.packets.attacked = false;
            self.flag(p, CheckId::PacketOrderB, "post-attack".into());
        }
    }

    fn on_pong(&mut self, p: &mut PlayerData, id: i32, now: u64) {
        if id & PING_MARK == 0 {
            return;
        }
        let id = (id & !PING_MARK) as u32;
        let Some(index) = p.tx.in_flight.iter().position(|f| f.0 == id) else {
            self.flag(p, CheckId::TransactionOrder, format!("unknown id {id}"));
            return;
        };
        if index != 0 {
            self.flag(p, CheckId::TransactionOrder, format!("skipped {index}"));
        }
        let (_, sent, tick) = p.tx.in_flight[index];
        p.tx.in_flight.drain(..=index);
        p.tx.last_received = id;
        p.tx.player_clock_at_least = Some(sent);
        p.tx.acked_tick = tick;
        p.pending_blocks.confirm(id);
        // Timer: the first transaction after a movement moves the clock.
        if p.timer.has_movement_after_transaction {
            p.timer.known_player_clock = p.timer.last_movement_player_clock;
            p.timer.last_movement_player_clock = sent as i64;
            p.timer.has_movement_after_transaction = false;
        }
        let _ = now;
    }

    fn on_accept_teleport(
        &mut self,
        p: &mut PlayerData,
        id: i32,
        pos: Vec3,
        rot: (f32, f32),
        world: &dyn BlockView,
    ) {
        let Some(index) = p.teleports.iter().position(|t| t.id == id) else {
            self.flag(p, CheckId::BadPacketsN, format!("unknown teleport {id}"));
            return;
        };
        let t = p.teleports.remove(index).expect("index from position");
        // Teleports sent before this one were skipped by the client.
        p.teleports.drain(..index);
        let m = &mut p.movement;
        let target = Vec3::new(
            if t.relatives & 1 != 0 {
                m.x + t.pos.x
            } else {
                t.pos.x
            },
            if t.relatives & 2 != 0 {
                m.y + t.pos.y
            } else {
                t.pos.y
            },
            if t.relatives & 4 != 0 {
                m.z + t.pos.z
            } else {
                t.pos.z
            },
        );
        // 26.3 clients send where they ended up: it must be the target.
        let to = if pos.is_finite() && pos.distance_squared(target) < 1e-4 {
            pos
        } else {
            target
        };
        let velocity = Vec3::new(
            if t.relatives & 32 != 0 {
                m.client_velocity.x + t.delta.x
            } else {
                t.delta.x
            },
            if t.relatives & 64 != 0 {
                m.client_velocity.y + t.delta.y
            } else {
                t.delta.y
            },
            if t.relatives & 128 != 0 {
                m.client_velocity.z + t.delta.z
            } else {
                t.delta.z
            },
        );
        if rot.0.is_finite() && rot.1.is_finite() {
            (m.last_yaw, m.last_pitch) = (m.yaw, m.pitch);
            (m.yaw, m.pitch) = (rot.0, rot.1);
        }
        let rotate_delta = t.relatives & 256 != 0;
        let mut kb = std::mem::take(&mut p.kb);
        let mut ex = std::mem::take(&mut p.explosion);
        {
            let mut cx = self.context(world, &mut kb, &mut ex, false);
            let update = PositionUpdate {
                to,
                on_ground: false,
                is_teleport: true,
                teleport_velocity: if rotate_delta { None } else { Some(velocity) },
            };
            prediction::handle_teleport(&mut p.movement, &mut cx, update);
        }
        p.kb = kb;
        p.explosion = ex;
        if rotate_delta {
            // The velocity is turned with the look: unknown, follow the client.
            p.movement.uncertainty.last_flying_status_change.reset();
        }
        p.packets.aim_exempt = true;
        p.packets.no_reminder_ticks = 0;
        p.last_good = Some(to);
        p.spawned = true;
        if t.setback
            || p.awaiting_setback
                .is_some_and(|s| s.distance_squared(to) < 1e-6)
        {
            p.awaiting_setback = None;
            p.setback_ticks_complete = 0;
        }
        // The client's ground flag is not changed by a teleport (Grim keeps
        // packetPlayerOnGround as it was).
    }

    fn context<'a>(
        &self,
        world: &'a dyn BlockView,
        kb: &'a mut KnockbackHandler,
        ex: &'a mut ExplosionHandler,
        disallow_stepping: bool,
    ) -> Context<'a> {
        Context {
            world,
            kb,
            explosion: ex,
            kb_cfg: self.config.velocity(),
            explosion_threshold: self.config.explosion.threshold,
            disallow_stepping,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn on_move(
        &mut self,
        p: &mut PlayerData,
        pos: Option<Vec3>,
        rot: Option<(f32, f32)>,
        on_ground: bool,
        world: &dyn BlockView,
        now: u64,
        modify: bool,
    ) -> Verdict {
        let mut verdict = Verdict::Keep;
        // CrashC: non-finite values.
        if pos.is_some_and(|v| !v.is_finite())
            || rot.is_some_and(|r| !r.0.is_finite() || !r.1.is_finite())
        {
            self.flag(p, CheckId::CrashC, String::new());
            return Verdict::Keep; // Horizon disconnects for these itself.
        }
        // CrashA: beyond the hard world border.
        if let Some(v) = pos
            && (v.x.abs() > 2.9999999e7
                || v.z.abs() > 2.9999999e7
                || v.y.abs() > f64::from(i32::MAX))
        {
            self.flag(p, CheckId::CrashA, String::new());
            self.request_setback(p);
            return if modify {
                Verdict::Cancel
            } else {
                Verdict::Keep
            };
        }
        // BadPacketsD.
        if let Some((_, pitch)) = rot
            && !(-90.0..=90.0).contains(&pitch)
        {
            self.flag(p, CheckId::BadPacketsD, format!("pitch={pitch}"));
            if modify {
                verdict = Verdict::Cancel;
            }
        }
        // TickTimer: each movement packet must be followed by a tick end.
        if !p.packets.received_tick_end {
            self.flag(
                p,
                CheckId::TickTimer,
                format!("type=flying, packets={}", p.packets.flying_packets),
            );
        }
        p.packets.received_tick_end = false;
        p.packets.flying_packets += 1;
        // Timer.
        self.timer_tick(p, now);
        // BadPacketsE.
        if pos.is_some() {
            p.packets.no_reminder_ticks = 0;
        } else if !p.movement.in_vehicle {
            p.packets.no_reminder_ticks += 1;
            if p.packets.no_reminder_ticks > 19 {
                let ticks = p.packets.no_reminder_ticks;
                self.flag(p, CheckId::BadPacketsE, format!("ticks={ticks}"));
            }
        }
        self.packet_order_reset(p);

        // Rotation.
        if let Some((yaw, pitch)) = rot {
            let (old_yaw, old_pitch) = (p.movement.yaw, p.movement.pitch);
            p.movement.last_yaw = old_yaw;
            p.movement.last_pitch = old_pitch;
            p.movement.yaw = yaw;
            p.movement.pitch = pitch.clamp(-90.0, 90.0);
            self.rotation_checks(p, old_yaw, old_pitch, yaw, pitch);
        }

        if !p.spawned || p.movement.in_vehicle {
            p.movement.packet_on_ground = on_ground;
            p.movement.did_last_last_movement_include_position =
                p.movement.did_last_movement_include_position;
            p.movement.did_last_movement_include_position = pos.is_some();
            p.movement.did_send_movement_before_tick_end = true;
            if let Some(v) = pos {
                let m = &mut p.movement;
                (m.last_x, m.last_y, m.last_z) = (m.x, m.y, m.z);
                (m.x, m.y, m.z) = (v.x, v.y, v.z);
                m.last_x = m.x;
                m.last_y = m.y;
                m.last_z = m.z;
                m.bounding_box = m.pose_box_at(v.x, v.y, v.z);
            }
            return verdict;
        }

        // Ground status changed without a position (handleFlying). Grim
        // answers an infeasible change with a resync teleport; without one
        // (alert-only mode) the engine must still follow the client, so a
        // claim of leaving the ground, which gains nothing, is always taken.
        if pos.is_none() && on_ground != p.movement.packet_on_ground {
            let m = &mut p.movement;
            let feasible = !on_ground
                || (collisions::slow_could_point_three_hit_ground(m, world, m.x, m.y, m.z)
                    && !(m.client_velocity.y > 0.06
                        && !m.uncertainty.was_affected_by_stuck_speed()));
            if feasible {
                m.last_on_ground = on_ground;
                m.client_claims_last_on_ground = on_ground;
                m.uncertainty.on_ground_uncertain = true;
            } else {
                self.request_setback(p);
            }
        }
        // NoFall for packets without a position.
        let mut nofall = false;
        if on_ground && pos.is_none() && p.awaiting_setback.is_none() && !self.near_ground(p, world)
        {
            self.flag_with_setback(p, CheckId::NoFall, String::new());
            nofall = true;
        }
        p.movement.packet_on_ground = on_ground;

        if let Some(to) = pos {
            let ground_spoofed = self.on_position(p, to, on_ground, world, now);
            if ground_spoofed && modify {
                verdict = Verdict::SetOnGround(!on_ground);
            }
        }
        if nofall && modify && verdict == Verdict::Keep {
            verdict = Verdict::SetOnGround(false);
        }
        p.movement.did_last_last_movement_include_position =
            p.movement.did_last_movement_include_position;
        p.movement.did_last_movement_include_position = pos.is_some();
        p.movement.did_send_movement_before_tick_end = true;
        self.tick_reach(p);
        verdict
    }

    fn rotation_checks(
        &mut self,
        p: &mut PlayerData,
        old_yaw: f32,
        old_pitch: f32,
        yaw: f32,
        pitch: f32,
    ) {
        let delta_yaw = yaw - old_yaw;
        if p.packets.aim_exempt {
            p.packets.aim_exempt = false;
            p.packets.last_delta_yaw = delta_yaw;
            return;
        }
        // AimModulo360.
        if yaw < 360.0
            && yaw > -360.0
            && delta_yaw.abs() > 320.0
            && p.packets.last_delta_yaw.abs() < 30.0
        {
            self.flag(p, CheckId::AimModulo360, format!("delta={delta_yaw:.1}"));
        } else {
            p.violations.reward(&self.config, CheckId::AimModulo360);
        }
        p.packets.last_delta_yaw = delta_yaw;
        // AimDuplicateLook.
        if !p.movement.in_vehicle && old_yaw == yaw && old_pitch == pitch {
            self.flag(p, CheckId::AimDuplicateLook, String::new());
        }
    }

    /// `NoFall.isNearGround`.
    fn near_ground(&self, p: &PlayerData, world: &dyn BlockView) -> bool {
        let m = &p.movement;
        let feet = m
            .box_at(m.x, m.y, m.z, 0.6, 0.001)
            .inflate(MOVEMENT_THRESHOLD);
        let mut boxes = Vec::new();
        world.collision_boxes(feet, &mut boxes);
        boxes.iter().any(|b| feet.collides_vertically(b))
            || p.pending_blocks.touches(feet.inflate(1.0))
    }

    fn timer_tick(&mut self, p: &mut PlayerData, now: u64) {
        let now = now as i64;
        if !p.timer.initialised {
            p.timer.initialised = true;
            p.timer.known_player_clock = now - 60_000_000_000;
            p.timer.last_movement_player_clock = now - 60_000_000_000;
        }
        p.timer.has_movement_after_transaction = true;
        p.timer.balance += 50_000_000;
        if p.timer.balance > now {
            if !self.checks_off(p) && p.spawned {
                self.flag_with_setback(p, CheckId::Timer, String::new());
            }
            p.timer.balance -= 50_000_000;
        }
        let drift = (self.config.timer.drift_ms * 1e6) as i64;
        p.timer.balance = p
            .timer
            .balance
            .max(p.timer.last_movement_player_clock - drift);
    }

    fn on_tick_end(&mut self, p: &mut PlayerData, world: &dyn BlockView, now: u64) {
        p.packets.received_tick_end = true;
        if p.packets.flying_packets > 1 {
            let n = p.packets.flying_packets;
            self.flag(p, CheckId::TickTimer, format!("type=end, packets={n}"));
        }
        p.packets.flying_packets = 0;
        if !p.movement.did_send_movement_before_tick_end {
            // A tick with no movement packet counts for the timer, and the
            // client updated its pose without telling us.
            self.timer_tick(p, now);
            let m = &mut p.movement;
            m.did_last_last_movement_include_position = m.did_last_movement_include_position;
            m.did_last_movement_include_position = false;
            if !m.in_vehicle && p.spawned {
                prediction::idle_tick(m, world);
            }
        }
        p.movement.did_send_movement_before_tick_end = false;
        self.tick_reach(p);
    }

    /// One position: the prediction and the checks that follow it. Returns
    /// whether the client's ground flag was found false.
    fn on_position(
        &mut self,
        p: &mut PlayerData,
        to: Vec3,
        on_ground: bool,
        world: &dyn BlockView,
        now: u64,
    ) -> bool {
        let _ = now;
        let m = &mut p.movement;
        (m.last_x, m.last_y, m.last_z) = (m.x, m.y, m.z);
        let to = Vec3::new(
            to.x.clamp(-3.0e7, 3.0e7),
            to.y.clamp(-2.0e7, 2.0e7),
            to.z.clamp(-3.0e7, 3.0e7),
        );
        (m.x, m.y, m.z) = (to.x, to.y, to.z);

        // Knockback and explosions the client must or may have applied.
        let tx = p.tx.last_received;
        if let Some(k) = p.kb.calculate_first_bread(tx) {
            m.first_bread_kb = Some(k);
        }
        if let Some(k) = p.kb.calculate_required(tx, false) {
            m.likely_kb = Some(k);
        }
        if let Some(e) = p.explosion.first_bread_added(tx) {
            m.first_bread_explosion = Some(e);
        }
        if let Some(e) = p.explosion.possible(tx, false) {
            m.likely_explosion = Some(e);
        }

        // Decline to judge what we cannot know.
        let area = m
            .pose_box_at(m.last_x, m.last_y, m.last_z)
            .union(m.pose_box_at(m.x, m.y, m.z))
            .inflate(1.0);
        let (x0, z0, x1, z1) = (
            area.min_x.floor() as i32,
            area.min_z.floor() as i32,
            area.max_x.floor() as i32,
            area.max_z.floor() as i32,
        );
        let unknown_world = !world.are_chunks_loaded(x0, z0, x1, z1);
        let unconfirmed_blocks = p.pending_blocks.touches(area);
        let awaiting_setback = p.awaiting_setback.is_some();
        let off = self.checks_off(p);

        let disallow_stepping = p.setback_ticks_complete == 1;
        let mut kb = std::mem::take(&mut p.kb);
        let mut ex = std::mem::take(&mut p.explosion);
        let start = std::time::Instant::now();
        let mut result = {
            let mut cx = self.context(world, &mut kb, &mut ex, disallow_stepping);
            let update = PositionUpdate {
                to,
                on_ground,
                is_teleport: false,
                teleport_velocity: None,
            };
            prediction::check(&mut p.movement, &mut cx, update)
        };
        self.stats.prediction_nanos += start.elapsed().as_nanos() as u64;
        self.stats.predictions += 1;

        let skip = if unknown_world {
            Some("unloaded chunks")
        } else if unconfirmed_blocks {
            Some("unconfirmed block changes")
        } else if awaiting_setback {
            Some("awaiting setback")
        } else if off {
            Some("exempt")
        } else {
            None
        };
        if let Some(reason) = skip {
            result = PredictionComplete {
                checked: false,
                skipped: Some(reason),
                offset: 0.0,
                ..result
            };
            KnockbackHandler::force_exempt(&mut p.movement);
            ExplosionHandler::force_exempt(&mut p.movement);
        }

        p.predictions += 1;
        p.last_offset = result.offset;
        p.last_skip_reason = result.skipped;
        if !result.checked {
            p.skipped += 1;
        }
        self.debug_line(p, &result);

        let mut ground_flag = false;
        // Simulation (OffsetHandler).
        if result.checked {
            let s = self.config.simulation.clone();
            let offset = result.offset;
            if offset >= s.threshold || offset >= s.immediate_setback_threshold {
                p.advantage_gained += offset;
                let flagged = self.flag(p, CheckId::Simulation, format_offset(offset));
                if flagged
                    && (p.advantage_gained >= s.max_advantage
                        || offset >= s.immediate_setback_threshold)
                    && p.violations.vl(CheckId::Simulation) >= s.setback_violation_threshold
                {
                    self.request_setback(p);
                }
                p.advantage_gained = p.advantage_gained.min(s.max_ceiling);
            } else {
                p.advantage_gained *= s.setback_decay_multiplier;
                p.violations.reward(&self.config, CheckId::Simulation);
            }
            // GroundSpoof.
            let m = &p.movement;
            if m.game_mode != GameMode::Spectator && !m.exempt_on_ground() {
                let claimed = m.client_claims_last_on_ground;
                if claimed != m.on_ground {
                    self.flag_with_setback(p, CheckId::GroundSpoof, format!("claimed {claimed}"));
                    ground_flag = true;
                } else {
                    p.violations.reward(&self.config, CheckId::GroundSpoof);
                }
            }
        }
        // AntiKB / AntiExplosion.
        let cfg = self.config.velocity();
        match kb.on_prediction_complete(
            &mut p.movement,
            result.offset,
            result.checked,
            result.teleport,
            &cfg,
        ) {
            VelocityVerdict::Flag {
                offset,
                ignored,
                wants_setback,
            } => {
                let text = if ignored {
                    "ignored knockback".to_owned()
                } else {
                    format!("o: {}", format_offset(offset))
                };
                if self.flag(p, CheckId::AntiKB, text) && wants_setback {
                    self.setback_if_above_vl(p, CheckId::AntiKB);
                }
            }
            VelocityVerdict::Taken => p.violations.reward(&self.config, CheckId::AntiKB),
            VelocityVerdict::None => {}
        }
        if result.checked {
            match ex.on_prediction_complete(
                &mut p.movement,
                result.offset,
                self.config.explosion.threshold,
            ) {
                VelocityVerdict::Flag {
                    offset, ignored, ..
                } => {
                    let text = if ignored {
                        "ignored explosion".to_owned()
                    } else {
                        format!("o: {}", format_offset(offset))
                    };
                    self.flag_with_setback(p, CheckId::AntiExplosion, text);
                }
                VelocityVerdict::Taken => p.violations.reward(&self.config, CheckId::AntiExplosion),
                VelocityVerdict::None => {}
            }
        } else {
            ExplosionHandler::force_exempt(&mut p.movement);
        }
        p.kb = kb;
        p.explosion = ex;

        // SetbackTeleportUtil: remember where the player last moved legitimately.
        if result.checked && result.offset < self.config.simulation.threshold && !ground_flag {
            p.last_good = Some(to);
        }
        if p.setback_ticks_complete > 0 {
            p.setback_ticks_complete += 1;
        }
        prediction::finish(&mut p.movement, result.offset, result.checked);
        ground_flag
    }

    fn debug_line(&mut self, p: &PlayerData, r: &PredictionComplete) {
        let watchers: Vec<PlayerKey> = self
            .players
            .values()
            .filter(|s| s.debug_targets.contains(&p.key))
            .map(|s| s.key)
            .collect();
        if watchers.is_empty() {
            return;
        }
        let m = &p.movement;
        let text = match r.skipped {
            Some(why) => format!("[Grim debug] {}: not checked ({why})", p.name),
            None => format!(
                "[Grim debug] {}: offset {} predicted ({:.5}, {:.5}, {:.5}) actual ({:.5}, {:.5}, {:.5}) ground {}",
                p.name,
                format_offset(r.offset),
                m.predicted.vector.x,
                m.predicted.vector.y,
                m.predicted.vector.z,
                m.actual_movement.x,
                m.actual_movement.y,
                m.actual_movement.z,
                m.on_ground
            ),
        };
        for key in watchers {
            self.actions.push(Action::Message {
                key,
                text: text.clone(),
            });
        }
    }

    fn on_attack(&mut self, p: &mut PlayerData, entity: i32, modify: bool) -> Verdict {
        if entity == p.entity_id && p.entity_id >= 0 {
            self.flag(p, CheckId::SelfInteract, String::new());
            return if modify {
                Verdict::Cancel
            } else {
                Verdict::Keep
            };
        }
        // PacketOrderB: 1.9+ clients swing after the attack, and swing at
        // least once between attacks.
        if p.movement.game_mode != GameMode::Spectator {
            if !p.packets.swung_since_attack {
                self.flag(p, CheckId::PacketOrderB, "pre-attack".into());
            }
            p.packets.attacked = true;
            p.packets.swung_since_attack = false;
        }
        // Attack slowdown possibilities for the next prediction.
        p.movement.max_attack_slow += 1;
        if p.attacks.len() < 10 {
            let acked = p.tx.acked_tick;
            let interp = u64::from(self.config.reach.interpolation_ticks) + 1;
            p.attacks.push(QueuedAttack {
                entity,
                pos: p.movement.position(),
                from_tick: acked.saturating_sub(interp),
                to_tick: self.tick,
            });
        }
        Verdict::Keep
    }

    /// `Reach.tickBetterReachCheckWithAngle`: judges queued attacks with the
    /// look the client had when it attacked (this tick's rotation, or the
    /// last one).
    fn tick_reach(&mut self, p: &mut PlayerData) {
        if p.attacks.is_empty() {
            return;
        }
        let attacks = std::mem::take(&mut p.attacks);
        let m = &p.movement;
        if m.game_mode == GameMode::Creative
            || m.game_mode == GameMode::Spectator
            || m.in_vehicle
            || self.checks_off(p)
        {
            return;
        }
        let Some(history) = self.entities.get(&p.dimension) else {
            return;
        };
        let base_reach = m.attr(Attr::EntityInteractionRange);
        let mut margin = self.config.reach.threshold;
        if !m.did_last_last_movement_include_position {
            margin += MOVEMENT_THRESHOLD;
        }
        let looks = [
            look_vector(m.yaw, m.pitch),
            look_vector(m.last_yaw, m.pitch),
            look_vector(m.last_yaw, m.last_pitch),
        ];
        let eyes = m.possible_eye_heights();
        let mut results = Vec::new();
        for a in attacks {
            if !history.covers(a.from_tick) {
                continue;
            }
            let Some((target, snapshot)) = history.possible_box(a.entity, a.from_tick, a.to_tick)
            else {
                continue;
            };
            if !snapshot.checkable || snapshot.riding {
                continue;
            }
            let target = target.inflate(margin);
            let max_reach = base_reach;
            let distance = max_reach + 3.0;
            let mut min_distance = f64::MAX;
            'outer: for look in looks {
                let look = look.scale(distance);
                for eye in eyes {
                    let eye_pos = Vec3::new(a.pos.x, a.pos.y + eye, a.pos.z);
                    if target.contains(eye_pos) {
                        min_distance = 0.0;
                        break 'outer;
                    }
                    if let Some(hit) = target.intercept(eye_pos, eye_pos.add(look)) {
                        min_distance = min_distance.min(eye_pos.distance(hit));
                    }
                }
            }
            results.push((a.entity, min_distance, max_reach));
        }
        for (entity, min_distance, max_reach) in results {
            if min_distance == f64::MAX {
                self.flag(p, CheckId::Hitboxes, format!("entity={entity}"));
            } else if min_distance > max_reach {
                self.flag(
                    p,
                    CheckId::Reach,
                    format!("{min_distance:.5} blocks, entity={entity}"),
                );
            } else {
                p.violations.reward(&self.config, CheckId::Reach);
            }
        }
    }

    fn request_setback(&mut self, p: &mut PlayerData) {
        if !self.config.setbacks_enabled() || self.checks_off(p) || !p.spawned {
            return;
        }
        if p.awaiting_setback.is_some() {
            return;
        }
        let Some(pos) = p.last_good.or(Some(p.movement.last_position())) else {
            return;
        };
        p.awaiting_setback = Some(pos);
        p.setback_ticks_complete = 1;
        self.actions.push(Action::Setback { key: p.key, pos });
    }

    fn setback_if_above_vl(&mut self, p: &mut PlayerData, c: CheckId) {
        let info = c.info();
        let cc = self.config.check(info.config_name);
        let level = match cc.setback_vl {
            Some(v) if v < 0.0 => f64::MAX,
            Some(v) => v,
            None => info.setback_vl,
        };
        if p.violations.vl(c) > level {
            self.request_setback(p);
        }
    }

    fn flag_with_setback(&mut self, p: &mut PlayerData, c: CheckId, verbose: String) -> bool {
        if self.flag(p, c, verbose) {
            self.setback_if_above_vl(p, c);
            return true;
        }
        false
    }

    /// Records a violation and produces the alerts. False when the check is
    /// disabled or the player exempt.
    fn flag(&mut self, p: &mut PlayerData, c: CheckId, verbose: String) -> bool {
        if self.checks_off(p) {
            return false;
        }
        let Some(flag) = p.violations.flag(&self.config, c, verbose) else {
            return false;
        };
        self.alert(p, &flag);
        if flag.kick {
            self.actions.push(Action::Kick {
                key: p.key,
                reason: format!("Unfair advantage ({})", c.info().name),
            });
            self.actions.push(Action::Log(format!(
                "[Grim] Kicked {} for {} (x{:.0})",
                p.name,
                c.info().name,
                flag.vl
            )));
        }
        true
    }

    fn alert(&mut self, p: &PlayerData, flag: &Flag) {
        let info = flag.check.info();
        let text = self
            .config
            .alerts
            .format
            .replace("%player%", &p.name)
            .replace("%check%", info.name)
            .replace("%vl%", &format!("{:.0}", flag.vl.floor()))
            .replace("%verbose%", &flag.verbose)
            .replace("%description%", info.description);
        let text = text.trim_end().to_owned();
        let level = self.config.permission_level;
        let mut recipients = Vec::new();
        for staff in self.players.values() {
            if staff.permission < level {
                continue;
            }
            if (flag.alert && staff.alerts) || staff.verbose {
                recipients.push(staff.key);
            }
        }
        // The flagged player is outside the map while it is being handled.
        if p.permission >= level && ((flag.alert && p.alerts) || p.verbose) {
            recipients.push(p.key);
        }
        for key in recipients {
            self.actions.push(Action::Message {
                key,
                text: text.clone(),
            });
        }
        if flag.alert && self.config.alerts.print_to_console {
            self.actions.push(Action::Log(text));
        }
    }

    /// The prediction offset the last movement of `key` had.
    pub fn last_offset(&self, key: PlayerKey) -> Option<f64> {
        self.players.get(&key).map(|p| p.last_offset)
    }
}

fn face_offset(face: i32) -> [i32; 3] {
    match face {
        0 => [0, -1, 0],
        1 => [0, 1, 0],
        2 => [0, 0, -1],
        3 => [0, 0, 1],
        4 => [-1, 0, 0],
        5 => [1, 0, 0],
        _ => [0, 0, 0],
    }
}

/// `Check.formatOffset`.
pub fn format_offset(offset: f64) -> String {
    if offset > 0.001 {
        format!("{offset:.5}")
    } else {
        format!("{offset:.2e}")
    }
}

/// `Mth.wrapDegrees` re-export for adapters.
pub fn wrap(d: f32) -> f32 {
    wrap_degrees(d)
}

/// An empty box, for adapters building entity snapshots.
pub fn empty_box() -> Aabb {
    Aabb::default()
}
