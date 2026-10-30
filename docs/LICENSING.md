# Licensing

* **Grim Anticheat** is licensed under the GNU GPL version 3 (its files say
  "version 3, or (at your option) any later version").
* **Grim-Horizon's engine** (`crates/grim-engine`) and adapter
  (`integration/horizon/grim-horizon`) translate Grim's Java into Rust: they
  are derivative works and are licensed **GPL-3.0-or-later**, with Grim's
  contributors credited in every file header. They are not relicensed under
  MIT. `LICENSE` is the GPLv3 text.
* **Horizon** and **horizon-mixins** are MIT-licensed. MIT code may be
  combined with GPL code.

## Distributing a built server

`dist/horizon-engine-grim` links Horizon (MIT) with the engine (GPL). The
combined program is distributed under the GPL's terms:

* give recipients the complete corresponding source: this repository, the
  Horizon commit it was built from (pinned in `horizon-mixin.toml`), the
  horizon-mixins commit, and the build script (`tools/build_server.py`
  reproduces the staged source);
* keep Horizon's MIT notice and Grim's GPL notice;
* recipients may modify and redistribute under the same terms.

Running the server privately or for players who connect over the network does
not trigger the GPL's distribution duties (the GPL is not the AGPL), but
publishing the binary does.

## Mojang

Nothing here contains or downloads Minecraft's code or assets. Horizon's
prepared resources come from the server owner's own Minecraft files and must
not be redistributed. Running a server requires accepting the Minecraft EULA.
