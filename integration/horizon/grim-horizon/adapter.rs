// Grim-Horizon adapter: Horizon's play hub and network sender on one side,
// the engine (grim_horizon_engine) on the other. Licensed GPL-3.0-or-later (it
// links the Grim port); see LICENSE in the Grim-Horizon repository.
//
// Threading: everything runs on Horizon's server thread. The engine sits in
// ENGINE behind a mutex; the network sender hook only ever touches OUTBOX, so
// the adapter may call Horizon (teleports, chat, kicks) while it holds ENGINE.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use grim_horizon_engine::engine::{Action, Clientbound as GC, Engine, Serverbound as GS, Verdict};
use grim_horizon_engine::entities::EntitySnapshot;
use grim_horizon_engine::math::{Aabb as GAabb, Vec3 as GVec3};
use grim_horizon_engine::player::{Attr, Effect, Modifier, ModifierOp};
use grim_horizon_engine::recovery::{GlobalOutcome, Recovery, panic_message};
use grim_horizon_engine::routing::SenderRoutes;
use grim_horizon_engine::world::{BlockFacts, BlockKind, BlockView, FluidFacts, FluidKind};
use grim_horizon_engine::{Config, PING_MARK};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_protocol::packets::configuration::Ping;
use horizon_protocol::packets::play::{self, Clientbound as C, Serverbound as S};
use horizon_world::block_collision::BlockCollisionRegistry;
use horizon_world::block_collision_scan::BlockCollisionScan;
use horizon_world::collision_context::CollisionContext;
use horizon_world::fluid::{FluidKind as HFluidKind, FluidState};
use horizon_world::registry::BlockStateId;

use super::{HostEvent, Hub, TickCtx};

pub const CONFIG_FILE: &str = "grim-horizon.toml";

/// What the send hook saw, decoded only as far as needed.
pub enum Outgoing {
    Ping(i32),
    Motion { entity: i32, velocity: [f64; 3] },
    Explode([f64; 3]),
    Teleport { id: i32, pos: [f64; 3], delta: [f64; 3], relatives: i32 },
    Attributes { entity: i32, values: Vec<(i32, f64, Vec<(String, f64, i32)>)> },
    Effect { entity: i32, effect: i32, amplifier: Option<i32> },
    Abilities { flags: i8, fly_speed: f32 },
    GameMode(i32),
    Respawn(i32),
    Login { entity: i32, game_mode: i32 },
    Health(f32),
    /// Positions and the block states they were set to.
    Blocks(Vec<([i32; 3], u32)>),
    /// Block changes in this section dropped because the outbox was full.
    BlocksLost([i32; 3]),
    Passengers { vehicle: i32, passengers: Vec<i32> },
}

static OUTBOX: Mutex<Vec<(Arc<AtomicUsize>, Outgoing)>> = Mutex::new(Vec::new());
static ENGINE: Mutex<Option<Adapter>> = Mutex::new(None);
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The server's entity sizes reached the engine (again after a rebuild).
static SIZES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static DISABLED: OnceLock<bool> = OnceLock::new();

/// What to do after a caught panic (grim_horizon_engine::recovery).
static RECOVERY: std::sync::LazyLock<Mutex<Recovery>> = std::sync::LazyLock::new(Default::default);
static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
/// Lines for online staff, delivered at the end of the tick.
static NOTICES: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn recovery() -> std::sync::MutexGuard<'static, Recovery> {
    RECOVERY.lock().unwrap_or_else(|e| e.into_inner())
}

fn disabled() -> bool {
    recovery().is_off() || *DISABLED.get_or_init(|| std::env::var("GRIM_HORIZON_DISABLE").is_ok_and(|v| v == "1"))
}

fn notice(line: String) {
    log_line(line.clone());
    let mut n = NOTICES.lock().unwrap_or_else(|e| e.into_inner());
    if n.len() < 64 {
        n.push(line);
    }
}

/// Every online operator (at the anti-cheat's permission level) is told.
fn tell_staff(hub: &Hub, line: &str, level: u8) {
    for g in hub.guests.iter().filter(|g| g.permission >= level) {
        g.send(&C::SystemChat(play::SystemChat { content: super::string(line), overlay: false }));
    }
}

/// Runs a hook that touches state shared by every player. After a panic
/// that state is unknown, so it is never used again: the engine is rebuilt
/// (players online stay unchecked until they rejoin, as their state would be
/// incomplete), or, after repeated failures, the anti-cheat turns off.
/// Either way staff are told at once, and reminded while it is off.
fn guarded<R>(fallback: R, f: impl FnOnce() -> R) -> R {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(e) => {
            let what = panic_message(&*e);
            let (outcome, line) = recovery().global_panic(START.elapsed().as_secs(), &what);
            let mut guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
            let online: Vec<u32> = guard.as_ref().map(|a| a.engine.players().map(|p| p.key).collect()).unwrap_or_default();
            *guard = match outcome {
                GlobalOutcome::Rebuild => {
                    SIZES.store(false, std::sync::atomic::Ordering::Relaxed);
                    let mut fresh = Adapter::new();
                    fresh.unchecked.extend(online);
                    Some(fresh)
                }
                GlobalOutcome::Off => None,
            };
            drop(guard);
            OUTBOX.lock().unwrap_or_else(|e| e.into_inner()).clear();
            notice(line);
            fallback
        }
    }
}

pub fn log_line(line: String) {
    let mut log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    if log.len() < 4096 {
        log.push(line);
    }
}

/// The adapter's log lines since the last call (printed by the game's tick hook).
pub fn take_log() -> Vec<String> {
    std::mem::take(&mut *LOG.lock().unwrap_or_else(|e| e.into_inner()))
}

pub struct Adapter {
    engine: Engine,
    start: Instant,
    /// PlayerSender (its `pending` Arc, held so the address stays unique)
    /// -> connection, and the join burst sent before the player was known.
    routes: SenderRoutes<AtomicUsize, Outgoing>,
    /// Inbox entries already given to the engine, by connection.
    seen: HashMap<u32, usize>,
    entity_ids: HashMap<u32, i32>,
    riding: HashMap<u32, Option<i32>>,
    attributes: Vec<Option<Attr>>,
    effects: Vec<Option<Effect>>,
    names_loaded: bool,
    facts: FactsCache,
    /// Guests whose inbox the engine read since the last `inbox_done`.
    processed: Vec<u32>,
    /// Kicked after an internal error while checking them: left alone until
    /// they are gone.
    kicked: HashSet<u32>,
    /// Online when the engine was rebuilt: their state is incomplete, so they
    /// are not checked until they rejoin.
    unchecked: HashSet<u32>,
}

/// Block facts by block state, decoded from names the first time.
#[derive(Default)]
pub struct FactsCache {
    facts: Vec<Option<(BlockFacts, FluidState)>>,
}

impl FactsCache {
    fn get(&mut self, binding: &BlockCollisionRegistry<'_>, id: BlockStateId) -> (BlockFacts, FluidState) {
        let i = id.0 as usize;
        if i >= self.facts.len() {
            self.facts.resize(i + 1, None);
        }
        if let Some(f) = self.facts[i] {
            return f;
        }
        let registry = binding.registry();
        let f = match registry.state(id) {
            Some(state) => {
                let fluid = FluidState::from_block(state).unwrap_or(FluidState::EMPTY);
                let shape = binding.cached_shapes().cached_shape(id).ok().flatten();
                let (full, empty) = match shape {
                    Some(s) => (s.is_block(), s.is_empty()),
                    None => (false, false),
                };
                let drag = state.properties.get("drag").is_some_and(|v| v == "true");
                (block_facts(&state.name, full, empty, drag), fluid)
            }
            None => (BlockFacts::AIR, FluidState::EMPTY),
        };
        self.facts[i] = Some(f);
        f
    }
}

impl Adapter {
    fn new() -> Self {
        let text = match std::fs::read_to_string(CONFIG_FILE) {
            Ok(t) => t,
            Err(_) => {
                let _ = std::fs::write(CONFIG_FILE, grim_horizon_engine::config::DEFAULT_CONFIG);
                grim_horizon_engine::config::DEFAULT_CONFIG.to_owned()
            }
        };
        let config = match Config::parse(&text) {
            Ok(c) => c,
            Err(e) => {
                log_line(format!("[Grim] {CONFIG_FILE} is invalid ({e}); using the defaults"));
                Config::default()
            }
        };
        log_line(format!(
            "[Grim] Grim-Horizon {} enabled in {:?} mode (a GPLv3 Rust port of Grim Anticheat)",
            env!("CARGO_PKG_VERSION"),
            config.mode
        ));
        Self {
            engine: Engine::new(config),
            start: Instant::now(),
            routes: SenderRoutes::default(),
            seen: HashMap::new(),
            entity_ids: HashMap::new(),
            riding: HashMap::new(),
            attributes: Vec::new(),
            effects: Vec::new(),
            names_loaded: false,
            facts: FactsCache::default(),
            processed: Vec::new(),
            kicked: HashSet::new(),
            unchecked: HashSet::new(),
        }
    }

    fn now(&self) -> u64 {
        self.start.elapsed().as_nanos() as u64
    }

    fn load_names(&mut self, hub: &Hub) {
        if self.names_loaded {
            return;
        }
        self.names_loaded = true;
        for id in 0..256 {
            self.attributes.push(hub.registry_name("minecraft:attribute", id).and_then(Attr::from_name));
            self.effects.push(hub.registry_name("minecraft:mob_effect", id).and_then(Effect::from_name));
        }
    }

    /// Everything about a connection, in the engine and here.
    fn forget(&mut self, conn: u32) {
        self.engine.remove_player(conn);
        self.seen.remove(&conn);
        self.entity_ids.remove(&conn);
        self.riding.remove(&conn);
        self.routes.unbind(conn);
    }

    /// Hands what the senders saw to the engine, in send order.
    fn drain_outbox(&mut self) {
        let out = std::mem::take(&mut *OUTBOX.lock().unwrap_or_else(|e| e.into_inner()));
        for (sender, o) in out {
            if let Some((conn, o)) = self.routes.route(&sender, o) {
                self.deliver(conn, o);
            }
        }
    }

    fn deliver(&mut self, conn: u32, o: Outgoing) {
        let me = self.entity_ids.get(&conn).copied().unwrap_or(i32::MIN);
        let v3 = |a: [f64; 3]| GVec3::new(a[0], a[1], a[2]);
        let event = match o {
            Outgoing::Ping(id) => GC::Ping { id },
            Outgoing::Motion { entity, velocity } if entity == me => GC::Velocity(v3(velocity)),
            Outgoing::Motion { .. } => return,
            Outgoing::Explode(k) => GC::Explosion(v3(k)),
            Outgoing::Teleport { id, pos, delta, relatives } => GC::Teleport { id, pos: v3(pos), delta: v3(delta), relatives },
            Outgoing::Attributes { entity, values } if entity == me => {
                for (attr, base, modifiers) in values {
                    let Some(attr) = self.attributes.get(attr as usize).copied().flatten() else { continue };
                    let modifiers = modifiers
                        .into_iter()
                        .map(|(id, amount, op)| Modifier {
                            id,
                            amount,
                            op: match op {
                                0 => ModifierOp::AddValue,
                                1 => ModifierOp::AddMultipliedBase,
                                _ => ModifierOp::AddMultipliedTotal,
                            },
                        })
                        .collect();
                    self.engine.on_outgoing(conn, GC::Attribute { attr, base, modifiers });
                }
                return;
            }
            Outgoing::Attributes { .. } => return,
            Outgoing::Effect { entity, effect, amplifier } if entity == me => {
                let Some(effect) = self.effects.get(effect as usize).copied().flatten() else { return };
                GC::Effect { effect, amplifier }
            }
            Outgoing::Effect { .. } => return,
            Outgoing::Abilities { flags, fly_speed } => GC::Abilities { flying: flags & 2 != 0, may_fly: flags & 4 != 0, fly_speed },
            Outgoing::GameMode(m) => GC::GameMode(m),
            Outgoing::Respawn(m) => {
                self.engine.on_outgoing(conn, GC::Respawn);
                GC::GameMode(m)
            }
            Outgoing::Login { entity, game_mode } => {
                self.entity_ids.insert(conn, entity);
                self.engine.on_outgoing(conn, GC::EntityId(entity));
                GC::GameMode(game_mode)
            }
            Outgoing::Health(h) => GC::Dead(h <= 0.0),
            Outgoing::Blocks(list) => {
                for (pos, state) in list {
                    self.engine.on_outgoing(conn, GC::BlockState(pos, state));
                }
                return;
            }
            Outgoing::BlocksLost(section) => GC::BlocksUnknownIn(section),
            Outgoing::Passengers { vehicle, passengers } => {
                let riding = self.riding.entry(conn).or_insert(None);
                if passengers.contains(&me) {
                    if *riding != Some(vehicle) {
                        *riding = Some(vehicle);
                        GC::Riding(Some(vehicle))
                    } else {
                        return;
                    }
                } else if *riding == Some(vehicle) {
                    *riding = None;
                    GC::Riding(None)
                } else {
                    return;
                }
            }
        };
        self.engine.on_outgoing(conn, event);
    }

    fn handle_actions(&mut self, hub: &mut Hub) {
        for action in self.engine.drain_actions() {
            match action {
                Action::SendPing { key, id } => {
                    if let Some(g) = hub.guest(key) {
                        g.send(&C::Ping(Ping { id }));
                    }
                }
                Action::Setback { key, pos } => {
                    if let Some(i) = hub.guests.iter().position(|g| g.conn == key) {
                        hub.teleport(i, Some([pos.x, pos.y, pos.z]));
                    }
                }
                Action::Kick { key, reason } => hub.kick(key, super::string(&reason)),
                Action::Message { key, text } => {
                    if let Some(g) = hub.guest(key) {
                        g.send(&C::SystemChat(play::SystemChat { content: super::string(&text), overlay: false }));
                    }
                }
                Action::Log(line) => log_line(line),
            }
        }
    }
}

/// Engine facts for a block, from its registry name.
fn block_facts(name: &str, full: bool, empty: bool, drag: bool) -> BlockFacts {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    let air = matches!(n, "air" | "cave_air" | "void_air");
    if air {
        return BlockFacts::AIR;
    }
    let kind = match n {
        "slime_block" => BlockKind::SlimeBlock,
        "honey_block" => BlockKind::HoneyBlock,
        "ice" | "packed_ice" | "frosted_ice" | "blue_ice" => BlockKind::Ice,
        "bubble_column" => BlockKind::BubbleColumn,
        "scaffolding" => BlockKind::Scaffolding,
        "powder_snow" => BlockKind::PowderSnow,
        "cobweb" => BlockKind::Cobweb,
        "sweet_berry_bush" => BlockKind::SweetBerryBush,
        "soul_sand" => BlockKind::SoulSand,
        "water" => BlockKind::Water,
        "lava" => BlockKind::Lava,
        _ if n.ends_with("_bed") => BlockKind::Bed,
        _ if n.ends_with("_fence") || n.ends_with("_wall") || n.ends_with("_fence_gate") => BlockKind::FenceLike,
        _ => BlockKind::Other,
    };
    let friction = match n {
        "ice" | "packed_ice" | "frosted_ice" => 0.98,
        "blue_ice" => 0.989,
        "slime_block" => 0.8,
        _ => 0.6,
    };
    let speed_factor = if matches!(n, "soul_sand" | "honey_block") { 0.4 } else { 1.0 };
    let jump_factor = if n == "honey_block" { 0.5 } else { 1.0 };
    let bounce = match kind {
        BlockKind::SlimeBlock => 1.0,
        BlockKind::Bed if n != "straw_bed" => 0.75,
        _ if n == "shelf_mushroom" => 0.75,
        _ => 0.0,
    };
    let climbable = matches!(
        n,
        "ladder" | "vine" | "scaffolding" | "weeping_vines" | "weeping_vines_plant" | "twisting_vines" | "twisting_vines_plant" | "cave_vines" | "cave_vines_plant"
    );
    let stuck = match kind {
        BlockKind::Cobweb => Some(GVec3::new(0.25, f64::from(0.05_f32), 0.25)),
        BlockKind::SweetBerryBush => Some(GVec3::new(f64::from(0.8_f32), 0.75, f64::from(0.8_f32))),
        BlockKind::PowderSnow => Some(GVec3::new(f64::from(0.9_f32), 1.5, f64::from(0.9_f32))),
        _ => None,
    };
    let blocks_motion = !empty && !matches!(n, "cobweb" | "bamboo_sapling") && kind != BlockKind::Water && kind != BlockKind::Lava;
    let non_suffocating = n.contains("glass") || n.ends_with("leaves") || matches!(n, "beacon" | "barrier" | "spawner" | "slime_block" | "honey_block");
    BlockFacts {
        air: false,
        friction,
        speed_factor,
        jump_factor,
        bounce,
        climbable,
        kind,
        stuck,
        suffocates: full && blocks_motion && !non_suffocating,
        blocks_motion,
        drag,
    }
}

/// The engine's view of a Horizon level.
struct View<'a, 'b, 'r> {
    world: &'a crate::world::World,
    binding: &'a BlockCollisionRegistry<'r>,
    sent_chunks: &'a crate::fast_hash::FxHashSet<(i32, i32)>,
    facts: std::cell::RefCell<&'b mut FactsCache>,
}

impl BlockView for View<'_, '_, '_> {
    fn min_y(&self) -> i32 {
        crate::chunk::MIN_Y
    }
    fn max_y(&self) -> i32 {
        crate::chunk::MIN_Y + crate::chunk::HEIGHT as i32
    }
    fn is_chunk_loaded(&self, cx: i32, cz: i32) -> bool {
        self.world.loaded(cx << 4, cz << 4) && self.sent_chunks.contains(&(cx, cz))
    }
    fn block(&self, x: i32, y: i32, z: i32) -> BlockFacts {
        let id = self.world.block(x, y, z);
        self.facts.borrow_mut().get(self.binding, id).0
    }
    fn fluid(&self, x: i32, y: i32, z: i32) -> FluidFacts {
        let fluid = {
            let id = self.world.block(x, y, z);
            self.facts.borrow_mut().get(self.binding, id).1
        };
        let above = {
            let id = self.world.block(x, y + 1, z);
            self.facts.borrow_mut().get(self.binding, id).1
        };
        fluid_facts(fluid, Some(above))
    }
    fn state_at(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        Some(self.world.block(x, y, z).0)
    }
    fn state_shape(&self, state: u32) -> Option<grim_horizon_engine::world::StateShape> {
        let id = BlockStateId(state);
        let (facts, fluid) = self.facts.borrow_mut().get(self.binding, id);
        // Shapes that depend on where they stand or who looks have no cached
        // shape: unknown, not judged.
        let shape = self.binding.cached_shapes().cached_shape(id).ok()??;
        let boxes = shape
            .boxes()
            .iter()
            .map(|b| {
                let (lo, hi) = (b.min_position(), b.max_position());
                GAabb::new(lo.x, lo.y, lo.z, hi.x, hi.y, hi.z)
            })
            .collect();
        Some(grim_horizon_engine::world::StateShape { facts, fluid: fluid_facts(fluid, None), boxes })
    }
    fn block_collision_boxes(&self, x: i32, y: i32, z: i32, out: &mut Vec<GAabb>) {
        // Scan the cell and its upper neighbour space; keep this block's hits.
        let (fx, fy, fz) = (f64::from(x), f64::from(y), f64::from(z));
        let bounds = Aabb::new(Vec3::new(fx - 0.5, fy - 0.5, fz - 0.5), Vec3::new(fx + 1.5, fy + 2.0, fz + 1.5));
        let mut scene = crate::collisions::SceneCollisions { world: self.world, binding: self.binding, subject: None };
        let context = CollisionContext::position(fy);
        let Ok(scan) = BlockCollisionScan::new(&mut scene, context, bounds, false, 1 << 12) else { return };
        for hit in scan {
            let Ok(hit) = hit else { break };
            if hit.position.x != x || hit.position.y != y || hit.position.z != z {
                continue;
            }
            for b in hit.shape.boxes() {
                let (lo, hi) = (b.min_position(), b.max_position());
                out.push(GAabb::new(lo.x, lo.y, lo.z, hi.x, hi.y, hi.z));
            }
        }
    }
    fn collision_boxes(&self, query: GAabb, out: &mut Vec<GAabb>) {
        let bounds = Aabb::new(
            Vec3::new(query.min_x - 1.0e-6, query.min_y - 1.0e-6, query.min_z - 1.0e-6),
            Vec3::new(query.max_x + 1.0e-6, query.max_y + 1.0e-6, query.max_z + 1.0e-6),
        );
        let mut scene = crate::collisions::SceneCollisions { world: self.world, binding: self.binding, subject: None };
        let context = CollisionContext::position(query.min_y);
        let Ok(scan) = BlockCollisionScan::new(&mut scene, context, bounds, false, 1 << 16) else { return };
        for hit in scan {
            let Ok(hit) = hit else { break };
            for b in hit.shape.boxes() {
                let (lo, hi) = (b.min_position(), b.max_position());
                out.push(GAabb::new(lo.x, lo.y, lo.z, hi.x, hi.y, hi.z));
            }
        }
    }
}

/// A fluid as the engine sees it; `above` (the fluid over it) makes it full
/// height when it is the same fluid.
fn fluid_facts(fluid: FluidState, above: Option<FluidState>) -> FluidFacts {
    let kind = match fluid.kind() {
        HFluidKind::Water | HFluidKind::FlowingWater => FluidKind::Water,
        HFluidKind::Lava | HFluidKind::FlowingLava => FluidKind::Lava,
        HFluidKind::Empty => return FluidFacts::default(),
    };
    let same_above = above.is_some_and(|a| match a.kind() {
        HFluidKind::Water | HFluidKind::FlowingWater => kind == FluidKind::Water,
        HFluidKind::Lava | HFluidKind::FlowingLava => kind == FluidKind::Lava,
        HFluidKind::Empty => false,
    });
    FluidFacts {
        kind,
        amount: fluid.amount(),
        source: fluid.is_source(),
        falling: fluid.falling(),
        height: if same_above { 1.0 } else { fluid.own_height() },
    }
}

/// A world that is not known: every chunk reads as unloaded.
struct Unknown;
impl BlockView for Unknown {
    fn min_y(&self) -> i32 {
        crate::chunk::MIN_Y
    }
    fn max_y(&self) -> i32 {
        crate::chunk::MIN_Y + crate::chunk::HEIGHT as i32
    }
    fn is_chunk_loaded(&self, _: i32, _: i32) -> bool {
        false
    }
    fn block(&self, _: i32, _: i32, _: i32) -> BlockFacts {
        BlockFacts::AIR
    }
    fn fluid(&self, _: i32, _: i32, _: i32) -> FluidFacts {
        FluidFacts::default()
    }
    fn collision_boxes(&self, _: GAabb, _: &mut Vec<GAabb>) {}
    fn block_collision_boxes(&self, _: i32, _: i32, _: i32, _: &mut Vec<GAabb>) {}
}

fn convert(packet: &S) -> GS {
    let v = |x: f64, y: f64, z: f64| GVec3::new(x, y, z);
    match packet {
        S::MovePlayerPos(p) => GS::Move { pos: Some(v(p.x, p.y, p.z)), rot: None, on_ground: p.flags & 1 != 0, horizontal_collision: p.flags & 2 != 0 },
        S::MovePlayerPosRot(p) => GS::Move { pos: Some(v(p.x, p.y, p.z)), rot: Some((p.yaw, p.pitch)), on_ground: p.flags & 1 != 0, horizontal_collision: p.flags & 2 != 0 },
        S::MovePlayerRot(p) => GS::Move { pos: None, rot: Some((p.yaw, p.pitch)), on_ground: p.flags & 1 != 0, horizontal_collision: p.flags & 2 != 0 },
        S::MovePlayerStatus(p) => GS::Move { pos: None, rot: None, on_ground: p.flags & 1 != 0, horizontal_collision: p.flags & 2 != 0 },
        S::AcceptTeleportation(p) => GS::AcceptTeleport { id: p.id, pos: v(p.x, p.y, p.z), rot: (p.yaw, p.pitch) },
        S::ClientTickEnd(_) => GS::TickEnd,
        S::PlayerInput(p) => GS::Input { flags: p.flags },
        S::PlayerCommand(p) => GS::PlayerCommand { action: p.action },
        S::PlayerAbilities(p) => GS::Abilities { flying: p.flags & 2 != 0 },
        S::Attack(p) => GS::Attack { entity: p.id },
        S::Interact(p) => GS::Interact { entity: p.id },
        S::Punch(_) => GS::Swing { main_hand: true },
        S::SetCarriedItem(p) => GS::HeldSlot { slot: i32::from(p.slot) },
        S::Pong(p) => GS::Pong { id: p.id },
        S::UseItemOn(p) => GS::UseItemOn { pos: p.pos, face: p.face },
        S::PlayerAction(p) => GS::Dig { action: p.action, pos: p.pos },
        S::UseItem(_) => GS::UseItem,
        _ => GS::Other,
    }
}

fn grim_command(packet: &S) -> Option<&str> {
    let command = match packet {
        S::ChatCommand(c) => c.command.as_str(),
        S::ChatCommandSigned(c) => c.command.as_str(),
        _ => return None,
    };
    let rest = command.strip_prefix("grim")?;
    (rest.is_empty() || rest.starts_with(' ')).then(|| rest.trim_start())
}

/// `Hub::process_inbox` HEAD: the guest's new packets, in order.
pub fn process_inbox(hub: &mut Hub, i: usize, ctx: &mut TickCtx<'_>, events: &mut Vec<HostEvent>) {
    guarded((), || process_inbox_inner(hub, i, ctx, events))
}

fn process_inbox_inner(hub: &mut Hub, i: usize, ctx: &mut TickCtx<'_>, events: &mut Vec<HostEvent>) {
    let _ = events;
    if disabled() || i >= hub.guests.len() {
        return;
    }
    let mut guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let adapter = guard.get_or_insert_with(Adapter::new);
    adapter.load_names(hub);
    let conn = hub.guests[i].conn;
    if adapter.kicked.contains(&conn) {
        return;
    }
    if adapter.engine.player(conn).is_none() {
        // Everything sent so far, in order, before the join burst.
        adapter.drain_outbox();
        let g = &hub.guests[i];
        adapter.engine.add_player(conn, g.uuid, &g.name, g.permission);
        if adapter.unchecked.contains(&conn) {
            adapter.engine.set_exempt(conn, true);
        }
        adapter.entity_ids.insert(conn, g.entity_id);
        adapter.engine.on_outgoing(conn, GC::EntityId(g.entity_id));
        adapter.engine.on_outgoing(conn, GC::GameMode(g.game_mode));
        adapter.seen.insert(conn, 0);
        for o in adapter.routes.bind(&hub.guests[i].sender.pending, conn) {
            adapter.deliver(conn, o);
        }
    }
    adapter.drain_outbox();
    {
        let g = &hub.guests[i];
        adapter.engine.set_permission(conn, g.permission);
        adapter.engine.set_dimension(conn, &g.dimension);
    }

    // The packets that arrived since the engine last looked.
    let seen = adapter.seen.get(&conn).copied().unwrap_or(0).min(hub.guests[i].inbox.len());
    let mut commands = Vec::new();
    let mut packets = Vec::with_capacity(hub.guests[i].inbox.len() - seen);
    for (k, p) in hub.guests[i].inbox.iter().enumerate().skip(seen) {
        if let Some(args) = grim_command(p) {
            commands.push((k, args.to_owned()));
        }
        packets.push(convert(p));
    }
    let now = adapter.now();
    // This player's packets: a panic here loses only this player's state
    // (the engine holds it outside its map while it works), so only this
    // player is reset, by a kick; nobody else's checks change.
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match ctx.collision {
        Some(binding) => {
            let g = &hub.guests[i];
            let view = View { world: &*ctx.world, binding, sent_chunks: &g.sent_chunks, facts: std::cell::RefCell::new(&mut adapter.facts) };
            adapter.engine.on_incoming(conn, &packets, &view, now)
        }
        None => adapter.engine.on_incoming(conn, &packets, &Unknown, now),
    }));
    let verdicts = match checked {
        Ok(v) => v,
        Err(e) => {
            let line = recovery().player_panic(&hub.guests[i].name, &panic_message(&*e));
            adapter.forget(conn);
            adapter.kicked.insert(conn);
            log_line(line.clone());
            tell_staff(hub, &line, adapter.engine.config.permission_level);
            hub.kick(conn, super::string("Grim-Horizon: internal error, please reconnect"));
            return;
        }
    };

    // Apply the verdicts back to front so indexes stay valid.
    let inbox = &mut hub.guests[i].inbox;
    let mut cancel: HashSet<usize> = commands.iter().map(|(k, _)| *k).collect();
    for (offset, v) in verdicts.iter().enumerate().rev() {
        let k = seen + offset;
        match v {
            Verdict::Keep => {}
            Verdict::Cancel => {
                cancel.insert(k);
            }
            Verdict::SetOnGround(on) => {
                let set = |flags: &mut u8| *flags = (*flags & !1) | u8::from(*on);
                match inbox.get_mut(k) {
                    Some(S::MovePlayerPos(p)) => set(&mut p.flags),
                    Some(S::MovePlayerPosRot(p)) => set(&mut p.flags),
                    Some(S::MovePlayerRot(p)) => set(&mut p.flags),
                    Some(S::MovePlayerStatus(p)) => set(&mut p.flags),
                    _ => {}
                }
            }
        }
    }
    let mut cancelled: Vec<usize> = cancel.into_iter().collect();
    cancelled.sort_unstable_by(|a, b| b.cmp(a));
    for k in cancelled {
        inbox.remove(k);
    }
    adapter.seen.insert(conn, inbox.len());
    adapter.processed.push(conn);

    for (_, args) in commands {
        let outcome = adapter.engine.command(Some(conn), &args, now);
        let mut lines = outcome.lines;
        if outcome.reload {
            lines.push(reload(adapter));
        }
        for line in lines {
            if let Some(g) = hub.guest(conn) {
                g.send(&C::SystemChat(play::SystemChat { content: super::string(&line), overlay: false }));
            }
        }
    }
    adapter.handle_actions(hub);
}

fn reload(adapter: &mut Adapter) -> String {
    match std::fs::read_to_string(CONFIG_FILE) {
        Ok(text) => match adapter.engine.reload(&text) {
            Ok(()) => format!("Reloaded {CONFIG_FILE} ({:?} mode)", adapter.engine.config.mode),
            Err(e) => format!("{CONFIG_FILE} is invalid, keeping the old settings: {e}"),
        },
        Err(e) => format!("Could not read {CONFIG_FILE}: {e}"),
    }
}

/// `Hub::sync_world` HEAD: what is left in the inboxes of the guests read
/// this tick was seen (packets only arrive in `poll`). By connection, not by
/// level: a guest who changed level since is no longer `here`, and a stale
/// count would hide its next packets (its spawn teleport's acceptance) from
/// the engine.
pub fn inbox_done(hub: &Hub) {
    guarded((), || inbox_done_inner(hub))
}

fn inbox_done_inner(hub: &Hub) {
    if disabled() {
        return;
    }
    let mut guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(adapter) = guard.as_mut() else { return };
    for conn in std::mem::take(&mut adapter.processed) {
        if let Some(g) = hub.guests.iter().find(|g| g.conn == conn) {
            adapter.seen.insert(conn, g.inbox.len());
        }
    }
}

/// `Hub::tick_level` HEAD: every entity and player of the level.
pub fn record_level(hub: &Hub, ctx: &TickCtx<'_>) {
    guarded((), || record_level_inner(hub, ctx))
}

fn record_level_inner(hub: &Hub, ctx: &TickCtx<'_>) {
    if disabled() {
        return;
    }
    let mut guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(adapter) = guard.as_mut() else { return };
    let mut list = Vec::with_capacity(ctx.entities.len() + hub.guests.len());
    for e in ctx.entities {
        let Some((w, h)) = adapter.engine.entity_size(&e.kind) else { continue };
        let checkable = !grim_horizon_engine::entities::is_reach_exempt_kind(&e.kind);
        list.push(EntitySnapshot { id: e.id, pos: GVec3::new(e.position[0], e.position[1], e.position[2]), width: w, height: h, checkable, riding: false });
    }
    for g in hub.guests.iter().filter(|g| hub.here(g)) {
        let d = g.dimensions();
        list.push(EntitySnapshot {
            id: g.entity_id,
            pos: GVec3::new(g.position[0], g.position[1], g.position[2]),
            width: d.width,
            height: d.height,
            checkable: !g.dead && !g.spectator(),
            riding: g.vehicle.is_some(),
        });
    }
    adapter.engine.record_entities(&hub.config.dimension, list);
}

/// `Hub::end_tick` HEAD: the end of the server tick.
pub fn end_tick(hub: &mut Hub) {
    guarded((), || end_tick_inner(hub));
    // Failures are told to staff at once (the tick they happen), and the
    // anti-cheat being off is repeated every minute.
    let level = ENGINE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map_or(2, |a| a.engine.config.permission_level);
    let reminder = recovery().reminder(START.elapsed().as_secs());
    let mut lines = std::mem::take(&mut *NOTICES.lock().unwrap_or_else(|e| e.into_inner()));
    if let Some(r) = reminder {
        log_line(r.clone());
        lines.push(r);
    }
    for line in lines {
        tell_staff(hub, &line, level);
    }
}

fn end_tick_inner(hub: &mut Hub) {
    if disabled() {
        return;
    }
    let mut guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(adapter) = guard.as_mut() else { return };
    adapter.drain_outbox();
    // Players who left.
    let online: HashSet<u32> = hub.guests.iter().map(|g| g.conn).collect();
    let gone: Vec<u32> = adapter.engine.players().map(|p| p.key).filter(|k| !online.contains(k)).collect();
    for key in gone {
        adapter.forget(key);
    }
    adapter.kicked.retain(|k| online.contains(k));
    adapter.unchecked.retain(|k| online.contains(k));
    adapter.routes.sweep();
    let now = adapter.now();
    adapter.engine.end_tick(now);
    adapter.handle_actions(hub);
}

/// A `grim ...` line from the server console; `None` for anything else.
pub fn console(line: &str) -> Option<Vec<String>> {
    guarded(None, || console_inner(line))
}

fn console_inner(line: &str) -> Option<Vec<String>> {
    let rest = line.strip_prefix("grim")?;
    if !(rest.is_empty() || rest.starts_with(' ')) {
        return None;
    }
    if recovery().is_off() {
        return Some(vec!["Grim-Horizon is OFF after repeated internal errors (see the log); restart the server.".into()]);
    }
    let mut guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let adapter = guard.get_or_insert_with(Adapter::new);
    let now = adapter.now();
    let outcome = adapter.engine.command(None, rest.trim_start(), now);
    let mut lines = outcome.lines;
    if outcome.reload {
        lines.push(reload(adapter));
    }
    Some(lines)
}

pub fn sizes_registered() -> bool {
    SIZES.load(std::sync::atomic::Ordering::Relaxed) || disabled()
}

/// Entity sizes from the server's own entity table.
pub fn register_sizes(sizes: &[(String, f32, f32)]) {
    guarded((), || register_sizes_inner(sizes))
}

fn register_sizes_inner(sizes: &[(String, f32, f32)]) {
    if disabled() {
        return;
    }
    let mut guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let adapter = guard.get_or_insert_with(Adapter::new);
    for (id, w, h) in sizes {
        adapter.engine.register_entity_size(id, *w, *h);
    }
    SIZES.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// `PlayerSender::send` HEAD: decodes the few packets the engine needs.
pub fn observe_send(sender: &Arc<AtomicUsize>, body: &[u8]) {
    guarded((), || observe_send_inner(sender, body))
}

fn observe_send_inner(sender: &Arc<AtomicUsize>, body: &[u8]) {
    if disabled() {
        return;
    }
    let mut r = horizon_protocol::buf::Reader::new(body);
    let Ok(id) = r.varint() else { return };
    use horizon_protocol::ids::play::clientbound as ids;
    let interesting = [
        ids::PING,
        ids::SET_ENTITY_MOTION,
        ids::EXPLODE,
        ids::PLAYER_POSITION,
        ids::UPDATE_ATTRIBUTES,
        ids::UPDATE_MOB_EFFECT,
        ids::REMOVE_MOB_EFFECT,
        ids::PLAYER_ABILITIES,
        ids::GAME_EVENT,
        ids::RESPAWN,
        ids::LOGIN,
        ids::SET_HEALTH,
        ids::BLOCK_UPDATE,
        ids::SECTION_BLOCKS_UPDATE,
        ids::SET_PASSENGERS,
    ];
    if !interesting.contains(&id) {
        return;
    }
    let Ok(packet) = C::decode(id, r.rest()) else { return };
    let o = match packet {
        C::Ping(p) if p.id & PING_MARK != 0 => Outgoing::Ping(p.id),
        C::SetEntityMotion(p) => Outgoing::Motion { entity: p.id, velocity: p.velocity },
        C::Explode(p) => match p.knockback {
            Some(k) => Outgoing::Explode(k),
            None => return,
        },
        C::PlayerPosition(p) => Outgoing::Teleport { id: p.teleport, pos: p.position, delta: p.delta, relatives: p.relatives },
        C::UpdateAttributes(p) => Outgoing::Attributes {
            entity: p.id,
            values: p
                .values
                .into_iter()
                .map(|v| (v.attribute, v.base, v.modifiers.into_iter().map(|m| (m.id, m.amount, m.operation)).collect()))
                .collect(),
        },
        C::UpdateMobEffect(p) => Outgoing::Effect { entity: p.id, effect: p.effect, amplifier: Some(p.amplifier) },
        C::RemoveMobEffect(p) => Outgoing::Effect { entity: p.id, effect: p.effect, amplifier: None },
        C::PlayerAbilities(p) => Outgoing::Abilities { flags: p.flags, fly_speed: p.flying_speed },
        C::GameEvent(p) if p.event == 3 => Outgoing::GameMode(p.param as i32),
        C::Respawn(p) => Outgoing::Respawn(p.spawn.game_type),
        C::Login(p) => Outgoing::Login { entity: p.player_id, game_mode: p.spawn.game_type },
        C::SetHealth(p) => Outgoing::Health(p.health),
        C::BlockUpdate(p) => Outgoing::Blocks(vec![(p.pos, p.state as u32)]),
        C::SectionBlocksUpdate(p) => {
            let [sx, sy, sz] = p.section;
            Outgoing::Blocks(
                p.changes
                    .iter()
                    .map(|c| ([sx * 16 + ((c >> 8) & 15) as i32, sy * 16 + (c & 15) as i32, sz * 16 + ((c >> 4) & 15) as i32], (c >> 12) as u32))
                    .collect(),
            )
        }
        C::SetPassengers(p) => Outgoing::Passengers { vehicle: p.vehicle, passengers: p.passengers },
        _ => return,
    };
    let mut out = OUTBOX.lock().unwrap_or_else(|e| e.into_inner());
    // Drained every tick; the cap only bounds a flood of block updates.
    // Teleports, pings and the like are never dropped (a lost one leaves the
    // player unjudged), and lost block changes leave their section unknown.
    let o = match o {
        Outgoing::Blocks(list) if out.len() >= 1 << 16 => match list.first() {
            Some((p, _)) => Outgoing::BlocksLost(p.map(|c| c >> 4)),
            None => return,
        },
        o => o,
    };
    out.push((sender.clone(), o));
}
