// SPDX-License-Identifier: GPL-3.0-only
// Where the engine declines to judge (unknown blocks, unloaded chunks), it
// must do so for the smallest region and time it can, and say so.

mod common;

use common::{Harness, KEY, Keys, flat_world, stone};
use grim_horizon_engine::engine::Clientbound;
use grim_horizon_engine::math::{Aabb, Vec3};
use grim_horizon_engine::world::BlockFacts;
use grim_horizon_engine::{Action, CheckId};

const SPAWN: Vec3 = Vec3::new(0.5, 64.0, 0.5);

fn logs(h: &Harness, text: &str) -> usize {
    h.actions
        .iter()
        .filter(|a| matches!(a, Action::Log(l) if l.contains(text)))
        .count()
}

/// States a test world can switch a block between: (stone, air).
fn states(h: &mut Harness) -> (u32, u32) {
    let full = Aabb::new(0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
    (
        h.world.add_state(stone(), &[full]),
        h.world.add_state(BlockFacts::AIR, &[]),
    )
}

/// The server sets a block and sends it, as Horizon's BlockUpdate does.
fn set(h: &mut Harness, pos: [i32; 3], state: u32) {
    h.world.set_state(pos[0], pos[1], pos[2], state);
    h.engine
        .on_outgoing(KEY, Clientbound::BlockState(pos, state));
}

#[test]
fn a_redstone_clock_beside_a_player_no_longer_hides_cheats() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    let (stone_state, air) = states(&mut h);
    let clock = [1, 64, 0];
    let toggle = |h: &mut Harness, t: u32| {
        set(
            h,
            clock,
            if (t / 2).is_multiple_of(2) {
                stone_state
            } else {
                air
            },
        )
    };
    // Legitimate: jumping in place beside the clock for 10 seconds.
    for t in 0..200 {
        toggle(&mut h, t);
        h.tick(Keys {
            jump: true,
            ..Keys::default()
        });
    }
    assert!(h.flags.is_empty(), "{:?}", h.flags);
    let s = h.engine.stats;
    assert!(s.judged_with_variants > 50, "{s:?}");
    // Only the very first change (whose old state is unknown) went unjudged.
    assert!(s.skipped_unconfirmed_blocks <= 3, "{s:?}");
    // A fly hack beside the clock is caught.
    for t in 200..230 {
        toggle(&mut h, t);
        h.client_tick(Keys::default(), |c| c.pos.y += 0.2);
    }
    assert!(
        h.flagged(CheckId::Simulation) > 0,
        "the clock hid a fly hack"
    );
}

#[test]
fn a_lagging_client_on_a_removed_floor_is_judged_and_clean() {
    let mut w = flat_world(stone());
    w.floor(-64, 64, 62, -64, 64, stone());
    let mut h = Harness::new(w, SPAWN);
    let (stone_state, air) = states(&mut h);
    let under = [0, 63, 0];
    // The block was sent once and settled: its state is known.
    set(&mut h, under, stone_state);
    for _ in 0..5 {
        h.tick(Keys::default());
    }
    // The server removes it; the client gets that 4 ticks later and answers
    // the transaction behind it 2 ticks after that (a 300 ms round trip).
    h.latency_ticks = 6;
    h.client_world = Some(h.world.clone());
    set(&mut h, under, air);
    let skipped = h.engine.stats.skipped_unconfirmed_blocks;
    for _ in 0..4 {
        h.client_tick(Keys::default(), |c| c.position_reminder = 19);
    }
    // Now it does, and falls onto the block below.
    h.client_world = None;
    for _ in 0..20 {
        h.tick(Keys::default());
    }
    assert!(h.client.pos.y < 63.1, "did not fall: {:?}", h.client.pos);
    assert!(h.flags.is_empty(), "{:?}", h.flags);
    assert_eq!(
        h.engine.stats.skipped_unconfirmed_blocks, skipped,
        "judged, not skipped"
    );
}

#[test]
fn a_block_changing_every_tick_next_to_a_player_is_reported() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    // A redstone clock beside the player: a change the client has not
    // confirmed is always pending there, so movement is never judged.
    for _ in 0..230 {
        h.engine
            .on_outgoing(KEY, Clientbound::BlockChanged([1, 64, 0]));
        h.tick(Keys::default());
        h.client.position_reminder = 19;
    }
    let s = h.engine.stats;
    assert!(s.skipped_unconfirmed_blocks >= 200, "{s:?}");
    assert!(s.longest_unjudged >= 200, "{s:?}");
    assert_eq!(logs(&h, "has not been judged"), 1, "once per run");
    // The clock stops: judged again, and a cheat flags.
    for _ in 0..5 {
        h.tick(Keys::default());
    }
    for _ in 0..5 {
        h.client_tick(Keys::forward(), |c| c.pos.z += 0.3);
    }
    assert!(h.flagged(CheckId::Simulation) > 0);
}

#[test]
fn lost_block_changes_only_hide_their_section() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..5 {
        h.tick(Keys::default());
    }
    // Changes far away were lost: the player's own movement is still judged.
    h.engine
        .on_outgoing(KEY, Clientbound::BlocksUnknownIn([20, 4, 20]));
    assert_eq!(h.engine.stats.blocks_lost, 1);
    h.client_tick(Keys::forward(), |c| c.pos.z += 0.3);
    assert!(h.flagged(CheckId::Simulation) > 0, "judged as usual");
}

#[test]
fn a_flood_of_block_changes_is_bounded_and_counted() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    // A TNT crater far from the player, bigger than the per-position cap.
    for i in 0..6000 {
        h.engine.on_outgoing(
            KEY,
            Clientbound::BlockChanged([200 + i % 40, 40 + i / 1600, 200 + (i / 40) % 40]),
        );
    }
    let s = h.engine.stats;
    assert_eq!(s.block_spills, 6000 - 4096);
    assert!(s.peak_pending <= 4096 + 1024);
    // Far from the player: its walking is judged meanwhile.
    for _ in 0..3 {
        h.client_tick(Keys::forward(), |c| c.pos.z += 0.3);
    }
    assert!(h.flagged(CheckId::Simulation) > 0);
}
