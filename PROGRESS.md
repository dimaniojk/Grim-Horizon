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

* 68 automated tests, including published vanilla numbers, cheat traces,
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

## Next

1. Run the smoke test with prepared resources and a real vanilla client;
   watch alerts in alert-only mode; move PORT_STATUS rows from
   "unverified" to verified only then.
2. Close the two remaining post-teleport mispredictions.
3. Ladder, honey and explosion traces; elytra and vehicles.
4. Hot-path allocation (reuse collision buffers).
5. Propose named server hook points upstream (COMPATIBILITY.md).
