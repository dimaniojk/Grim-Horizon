// SPDX-License-Identifier: GPL-3.0-only
// Block-update load: TNT-scale block changes broadcast to 50-100 players
// while they move. Run with `cargo run --release --example bench_blocks`.

use std::time::Instant;

use grim_horizon_engine::Config;
use grim_horizon_engine::engine::{Action, Clientbound, Engine, Serverbound};
use grim_horizon_engine::math::Vec3;
use grim_horizon_engine::world::{BlockFacts, TestWorld};

fn run(players: u32, blocks_per_tick: i32, ticks: u32) -> (f64, usize) {
    let mut world = TestWorld::new();
    world.floor(-200, 200, 63, -20, 20, BlockFacts::SOLID);
    let mut engine = Engine::new(Config::default());
    let mut now = 1_000_000_000u64;
    for k in 0..players {
        engine.add_player(k, u128::from(k), &format!("P{k}"), 0);
        let pos = Vec3::new(f64::from(k) * 2.0 + 0.5, 64.0, 0.5);
        engine.on_outgoing(
            k,
            Clientbound::Teleport {
                id: 1,
                pos,
                delta: Vec3::ZERO,
                relatives: 0,
            },
        );
        engine.on_incoming(
            k,
            &[Serverbound::AcceptTeleport {
                id: 1,
                pos,
                rot: (0.0, 0.0),
            }],
            &world,
            now,
        );
    }
    let mut pongs: Vec<(u32, i32)> = Vec::new();
    let mut flags = 0;
    let start = Instant::now();
    for t in 0..ticks {
        // A crater of `blocks_per_tick` changes near the players, sent to all.
        for k in 0..players {
            for b in 0..blocks_per_tick {
                let pos = [
                    (b % 40) + (t as i32 % 5) * 40 - 100,
                    60 + (b / 400),
                    (b / 40) % 10 - 5,
                ];
                engine.on_outgoing(k, Clientbound::BlockChanged(pos));
            }
        }
        for k in 0..players {
            let mut packets: Vec<Serverbound> = pongs
                .iter()
                .filter(|p| p.0 == k)
                .map(|p| Serverbound::Pong { id: p.1 })
                .collect();
            // Standing still sends a position reminder every 20 ticks; every
            // tick here, so each one runs a prediction next to the crater.
            let pos = Vec3::new(f64::from(k) * 2.0 + 0.5, 64.0, 0.5);
            packets.push(Serverbound::Move {
                pos: Some(pos),
                rot: Some(((t % 360) as f32, 0.0)),
                on_ground: true,
                horizontal_collision: false,
            });
            packets.push(Serverbound::TickEnd);
            engine.on_incoming(k, &packets, &world, now);
        }
        pongs.clear();
        now += 50_000_000;
        engine.end_tick(now);
        for a in engine.drain_actions() {
            match a {
                Action::SendPing { key, id } => {
                    engine.on_outgoing(key, Clientbound::Ping { id });
                    pongs.push((key, id));
                }
                Action::Log(l) if l.contains("failed") => {
                    if flags == 0 {
                        eprintln!("first alert: {l}");
                    }
                    flags += 1
                }
                _ => {}
            }
        }
    }
    (
        start.elapsed().as_secs_f64() * 1e3 / f64::from(ticks),
        flags,
    )
}

fn main() {
    println!("TNT-scale block updates (engine work per server tick, all players)");
    for players in [50, 100] {
        for blocks in [500, 2000] {
            let (ms, flags) = run(players, blocks, 200);
            println!(
                "{players:>4} players, {blocks:>5} block changes/tick each: {ms:>8.3} ms/tick, alerts {flags}"
            );
        }
    }
}
