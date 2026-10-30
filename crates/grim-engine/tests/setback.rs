// SPDX-License-Identifier: GPL-3.0-only
// Setback lifecycle (SetbackTeleportUtil): a setback must end when the
// client accepts the newest server teleport, however it was superseded, and
// the first movement after it may not step up.

mod common;

use common::{Harness, KEY, Keys, flat_world, stone};
use grim_horizon_engine::config::Mode;
use grim_horizon_engine::engine::{Clientbound, Serverbound};
use grim_horizon_engine::math::{Aabb, Vec3};
use grim_horizon_engine::{Action, CheckId};

const SPAWN: Vec3 = Vec3::new(0.5, 64.0, 0.5);

fn setback_harness() -> Harness {
    Harness::new(flat_world(stone()), SPAWN).with_config(|c| c.mode = Mode::Setback)
}

/// Speeds until the engine asks for a setback; returns where to.
fn cheat_until_setback(h: &mut Harness) -> Vec3 {
    for _ in 0..40 {
        h.client_tick(Keys::forward(), |c| c.pos.z += 0.3);
        if let Some(pos) = h.actions.iter().find_map(|a| match a {
            Action::Setback { key: KEY, pos } => Some(*pos),
            _ => None,
        }) {
            return pos;
        }
    }
    panic!("no setback: {:?}", h.flags);
}

/// The server sends a teleport; the client has not answered yet.
fn send_teleport(h: &mut Harness, pos: Vec3) -> i32 {
    h.teleport_id += 1;
    let id = h.teleport_id;
    h.engine.on_outgoing(
        KEY,
        Clientbound::Teleport {
            id,
            pos,
            delta: Vec3::ZERO,
            relatives: 0,
        },
    );
    id
}

fn accept(h: &mut Harness, id: i32, pos: Vec3) {
    h.client.pos = pos;
    h.client.vel = Vec3::ZERO;
    let rot = (h.client.yaw, h.client.pitch);
    h.send(vec![Serverbound::AcceptTeleport { id, pos, rot }]);
}

/// Movement is judged again: legitimate walking is checked, not skipped,
/// and a speed hack flags.
fn assert_judged(h: &mut Harness) {
    let (_, skipped) = h.checked_predictions();
    for _ in 0..10 {
        h.tick(Keys::forward());
    }
    assert_eq!(h.checked_predictions().1, skipped, "movement still skipped");
    let before = h.flagged(CheckId::Simulation);
    for _ in 0..5 {
        h.client_tick(Keys::forward(), |c| c.pos.z += 0.3);
    }
    assert!(
        h.flagged(CheckId::Simulation) > before,
        "checks did not resume"
    );
}

#[test]
fn accepted_setback_resumes_checking() {
    let mut h = setback_harness();
    let to = cheat_until_setback(&mut h);
    let id = send_teleport(&mut h, to);
    accept(&mut h, id, to);
    assert_judged(&mut h);
}

#[test]
fn setback_superseded_by_a_later_teleport_resumes_checking() {
    let mut h = setback_harness();
    let to = cheat_until_setback(&mut h);
    send_teleport(&mut h, to);
    // A command teleport right after: the client may skip the setback and
    // accept only the newest (Horizon waits for the newest id only).
    let elsewhere = Vec3::new(10.5, 64.0, 10.5);
    let tp = send_teleport(&mut h, elsewhere);
    accept(&mut h, tp, elsewhere);
    assert_judged(&mut h);
}

#[test]
fn stale_setback_ack_does_not_end_the_setback() {
    let mut h = setback_harness();
    let to = cheat_until_setback(&mut h);
    let setback = send_teleport(&mut h, to);
    let elsewhere = Vec3::new(10.5, 64.0, 10.5);
    let tp = send_teleport(&mut h, elsewhere);
    // The setback is accepted, but a newer teleport is still on its way:
    // moves until it is accepted are not where the server put the player.
    accept(&mut h, setback, to);
    let (_, skipped) = h.checked_predictions();
    // (0.5, not the 0.3 that got the setback: that lands on the position the
    // client last reported, so it would send nothing.)
    h.client_tick(Keys::forward(), |c| c.pos.z += 0.5);
    assert!(
        h.checked_predictions().1 > skipped,
        "judged before the newest teleport"
    );
    accept(&mut h, tp, elsewhere);
    assert_judged(&mut h);
}

#[test]
fn respawn_supersedes_a_setback() {
    let mut h = setback_harness();
    let to = cheat_until_setback(&mut h);
    send_teleport(&mut h, to);
    h.engine.on_outgoing(KEY, Clientbound::Respawn);
    let spawn = Vec3::new(-20.5, 64.0, -20.5);
    let id = send_teleport(&mut h, spawn);
    accept(&mut h, id, spawn);
    assert_judged(&mut h);
}

#[test]
fn setback_that_was_never_sent_does_not_stop_checks_for_good() {
    let mut h = setback_harness();
    let _ = cheat_until_setback(&mut h);
    // The server never sends the teleport. Stand still until it gives up.
    for _ in 0..45 {
        h.tick(Keys::default());
    }
    assert!(
        h.actions
            .iter()
            .any(|a| matches!(a, Action::Log(l) if l.contains("never sent"))),
        "{:?}",
        h.actions
    );
    // Back where the engine last saw the player legitimately is unknown to
    // the client model, so resync it with a plain teleport first.
    h.teleport(SPAWN);
    assert_judged(&mut h);
}

/// A slab at z = 3; the player has stood on the ground long enough for
/// the client and the engine to agree on it.
fn slab_harness() -> Harness {
    let mut w = flat_world(stone());
    for x in -4..=4 {
        w.set_shape(
            x,
            64,
            3,
            stone(),
            &[Aabb::new(0.0, 0.0, 0.0, 1.0, 0.5, 1.0)],
        );
    }
    let mut h = Harness::new(w, SPAWN);
    for _ in 0..25 {
        h.tick(Keys::default());
    }
    assert!(h.client.on_ground);
    h
}

#[test]
fn first_move_after_a_completed_setback_may_not_step() {
    let mut h = slab_harness().with_config(|c| c.mode = Mode::Setback);
    // Last good position 0.01 in front of the slab: the first walking tick
    // (0.02 from rest) runs into it and a vanilla client steps up.
    let spot = Vec3::new(0.5, 64.0, 2.69);
    h.teleport(spot);
    h.client_tick(Keys::default(), |c| c.pos.z -= 0.5);
    let to = h
        .actions
        .iter()
        .find_map(|a| match a {
            Action::Setback { key: KEY, pos } => Some(*pos),
            _ => None,
        })
        .expect("set back");
    assert!(to.distance_squared(spot) < 1e-9, "set back to {to:?}");
    let id = send_teleport(&mut h, to);
    accept(&mut h, id, to);
    h.client.on_ground = true;
    let flags = h.flags.len();
    h.tick(Keys::forward());
    assert!(
        h.client.pos.y > 64.4,
        "the client did not step: {}",
        h.client.pos.y
    );
    let offset: f64 = h.flags[flags..]
        .iter()
        .find(|(c, _)| *c == CheckId::Simulation)
        .map_or(0.0, |(_, v)| v.parse().unwrap());
    // Exactly the step that was refused.
    assert!(
        (offset - 0.5).abs() < 0.01,
        "the step-up restriction did not run: {:?}",
        h.flags
    );
}

#[test]
fn stepping_right_after_a_plain_teleport_is_clean() {
    let mut h = slab_harness();
    h.teleport(Vec3::new(0.5, 64.0, 2.69));
    h.tick(Keys::forward());
    assert!(
        h.client.pos.y > 64.4,
        "the client did not step: {}",
        h.client.pos.y
    );
    assert!(h.flags.is_empty(), "{:?}", h.flags);
}

#[test]
fn stepping_on_later_moves_after_a_teleport_is_clean() {
    let mut h = slab_harness();
    // Further away: the first moves are plain walking, the step comes later.
    h.teleport(Vec3::new(0.5, 64.0, 2.3));
    for _ in 0..8 {
        h.tick(Keys::forward());
    }
    assert!(
        h.client.pos.y > 64.4,
        "the client did not step: {}",
        h.client.pos.y
    );
    assert!(h.flags.is_empty(), "{:?}", h.flags);
}

fn setbacks(h: &Harness) -> usize {
    h.actions
        .iter()
        .filter(|a| matches!(a, Action::Setback { key: KEY, .. }))
        .count()
}

#[test]
fn jumping_right_after_a_setback_is_resynced() {
    let mut h = setback_harness();
    for _ in 0..5 {
        h.tick(Keys::default());
    }
    let to = cheat_until_setback(&mut h);
    let id = send_teleport(&mut h, to);
    accept(&mut h, id, to);
    let before = setbacks(&h);
    let flags = h.flags.len();
    // The setback left no velocity into the ground to jump from (Grim's
    // LiquidBounce Jesus / AirJump fix): put back again, without a flag.
    h.tick(Keys {
        jump: true,
        ..Keys::default()
    });
    assert_eq!(setbacks(&h), before + 1, "{:?}", h.flags);
    assert_eq!(h.flags.len(), flags, "a resync is not a violation");
}

#[test]
fn jumping_right_after_a_plain_teleport_onto_the_ground_is_not_resynced() {
    let mut h = setback_harness();
    for _ in 0..5 {
        h.tick(Keys::default());
    }
    h.teleport(Vec3::new(5.5, 64.0, 5.5));
    h.tick(Keys {
        jump: true,
        ..Keys::default()
    });
    for _ in 0..10 {
        h.tick(Keys::default());
    }
    assert_eq!(setbacks(&h), 0, "{:?}", h.flags);
    assert!(h.flags.is_empty(), "{:?}", h.flags);
}
