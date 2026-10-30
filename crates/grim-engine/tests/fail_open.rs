// Where the engine declines to judge (unknown blocks, unloaded chunks), it
// must do so for the smallest region and time it can, and say so.

mod common;

use common::{Harness, KEY, Keys, flat_world, stone};
use grim_horizon_engine::engine::Clientbound;
use grim_horizon_engine::math::Vec3;
use grim_horizon_engine::{Action, CheckId};

const SPAWN: Vec3 = Vec3::new(0.5, 64.0, 0.5);

fn logs(h: &Harness, text: &str) -> usize {
    h.actions
        .iter()
        .filter(|a| matches!(a, Action::Log(l) if l.contains(text)))
        .count()
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
