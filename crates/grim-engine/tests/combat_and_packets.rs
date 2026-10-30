// SPDX-License-Identifier: GPL-3.0-only
// Reach, Hitboxes and the packet checks, driven through the engine API.

mod common;

use common::{Harness, KEY, Keys, flat_world, stone};
use grim_horizon_engine::engine::Serverbound;
use grim_horizon_engine::entities::EntitySnapshot;
use grim_horizon_engine::math::Vec3;
use grim_horizon_engine::{CheckId, Clientbound};

const SPAWN: Vec3 = Vec3::new(0.5, 64.0, 0.5);

fn zombie_at(x: f64, z: f64) -> EntitySnapshot {
    EntitySnapshot {
        id: 500,
        pos: Vec3::new(x, 64.0, z),
        width: 0.6,
        height: 1.95,
        checkable: true,
        riding: false,
    }
}

/// Settles the player, keeping a zombie recorded every tick.
fn settle(h: &mut Harness, zombie: EntitySnapshot) {
    for _ in 0..20 {
        h.engine.record_entities("minecraft:overworld", [zombie]);
        h.tick(Keys::default());
    }
}

fn attack(h: &mut Harness, zombie: EntitySnapshot) {
    h.engine.record_entities("minecraft:overworld", [zombie]);
    // 1.9+ order: attack, then the swing, then the movement of the tick.
    h.send(vec![
        Serverbound::Attack { entity: 500 },
        Serverbound::Swing { main_hand: true },
    ]);
    h.tick(Keys::default());
    h.engine.record_entities("minecraft:overworld", [zombie]);
    h.tick(Keys::default());
}

#[test]
fn legit_hit_in_front_is_clean() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    // Looking south (+z); the zombie's near face is 2.2 blocks from the eyes.
    let z = zombie_at(0.5, 3.0);
    settle(&mut h, z);
    attack(&mut h, z);
    assert_eq!(h.flagged(CheckId::Reach), 0, "{:?}", h.flags);
    assert_eq!(h.flagged(CheckId::Hitboxes), 0, "{:?}", h.flags);
}

#[test]
fn reach_beyond_three_blocks_flags() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    // Near face 4.2 blocks away: vanilla's 3-block entity reach is exceeded.
    let z = zombie_at(0.5, 5.0);
    settle(&mut h, z);
    attack(&mut h, z);
    assert_eq!(h.flagged(CheckId::Reach), 1, "{:?}", h.flags);
}

#[test]
fn hit_behind_the_player_flags_hitboxes() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    // In range, but behind the player while they look south.
    let z = zombie_at(0.5, -1.5);
    settle(&mut h, z);
    attack(&mut h, z);
    assert_eq!(h.flagged(CheckId::Hitboxes), 1, "{:?}", h.flags);
}

#[test]
fn moving_target_under_latency_is_clean() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    h.latency_ticks = 6;
    // The zombie walks away; the client still sees it where it was 6 ticks ago.
    for t in 0..30 {
        h.engine.record_entities(
            "minecraft:overworld",
            [zombie_at(0.5, 2.5 + f64::from(t) * 0.1)],
        );
        h.tick(Keys::default());
    }
    // Even the oldest position the client may still be showing is out of
    // reach: this flags despite the latency.
    attack(&mut h, zombie_at(0.5, 5.5));
    assert_eq!(h.flagged(CheckId::Reach), 1, "{:?}", h.flags);

    // The same chase where the client hit while the zombie was 2.5 blocks
    // away, five ticks before the server moved it out: no flag.
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    h.latency_ticks = 6;
    for t in 0..30 {
        let z = if t < 25 {
            2.6
        } else {
            2.6 + f64::from(t - 24) * 0.5
        };
        h.engine
            .record_entities("minecraft:overworld", [zombie_at(0.5, z)]);
        h.tick(Keys::default());
    }
    h.send(vec![
        Serverbound::Attack { entity: 500 },
        Serverbound::Swing { main_hand: true },
    ]);
    h.tick(Keys::default());
    assert_eq!(h.flagged(CheckId::Reach), 0, "{:?}", h.flags);
}

#[test]
fn self_interact_flags() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    h.send(vec![Serverbound::Attack { entity: 42 }]);
    assert_eq!(h.flagged(CheckId::SelfInteract), 1);
}

#[test]
fn attack_without_swing_flags_packet_order() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    let z = zombie_at(0.5, 3.0);
    settle(&mut h, z);
    h.engine.record_entities("minecraft:overworld", [z]);
    h.send(vec![Serverbound::Attack { entity: 500 }]);
    h.tick(Keys::default());
    assert!(h.flagged(CheckId::PacketOrderB) >= 1, "{:?}", h.flags);
}

#[test]
fn slot_checks() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    h.send(vec![
        Serverbound::HeldSlot { slot: 3 },
        Serverbound::HeldSlot { slot: 3 },
        Serverbound::HeldSlot { slot: 12 },
    ]);
    assert_eq!(h.flagged(CheckId::BadPacketsA), 1);
    assert_eq!(h.flagged(CheckId::BadPacketsY), 1);
}

#[test]
fn invalid_pitch_and_nan_flag() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    h.send(vec![
        Serverbound::Move {
            pos: None,
            rot: Some((0.0, 95.0)),
            on_ground: true,
            horizontal_collision: false,
        },
        Serverbound::TickEnd,
        Serverbound::Move {
            pos: Some(Vec3::new(f64::NAN, 64.0, 0.5)),
            rot: None,
            on_ground: true,
            horizontal_collision: false,
        },
        Serverbound::TickEnd,
    ]);
    assert_eq!(h.flagged(CheckId::BadPacketsD), 1);
    assert_eq!(h.flagged(CheckId::CrashC), 1);
}

#[test]
fn flying_without_permission_flags() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    h.send(vec![Serverbound::Abilities { flying: true }]);
    assert_eq!(h.flagged(CheckId::BadPacketsI), 1);
    // A creative player may fly.
    h.engine.on_outgoing(
        KEY,
        Clientbound::Abilities {
            flying: false,
            may_fly: true,
            fly_speed: 0.05,
        },
    );
    h.send(vec![Serverbound::Abilities { flying: true }]);
    assert_eq!(h.flagged(CheckId::BadPacketsI), 1);
}

#[test]
fn duplicate_sprint_flags() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    // The first duplicate is exempt (the client may start either way).
    h.send(vec![
        Serverbound::PlayerCommand { action: 2 },
        Serverbound::PlayerCommand { action: 2 },
    ]);
    h.send(vec![
        Serverbound::PlayerCommand { action: 1 },
        Serverbound::PlayerCommand { action: 1 },
    ]);
    assert_eq!(h.flagged(CheckId::BadPacketsF), 1, "{:?}", h.flags);
}

#[test]
fn missing_tick_end_flags_tick_timer() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    h.tick(Keys::default());
    h.send(vec![
        Serverbound::Move {
            pos: None,
            rot: Some((10.0, 0.0)),
            on_ground: true,
            horizontal_collision: false,
        },
        Serverbound::Move {
            pos: None,
            rot: Some((20.0, 0.0)),
            on_ground: true,
            horizontal_collision: false,
        },
        Serverbound::TickEnd,
    ]);
    assert!(h.flagged(CheckId::TickTimer) >= 1, "{:?}", h.flags);
}

#[test]
fn unknown_transaction_flags_order() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    h.send(vec![Serverbound::Pong {
        id: grim_horizon_engine::PING_MARK | 99_999,
    }]);
    assert_eq!(h.flagged(CheckId::TransactionOrder), 1);
    // Pongs for pings that are not ours are ignored.
    h.send(vec![Serverbound::Pong { id: 5 }]);
    assert_eq!(h.flagged(CheckId::TransactionOrder), 1);
}

#[test]
fn unknown_teleport_accept_flags() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    h.send(vec![Serverbound::AcceptTeleport {
        id: 77,
        pos: SPAWN,
        rot: (0.0, 0.0),
    }]);
    assert_eq!(h.flagged(CheckId::BadPacketsN), 1);
}
