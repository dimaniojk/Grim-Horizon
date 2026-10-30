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
