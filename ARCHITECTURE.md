# Architecture

```
 vanilla 26.3 client ──TCP──► Horizon dedicated server (horizon-client --dedicated, built with the bundle)
                                │
   reader thread (lan/conn.rs)  │  decode → NetEvent::Packet → Hub::poll → Guest::inbox
                                ▼
   server thread ─ Hub::tick_level ─► [hook level]  entity positions ─────────────┐
                 ─ Hub::process_inbox ─► [hook inbox] new packets ─┐              │
                 │        (then Horizon handles what is left)      │              │
                 ─ Hub::sync_world ─► [hook inbox-done]            │              │
                 ─ Hub::end_tick ─► [hook end-tick] ── pings ──┐   │              │
                 ─ PlayerSender::send ─► [hook send] ── OUTBOX ─┼───┤              │
                                                                ▼   ▼              ▼
                       adapter (support module, lan/hub.rs::grim_horizon, GPL)
                         · Horizon types → engine events, BlockView over Horizon's World
                         · applies Actions (setback = Hub::teleport, kick, chat) and Verdicts
                                │
                                ▼
                       grim-horizon-engine (bundle crate, no Horizon types)
                         engine.rs      per-player state, packet order, transactions, setbacks, checks
                         prediction.rs  MovementCheckRunner / PlayerBaseTick / MovementTicker / PredictionEngine
                         collisions.rs  Collisions, supporting block, BlockProperties, StuckSpeed
                         fluid.rs       EntityFluidInteraction (26.3), FluidTypeFlowing
                         velocity.rs    KnockbackHandler (AntiKB), ExplosionHandler
                         entities.rs    entity history for Reach / Hitboxes
                         checks.rs      check registry, violations, decay, alerts
                         config.rs      grim-horizon.toml       commands.rs  /grim ...
```

## The engine (`crates/grim-engine`)

A plain Rust library with two dependencies (`serde`, `toml`) and no knowledge
of Horizon. Its contract:

* `Engine::on_incoming(key, &[Serverbound], &dyn BlockView, now) -> Vec<Verdict>`:
  the client's packets **in the order the client sent them**, against the
  level the player is in. One verdict per packet (`Keep`, `Cancel`,
  `SetOnGround(bool)`).
* `Engine::on_outgoing(key, Clientbound)`: what the server sent, in send order.
* `Engine::end_tick(now)`: one transaction (ping) per player.
* `Engine::record_entities(dimension, snapshots)`: positions for Reach.
* `Engine::drain_actions() -> Vec<Action>`: setbacks, kicks, chat lines,
  log lines, pings to send.
* `BlockView`: block facts (friction, speed/jump factors, bounce, climbable,
  stuck speed...), fluids, per-region and per-block collision boxes, chunk
  loading.

### Transactions

Grim wraps state changes in ping/pong "transactions" so it knows which state
the client had when it sent each packet. Here every server tick ends with one
`ClientboundPing` (ids carry `PING_MARK`, so other pings are ignored). Each
outgoing event is tagged with the last transaction sent before it. When the
client answers transaction *k*, everything sent before *k* has been handled by
the client: a velocity tagged *k* becomes "first bread" (may be applied) when
*k* is answered and "required" once a later one is. Pong send times drive the
Timer check's clock and the reach window.

### World compensation

Grim keeps a full client-side copy of every chunk. This port reads Horizon's
authoritative world and compensates instead:

* every block the server changes is recorded with the transaction that
  follows it; until the client answers that transaction, predictions whose
  movement touches the block are **not judged** (the client may see the old or
  the new state);
* the player's own digs and placements are uncertain until the next
  transaction;
* chunks count as loaded only when the server has them *and* has sent them to
  the player (`Guest::sent_chunks`); movement in other chunks is not judged.

### Prediction

A faithful port of Grim's 26.3 path: start vectors (carried velocity,
knockback, explosions, swim hop, climbing), fluid pushing, attack slowdown,
the 1.21.5+ movement threshold, jumps, the client's *known* input (one
movement input instead of Grim's 9 guesses), both states of the crouch
slowdown and item use, stuck speed, climbing, the uncertainty box
(`handleStartingVelocityUncertainty`), collisions with step-up trying the
client's axis orders, `MovementTicker.move` (ground, bounce, speed factor),
end-of-tick friction and gravity, and Grim's point-three estimator for ticks
without a position packet. The best candidate's distance to the real movement
is the offset that the Simulation check judges.

Deviations from Grim, each to avoid a false positive found by the
random-play stress test (`tests/random_play.rs`):

* ticks without a movement packet still advance swimming, pose and sprint
  history (`prediction::idle_tick`);
* edge detection for sneaking also probes 0.05 and "velocity + one input
  step", not only "last movement + 0.05" (a 1x1 hole fooled Grim's probe);
* "0.03" candidates are moved with the real pose box as well as Grim's
  0.6x0.6 box;
* both sides of the 1.21.5 tiny-velocity cut are kept near the threshold;
* a claim of *leaving* the ground without a position is always accepted
  (Grim resyncs instead, which alert-only mode cannot do);
* the first prediction after a teleport gets the sneaking lenience when a
  tick was skipped.

## Threading and performance

All hooks run on Horizon's server thread. The engine is behind one mutex
(`ENGINE`); the send hook only touches a second mutex (`OUTBOX`), so the
adapter may call `Hub::teleport` or `Guest::send` while it holds the engine.
Nothing blocks: no I/O on the tick path except `/grim reload`. Per-player
state is bounded: queues are capped (velocities 64, unanswered transactions
1200, pending block changes 4096, entity history 40 ticks per level).

Measured cost (benchmarks/README.md): about 6-10 µs per predicted movement and
0.8 ms of engine work per server tick for 100 players moving every tick, on an
Apple M1, against the in-memory test world (Horizon's collision scan will add
to this; not yet measured live).

## Why one engine crate

horizon-mixins bundle crates may depend only on registry crates already in
Horizon's `Cargo.lock`, not on each other, so the suggested `grim-core`,
`grim-physics`, `grim-world`, `grim-checks` split lives as modules of one
crate. The Horizon-specific code is the adapter module plus seven helpers in
`integration/horizon/grim-horizon/`.
