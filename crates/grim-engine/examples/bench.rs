// SPDX-License-Identifier: GPL-3.0-only
// Engine benchmark, independent of any server: drives N simulated players
// through the engine and reports time per movement packet and per server
// tick. Run with `cargo run --release --example bench`.

#[path = "../tests/common/mod.rs"]
mod common;

use common::{Harness, Keys, flat_world, stone};
use grim_horizon_engine::math::Vec3;
use grim_horizon_engine::world::{BlockFacts, BlockKind};

fn scenario(
    name: &str,
    world: impl Fn() -> grim_horizon_engine::world::TestWorld,
    spawn: Vec3,
    keys: impl Fn(usize) -> Keys,
    ticks: usize,
) {
    // Warm up, then measure.
    let mut h = Harness::new(world(), spawn);
    for i in 0..40 {
        h.tick(keys(i));
    }
    let start_stats = h.engine.stats;
    let start_engine = h.engine_nanos;
    for i in 0..ticks {
        h.client.yaw = (i as f32 * 0.7).sin() * 30.0;
        h.tick(keys(i));
    }
    let engine_ns = (h.engine_nanos - start_engine) as f64;
    let s = h.engine.stats;
    let predictions = s.predictions - start_stats.predictions;
    let packets = s.packets - start_stats.packets;
    let prediction_ns =
        (s.prediction_nanos - start_stats.prediction_nanos) as f64 / predictions.max(1) as f64;
    println!(
        "{name:<16} {ticks:>6} ticks {predictions:>6} predictions {:>7.2} µs/prediction {:>7.2} µs/player-tick (engine total) {:>7} packets, flags {}",
        prediction_ns / 1000.0,
        engine_ns / 1000.0 / ticks as f64,
        packets,
        h.flags.len()
    );
}

fn main() {
    let ticks = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(20_000);
    println!("grim-horizon-engine benchmark ({ticks} ticks per scenario, one player)");
    let spawn = Vec3::new(0.5, 64.0, 0.5);
    let flat = || flat_world(stone());
    scenario("standing", flat, spawn, |_| Keys::default(), ticks);
    scenario("walking", flat, spawn, |_| Keys::forward(), ticks);
    scenario(
        "sprint-jumping",
        flat,
        spawn,
        |_| Keys {
            sprint: true,
            jump: true,
            ..Keys::forward()
        },
        ticks,
    );
    let ice = || {
        flat_world(BlockFacts {
            friction: 0.98,
            kind: BlockKind::Ice,
            ..stone()
        })
    };
    scenario(
        "ice sprinting",
        ice,
        spawn,
        |_| Keys {
            sprint: true,
            ..Keys::forward()
        },
        ticks,
    );
    let water = || {
        let mut w = flat_world(stone());
        w.water_pool(-64, 64, 64, 70, -64, 64);
        w
    };
    scenario(
        "swimming",
        water,
        Vec3::new(0.5, 66.0, 0.5),
        |i| {
            if i % 40 < 20 {
                Keys::forward()
            } else {
                Keys {
                    jump: true,
                    ..Keys::forward()
                }
            }
        },
        ticks,
    );

    // Many players: per-tick cost of the whole engine.
    for players in [1usize, 10, 50, 100] {
        let mut harnesses: Vec<Harness> = (0..players)
            .map(|i| Harness::new(flat(), Vec3::new(i as f64 * 3.0 + 0.5, 64.0, 0.5)))
            .collect();
        for h in harnesses.iter_mut() {
            for _ in 0..20 {
                h.tick(Keys::forward());
            }
        }
        let n = 1000;
        let before: u64 = harnesses.iter().map(|h| h.engine_nanos).sum();
        for i in 0..n {
            for h in harnesses.iter_mut() {
                h.client.yaw = (i as f32 * 0.3).sin() * 40.0;
                h.tick(Keys {
                    sprint: true,
                    jump: i % 30 == 0,
                    ..Keys::forward()
                });
            }
        }
        let after: u64 = harnesses.iter().map(|h| h.engine_nanos).sum();
        let per_tick = (after - before) as f64 / 1e6 / f64::from(n);
        println!(
            "{players:>4} players: {per_tick:.3} ms per server tick of engine work ({:.1}% of a 50 ms tick)",
            per_tick / 50.0 * 100.0
        );
    }
    println!(
        "size of one player's engine state: {} bytes (plus heap: attribute modifiers, queues)",
        std::mem::size_of::<grim_horizon_engine::engine::PlayerData>()
    );
}
