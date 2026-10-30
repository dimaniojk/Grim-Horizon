# Contributing

Grim-Horizon is experimental. The most useful contributions right now are
**reports from real servers**: false positives with the alert line and
`/grim debug` output, cheats that were not caught, crashes. Use the bug report
template.

## Before you change code

* Read ARCHITECTURE.md (how the engine and the Horizon hooks fit together) and
  PORT_STATUS.md (what is ported, what is tested, what is not).
* This is a port: when the engine and Grim disagree, Grim's Java
  (https://github.com/GrimAnticheat/Grim) and vanilla 26.3 are the reference.
  Say which one a change follows, and why when it departs from Grim.
* No new check categories until the existing ones have been validated on a
  live server (PROGRESS.md, "Next").

## Every change

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --release
```

CI runs the same. A bug fix comes with a test that fails without it: a
scenario in `crates/grim-engine/tests`, or a random-play seed in
`former_false_positive_seeds_are_clean`. For prediction changes, also run the
random-play sweep and report the result:

```sh
SEEDS=2000 cargo test --release --test random_play sweep -- --ignored --nocapture
```

A false positive is fixed by finding why the prediction differs from the
client, never by raising a threshold. Changes that skip judging a player must
stay bounded and counted (`grim stats`).

## Never commit

Minecraft's files: game or server jars, Horizon's prepared game files
(`.local/`), world saves, `eula.txt`, server configuration with personal data.

## License

Contributions are accepted under the project's license, GPL-3.0-only
(docs/LICENSING.md). New source files start with
`// SPDX-License-Identifier: GPL-3.0-only`.
