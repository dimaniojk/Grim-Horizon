# Progress

## Phase 1: repository analysis (done)

* Cloned Grim, Horizon, the native mod template and horizon-mixins; commits in
  COMPATIBILITY.md.
* Read Horizon's AGENTS.md, docs/server.md, docs/hosting.md, docs/mods.md,
  docs/native-mods.md, the template README and horizon-mixins' docs; traced the
  dedicated server's packet path through `lan/conn.rs`, `lan/hub.rs`,
  `lan/guest.rs`, `game/lan_actions.rs`, `game/levels.rs`, `game/dedicated.rs`.
* Read Grim's prediction engine (MovementCheckRunner, PlayerBaseTick,
  MovementTicker, PredictionEngine and its Normal/Water/Lava variants,
  UncertaintyHandler, PointThreeEstimator, SneakingEstimator, Collisions,
  BlockProperties, EntityFluidInteraction, FluidTypeFlowing, JumpPower,
  input transformers) and checks (OffsetHandler, GroundSpoof, NoFall, Timer,
  TickTimer, KnockbackHandler, ExplosionHandler, Reach, BadPackets, Crash,
  Aim, PacketOrder).
* Decision: a horizon-mixins bundle with HEAD hooks (COMPATIBILITY.md).

## Phase 2: workspace and integration (done, not run live)

* `crates/grim-engine`: the engine, independent of Horizon.
* `integration/horizon/grim-horizon`: 7 hooks, the adapter module, the engine
  as a bundle crate. `tools/build_server.py` builds `dist/horizon-engine-grim`
  reproducibly from the pinned commit. The binary builds and starts; it stops
  at the EULA check as it should.

## Phase 3: first checks (done)

Packet checks, transactions, Timer/TickTimer, alerts, verbose, debug,
profile, exemptions, config and reload, alert-only default.

## Phase 4: prediction engine (done for players on foot)

Ground, air, jumps, sprinting, crouching, item-use slowdown, water,
swimming, lava, climbing, slime/honey/ice/soul sand/beds, stuck speed,
bubble columns, collisions with step-up, knockback and explosions,
attributes and effects, ticks without a position packet.

## Phase 5: advanced checks (done)

Reach and Hitboxes over an entity history window, AntiKB, AntiExplosion,
Timer, GroundSpoof, NoFall, Simulation.

## Phase 6: validation and packaging (partly done)

* 112 engine tests (plus 2 for grim-bot), 2,000 random seeds clean, including published vanilla numbers, cheat traces,
  latency, bursts, teleports, unloaded chunks and block-change compensation.
* Random legit play: 300 seeds × 6,000 ticks; 298 clean.
* Benchmarks (benchmarks/README.md).
* Smoke test script against a real server (`tools/smoke_test.py`) written;
  **not run**: it needs Minecraft 26.3's prepared resources, which are not on
  this machine, and someone to accept the Minecraft EULA.

## Bugs found by the stress test and fixed

In the engine:
1. Per-block collision queries attributed neighbouring blocks' boxes to the
   wrong cell, so the supporting block (friction) could be wrong.
2. Ticks without a movement packet did not advance swimming, pose and sprint
   history (Grim has the same gap).
3. A claim of leaving the ground without a position was ignored in
   alert-only mode (Grim resyncs instead).
4. Grim's edge probes miss a 1x1 hole when the last movement was long.
5. Edge probes shorter than the current movement missed edges.
6. "0.03" candidates only tried Grim's 0.6x0.6 box and slipped under ledges.
7. Velocities right at the 1.21.5 tiny-velocity cut could fall on either side.

In the test model of the vanilla client (each confirmed against vanilla's
behaviour, where the engine was already right): supporting-block friction,
float bounding boxes, the sneak slowdown's one-tick delay, crouching when
there is no room to stand, crawling after swimming, sinking with shift in
water, swimming, the sprint impulse rule.

## Review fixes (2026-10-30)

From a code review of the last commit, each with a regression test:

1. **Pending block changes were O(n)** (a `VecDeque` scanned on every
   movement and drained from the front). Now a map keyed by block position;
   past 4,096 entries every block counts as unknown until the client confirms,
   never as confirmed. 100 players with 2,000 changes a tick each: 335.6 ms
   → 19.8 ms per server tick (benchmarks/README.md).
2. **Packets could reach the wrong player.** The send hook knew a sender only
   by its address, and a freed address can be reused by the next connection.
   `routing::SenderRoutes` now holds a strong reference to every sender it
   maps or buffers for, so an address cannot be reused while it is known;
   buffers of closed connections are swept every tick.
3. **A superseded setback stopped checks for good.** Ported Grim's rule
   (SetbackTeleportUtil.checkTeleportQueue): every server teleport replaces
   the required setback, and accepting the newest one ends it. A respawn ends
   it too, and a setback whose teleport is never sent gives up after 40 ticks.
   An accepted older teleport while a newer one is pending does not end it.
4. **The step-up restriction never ran.** `setback_ticks_complete` was set to
   1 when the setback was asked for and reset to 0 on acceptance. It now
   follows Grim's SetBackData: 0 while unaccepted, 1 for the first movement
   after the newest teleport is accepted (stepping refused), then counting.

Found while auditing the Horizon adapter for ways to stop judging a player:

5. The inbox index was reset only for guests still in the level at
   `sync_world`; a guest who changed level kept a stale count, and the engine
   could miss its next packets, including the spawn teleport's acceptance
   (the player then stayed unjudged). Now reset by connection for every guest
   read that tick.
6. The outbox dropped any packet past 65,536 a tick, a lost teleport or ping
   included. Only block updates are dropped now, and a lost one makes every
   block unknown until the next transaction (`Clientbound::BlocksUnknown`).

Random play over 300 seeds: 298 clean, the same two seeds as before.

## Validation milestone (2026-10-30)

### Random-play failures, traced and fixed

Each seed was replayed with the debug trail (`SEED=n cargo test --release
--test random_play debug_first_flag -- --ignored --nocapture`), predicted and
actual movement compared, and the cause fixed where it was; the seeds are now
the regression test `former_false_positive_seeds_are_clean`.

| Seed | Check | What happened | Cause | Fix |
|---|---|---|---|---|
| 266 | Simulation 0.0018 | sneaking teleport into the air; tick 1 backs off to zero and sends nothing | test model: a teleport reset LocalPlayer's sendPosition baseline, vanilla does not (Horizon's client port says so) | model |
| 236 | Simulation 0.0317 | sprinting teleport; 14 degrees between predicted and actual direction | test model: an absolute teleport keeps the server's look, the model kept its own | model |
| 110 | Simulation 0.5 | sneaking against a slab wall with x velocity 7.7e-18 | engine: wall seeking (Grim's 1e-7 push) turned float noise into a movement that stepped up the slab | engine: no seeking below 1e-7 (vanilla's Shapes.collide); model and engine also zero sub-1e-7 axis movement |
| 216, 274 | Simulation 0.0076 | sneak-edge lenience cut the movement to the wall-flush value | engine (and Grim): the cut value became the velocity; vanilla keeps the uncut velocity or zero after the wall | engine: one tick of lenience between the two |
| 142, 171, 287 | Simulation 0.0196 | sprint started on a tick that sent only a rotation | engine: such a tick ran no prediction and no idle tick, so sprint, swimming and pose did not advance | engine: advance the state on every tick without a prediction |

Random play now: 300 of 300 seeds clean; 1,000 seeds: 997 clean. Open: 415,
536 (GroundSpoof in shallow water at the floor), 651 (sneak release on an ice
edge); see PORT_STATUS.md.

### Teleports and setbacks

* Step-up after a teleport: Grim refuses stepping on the first move after
  every teleport, because in Grim every server teleport is the required
  setback. The exploit it prevents needs a setback; refusing a vanilla
  player's step after /tp, a respawn or a portal is a false positive. Now
  only after the engine's own setbacks.
* Ported Grim's post-setback jump resync (MovementCheckRunner): a jump in the
  first move after a setback with no velocity into the ground, or with no
  ground in reach, gets another setback (no flag). Grim's second clause
  (setback velocity) does not apply: this port sends none.
* Found by the load test: the first tick after a teleport can step exactly
  back onto the client's last reported position, which vanilla then does not
  report. The engine now tries that hypothesis when standing still does not
  explain the next move (the hidden step must itself pass); a test reproduces
  it exactly.

### Fail-open paths

* Pending block changes: by position (4,096), then by 16x16x16 section
  (1,024), then everything; a lost outbox update hides only its section.
  Lookups cost at most the map's size even for a claimed move across the
  world.
* Metrics in `grim stats`: predictions not judged by reason, the longest
  unjudged run, block changes lost / spilled to sections / overflowed, peak
  pending. A player unjudged for 200 moves in a row is logged once per run.
* Known and measured: a block changing beside a player every tick keeps that
  player unjudged (bench: 145 moves in a row under jitter).

### Integration test

* `tools/grim-bot`: a protocol bot that moves with the test suite's vanilla
  model (legit, speed cheat, standing staff), answers pings in order with
  what it applies, respawns, and prints what happens. Its walk is checked
  against the engine in process (6,000 ticks clean; the cheat flags).
* `tools/smoke_test.py`: 27 separate assertions over three modes (BUILD.md,
  section 5). **Not run**: no 26.3 game files on this machine and the EULA is
  the owner's decision. The server binary was built and started in a scratch
  folder (`--initSettings`, `eula=false` written, nothing accepted).

### Load (engine only, benchmarks/README.md)

100 players walking: 1.06 ms per tick average, 1.35 ms p99; with 2,000 block
changes per player per tick, teleport bursts, a reconnection storm, network
jitter and a block changing beside every player all at once: 11.9 ms
average, 15.3 ms p99, 76 KB per player, no alert on any legitimate player.

## Live-validation milestone (2026-10-30, second pass)

Three kinds of evidence, kept apart:

* **Automated tests** (engine, grim-bot, sweep, benchmarks): below.
* **Server-binary checks**: the Horizon server with Grim-Horizon builds from
  the pinned commit, starts in a scratch folder (`--initSettings`, 0.6 s),
  and refuses to run without the EULA (`eula=false` kept, exit 0).
* **Live-server tests**: none. No grim-bot run and no vanilla-client run
  against a running server exists.

### Live server: blocked

On this Mac: Java 25 (Temurin) is installed; the only Minecraft install is
Feather's 1.21.8 profile in `~/Library/Application Support/minecraft` (no
`versions/26.3`, no 26.3 server jar), which Horizon cannot use. Needed from
the owner: the official launcher's 26.3 install, the 26.3 server jar, and a
decision on the EULA. Exact commands: BUILD.md, section 5. The next milestone
is the 27-check smoke test on a running server; no new checks before it.

### False positives fixed (each a regression seed)

| Seed | Check | Cause | Fix |
|---|---|---|---|
| 415 | GroundSpoof | swimming down exactly flush onto the floor: vanilla's collide returns such a move unchanged (no collision, no ground); the engine's 1e-7 wall seek made it a collision | a movement that ends exactly on a surface, not cut by uncertainty, is not a vertical collision |
| 536 | GroundSpoof | sinking in water, the client stopped 5e-5 above the floor (Entity.move skips collided moves under sqrt(1e-7)) and stood | the client's ground claim is trusted within that gap (HOVER_GAP), as Grim trusts it at an exact touch |
| 1380 | NoFall | the same hover, claimed in a packet without a position | NoFall's near-ground box reaches HOVER_GAP down |
| 651 | Simulation 0.009 | sneak-held velocity on ice: Grim's SneakingEstimator keeps the larger of the old hidden velocity and the new input instead of their sum, falling behind on ice | the hidden velocity takes each tick's input |
| 1258 | Simulation 0.0017 | uncertainty cut a move to exactly where a head-height block stops it; the client really hit the block (velocity zeroed), the engine (and Grim) assumed it stopped short (velocity kept) | for one tick, either velocity is allowed (never more than the real one) |

Candidates are also ranked by the movement Entity.move applies (zero when
the collided move is under sqrt(1e-7)), not the raw collision.

### Block changes no longer suppress checks

Before, any unconfirmed block change near a player skipped its moves; a block
that kept changing (a redstone clock) kept the player unjudged forever, and
the player's own dig and place packets did the same for free. Now:

* the adapter passes each change's block state; `BlockView::state_shape`
  describes any state from Horizon's registry and shape cache;
* per nearby position the engine keeps the state the client settled in and
  each unconfirmed change, confirmed step by step;
* a client applies changes in order and answers a transaction only after
  everything sent before it, so the worlds it may show are the changes up to
  each unanswered transaction (not every combination of cells): a move is
  predicted in each (at most 16, physically equal ones once) and passes if
  any explains it;
* own digs and places: the client shows the server's state or its own guess
  until the server answers the cell; counted only within 8 blocks and 12 a
  second.

Adversarial scenarios (`tests/redstone_attacks.rs`), each with a legitimate
client (no alert expected) and a cheating one (alert expected):

| Scenario | Legit: alerts / longest unjudged run | Cheat caught |
|---|---|---|
| clock beside the player | 0 / at most 2 | fly: yes |
| a wall of 42 clocks along a walk | 0 / at most 2 | speed: yes |
| full, slab, air every tick on two cells | 0 / at most 2 | fly: yes |
| a 7x44 floor switching stone and ice | 0 / at most 2 | speed: yes |
| a clock with a 300 ms round trip | 0 / at most 2 | fly: yes |
| a block changing every tick, 1.5 s round trip (31 worlds) | 0 / unjudged while it lasts, near that block only | speed leaving the area: yes |
| 5,000 changes a tick 40 blocks away | 0 / 0 | fly: yes |
| own place/dig spam around a fly hack | — | yes (hidden before) |
| bridging with answered placements | 0 / at most one move (the first cell) | — |

### Crash handling

Audit: no panic is reachable from packet contents in the engine as written
(coordinates are clamped or checked for NaN, slots and faces matched,
lookups checked; overflow wraps in Horizon's release profiles), but the first
guard version turned the whole anti-cheat off on any panic, which would give
one player that power, and left the player's state lost but connected and
unchecked. Now (`recovery.rs`, ARCHITECTURE.md): a panic while checking one
player kicks only that player after forgetting the connection, and tells
staff; a panic in a shared hook rebuilds the engine (players online
unchecked until they rejoin, staff told); three such within ten minutes
turn it off with a staff reminder every minute. Tested with injected panics
(`tests/recovery.rs`); not exercised on a live server.

### Performance and memory (A/B on the same Mac, same session)

Keeping state histories only near the player and only for known states
(far or stateless changes are 16-byte "unknown until N" entries again):

| bench_load, 100 players | before redstone work (0b103c6) | first version | now |
|---|---|---|---|
| TNT 2,000 changes/tick: ms per tick | 12.45 | 13.06 | 10.38 |
| TNT: heap per player | 73.6 KB | 202 KB | 73.8 KB |
| all stresses at once: ms per tick | 11.81 | 12.68 | 10.66 |
| block beside each player: moves not judged | 22,614 | 34 | 34 (now judged in 13,873 moves' worlds) |

bench_blocks (100 players, 2,000 changes per tick each): 20.0-21.5 ms before,
21.8-22.1 ms now, back to back.

## Next (after the live milestone)

1. Run `tools/smoke_test.py` with prepared game files (BUILD.md section 5),
   then play with a real vanilla 26.3 client in alert-only mode; move
   PORT_STATUS rows to live-tested only then. No new checks before this.
2. The first change of a cell is still unknown (one round trip unjudged);
   reading the state the chunk was sent with would close that.
3. Shape lookups for pending states are not cached (`state_shape` per move
   near changes).
4. Ladder, honey and explosion traces; elytra and vehicles.
5. Propose named server hook points upstream (COMPATIBILITY.md).
