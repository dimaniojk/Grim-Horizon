// Movement scenarios: an independent model of the vanilla 26.3 client moves,
// and the engine must predict it without flagging. Cheating variants must flag.

mod common;

use common::{Harness, KEY, Keys, flat_world, stone};
use grim_horizon_engine::CheckId;
use grim_horizon_engine::math::Vec3;
use grim_horizon_engine::world::{BlockFacts, BlockKind};

const SPAWN: Vec3 = Vec3::new(0.5, 64.0, 0.5);

fn assert_clean(h: &Harness) {
    assert!(h.flags.is_empty(), "unexpected flags: {:?}", h.flags);
}

#[test]
fn reference_client_matches_published_vanilla_numbers() {
    // Independent sanity check of the test client itself: vanilla walks at
    // 4.317 m/s and sprints at 5.612 m/s on flat stone.
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..40 {
        h.tick(Keys::forward());
    }
    let before = h.client.pos.z;
    h.tick(Keys::forward());
    let walked = h.client.pos.z - before;
    assert!((walked - 0.21585).abs() < 1e-4, "walk speed {walked}");

    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..40 {
        h.tick(Keys {
            sprint: true,
            ..Keys::forward()
        });
    }
    let before = h.client.pos.z;
    h.tick(Keys {
        sprint: true,
        ..Keys::forward()
    });
    let sprinted = h.client.pos.z - before;
    assert!(
        (sprinted - 0.280617).abs() < 1e-4,
        "sprint speed {sprinted}"
    );
}

#[test]
fn standing_still_is_clean() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..100 {
        h.tick(Keys::default());
    }
    assert_clean(&h);
}

#[test]
fn walking_is_predicted_exactly() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    let mut worst: f64 = 0.0;
    for i in 0..200 {
        h.client.yaw = i as f32 * 3.0 - 180.0;
        h.tick(Keys::forward());
        worst = worst.max(h.last_offset());
    }
    assert_clean(&h);
    assert!(worst < 1e-6, "worst offset {worst}");
    let (predictions, skipped) = h.checked_predictions();
    assert!(
        predictions > 150 && skipped == 0,
        "{predictions} predictions, {skipped} skipped"
    );
}

#[test]
fn diagonal_and_strafing_inputs() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    let patterns = [
        Keys {
            forward: true,
            left: true,
            ..Default::default()
        },
        Keys {
            backward: true,
            right: true,
            ..Default::default()
        },
        Keys {
            left: true,
            ..Default::default()
        },
        Keys {
            backward: true,
            ..Default::default()
        },
    ];
    for i in 0..160 {
        h.tick(patterns[(i / 40) % 4]);
    }
    assert_clean(&h);
}

#[test]
fn sprinting_is_clean() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..100 {
        h.tick(Keys {
            sprint: true,
            ..Keys::forward()
        });
    }
    assert_clean(&h);
}

#[test]
fn jumping_matches_vanilla_and_is_clean() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..5 {
        h.tick(Keys::default());
    }
    let y0 = h.client.pos.y;
    h.tick(Keys {
        jump: true,
        ..Default::default()
    });
    assert!(
        (h.client.pos.y - y0 - 0.42).abs() < 1e-6,
        "first jump tick {}",
        h.client.pos.y - y0
    );
    let mut peak = 0.0_f64;
    for _ in 0..40 {
        h.tick(Keys {
            jump: true,
            ..Default::default()
        });
        peak = peak.max(h.client.pos.y - y0);
    }
    assert!((peak - 1.2522).abs() < 1e-3, "jump height {peak}");
    assert_clean(&h);
}

#[test]
fn sprint_jumping_is_clean() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for i in 0..120 {
        h.client.yaw = (i as f32 * 1.5).sin() * 40.0;
        h.tick(Keys {
            sprint: true,
            jump: true,
            ..Keys::forward()
        });
    }
    assert_clean(&h);
}

#[test]
fn falling_is_clean_and_follows_vanilla_velocities() {
    let mut h = Harness::new(flat_world(stone()), Vec3::new(0.5, 90.0, 0.5));
    let mut ys = Vec::new();
    for _ in 0..4 {
        let y = h.client.pos.y;
        h.tick(Keys::default());
        ys.push(h.client.pos.y - y);
    }
    // Movement each tick: 0, -0.0784, -0.155232, -0.23052736.
    assert!(
        ys[1] + 0.0784 < 1e-6
            && (ys[2] + 0.155232).abs() < 1e-6
            && (ys[3] + 0.23052736).abs() < 1e-6,
        "{ys:?}"
    );
    for _ in 0..60 {
        h.tick(Keys::default());
    }
    assert!(h.client.on_ground);
    assert_clean(&h);
}

#[test]
fn sneaking_at_an_edge_is_clean() {
    let mut w = grim_horizon_engine::world::TestWorld::new();
    w.floor(-4, 4, 63, -4, 0, stone());
    let mut h = Harness::new(w, Vec3::new(0.5, 64.0, -2.5));
    for _ in 0..80 {
        h.tick(Keys {
            sneak: true,
            ..Keys::forward()
        });
    }
    // The player stopped at the edge.
    assert!(h.client.pos.z < 1.35, "fell off: {}", h.client.pos.z);
    assert_clean(&h);
}

#[test]
fn ice_is_clean() {
    let ice = BlockFacts {
        friction: 0.98,
        kind: BlockKind::Ice,
        ..stone()
    };
    let mut h = Harness::new(flat_world(ice), SPAWN);
    for i in 0..120 {
        let keys = if i < 60 {
            Keys {
                sprint: true,
                ..Keys::forward()
            }
        } else {
            Keys::default()
        };
        h.tick(keys);
    }
    assert_clean(&h);
}

#[test]
fn soul_sand_is_clean() {
    let sand = BlockFacts {
        speed_factor: 0.4,
        kind: BlockKind::SoulSand,
        ..stone()
    };
    let mut h = Harness::new(flat_world(sand), SPAWN);
    for _ in 0..80 {
        h.tick(Keys::forward());
    }
    assert_clean(&h);
}

#[test]
fn still_water_is_clean() {
    let mut w = flat_world(stone());
    w.water_pool(-8, 8, 64, 70, -8, 8);
    let mut h = Harness::new(w, Vec3::new(0.5, 66.0, 0.5));
    for i in 0..120 {
        let keys = match i / 30 {
            0 => Keys::default(),
            1 => Keys::forward(),
            2 => Keys {
                jump: true,
                ..Keys::forward()
            },
            _ => Keys {
                left: true,
                ..Default::default()
            },
        };
        h.tick(keys);
    }
    assert_clean(&h);
}

#[test]
fn slime_bounce_is_clean() {
    let slime = BlockFacts {
        friction: 0.8,
        bounce: 1.0,
        kind: BlockKind::SlimeBlock,
        ..stone()
    };
    let mut h = Harness::new(flat_world(slime), Vec3::new(0.5, 70.0, 0.5));
    for _ in 0..80 {
        h.tick(Keys::default());
    }
    assert_clean(&h);
}

#[test]
fn stepping_up_a_slab_is_clean() {
    use grim_horizon_engine::math::Aabb;
    let mut w = flat_world(stone());
    for x in -4..=4 {
        w.set_shape(
            x,
            64,
            3,
            stone(),
            &[Aabb::new(0.0, 0.0, 0.0, 1.0, 0.5, 1.0)],
        );
        for z in 4..10 {
            w.set_full(x, 64, z, stone());
        }
    }
    let mut h = Harness::new(w, SPAWN);
    for _ in 0..30 {
        h.tick(Keys::forward());
    }
    assert!(h.client.pos.y > 64.9, "did not climb: {}", h.client.pos.y);
    assert_clean(&h);
}

#[test]
fn walking_into_a_wall_is_clean() {
    let mut w = flat_world(stone());
    for x in -4..=4 {
        for y in 64..67 {
            w.set_full(x, y, 3, stone());
        }
    }
    let mut h = Harness::new(w, SPAWN);
    for i in 0..80 {
        h.client.yaw = if i < 40 { 0.0 } else { 30.0 };
        h.tick(Keys {
            sprint: true,
            ..Keys::forward()
        });
    }
    assert_clean(&h);
}

#[test]
fn knockback_taken_is_clean() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..5 {
        h.tick(Keys::default());
    }
    let kb = Vec3::new(0.4, 0.36, 0.0);
    h.velocity(kb);
    // The client applies it on its next tick (it arrives before the ping).
    h.client.vel = kb;
    for _ in 0..30 {
        h.tick(Keys::default());
    }
    assert_clean(&h);
}

#[test]
fn knockback_ignored_flags_antikb() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..5 {
        h.tick(Keys::default());
    }
    h.velocity(Vec3::new(0.4, 0.36, 0.0));
    // The cheating client never applies it and keeps walking.
    for _ in 0..10 {
        h.tick(Keys::forward());
    }
    assert!(h.flagged(CheckId::AntiKB) >= 1, "flags: {:?}", h.flags);
}

#[test]
fn knockback_under_high_latency_is_clean() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    h.latency_ticks = 8;
    for _ in 0..20 {
        h.tick(Keys::forward());
    }
    let kb = Vec3::new(0.0, 0.36, -0.4);
    h.velocity(kb);
    // The velocity reaches the client together with the next transaction.
    for i in 0..40 {
        if i == 8 {
            h.client.vel = kb;
        }
        h.tick(Keys::forward());
    }
    assert_clean(&h);
}

#[test]
fn speed_hack_flags_simulation() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..40 {
        h.client_tick(Keys::forward(), |c| {
            // 30% more horizontal distance every tick.
            c.pos.z += 0.065;
        });
    }
    assert!(h.flagged(CheckId::Simulation) >= 10, "flags: {:?}", h.flags);
}

#[test]
fn flying_hack_flags() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..10 {
        h.tick(Keys::default());
    }
    for _ in 0..30 {
        h.client_tick(Keys::forward(), |c| {
            c.vel.y = 0.0;
            c.pos.y = 66.0;
            c.on_ground = false;
        });
    }
    assert!(h.flagged(CheckId::Simulation) >= 1, "flags: {:?}", h.flags);
}

#[test]
fn ground_spoof_flags() {
    let mut h = Harness::new(flat_world(stone()), Vec3::new(0.5, 80.0, 0.5));
    for _ in 0..12 {
        h.client_tick(Keys::default(), |c| c.on_ground = true);
    }
    assert!(h.flagged(CheckId::GroundSpoof) >= 1, "flags: {:?}", h.flags);
}

#[test]
fn timer_flags_and_normal_rate_does_not() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..200 {
        h.tick(Keys::forward());
    }
    assert_eq!(h.flagged(CheckId::Timer), 0);
    // Two client ticks per server tick: a 2x timer.
    for _ in 0..100 {
        h.tick(Keys::forward());
        h.now -= 50_000_000; // the second tick took no real time
        h.client_tick(Keys::forward(), |_| {});
    }
    assert!(h.flagged(CheckId::Timer) >= 10, "flags: {:?}", h.flags);
}

#[test]
fn packet_bursts_after_lag_are_clean() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..60 {
        h.tick(Keys::forward());
    }
    // The client stalls for 5 ticks, then catches up: five client ticks
    // arrive within one server tick.
    for _ in 0..10 {
        for _ in 0..4 {
            h.server_tick();
        }
        h.burst(5, Keys::forward());
        for _ in 0..5 {
            h.tick(Keys::forward());
        }
    }
    assert_eq!(h.flagged(CheckId::Timer), 0, "flags: {:?}", h.flags);
}

#[test]
fn teleports_reset_prediction() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..20 {
        h.tick(Keys {
            sprint: true,
            ..Keys::forward()
        });
    }
    h.teleport(Vec3::new(20.5, 64.0, 20.5));
    for _ in 0..20 {
        h.tick(Keys::forward());
    }
    assert_clean(&h);
}

#[test]
fn alert_only_mode_never_sets_back_or_modifies() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..40 {
        h.client_tick(Keys::forward(), |c| c.pos.z += 0.3);
    }
    assert!(h.flagged(CheckId::Simulation) > 0);
    assert!(!h.actions.iter().any(|a| matches!(
        a,
        grim_horizon_engine::Action::Setback { .. } | grim_horizon_engine::Action::Kick { .. }
    )));
    assert!(
        h.verdicts
            .iter()
            .all(|v| *v == grim_horizon_engine::Verdict::Keep)
    );
}

#[test]
fn setback_mode_sets_back_speed() {
    let mut h = Harness::new(flat_world(stone()), SPAWN)
        .with_config(|c| c.mode = grim_horizon_engine::config::Mode::Setback);
    for _ in 0..40 {
        h.client_tick(Keys::forward(), |c| c.pos.z += 0.3);
    }
    let setbacks = h
        .actions
        .iter()
        .filter(|a| matches!(a, grim_horizon_engine::Action::Setback { key: KEY, .. }))
        .count();
    assert!(setbacks >= 1, "actions: {:?}", h.actions);
}

#[test]
fn unloaded_chunks_are_not_judged() {
    let mut w = flat_world(stone());
    w.unloaded.insert((0, 0));
    let mut h = Harness::new(w, SPAWN);
    for _ in 0..20 {
        h.client_tick(Keys::forward(), |c| c.pos.z += 0.3);
    }
    assert_eq!(h.flagged(CheckId::Simulation), 0);
    let (_, skipped) = h.checked_predictions();
    assert!(skipped > 0);
}

#[test]
fn block_changes_near_the_player_wait_for_transactions() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..5 {
        h.tick(Keys::default());
    }
    // The server removed the floor under the player; until the client
    // answers the next transaction it may still stand on it.
    h.engine.on_outgoing(
        KEY,
        grim_horizon_engine::Clientbound::BlockChanged([0, 63, 0]),
    );
    h.world.remove(0, 63, 0);
    h.tick(Keys::default());
    assert_clean(&h);
}

#[test]
fn unreported_block_changes_are_unknown_until_confirmed() {
    let mut h = Harness::new(flat_world(stone()), SPAWN);
    for _ in 0..5 {
        h.tick(Keys::default());
    }
    // The integration lost which blocks changed (its outbox overflowed):
    // the floor may be gone or not, so nothing is judged until the client
    // answers the next transaction.
    h.engine
        .on_outgoing(KEY, grim_horizon_engine::Clientbound::BlocksUnknown);
    h.world.remove(0, 63, 0);
    let (_, skipped) = h.checked_predictions();
    h.client_tick(Keys::default(), |c| c.position_reminder = 19);
    assert_clean(&h);
    assert!(
        h.checked_predictions().1 > skipped,
        "judged against unknown blocks"
    );
}
