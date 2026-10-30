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

Measured again after the block-state work (same Mac, same session as the
A/B in PROGRESS.md):

| Scenario | Players | avg ms | p95 ms | p99 ms | Heap/player | Alerts on legit players | Moves not judged (blocks / chunks) | Moves judged in block-state worlds | Longest unjudged run | Peak pending | Spilled / overflowed |
|---|---|---|---|---|---|---|---|---|---|---|---|
| walking | 10 | 0.105 | 0.193 | 0.248 | 32.8 KB | 0 | 0 / 0 | 0 | 0 | 0 | 0 / 0 |
| walking | 50 | 0.530 | 0.680 | 0.727 | 5.8 KB | 0 | 0 / 0 | 0 | 0 | 0 | 0 / 0 |
| walking | 100 | 1.079 | 1.272 | 1.350 | 5.7 KB | 0 | 0 / 0 | 0 | 0 | 0 | 0 / 0 |
| TNT: 2,000 changes per player per tick, 30 blocks away | 100 | 10.378 | 10.929 | 11.326 | 73.8 KB | 0 | 0 / 0 | 0 | 0 | 2,160 | 0 / 0 |
| every player teleported every 100 ticks | 100 | 1.065 | 1.259 | 1.317 | 5.7 KB | 0 | 0 / 0 | 0 | 0 | 0 | 0 / 0 |
| reconnection storm: 20 % leave and rejoin every 60 ticks | 100 | 1.064 | 1.270 | 1.363 | 8.4 KB | 0 | 0 / 0 | 0 | 0 | 0 | 0 / 0 |
| network jitter: pongs 1-8 ticks late, packets held up to 3 ticks | 100 | 1.082 | 1.320 | 1.456 | 6.1 KB | 0 | 0 / 0 | 0 | 0 | 0 | 0 / 0 |
| a block beside every player switching stone/air every 5 ticks | 100 | 1.256 | 1.992 | 2.179 | 5.2 KB | 0 | 34 / 0 | 13,873 | 1 | 1 | 0 / 0 |
| all of the above at once | 100 | 10.658 | 11.330 | 13.857 | 76.3 KB | 0 | 1,363 / 0 | 63,767 | 12 | 2,161 | 0 / 0 |

Before the block-state work (0b103c6, same session): TNT 12.45 ms and 73.6 KB
per player; all stresses 11.81 ms; the block beside each player (sent without
states then) left 22,614 moves unjudged. The first block-state version cost
13.06 ms and 202 KB per player under TNT; keeping histories only for nearby,
known states brought it to the numbers above.

`bench_blocks` (100 players, 2,000 changes each per tick, near them): 22.5 ms
per tick now; 20.0-21.5 ms for 0b103c6 and 21.8-22.1 ms for this code when
run back to back.

Read with care:

* The changing block beside each player is now judged in each world the
  client may show; the 34 moves not judged are each cell's first change,
  whose earlier state is not known.
* No update was dropped or spilled here: 2,000 changes a tick stay under the
  4,096-position limit because each is confirmed within a few ticks.
* The first version of this benchmark found two problems in the benchmark
  itself (yaw snapped across 180 degrees, which AimModulo360 rightly flagged;
  teleports accepted out of order with held packets) and one engine false
  positive (the unreported first step after a teleport, now fixed).
* Standing now costs 3.3 µs per player-tick (was 1.8): ticks without a
  position advance sprint, swimming and pose, as the client does.
