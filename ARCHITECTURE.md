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
authoritative world and tracks what the client may still see instead:

* every block change the server sends carries its block state (BlockUpdate,
  SectionBlocksUpdate); `BlockView::state_shape` describes any state (facts,
  fluid, collision boxes) from Horizon's registry and shape cache;
* per nearby position (16 blocks) the engine keeps the state the client
  settled in and each change not yet confirmed by a transaction; a client
  answers a transaction only after applying everything sent before it, so
  what it may show is the changes up to some transaction: one world per
  unanswered transaction (`world::Around::Worlds`), never one per combination
  of cells;
* a move near unconfirmed changes is predicted in each of those worlds
  (physically equal ones once, at most 16) through `world::Overlay`, and
  passes if any explains it;
* the player's own digs and places: the client shows the block as it was (the
  server's state, `BlockView::state_at`) or its own guess until the server
  answers with the cell's state, which then stands for the guess; counted only
  within 8 blocks and 12 a second, so packet spam cannot hide moves;
* not judged only when a state is not known (a cell's first change, shapes
  that depend on context, changes far away when a teleport brings the player
  there), when more than 16 worlds would be needed (a round trip over ~800 ms
  with a block changing beside the player every tick), or in chunks the
  player has not been sent. All of it is counted in `grim stats`, and a run
  of 200 unjudged moves is logged.

Far-away or stateless changes are kept as "unknown until transaction N" (16
bytes each); positions past 4,096 per player fall back to their 16x16x16
section, sections past 1,024 to "everything unknown", all until confirmed. A
change the send hook could not record (outbox past 65,536 a tick) makes its
section unknown; teleports and pings are never dropped.

### Crash handling

Every Horizon hook runs under `catch_unwind` (Horizon's profiles unwind);
`recovery.rs` decides what follows, and nothing resumes on state a panic may
have left half-updated:

* a panic while checking one player's packets: that player's state is already
  out of the engine (it is held outside the map while it works); the adapter
  forgets the connection, kicks the player ("please reconnect": a rejoin
  resends position, attributes and effects) and tells online staff. Nobody
  else's checks change, so a player who finds such a bug only disconnects
  themself;
* a panic in a hook shared by everyone: the engine is rebuilt from the config,
  players online stay unchecked until they rejoin (their state would be
  incomplete), staff are told;
* three shared failures within ten minutes: the anti-cheat turns off, staff
  are reminded every minute, `grim` on the console says so.

### The Horizon side, packet by packet

* The send hook knows a sender only by its `PlayerSender` (`routing.rs`):
  the adapter keeps a strong reference to each one it maps or buffers for, so
  a closed connection's address cannot be reused while it is known; the join
  burst is buffered until the player is registered.
* The inbox hook reads only packets the engine has not seen. Horizon pops at
  most 400 a tick and may stop early; what is left was seen. The count is
  reset in `inbox_done` for every guest read that tick, by connection, since
  a guest may have changed level in between.

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
1200, pending block changes 4,096 positions and 1,024 sections, entity
history 40 ticks per level); about 6 KB of heap per player while walking,
75 KB under TNT-scale block changes.

Measured cost (benchmarks/README.md): about 6-10 µs per predicted movement and
about 1 ms of engine work per server tick for 100 players (1.4 ms p99), 12-14
ms with 2,000 block changes per player per tick, on an Apple M1, against the
in-memory test world (Horizon's collision scan will add to this; not yet
measured live).

## Testing on a real server

`tools/grim-bot` is a protocol client that moves with the engine tests'
vanilla-model client; `tools/smoke_test.py` runs a scratch Horizon server on a
superflat world and checks joining, legit play, cheats, staff alerts,
teleports, respawn, dimension changes, setbacks, kicks and reconnects against
it (BUILD.md, section 5). It needs Minecraft 26.3 game files and the EULA
decision, so it has not run here.

## Why one engine crate

horizon-mixins bundle crates may depend only on registry crates already in
Horizon's `Cargo.lock`, not on each other, so the suggested `grim-core`,
`grim-physics`, `grim-world`, `grim-checks` split lives as modules of one
crate. The Horizon-specific code is the adapter module plus seven helpers in
`integration/horizon/grim-horizon/`.
