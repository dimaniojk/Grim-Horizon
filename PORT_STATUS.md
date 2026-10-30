# Port status

Two separate questions are answered for every check:

* **Engine**: does the Rust implementation work against packet traces? The
  traces come from an independent model of the vanilla 26.3 client
  (`tests/common/mod.rs`, checked against published vanilla numbers: 4.317 m/s
  walk, 5.612 m/s sprint, 0.42 jump, 1.2522 jump height, the fall sequence) and
  from scripted cheats.
* **Live**: has it been seen working on a real Horizon dedicated server with
  real clients? **Not yet for any check**: the server needs Minecraft 26.3's
  prepared resources, which are not on this machine (see BUILD.md, "Smoke
  test"). The integration compiles into the real server binary and the binary
  starts; the hooks have not run against a live client.

Status words: IMPLEMENTED AND TESTED · IMPLEMENTED BUT UNVERIFIED ·
PARTIALLY IMPLEMENTED · BLOCKED · NOT IMPLEMENTED.

## Checks

| Grim check (config name) | Engine | Live | Tests and notes |
|---|---|---|---|
| Simulation | IMPLEMENTED AND TESTED | unverified | Walking, sprinting, diagonal input, jumping, sprint-jumping, falling, sneaking at edges, ice, soul sand, still water and swimming, slime bounces, slabs/steps, walls, teleports; 300 seeds × 6000 ticks of random legit play over slabs, holes, ice and a pool: 298 seeds clean (see Limitations). Speed and fly cheats flag. |
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
| Teleports and setbacks | IMPLEMENTED AND TESTED (engine: superseded, stale, respawn and unsent setbacks; step-up refused on the first move after); setbacks are off in the default alert-only mode; not run live |
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
| Client world compensation | PARTIALLY IMPLEMENTED (uncertain blocks are skipped, not replayed) |
| Violations, decay, alerts, verbose, debug, profile, exemptions, config reload | IMPLEMENTED AND TESTED (engine) |
| Punishments | IMPLEMENTED AND TESTED: per-check `kick_vl` in `punish` mode only |
| `/grim` for players | IMPLEMENTED BUT UNVERIFIED; the client gets no tab completion (the command is not in Horizon's command tree) |
| `grim` on the server console | IMPLEMENTED BUT UNVERIFIED |
| Permissions | Operator level (`permission_level`, default 2); Horizon has no permission nodes |

## Limitations found by the stress test

`tests/random_play.rs` runs the vanilla-model client with random inputs. 298 of
300 seeds (6000 ticks each, ~1.8 million ticks) produce no flag. The two that
do are the first movement after a server teleport while the client still
carries stale ground and supporting-block state; in alert-only mode they show
as single low Simulation alerts.

The reference client is a model of vanilla written for these tests, not
vanilla itself. Every disagreement found so far was investigated, and each one
turned out to be either a bug in the model or a real engine bug; both kinds
were fixed (see PROGRESS.md). Live testing with real clients is still required
before setbacks or kicks are enabled.
