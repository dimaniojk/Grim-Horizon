// Long random legit play over varied terrain: the engine must never flag
// what the vanilla-model client does.

mod common;

use common::{Harness, Keys, stone};
use grim_horizon_engine::math::{Aabb, Vec3};
use grim_horizon_engine::world::{BlockFacts, BlockKind, TestWorld};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn chance(&mut self, percent: u32) -> bool {
        self.next() % 100 < percent
    }
}

fn varied_world(seed: u64) -> TestWorld {
    let mut rng = Lcg(seed);
    let mut w = TestWorld::new();
    let ice = BlockFacts {
        friction: 0.98,
        kind: BlockKind::Ice,
        ..stone()
    };
    for x in -40..=40 {
        for z in -40..=40 {
            let facts = if (x / 6 + z / 6) % 5 == 0 {
                ice
            } else {
                stone()
            };
            w.set_full(x, 63, z, facts);
            match rng.next() % 40 {
                0 => w.set_shape(
                    x,
                    64,
                    z,
                    stone(),
                    &[Aabb::new(0.0, 0.0, 0.0, 1.0, 0.5, 1.0)],
                ),
                1 => w.set_full(x, 64, z, stone()),
                2 => {
                    w.set_full(x, 64, z, stone());
                    w.set_full(x, 65, z, stone());
                }
                3 => w.remove(x, 63, z),
                _ => {}
            }
        }
    }
    // A walled pool to wander into.
    for x in 10..=18 {
        for z in 10..=18 {
            w.remove(x, 64, z);
            w.remove(x, 63, z);
            w.set_full(x, 61, z, stone());
        }
    }
    w.water_pool(10, 18, 62, 63, 10, 18);
    w
}

fn play(seed: u64, ticks: usize) -> Harness {
    play_debug(seed, ticks, false)
}

fn play_debug(seed: u64, ticks: usize, debug: bool) -> Harness {
    let mut rng = Lcg(seed ^ 0x9e37_79b9);
    let mut h = Harness::new(varied_world(seed), Vec3::new(0.5, 66.0, 0.5));
    let mut keys = Keys::default();
    let mut trail: std::collections::VecDeque<String> = Default::default();
    for _ in 0..ticks {
        if rng.chance(8) {
            keys = Keys {
                forward: rng.chance(70),
                backward: rng.chance(10),
                left: rng.chance(20),
                right: rng.chance(20),
                jump: rng.chance(25),
                sneak: rng.chance(10),
                sprint: rng.chance(50),
            };
        }
        if rng.chance(30) {
            h.client.yaw += (rng.next() % 41) as f32 - 20.0;
            h.client.pitch = ((rng.next() % 120) as f32 - 60.0).clamp(-90.0, 90.0);
        }
        // Stay inside the built area.
        if h.client.pos.x.abs() > 35.0 || h.client.pos.z.abs() > 35.0 || h.client.pos.y < 40.0 {
            if debug {
                trail.push_back(format!(
                    "TELEPORT from {:?} ground {} sprint {}",
                    h.client.pos, h.client.on_ground, h.client.sprinting
                ));
            }
            h.teleport(Vec3::new(0.5, 66.0, 0.5));
            if debug {
                let m = &h.engine.player(common::KEY).unwrap().movement;
                trail.push_back(format!(
                    "AFTER TP client yaw {} engine yaw {} last {}",
                    h.client.yaw, m.yaw, m.last_yaw
                ));
            }
            continue;
        }
        let before = (h.client.clone(), h.flags.len());
        h.tick(keys);
        if debug {
            let m = &h.engine.player(common::KEY).unwrap().movement;
            trail.push_back(format!("tick keys {:?} pos {:?} vel {:?} ground {} sprint {} swim {} pose_h {} eye_in {} | eng pred {:?} act {:?} ground {} sprint {} swim {} pose {:?} eye {} touch {} hidden {:?} edge {}", keys, h.client.pos, h.client.vel, h.client.on_ground, h.client.sprinting, h.client.swimming, h.client.pose_height, h.client.eye_in_water_last, m.predicted.vector, m.actual_movement, m.on_ground, m.is_sprinting, m.is_swimming, m.pose, m.eye_in_water, m.was_touching_water, (m.sneaking_hidden.min_x, m.sneaking_hidden.max_x, m.sneaking_hidden.min_z, m.sneaking_hidden.max_z), m.uncertainty.stuck_on_edge.has_occurred_since(1)));
            if trail.len() > 5 {
                trail.pop_front();
            }
        }
        if debug && h.flags.len() > before.1 {
            for t in &trail {
                println!("  TRAIL {t}");
            }
            let m = &h.engine.player(common::KEY).unwrap().movement;
            println!(
                "FLAG {:?} keys {:?} client yaw {} before {} sent {:?}",
                &h.flags[before.1..],
                keys,
                h.client.yaw,
                before.0.yaw,
                before.0.last_sent_rot
            );
            println!(
                "  before pos {:?} vel {:?} ground {} sprint {} sneak {} water {} support {:?} last_move {:?}",
                before.0.pos,
                before.0.vel,
                before.0.on_ground,
                before.0.sprinting,
                before.0.sneaking,
                before.0.in_water,
                before.0.support,
                before.0.last_move
            );
            println!(
                "  after  pos {:?} vel {:?} ground {}",
                h.client.pos, h.client.vel, h.client.on_ground
            );
            println!(
                "  engine pred {:?} act {:?} on_ground {} last_on_ground {} sprint {} slow {} water {} stepped {} yaw {} supporting {:?} friction {}",
                m.predicted.vector,
                m.actual_movement,
                m.on_ground,
                m.last_on_ground,
                m.is_sprinting,
                m.is_slow_movement,
                m.was_touching_water,
                m.uncertainty.is_step_movement,
                m.yaw,
                m.supporting,
                m.friction
            );
            let p = before.0.pos;
            for dy in -1..=1 {
                let mut row = String::new();
                for dz in -2..=2 {
                    for dx in -2..=2 {
                        use grim_horizon_engine::world::BlockView;
                        let b = h.world.block(
                            p.x.floor() as i32 + dx,
                            p.y.floor() as i32 + dy,
                            p.z.floor() as i32 + dz,
                        );
                        row.push(if b.air {
                            '.'
                        } else if b.kind == BlockKind::Ice {
                            'i'
                        } else {
                            '#'
                        });
                    }
                    row.push(' ');
                }
                println!("  y{dy:+} {row}");
            }
            return h;
        }
    }
    h
}

#[test]
#[ignore]
fn debug_first_flag() {
    let seed = std::env::var("SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let ticks = std::env::var("TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6000);
    play_debug(seed, ticks, true);
}

#[test]
fn random_legit_play_never_flags() {
    for seed in 1..=20 {
        let h = play(seed, 6000);
        assert!(
            h.flags.is_empty(),
            "seed {seed}: {:?}",
            &h.flags[..h.flags.len().min(8)]
        );
    }
}

/// Seeds that flagged legitimate play, each for its own reason; all fixed.
#[test]
fn former_false_positive_seeds_are_clean() {
    for (seed, cause) in [
        (
            236,
            "teleport while sprinting: the client took the server's look",
        ),
        (
            266,
            "teleport while sneaking: the next tick reports its position",
        ),
        (
            110,
            "sneaking against a slab wall: 1e-7 wall seeking stepped up",
        ),
        (
            216,
            "sneak edge lenience cut the movement, then velocity was wrong",
        ),
        (
            274,
            "sneak edge lenience cut the movement, then velocity was wrong",
        ),
        (
            142,
            "sprint started on a rotation-only tick: swimming one tick late",
        ),
        (
            171,
            "a tick with only a rotation packet did not advance state",
        ),
        (
            287,
            "a tick with only a rotation packet did not advance state",
        ),
    ] {
        let h = play(seed, 6000);
        assert!(
            h.flags.is_empty(),
            "seed {seed} ({cause}): {:?}",
            &h.flags[..h.flags.len().min(4)]
        );
    }
}

/// `SEEDS=300 cargo test --release --test random_play sweep -- --ignored --nocapture`:
/// how many seeds flag, for comparing engine changes.
#[test]
#[ignore]
fn sweep() {
    let seeds: u64 = std::env::var("SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let flagged: Vec<(u64, usize)> = (1..=seeds)
        .map(|s| (s, play(s, 6000).flags.len()))
        .filter(|(_, n)| *n > 0)
        .collect();
    println!(
        "{} of {seeds} seeds clean; flagged: {flagged:?}",
        seeds - flagged.len() as u64
    );
}
