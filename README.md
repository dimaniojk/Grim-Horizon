# Grim-Horizon

An experimental native Rust port of [Grim Anticheat](https://github.com/GrimAnticheat/Grim)
for [Horizon](https://github.com/horizon-rust/horizon) Minecraft dedicated
servers (Minecraft Java Edition 26.3). Grim's predictive movement engine and
its checks run inside the server process: no JVM, no Bukkit, no proxy,
nothing to install on the client.

> [!WARNING]
> **Experimental, not production-ready.** The engine is ported and tested
> against a model of the vanilla client, and the integration compiles into a
> working Horizon server binary, but **it has not yet run on a live server
> with real players.** It ships in alert-only mode: it never sets anyone back
> or kicks anyone unless you configure it to. Not affiliated with or endorsed
> by GrimAC, Horizon or Mojang; this is not an official Horizon component.

## What it does

* **Movement prediction** ported from Grim (`MovementCheckRunner`,
  `PredictionEngine`, `UncertaintyHandler`, `PointThreeEstimator`,
  `SneakingEstimator`, collisions with step-up, fluids, knockback and
  explosions) for players on foot: walking, sprinting, jumping, sneaking,
  swimming, ice, slime, soul sand, slabs, attributes and effects.
* **Checks**: Simulation, GroundSpoof, NoFall, Timer, TickTimer, AntiKB,
  AntiExplosion, Reach, Hitboxes, BadPackets (A, D, E, F, I, N, Y), Crash (A,
  C), SelfInteract, TransactionOrder, PacketOrderB, AimModulo360,
  AimDuplicateLook. Every row with its status: [PORT_STATUS.md](PORT_STATUS.md).
* **Transactions** (ping/pong) to know what the client has applied, setbacks
  following Grim's `SetbackTeleportUtil`, violation levels, alerts to staff,
  `/grim` commands (`alerts`, `verbose`, `debug`, `profile`, `checks`,
  `stats`, `exempt`, `reload`), and a TOML configuration with three modes:
  `alert-only` (default), `setback`, `punish`.
* **Block changes** the client may not have applied yet are judged in each
  world it may still see instead of being skipped, so changing blocks
  (redstone clocks, TNT, placements) do not switch checks off.
* **Crash isolation**: a panic while checking one player disconnects that
  player only; staff are told.

## Status and exact limitations

| | |
|---|---|
| Automated tests | 112 engine tests; 2,000 seeds x 6,000 ticks of random legitimate play with no false positive; adversarial block-change scenarios; injected panics |
| Server binary | builds from Horizon's pinned commit; starts; refuses to run without the EULA |
| Live server | **not run.** No result from a real Horizon server or a real vanilla client exists yet |

Limitations, measured and documented in [PORT_STATUS.md](PORT_STATUS.md):

* Tested only against a model of the vanilla 26.3 client written for this
  project, not the real client.
* Not judged (by design, counted in `grim stats`): elytra, vehicles, riptide,
  creative flight; chunks the player has not been sent; one round trip near a
  block whose first change is unconfirmed; near a block changing every tick
  when the player's round trip is over about 800 ms (only within about a
  block of it).
* Reach and Hitboxes use server-side entity positions over the latency
  window, not Grim's client-side entity replay: more lenient than Grim.
* Timer uses tick-processing time (Horizon batches packets per tick): more
  lenient than Grim.
* Most of Grim's other checks (inventory, placement, elytra, vehicles, chat,
  most BadPackets and PacketOrder) are not ported. BadPacketsO needs a Horizon
  hook that does not exist.
* Horizon hooks are applied with [horizon-mixins](https://github.com/horizon-rust/horizon-mixins)
  at one pinned Horizon commit (`c5410785`); other Horizon versions are
  untested.

## Build

Requirements: Rust 1.99.1 (edition 2024), Python 3, git, and for the server
the Horizon and horizon-mixins sources next to this repository:

```sh
mkdir grimhorrizon && cd grimhorrizon
git clone https://github.com/<owner>/Grim-Horizon grim-horizon
mkdir upstream
git clone https://github.com/horizon-rust/horizon upstream/horizon
git clone https://github.com/horizon-rust/horizon-mixins upstream/horizon-mixins
cd grim-horizon
```

The engine alone (no Horizon, no Minecraft files):

```sh
cargo build --release
```

The Horizon dedicated server with Grim-Horizon built in:

```sh
python3 tools/build_server.py --horizon ../upstream/horizon --mixins ../upstream/horizon-mixins
# -> dist/horizon-engine-grim (Horizon's horizon-engine with the anti-cheat)
```

Installing it, the configuration and the commands: [BUILD.md](BUILD.md).
Running a server needs Minecraft 26.3 game files prepared from your own
Minecraft installation (Horizon's `horizon-prepare`) and your acceptance of
the Minecraft EULA; this repository contains neither.

## Test

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --release
```

The tests drive the engine with packets from an independent model of the
vanilla client (`crates/grim-engine/tests/common`, checked against published
vanilla numbers: 4.317 m/s walking, 5.612 m/s sprinting, 1.2522-block jump)
and with scripted cheats. Random legitimate play over slabs, holes, ice and
water:

```sh
SEEDS=2000 cargo test --release --test random_play sweep -- --ignored --nocapture
```

The live test on a real server (`tools/smoke_test.py`, 27 checks with
protocol bots, `tools/grim-bot`) needs your prepared 26.3 game files and your
EULA decision: BUILD.md, section 5. It has not been run yet.

CI runs formatting, Clippy and the engine tests on every push; it needs no
Minecraft files.

## Performance

Engine work only (every `on_incoming`, `on_outgoing` and `end_tick` call),
measured on an Apple M1 (macOS, Rust 1.99.1, release profile) with
`cargo run --release --example bench_load`: vanilla-model players walking,
jumping, sprinting and sneaking at random for 1,200 ticks in an in-memory
world. A real server tick adds Horizon's own work (world, chunks, network,
its collision scan), which only a live run measures.

| Scenario | Players | avg ms/tick | p99 ms/tick | Heap per player | Alerts on legitimate players |
|---|---|---|---|---|---|
| walking | 10 | 0.105 | 0.248 | 32.8 KB | 0 |
| walking | 50 | 0.530 | 0.727 | 5.8 KB | 0 |
| walking | 100 | 1.079 | 1.350 | 5.7 KB | 0 |
| TNT: 2,000 block changes per player per tick | 100 | 10.378 | 11.326 | 73.8 KB | 0 |
| teleport bursts, reconnect storm, network jitter, changing blocks and TNT at once | 100 | 10.658 | 13.857 | 76.3 KB | 0 |

One prediction costs about 6-10 µs. Methodology, every scenario and the
before/after history: [benchmarks/README.md](benchmarks/README.md).

## Repository

```
crates/grim-engine/                  the engine: no Horizon types
  src/prediction.rs                  Grim's movement simulation, 26.3 path
  src/world.rs                       client-world compensation
  tests/                             vanilla-model client, scenarios, random play
  examples/                          benchmarks
integration/horizon/grim-horizon/    horizon-mixins bundle: hooks and adapter
tools/build_server.py                reproducible server build
tools/smoke_test.py, tools/grim-bot/ live test on a real server
configs/grim-horizon.toml            default configuration
```

| Document | |
|---|---|
| [BUILD.md](BUILD.md) | Build, install, configuration, commands, live test |
| [ARCHITECTURE.md](ARCHITECTURE.md) | How the engine and the Horizon hooks fit together |
| [PORT_STATUS.md](PORT_STATUS.md) | Readiness matrix: every check, tested how, known gaps |
| [COMPATIBILITY.md](COMPATIBILITY.md) | Upstream commits and Horizon's integration points |
| [PROGRESS.md](PROGRESS.md) | What was done, bugs found and fixed, what is next |
| [CONTRIBUTING.md](CONTRIBUTING.md) | How to report problems and contribute |

## Credits

* **[Grim Anticheat](https://github.com/GrimAnticheat/Grim)** by DefineOutside
  and the GrimAC contributors: the prediction engine and checks this project
  ports. All credit for the design belongs to them; the bugs of the port are
  ours.
* **[Horizon](https://github.com/horizon-rust/horizon)** by Ludovic Marie and
  the Horizon contributors: the Rust Minecraft server it runs in; its
  `horizon-bots` code is adapted in `tools/grim-bot` (MIT,
  [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)).
* **[horizon-mixins](https://github.com/horizon-rust/horizon-mixins)**: the
  tool that applies the integration to Horizon.

## License

GPL-3.0-only, as a derivative work of Grim Anticheat (GPLv3). See
[LICENSE](LICENSE) and [docs/LICENSING.md](docs/LICENSING.md), including what
the license means for a server binary you distribute. Minecraft is a
trademark of Mojang Synergies AB; nothing of Mojang's is included.
