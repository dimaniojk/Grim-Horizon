# Port status

**Not production ready.** Nothing has run on a live server with a real client
yet (see "Live" below). Every check ships in alert-only mode by default.

Words used here:

* **Unit tested**: engine tests against packet traces from an independent
  model of the vanilla 26.3 client (`crates/grim-engine/tests`, checked against
  published vanilla numbers) and from scripted cheats, plus random play
  (`tests/random_play.rs`, 1,000 seeds x 6,000 ticks: 997 clean).
* **Integration tested**: through the Horizon adapter on a running Horizon
  server. `tools/smoke_test.py` does this with protocol bots for the rows
  marked "smoke test"; **it has not run** (no Minecraft 26.3 game files on
  this machine, and the EULA is the owner's decision; BUILD.md, section 5).
  The integration compiles into the real server binary, and the binary
  starts (`--initSettings`, scratch folder).
* **Live tested**: a real vanilla client on that server. **No check yet.**

## Readiness matrix

| Check | Implemented | Unit tested | Integration tested | Live tested | False-positive issues | Known bypasses | Remaining work |
|---|---|---|---|---|---|---|---|
| Simulation | yes | yes: movement, cheats, random play, load | smoke test (not run) | no | 3 of 1,000 random seeds (below); a hidden step right after a teleport is retried, not skipped | anything near unconfirmed blocks, unloaded chunks, while awaiting a setback, elytra/vehicles/riptide (exempt) | live test; old-and-new block states instead of skipping |
| GroundSpoof | yes | yes | not run | no | 2 of 1,000 seeds: shallow water at the floor (seeds 415, 536) | as Simulation | investigate the water-floor ground state |
| NoFall | yes | indirect only | not run | no | none seen | as Simulation | a dedicated trace |
| Timer (TimerA) | yes | yes (2x timer, latency, bursts) | not run | no | none seen | uses tick-processing time, more lenient than Grim | live latency data |
| TickTimer | yes | yes | not run | no | none seen | — | — |
| AntiKB (Knockback) | yes | yes (incl. 8 ticks latency) | not run | no | none seen | — | live combat |
| AntiExplosion | yes | no trace | not run | no | unknown | — | an explosion trace |
| Reach | yes | yes | not run | no | none seen | server-side positions, not Grim's client entity replay: more lenient | live combat |
| Hitboxes | yes | yes | not run | no | none seen | as Reach | — |
| BadPacketsA, D, F, I, N, Y | yes | yes | not run | no | none seen | — | — |
| BadPacketsE | yes | indirect | not run | no | none seen | — | a trace |
| CrashA, CrashC | yes | CrashC yes | not run | no | none seen | Horizon kicks for NaN itself | — |
| SelfInteract, TransactionOrder, PacketOrderB | yes | yes | not run | no | none seen | — | — |
| AimModulo360 | yes | yes | not run | no | none seen (it caught two of this repo's own test bots) | — | — |
| AimDuplicateLook | yes | no trace | not run | no | unknown | — | a trace |
| Setbacks (SetbackTeleportUtil) | yes | yes: superseded, stale, respawn, unsent, step-up refused after an engine setback, jump resync | smoke test (not run) | no | none seen | — | live test before enabling |
| Punishments (kick at kick_vl) | yes | yes | smoke test (not run) | no | — | — | set kick_vl per check after live data |
| Staff alerts, commands, profile, stats | yes | yes | smoke test (not run) | no | — | — | — |
| BadPacketsO (keep-alive) | blocked | — | — | — | — | — | needs a Horizon hook in `lan/conn.rs` |
| Other Grim checks (below) | no | — | — | — | — | — | not started; no new checks until the above is live-validated |

## Checks in detail

| Grim check (config name) | Engine | Live | Tests and notes |
|---|---|---|---|
| Simulation | IMPLEMENTED AND TESTED | unverified | Walking, sprinting, diagonal input, jumping, sprint-jumping, falling, sneaking at edges, ice, soul sand, still water and swimming, slime bounces, slabs/steps, walls, teleports; 1,000 seeds × 6,000 ticks of random legit play over slabs, holes, ice and a pool: 997 seeds clean (see Limitations). Speed and fly cheats flag. |
| GroundSpoof | IMPLEMENTED AND TESTED | unverified | Claiming ground while falling flags; legit play does not. |
| NoFall | IMPLEMENTED BUT UNVERIFIED | unverified | Packet NoFall for ground claims without a position (Grim's `isNearGround`). Covered indirectly; no dedicated cheat trace yet. |
| Timer (TimerA) | IMPLEMENTED AND TESTED | unverified | 2x timer flags; 20 TPS with latency and bursts after a stall do not. Uses tick-processing time (Horizon batches packets per tick), which can only make it more lenient. |
| TickTimer | IMPLEMENTED AND TESTED | unverified | Two movements before a tick end flag. |
| AntiKB (Knockback) | IMPLEMENTED AND TESTED | unverified | Taken knockback clean (also at 8 ticks latency); ignored knockback flags. |
| AntiExplosion (Explosion) | IMPLEMENTED BUT UNVERIFIED | unverified | Same machinery as AntiKB; no explosion trace yet. |
| Reach | IMPLEMENTED AND TESTED | unverified | 3-block reach, latency window, moving targets. Targets are server positions over the window the client may display (not Grim's client-side entity replay). |
| Hitboxes | IMPLEMENTED AND TESTED | unverified | Hitting behind the player flags. |
| BadPacketsA | IMPLEMENTED AND TESTED | unverified | Duplicate slot. |
| BadPacketsD | IMPLEMENTED AND TESTED | unverified | Pitch outside ±90. |
| BadPacketsE | IMPLEMENTED BUT UNVERIFIED | unverified | >19 movement packets without a position. |
| BadPacketsF | IMPLEMENTED AND TESTED | unverified | Duplicate sprint command. |
| BadPacketsI | IMPLEMENTED AND TESTED | unverified | Flying without permission. |
| BadPacketsN | IMPLEMENTED AND TESTED | unverified | Accepting a teleport that was never sent. |
| BadPacketsY | IMPLEMENTED AND TESTED | unverified | Slot outside 0-8. |
| CrashA | IMPLEMENTED BUT UNVERIFIED | unverified | Position beyond the 30M border. |
| CrashC | IMPLEMENTED AND TESTED | unverified | NaN/infinite values (Horizon also kicks for them). |
| SelfInteract | IMPLEMENTED AND TESTED | unverified | |
| TransactionOrder | IMPLEMENTED AND TESTED | unverified | Unknown or skipped pong ids. |
| AimModulo360 | IMPLEMENTED AND TESTED | unverified | Found by a test that wrapped yaw like a cheat would. |
| AimDuplicateLook | IMPLEMENTED BUT UNVERIFIED | unverified | |
| PacketOrderB | IMPLEMENTED AND TESTED | unverified | Attack without swing. |
| BadPacketsG | NOT IMPLEMENTED | — | Not applicable to 1.21.2+ clients in Grim either (sneaking comes from PlayerInput). |
| BadPacketsO (keep-alive) | BLOCKED | — | Horizon answers keep-alives on the network thread; they never reach the hub. Needs a Horizon hook in `lan/conn.rs` `read_loop`. |
| BadPacketsB, C, H, J, K, L, M, P, Q, R, S, U, V, X, Z | NOT IMPLEMENTED | — | Need inventory, rotation-packet, block-placement or vehicle state. |
| Sprint A-G, NoSlow | NOT IMPLEMENTED | — | Need hunger, item-use and blindness state (the item-use slowdown is already simulated in the prediction). |
| Elytra A-I, FlightA, Phase | NOT IMPLEMENTED | — | Elytra gliding and creative flight are exempt from prediction (Grim also does not predict flight). |
| Vehicle A-F, VehicleTimer | NOT IMPLEMENTED | — | Vehicles are not simulated; riding players are exempt. |
| Crash B, D, E, F, G, H, I; Exploit A, B; Chat A-D | NOT IMPLEMENTED | — | Inventory/chat/book packet validation. |
| MultiActions A-G, MultiInteract A-B, PacketOrder A, C-P, Post, TimerLimit, NegativeTimer | NOT IMPLEMENTED | — | |
| Place/Break checks (FarPlace, FastBreak, ...), Baritone, AimProcessor | NOT IMPLEMENTED | — | |

## Subsystems

| Subsystem | Status |
|---|---|
| Packet ingestion in client order (Horizon hooks) | IMPLEMENTED BUT UNVERIFIED (compiles into the real server; not run live) |
| Outgoing packet observation | IMPLEMENTED BUT UNVERIFIED (sender routing unit-tested; never drops teleports or pings) |
| Transactions (ping/pong), latency, timeouts | IMPLEMENTED AND TESTED (engine) |
| Teleports and setbacks | IMPLEMENTED AND TESTED (engine: superseded, stale, respawn and unsent setbacks; step-up refused on the first move after an engine setback; Grim's post-setback jump resync; the unreported first step after a teleport); setbacks are off in the default alert-only mode; not run live |
| Movement prediction: ground, air, jump, sprint, sneak, crouch slowdown, item-use slowdown, attributes (speed, gravity, jump strength, step height, scale, sneaking speed, friction/drag modifiers, bounciness), effects (jump boost, levitation, slow falling, dolphin's grace) | IMPLEMENTED AND TESTED |
| Water, swimming, swim hop, lava | Water IMPLEMENTED AND TESTED; lava IMPLEMENTED BUT UNVERIFIED |
| Climbing (ladders, vines, scaffolding) | IMPLEMENTED BUT UNVERIFIED (no ladder trace yet; open trapdoors above ladders not detected) |
| Slime, honey, ice, soul sand, beds | Slime/ice/soul sand TESTED; honey and beds UNVERIFIED; honey-wall sliding NOT IMPLEMENTED |
| Cobwebs, berry bushes, powder snow (stuck speed) | IMPLEMENTED BUT UNVERIFIED |
| Bubble columns | IMPLEMENTED BUT UNVERIFIED |
| Collisions with step-up | IMPLEMENTED AND TESTED |
| Knockback / explosions | Knockback TESTED, explosions UNVERIFIED |
| Elytra, riptide, fireworks, vehicles, pistons, entity pushing, shulkers, potent sulfur geyser | NOT IMPLEMENTED (the player is not judged while affected) |
| World border collisions | NOT IMPLEMENTED (Horizon's scan does not include the border) |
| Client world compensation | PARTIALLY IMPLEMENTED (uncertain blocks are skipped, not replayed; by position, then by section, with metrics) |
| Violations, decay, alerts, verbose, debug, profile, exemptions, config reload | IMPLEMENTED AND TESTED (engine) |
| Punishments | IMPLEMENTED AND TESTED: per-check `kick_vl` in `punish` mode only |
| `/grim` for players | IMPLEMENTED BUT UNVERIFIED; the client gets no tab completion (the command is not in Horizon's command tree) |
| `grim` on the server console | IMPLEMENTED BUT UNVERIFIED |
| Permissions | Operator level (`permission_level`, default 2); Horizon has no permission nodes |

## Limitations found by the stress test

`tests/random_play.rs` runs the vanilla-model client with random inputs. 997
of 1,000 seeds (6,000 ticks each, 6 million ticks) produce no flag. Every
earlier failing seed was traced to its cause and fixed (PROGRESS.md); the
regression test `former_false_positive_seeds_are_clean` replays them. Open:

* seeds 415 and 536: GroundSpoof, a swimming or standing player in shallow
  water right at the floor (movements of 0.003 to 0.005 blocks);
* seed 651: Simulation 0.009, a player who stops sneaking at an ice edge
  while carrying velocity the edge hid.

Fail-open (movement deliberately not judged), measured and visible:

* near block changes the client has not confirmed. Overflowing changes fall
  back to their 16x16x16 section, never to the whole world unless 1,024
  sections are pending. A block that keeps changing next to a player (a
  redstone clock) keeps that player unjudged as long as it runs: a known
  bypass. It is counted (`grim stats`) and logged once per 10 s run;
* in chunks the client has not been sent;
* while a setback is awaited (bounded: superseded by any newer teleport, a
  respawn, or 40 ticks without the teleport being sent).

The reference client is a model of vanilla written for these tests, not
vanilla itself, and the bots of the live test use the same model. Live
testing with real clients is still required before setbacks or kicks are
enabled.
