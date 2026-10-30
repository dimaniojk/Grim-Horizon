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

* 83 automated tests, including published vanilla numbers, cheat traces,
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

## Next

1. Run the smoke test with prepared resources and a real vanilla client;
   watch alerts in alert-only mode; move PORT_STATUS rows from
   "unverified" to verified only then.
2. Close the two remaining post-teleport mispredictions.
3. Ladder, honey and explosion traces; elytra and vehicles.
4. Hot-path allocation (reuse collision buffers).
5. Propose named server hook points upstream (COMPATIBILITY.md).
