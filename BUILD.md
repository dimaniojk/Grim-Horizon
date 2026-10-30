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

## 5. Live test on a real server

Nothing in this repository has run against a live server yet: it needs game
files only you can provide, and a EULA only you can accept. These are the
exact steps (all on your own machine; nothing touches your worlds).

1. **Minecraft 26.3, from the official launcher.** Install Minecraft Java
   Edition with the official launcher (https://www.minecraft.net/download),
   sign in with the Microsoft account that owns the game, select release
   26.3 and play it once, so the launcher keeps its files in
   `~/Library/Application Support/minecraft` (macOS) or `~/.minecraft`.
2. **The 26.3 server jar.** In the launcher's
   `versions/26.3/26.3.json`, `downloads.server.url` is Mojang's official
   download for it. Save it anywhere.
3. **A Java 25 runtime** (`java -version`; Temurin 25 is already on this Mac).
4. **Prepare Horizon's game files** from your copy (Horizon's `docs/hosting.md`,
   step 2), with the `horizon-prepare` of Horizon's pinned commit:

   ```sh
   (cd ../upstream/horizon && cargo run --release -p horizon-prepare -- \
       --prepare --server-jar /path/to/server.jar)
   # -> ../upstream/horizon/.local/horizon-client/26.3
   ```

   Add `--minecraft-dir PATH` if your launcher keeps the game elsewhere.
5. **Build the server** (section 3) and the test player:

   ```sh
   cargo build --release --offline --manifest-path tools/grim-bot/Cargo.toml
   ```

6. **Decide on the Minecraft EULA.** A server refuses to start until its
   `eula.txt` says `eula=true`. Read https://aka.ms/MinecraftEULA; if you
   accept it, pass `--accept-eula` below. The flag writes `eula=true` in a
   temporary server folder only, and nowhere else.
7. **Run:**

   ```sh
   python3 tools/smoke_test.py --resources ../upstream/horizon/.local/horizon-client/26.3 --accept-eula
   ```

The test starts the server in a new temporary folder (its own HORIZON_HOME and
XDG folders, a superflat world) and connects `grim-bot` players. They speak the
real protocol and move with the same vanilla-model client the engine's tests
use. It checks, each on its own line: startup, the console, joining, a
legitimate player walking/jumping/sprinting/sneaking with no alert, a speed
cheater flagged, staff alerts reaching an operator in game, a console
teleport, death and respawn, the Nether and back, setback mode (cheater pulled
back, legitimate player never), punish mode (kick, reconnect, judged and
kicked again), reconnecting cleanly, ping transactions, no long unjudged runs,
and a clean stop. Logs go to `work/smoke/`. It prints `LIVE SMOKE TEST
PASSED` only when every check passed on the running server. Without
`--accept-eula` or game files it prints `NOT RUN`.

The bots are models, not the real client. After the script passes, join the
same kind of server with a real vanilla 26.3 client and play; `/grim debug
<you>` prints each prediction's offset. Only that counts as real-client
testing.
