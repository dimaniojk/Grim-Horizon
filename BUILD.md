# Building and installing

Grim-Horizon builds a Horizon dedicated server with the anti-cheat inside it.
Nothing of Mojang's is downloaded or distributed by these steps.

## Requirements

* Rust 1.99.1 (`rustup toolchain install 1.99.1`; Horizon's launcher pins the
  same version for native builds). horizon-mixins pins 1.95.0 for itself and
  rustup installs it automatically.
* git, Python 3.9+, rsync.
* About 4 GB of disk for the Horizon build.

## 1. Get the sources

```sh
mkdir upstream && cd upstream
git clone https://github.com/horizon-rust/horizon
git clone https://github.com/horizon-rust/horizon-mixins
cd ..
```

The Horizon checkout must contain the commit pinned in
`integration/horizon/grim-horizon/horizon-mixin.toml`
(`c5410785c0a0bc9b17eefa85fb108793e57d1d38`). The build never modifies it.

## 2. Test the engine (no Horizon needed)

```sh
cargo test --release
cargo run --release --example bench
```

## 3. Build the server

```sh
python3 tools/build_server.py --horizon upstream/horizon --mixins upstream/horizon-mixins --profile fast
```

The script exports Horizon at the pinned commit (`git archive`), assembles
the bundle (`integration/horizon/grim-horizon` plus `crates/grim-engine` as
its `engine/` crate), stages the modded source with `horizon-mixins apply`,
and builds `horizon-client` without audio and controllers (Horizon's own
server build). The result is `dist/horizon-engine-grim`. `--profile release`
gives Horizon's release profile; `--check-only` only type-checks.

On macOS horizon-mixins cannot replace a stage in place, so the script
stages into a fresh folder and syncs it into `work/staged` by content, which
keeps Cargo's incremental builds working.

## 4. Install

`horizon-engine-grim` is a drop-in replacement for Horizon's `horizon-engine`
(or `horizon-client --dedicated`):

* with `horizon-server`: put it beside `horizon-server` as `horizon-engine`,
  or set `[engine] path` in `server.toml`;
* by hand: `horizon-engine-grim --resources <prepared 26.3 resources> --dedicated --nogui`
  in the server folder.

On the first start it writes `grim-horizon.toml` (a copy is in
`configs/grim-horizon.toml`) in the server folder. It starts in
**alert-only** mode: operators (level 2+) get alerts in chat and every alert
goes to the server log; nobody is set back or kicked. Change `mode` to
`setback` or `punish` only after watching alerts on your server. `/grim
reload` (or `grim reload` on the console) re-reads the file.

`GRIM_HORIZON_DISABLE=1` in the environment turns the anti-cheat off without
rebuilding.

### Commands

`/grim alerts`, `/grim verbose`, `/grim profile <player>`, `/grim debug <player>`,
`/grim checks`, `/grim exempt <player>`, `/grim reload`, `/grim stats`, for
operators at `permission_level`. The same commands work on the server console
without the slash. Clients show `/grim` in red because it is not in the
server's command tree; it still runs.

## 5. Smoke test on a real server

The server needs Horizon's prepared resources for Minecraft 26.3, made from
your own Minecraft installation:

```sh
# from a Horizon release, or a build of upstream/horizon:
horizon-prepare --prepare --server-jar /path/to/your/26.3/server.jar
```

(see Horizon's `docs/hosting.md`; it needs a JDK and the official
launcher's 26.3 client files). Then build the bots and run the test:

```sh
(cd work/staged && CARGO_TARGET_DIR=../target cargo build --locked --profile fast -p horizon-bots)
python3 tools/smoke_test.py --resources /path/to/.local/horizon-client/26.3 \
    --bots work/target/fast/horizon-bots --accept-eula
```

`--accept-eula` writes `eula=true` in a scratch server folder: passing it
means you accept the Minecraft EULA (https://aka.ms/MinecraftEULA). The test
starts the server in a temporary folder (never your worlds), runs
`grim checks` on the console, connects two of Horizon's protocol bots, which
fly upwards in a survival world without permission, and expects Simulation
alerts, answered transactions and a clean stop. The server log is kept in
`work/smoke-server.log`.

For legit movement, join the smoke server with a vanilla 26.3 client and
play; `/grim debug <you>` prints each prediction's offset.
