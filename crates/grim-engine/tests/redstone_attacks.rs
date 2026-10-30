// SPDX-License-Identifier: GPL-3.0-only
// Block changes around a player must not become a way to switch movement
// checks off. Each scenario plays a legitimate client (no alert expected) and
// a cheating one (an alert expected) through the same changing world, and
// measures how long a player goes unjudged.

mod common;

use common::{Client, Harness, KEY, Keys, flat_world, stone};
use grim_horizon_engine::CheckId;
use grim_horizon_engine::engine::{Clientbound, Serverbound};
use grim_horizon_engine::math::{Aabb, Vec3};
use grim_horizon_engine::world::{BlockFacts, BlockKind};

const SPAWN: Vec3 = Vec3::new(0.5, 64.0, 0.5);

struct States {
    stone: u32,
    air: u32,
    slab: u32,
    ice: u32,
}

fn states(h: &mut Harness) -> States {
    let full = Aabb::new(0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
    let ice = BlockFacts {
        friction: 0.98,
        kind: BlockKind::Ice,
        ..stone()
    };
    States {
        stone: h.world.add_state(stone(), &[full]),
        air: h.world.add_state(BlockFacts::AIR, &[]),
        slab: h
            .world
            .add_state(stone(), &[Aabb::new(0.0, 0.0, 0.0, 1.0, 0.5, 1.0)]),
        ice: h.world.add_state(ice, &[full]),
    }
}

/// The server sets a block and sends it.
fn set(h: &mut Harness, pos: [i32; 3], state: u32) {
    h.world.set_state(pos[0], pos[1], pos[2], state);
    h.engine
        .on_outgoing(KEY, Clientbound::BlockState(pos, state));
}

/// Plays `ticks` with `world` changing the blocks each tick; returns the
/// alerts and the longest run of moves that were not judged.
fn play(
    latency: usize,
    ticks: u32,
    keys: Keys,
    cheat: impl Fn(&mut Client),
    world: impl Fn(&mut Harness, &States, u32),
) -> (Vec<CheckId>, u32) {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    let s = states(&mut h);
    h.latency_ticks = latency;
    // Every block the scenario uses was sent once and settled.
    for t in 0..2 {
        world(&mut h, &s, t);
    }
    for _ in 0..10 {
        h.tick(Keys::default());
    }
    for t in 2..ticks {
        world(&mut h, &s, t);
        h.client_tick(keys, &cheat);
        h.client.position_reminder = 19;
    }
    (
        h.flags.iter().map(|(c, _)| *c).collect(),
        h.engine.stats.longest_unjudged,
    )
}

fn legit(_: &mut Client) {}
fn fly(c: &mut Client) {
    c.pos.y += 0.2;
}
fn speed(c: &mut Client) {
    c.pos.z += 0.25;
}
fn jump() -> Keys {
    Keys {
        jump: true,
        ..Keys::default()
    }
}

fn assert_clean((flags, run): (Vec<CheckId>, u32), max_run: u32) {
    assert!(flags.is_empty(), "legitimate movement flagged: {flags:?}");
    assert!(run <= max_run, "unjudged for {run} moves in a row");
}
fn assert_caught((flags, _): (Vec<CheckId>, u32)) {
    assert!(
        flags.contains(&CheckId::Simulation),
        "cheat not caught: {flags:?}"
    );
}

/// A clock: stone and air every 2 ticks.
fn clock(at: [i32; 3]) -> impl Fn(&mut Harness, &States, u32) {
    move |h, s, t| {
        set(
            h,
            at,
            if (t / 2).is_multiple_of(2) {
                s.stone
            } else {
                s.air
            },
        )
    }
}

#[test]
fn clock_beside_the_player() {
    assert_clean(play(1, 200, jump(), legit, clock([1, 64, 0])), 2);
    assert_caught(play(1, 60, Keys::default(), fly, clock([1, 64, 0])));
}

#[test]
fn a_wall_of_clocks_along_the_path() {
    // Forty cells switching together beside a straight walk.
    let wall = |h: &mut Harness, s: &States, t: u32| {
        let state = if (t / 2).is_multiple_of(2) {
            s.stone
        } else {
            s.air
        };
        for z in -2..40 {
            set(h, [2, 64, z], state);
        }
    };
    assert_clean(play(1, 120, Keys::forward(), legit, wall), 2);
    assert_caught(play(1, 60, Keys::forward(), speed, wall));
}

#[test]
fn rapidly_alternating_collision_shapes() {
    // Full, slab, air: a different shape every tick, beside the player.
    let shapes = |h: &mut Harness, s: &States, t: u32| {
        set(h, [1, 64, 0], [s.stone, s.slab, s.air][t as usize % 3]);
        set(h, [-1, 64, 0], [s.air, s.stone, s.slab][t as usize % 3]);
    };
    assert_clean(play(1, 200, jump(), legit, shapes), 2);
    assert_caught(play(1, 60, Keys::default(), fly, shapes));
}

#[test]
fn a_continuously_changing_floor() {
    // The floor under and around the player switches between stone and ice
    // (different friction) every 3 ticks.
    let floor = |h: &mut Harness, s: &States, t: u32| {
        let state = if (t / 3).is_multiple_of(2) {
            s.stone
        } else {
            s.ice
        };
        for x in -3..=3 {
            for z in -3..=40 {
                set(h, [x, 63, z], state);
            }
        }
    };
    assert_clean(play(1, 120, Keys::forward(), legit, floor), 2);
    assert_caught(play(1, 60, Keys::forward(), speed, floor));
}

#[test]
fn block_updates_delayed_by_latency() {
    // A 300 ms round trip: six transactions unanswered at any time.
    assert_clean(play(6, 200, jump(), legit, clock([1, 64, 0])), 2);
    assert_caught(play(6, 60, Keys::default(), fly, clock([1, 64, 0])));
}

#[test]
fn more_worlds_than_the_engine_judges_in() {
    // A 1.5 s round trip with a block changing beside the player every tick:
    // 31 worlds the client may show, over the cap of 16, so its moves near
    // the block go unjudged, for as long as both last. Bounded to the
    // block's surroundings, counted, and reported to staff.
    let every_tick = |h: &mut Harness, s: &States, t: u32| {
        set(
            h,
            [1, 64, 0],
            if t.is_multiple_of(2) { s.stone } else { s.air },
        )
    };
    let (flags, run) = play(30, 300, jump(), legit, every_tick);
    assert!(flags.is_empty(), "{flags:?}");
    assert!(run > 200, "expected unjudged, got a run of {run}");
}

#[test]
fn moving_away_from_the_changing_blocks_is_judged_again() {
    // The same overloaded corner; a speed hack that leaves it is caught.
    let every_tick = |h: &mut Harness, s: &States, t: u32| {
        set(
            h,
            [1, 64, 0],
            if t.is_multiple_of(2) { s.stone } else { s.air },
        )
    };
    assert_caught(play(30, 80, Keys::forward(), speed, every_tick));
}

#[test]
fn repeated_block_update_overflow_far_away_hides_nothing_nearby() {
    // 5,000 changes a tick, 40 blocks away: positions spill to sections,
    // which are far from the player.
    let flood = |h: &mut Harness, _: &States, t: u32| {
        for i in 0..5000 {
            h.engine.on_outgoing(
                KEY,
                Clientbound::BlockChanged([
                    40 + i % 50,
                    60 + (i / 2500),
                    (i / 50) % 50 + (t % 2) as i32,
                ]),
            );
        }
    };
    assert_clean(play(1, 60, Keys::default(), legit, flood), 0);
    assert_caught(play(1, 40, Keys::default(), fly, flood));
}

#[test]
fn spamming_own_dig_and_place_packets_does_not_hide_a_fly_hack() {
    // A cheat client sends place and dig packets for the cells around it
    // every tick, which the server neither applies nor answers.
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..10 {
        h.tick(Keys::default());
    }
    for _ in 0..60 {
        let mut spam = Vec::new();
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let c = h.client.pos;
            let pos = [
                c.x.floor() as i32 + dx,
                c.y.floor() as i32,
                c.z.floor() as i32 + dz,
            ];
            spam.push(Serverbound::UseItemOn { pos, face: 1 });
            spam.push(Serverbound::Dig { action: 0, pos });
        }
        h.send(spam);
        h.client_tick(Keys::default(), fly);
    }
    assert!(h.flagged(CheckId::Simulation) > 0, "{:?}", h.engine.stats);
}

#[test]
fn bridging_with_answered_placements_is_judged_and_clean() {
    // A vanilla client places a block every 4 ticks beside it, on the floor;
    // the server applies it and answers with both cells' states, as vanilla's
    // handleUseItemOn does.
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    let s = states(&mut h);
    for x in -4..=4 {
        for z in -4..=4 {
            h.world.set_state(x, 63, z, s.stone);
        }
    }
    for _ in 0..10 {
        h.tick(Keys::default());
    }
    let skipped = h.engine.stats.skipped_unconfirmed_blocks;
    for t in 0..120 {
        if t % 4 == 0 {
            let c = h.client.pos;
            let (x, z) = (c.x.floor() as i32 + 1, c.z.floor() as i32);
            h.send(vec![Serverbound::UseItemOn {
                pos: [x, 63, z],
                face: 1,
            }]);
            set(&mut h, [x, 63, z], s.stone);
            set(&mut h, [x, 64, z], if t % 8 == 0 { s.stone } else { s.air });
        }
        h.tick(jump());
        h.client.position_reminder = 19;
    }
    assert!(h.flags.is_empty(), "{:?}", h.flags);
    // At most the first placement, whose cell had never been sent: its
    // state before is not known.
    assert!(
        h.engine.stats.skipped_unconfirmed_blocks - skipped <= 1,
        "{:?}",
        h.engine.stats
    );
    assert!(h.engine.stats.judged_with_variants > 0);
}
