# Grim-Horizon

A native Rust port of [Grim Anticheat](https://github.com/GrimAnticheat/Grim)'s
predictive movement engine and checks, built into the
[Horizon](https://github.com/horizon-rust/horizon) dedicated server
(Minecraft Java 26.3). No JVM, no Bukkit, no proxy, nothing to install on the
client.

**Status: experimental.** The engine is ported and tested against simulated
vanilla clients; the server integration compiles into a working Horizon
server binary but has **not yet run with real players** (see PORT_STATUS.md).
It ships in alert-only mode: it never sets back or kicks unless you configure
it to.

| Document | |
|---|---|
| [BUILD.md](BUILD.md) | Build, install, commands, smoke test |
| [ARCHITECTURE.md](ARCHITECTURE.md) | How the engine and the Horizon hooks fit together |
| [COMPATIBILITY.md](COMPATIBILITY.md) | Upstream commits, Horizon's integration points, why horizon-mixins |
| [PORT_STATUS.md](PORT_STATUS.md) | Every Grim check: implemented, tested, verified, missing |
| [PROGRESS.md](PROGRESS.md) | What was done, bugs found, what's next |
| [docs/LICENSING.md](docs/LICENSING.md) | GPLv3 and what it means for a built server |
| [benchmarks/README.md](benchmarks/README.md) | Measured engine cost |

```
crates/grim-engine/            the engine (GPL-3.0-or-later), no Horizon types
  src/prediction.rs            Grim's movement simulation, 26.3 path
  tests/                       vanilla-model client, scenarios, random play
  examples/bench.rs            benchmark
integration/horizon/grim-horizon/   horizon-mixins bundle: hooks + adapter
tools/build_server.py          reproducible server build
tools/smoke_test.py            end-to-end test on a real server
configs/grim-horizon.toml      default configuration
```

Quick start:

```sh
cargo test --release                       # the engine
python3 tools/build_server.py --horizon ../upstream/horizon --mixins ../upstream/horizon-mixins
# -> dist/horizon-engine-grim, a drop-in horizon-engine
```

Grim-Horizon is not affiliated with GrimAC, Horizon or Mojang.
