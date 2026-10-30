// SPDX-License-Identifier: GPL-3.0-only
// Load and stability: many vanilla-model players (the test suite's client)
// against one engine, under the stresses a server sees. Engine time only
// (on_incoming, on_outgoing, end_tick): a real server's tick adds Horizon's
// own work, which only a live run measures.
//
//   cargo run --release --example bench_load

#[path = "../tests/common/mod.rs"]
mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Instant;

use common::{Client, Keys};
use grim_horizon_engine::Config;
use grim_horizon_engine::engine::{Action, Clientbound, Engine, Serverbound};
use grim_horizon_engine::math::Vec3;
use grim_horizon_engine::world::{BlockFacts, TestWorld};

/// Live heap bytes, for memory per player.
struct Counting;
static LIVE: AtomicIsize = AtomicIsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        LIVE.fetch_add(l.size() as isize, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size() as isize, Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static A: Counting = Counting;

#[derive(Clone, Copy, Default)]
struct Stress {
    /// Block changes a tick in a crater 30 blocks from everyone.
    tnt: i32,
    /// Every player teleported every this many ticks.
    teleport_every: u32,
    /// A fifth of the players leave and rejoin every this many ticks.
    reconnect_every: u32,
    /// Pongs 1 to 8 ticks late, and packets held back up to 3 ticks.
    jitter: bool,
    /// A block beside every player changes every this many ticks.
    beside_every: u32,
}

struct Player {
    key: u32,
    client: Client,
    home: Vec3,
    keys: Keys,
    pongs: VecDeque<(u32, i32)>,
    held: Vec<Serverbound>,
    hold_until: u32,
    /// A teleport the client applies at the start of its next tick.
    teleport: Option<(i32, Vec3)>,
    spawned: bool,
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }
    fn chance(&mut self, p: u32) -> bool {
        self.next() % 100 < p
    }
}

/// The server sends a teleport; the client applies it on its next tick,
/// in order with everything else it sends.
fn teleport(engine: &mut Engine, p: &mut Player, id: i32, to: Vec3) {
    engine.on_outgoing(
        p.key,
        Clientbound::Teleport {
            id,
            pos: to,
            delta: Vec3::ZERO,
            relatives: 0,
        },
    );
    p.teleport = Some((id, to));
}

struct Row {
    avg: f64,
    p95: f64,
    p99: f64,
    bytes_per_player: f64,
    flags: usize,
    stats: grim_horizon_engine::engine::Stats,
}

fn run(players: u32, ticks: u32, s: Stress) -> Row {
    let mut world = TestWorld::new();
    world.min_y = -64;
    world.max_y = 320;
    world.floor(-120, 120, 63, -120, 120, BlockFacts::SOLID);
    let stone_id = world.add_state(
        BlockFacts::SOLID,
        &[grim_horizon_engine::math::Aabb::new(
            0.0, 0.0, 0.0, 1.0, 1.0, 1.0,
        )],
    );
    let air_id = world.add_state(BlockFacts::AIR, &[]);
    let mut engine = Engine::new(Config::default());
    let mut rng = Rng(42);
    let mut now = 1_000_000_000u64;
    let mut next_key = 0u32;
    let mut tp_id = 0;
    let home = |i: u32| {
        Vec3::new(
            f64::from(i % 10) * 8.0 - 40.0 + 0.5,
            64.0,
            f64::from(i / 10) * 8.0 - 40.0 + 0.5,
        )
    };
    let base = LIVE.load(Ordering::Relaxed);
    let mut list: Vec<Player> = Vec::new();
    let mut join = |engine: &mut Engine, list: &mut Vec<Player>, slot: u32, tp_id: &mut i32| {
        let key = next_key;
        next_key += 1;
        engine.add_player(key, u128::from(key), &format!("P{key}"), 0);
        let mut p = Player {
            key,
            client: Client::new(home(slot)),
            home: home(slot),
            keys: Keys::default(),
            pongs: VecDeque::new(),
            held: Vec::new(),
            hold_until: 0,
            teleport: None,
            spawned: false,
        };
        *tp_id += 1;
        let to = p.home;
        teleport(engine, &mut p, *tp_id, to);
        list.push(p);
    };
    for i in 0..players {
        join(&mut engine, &mut list, i, &mut tp_id);
    }
    let mut times = Vec::with_capacity(ticks as usize);
    let mut flags = 0;
    let mut peak_bytes = 0isize;
    for t in 0..ticks {
        let mut engine_ns = 0u128;
        // Server side of the tick: block changes and stresses.
        let start = Instant::now();
        if s.tnt > 0 {
            for p in &list {
                for b in 0..s.tnt {
                    let pos = [
                        b % 40 + 60,
                        60 + b / 1600,
                        (b / 40) % 40 - 20 + (t as i32 % 3),
                    ];
                    engine.on_outgoing(p.key, Clientbound::BlockChanged(pos));
                }
            }
        }
        if s.beside_every > 0 && t % s.beside_every == 0 {
            // A redstone clock beside each player's home: stone, then air.
            let state = if (t / s.beside_every).is_multiple_of(2) {
                stone_id
            } else {
                air_id
            };
            for p in &list {
                let c = [p.home.x.floor() as i32 + 2, 64, p.home.z.floor() as i32];
                world.set_state(c[0], c[1], c[2], state);
                engine.on_outgoing(p.key, Clientbound::BlockState(c, state));
            }
        }
        engine_ns += start.elapsed().as_nanos();
        if s.teleport_every > 0 && t > 0 && t % s.teleport_every == 0 {
            for p in &mut list {
                tp_id += 1;
                let to = p.home;
                let start = Instant::now();
                teleport(&mut engine, p, tp_id, to);
                engine_ns += start.elapsed().as_nanos();
            }
        }
        if s.reconnect_every > 0 && t > 0 && t % s.reconnect_every == 0 {
            let gone: Vec<usize> = (0..list.len())
                .filter(|i| i % 5 == (t / s.reconnect_every) as usize % 5)
                .collect();
            for &i in gone.iter().rev() {
                let p = list.remove(i);
                let start = Instant::now();
                engine.remove_player(p.key);
                engine_ns += start.elapsed().as_nanos();
            }
            for slot in gone {
                let start = Instant::now();
                join(&mut engine, &mut list, slot as u32, &mut tp_id);
                engine_ns += start.elapsed().as_nanos();
            }
        }
        // Each client's tick.
        for p in &mut list {
            if rng.chance(8) {
                p.keys = Keys {
                    forward: rng.chance(80),
                    backward: false,
                    left: rng.chance(15),
                    right: rng.chance(15),
                    jump: rng.chance(20),
                    sneak: rng.chance(10),
                    sprint: rng.chance(50),
                };
            }
            let away = p.client.pos.sub(p.home);
            if away.x.hypot(away.z) > 3.0 {
                // Turn toward home by the short way, as a mouse does.
                let target = away.x.atan2(-away.z).to_degrees() as f32;
                p.client.yaw +=
                    ((target - p.client.yaw + 540.0).rem_euclid(360.0) - 180.0).clamp(-30.0, 30.0);
                p.keys.forward = true;
            } else if rng.chance(20) {
                p.client.yaw += (rng.next() % 31) as f32 - 15.0;
            }
            let mut packets: Vec<Serverbound> = Vec::new();
            while p.pongs.front().is_some_and(|&(due, _)| due <= t) {
                let (_, id) = p.pongs.pop_front().unwrap();
                packets.push(Serverbound::Pong { id });
            }
            if let Some((id, to)) = p.teleport.take() {
                p.client.pos = to;
                p.client.vel = Vec3::ZERO;
                (p.client.yaw, p.client.pitch) = p.client.last_sent_rot;
                packets.push(Serverbound::AcceptTeleport {
                    id,
                    pos: to,
                    rot: (p.client.yaw, p.client.pitch),
                });
                p.spawned = true;
            }
            if p.spawned {
                packets.extend(p.client.packets(&world, p.keys, |_| {}));
            } else {
                packets.push(Serverbound::TickEnd);
            }
            p.held.extend(packets);
            if s.jitter && p.hold_until <= t && rng.chance(3) {
                p.hold_until = t + 1 + rng.next() % 3;
            }
            if p.hold_until <= t {
                let batch = std::mem::take(&mut p.held);
                let start = Instant::now();
                engine.on_incoming(p.key, &batch, &world, now);
                engine_ns += start.elapsed().as_nanos();
            }
        }
        now += 50_000_000;
        let start = Instant::now();
        engine.end_tick(now);
        let actions = engine.drain_actions();
        for a in actions {
            match a {
                Action::SendPing { key, id } => {
                    engine.on_outgoing(key, Clientbound::Ping { id });
                    let delay = if s.jitter { 1 + rng.next() % 8 } else { 1 };
                    if let Some(p) = list.iter_mut().find(|p| p.key == key) {
                        p.pongs.push_back((t + delay, id));
                    }
                }
                Action::Log(l) if l.contains(" failed ") => {
                    if flags == 0 {
                        eprintln!("first alert: {l}");
                    }
                    flags += 1;
                }
                _ => {}
            }
        }
        engine_ns += start.elapsed().as_nanos();
        times.push(engine_ns as f64 / 1e6);
        peak_bytes = peak_bytes.max(LIVE.load(Ordering::Relaxed) - base);
    }
    let live = LIVE.load(Ordering::Relaxed) - base;
    let _ = peak_bytes;
    times.sort_by(f64::total_cmp);
    let pct = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
    Row {
        avg: times.iter().sum::<f64>() / times.len() as f64,
        p95: pct(0.95),
        p99: pct(0.99),
        // The engine's and the model clients' heap together; the clients are
        // small next to the engine's per-player state and queues.
        bytes_per_player: live as f64 / f64::from(players),
        flags,
        stats: engine.stats,
    }
}

fn main() {
    println!(
        "Engine work per server tick (ms), 1,200 ticks, vanilla-model players walking, jumping, sprinting, sneaking"
    );
    println!(
        "| scenario | players | avg | p95 | p99 | heap/player | legit alerts | not judged (blocks / world) | judged in block variants | longest unjudged run | peak pending | spilled / overflowed |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|");
    let walk = Stress::default();
    let cases: Vec<(&str, u32, Stress)> = vec![
        ("walking", 10, walk),
        ("walking", 50, walk),
        ("walking", 100, walk),
        ("TNT 2,000 changes/tick", 100, Stress { tnt: 2000, ..walk }),
        (
            "teleport burst every 100 ticks",
            100,
            Stress {
                teleport_every: 100,
                ..walk
            },
        ),
        (
            "reconnect storm (20 % every 60 ticks)",
            100,
            Stress {
                reconnect_every: 60,
                ..walk
            },
        ),
        (
            "network jitter + held packets",
            100,
            Stress {
                jitter: true,
                ..walk
            },
        ),
        (
            "block beside each player every 5 ticks",
            100,
            Stress {
                beside_every: 5,
                ..walk
            },
        ),
        (
            "all of the above",
            100,
            Stress {
                tnt: 2000,
                teleport_every: 100,
                reconnect_every: 60,
                jitter: true,
                beside_every: 5,
            },
        ),
    ];
    for (name, players, s) in cases {
        let r = run(players, 1200, s);
        let st = r.stats;
        println!(
            "| {name} | {players} | {:.3} | {:.3} | {:.3} | {:.1} KB | {} | {} / {} | {} | {} | {} | {} / {} |",
            r.avg,
            r.p95,
            r.p99,
            r.bytes_per_player / 1024.0,
            r.flags,
            st.skipped_unconfirmed_blocks,
            st.skipped_unknown_world,
            st.judged_with_variants,
            st.longest_unjudged,
            st.peak_pending,
            st.block_spills,
            st.block_overflows
        );
    }
}
