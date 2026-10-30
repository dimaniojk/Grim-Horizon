# Engine benchmark

`cargo run --release --example bench` (crates/grim-engine/examples/bench.rs)
drives simulated vanilla-model players through the engine and times only the
engine calls (`on_incoming`, `on_outgoing`, `end_tick`), not the simulated
client. The world is the in-memory `TestWorld`; on a Horizon server the
collision queries go through Horizon's `BlockCollisionScan`, whose cost is not
included here.

Measured 2026-10-30 on an Apple M1 (macOS, Rust 1.99.1, release profile), two
runs, both shown:

| Scenario (20,000 ticks, 1 player) | µs per prediction | µs per player-tick, all packets | flags |
|---|---|---|---|
| standing (1 position reminder per 20 ticks) | 10.27 / 9.32 | 1.73 / 1.58 | 0 |
| walking | 7.63 / 7.61 | 7.98 / 7.96 | 0 |
| sprint-jumping | 5.90 / 5.99 | 6.24 / 6.34 | 0 |
| ice sprinting | 5.91 / 5.91 | 6.25 / 6.26 | 0 |
| swimming | 6.45 / 6.35 | 6.81 / 6.71 | 0 |

| Players, all sprinting and moving every tick | engine time per server tick |
|---|---|
| 1 | 0.008 ms |
| 10 | 0.081 ms |
| 50 | 0.403 / 0.404 ms |
| 100 | 0.822 / 0.805 ms (1.6% of a 50 ms tick) |

One player's engine state is 1,984 bytes plus small heap allocations
(attribute modifiers, bounded queues). Allocation on the hot path has not
been optimised yet: collision queries allocate a `Vec` per query.

## Block updates (TNT scale)

`cargo run --release --example bench_blocks`: every player is sent N block
changes a tick near the players, then moves, answers pings, and the engine
predicts each move against those pending changes. Apple M1, 200 ticks, engine
time per server tick for all players; alerts 0 in every run.

| Players | Changes per tick each | Before (VecDeque) | After (map) |
|---|---|---|---|
| 50 | 500 | 11.555 ms | 2.914 ms |
| 50 | 2,000 | 167.877 ms | 9.691 ms |
| 100 | 500 | 22.848 ms | 5.604 ms |
| 100 | 2,000 | 335.553 ms | 19.409 ms |

2,000 changes per player per tick is above a large TNT blast; 100 players at
that rate still take 19 ms of a 50 ms tick, so heavy block churn remains the
engine's most expensive load.

## Load and stability

`cargo run --release --example bench_load`: one engine, vanilla-model
players (the test suite's client) walking, jumping, sprinting and sneaking at
random near their homes for 1,200 ticks, with the stresses below. Times are
engine work per server tick (all `on_incoming`, `on_outgoing` and `end_tick`
calls), measured 2026-10-30 on an Apple M1. **This is engine time only**: a
real server tick adds Horizon's own work (the world, chunks, network), which
only a live run measures (Horizon's `horizon-server status` reports MSPT);
none has run yet. Heap per player is live heap divided by players, so the
10-player row carries the engine's fixed tables.

| Scenario | Players | avg ms | p95 ms | p99 ms | Heap/player | Alerts on legit players | Moves not judged (blocks / chunks) | Longest unjudged run | Peak pending | Spilled / overflowed |
|---|---|---|---|---|---|---|---|---|---|---|
| walking | 10 | 0.103 | 0.188 | 0.232 | 32.7 KB | 0 | 0 / 0 | 0 | 0 | 0 / 0 |
| walking | 50 | 0.525 | 0.668 | 0.713 | 5.7 KB | 0 | 0 / 0 | 0 | 0 | 0 / 0 |
| walking | 100 | 1.072 | 1.273 | 1.385 | 5.6 KB | 0 | 0 / 0 | 0 | 0 | 0 / 0 |
| TNT: 2,000 changes per player per tick, 30 blocks away | 100 | 12.269 | 13.497 | 13.981 | 73.6 KB | 0 | 0 / 0 | 0 | 2,160 | 0 / 0 |
| every player teleported every 100 ticks | 100 | 1.052 | 1.248 | 1.314 | 5.6 KB | 0 | 0 / 0 | 0 | 0 | 0 / 0 |
| reconnection storm: 20 % leave and rejoin every 60 ticks | 100 | 1.051 | 1.245 | 1.330 | 8.1 KB | 0 | 0 / 0 | 0 | 0 | 0 / 0 |
| network jitter: pongs 1-8 ticks late, packets held up to 3 ticks | 100 | 1.069 | 1.312 | 1.437 | 6.0 KB | 0 | 0 / 0 | 0 | 0 | 0 / 0 |
| a block beside every player changing every 5 ticks | 100 | 1.070 | 1.270 | 1.352 | 5.7 KB | 0 | 22,614 / 0 | 3 | 1 | 0 / 0 |
| all of the above at once | 100 | 11.633 | 13.350 | 14.150 | 76.3 KB | 0 | 96,738 / 0 | 116 | 2,163 | 0 / 0 |

Read with care:

* The changing block beside each player is the fail-open case: those moves
  are not judged. Under jitter a player went 116 moves in a row unjudged;
  past 200 the engine logs it once.
* No update was dropped or spilled here: 2,000 changes a tick stay under the
  4,096-position limit because each is confirmed within a few ticks.
* The first version of this benchmark found two problems in the benchmark
  itself (yaw snapped across 180 degrees, which AimModulo360 rightly flagged;
  teleports accepted out of order with held packets) and one engine false
  positive (the unreported first step after a teleport, now fixed).
* Standing now costs 3.3 µs per player-tick (was 1.8): ticks without a
  position advance sprint, swimming and pose, as the client does.
