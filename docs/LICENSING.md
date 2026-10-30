# Licensing

Grim-Horizon is licensed under the **GNU General Public License, version 3
only** (`GPL-3.0-only`); `LICENSE` is the GPLv3 text. Every source file carries
an SPDX identifier.

## Why GPL-3.0-only

* **Grim Anticheat** (https://github.com/GrimAnticheat/Grim) is distributed
  with the GPLv3 text as its `LICENSE`. A few of its files grant "version 3,
  or (at your option) any later version"; most carry no header, so for them
  version 3 is the license given.
* The engine (`crates/grim-engine`) translates Grim's Java (movement
  prediction, uncertainty handling, collisions, checks) into Rust and is a
  derivative work of Grim. It cannot offer an "any later version" option for
  Grim's code that Grim itself does not offer, so the whole project is
  GPL-3.0-only. Grim's contributors are credited in the file headers
  ("GrimAC contributors (original Java)") and in the README.
* The Horizon adapter (`integration/horizon/grim-horizon`) and the tools are
  original code that links or drives the engine, under the same license.

## Third-party code

* **Horizon** (https://github.com/horizon-rust/horizon) is MIT-licensed.
  `tools/grim-bot` adapts the handshake, login and configuration code of
  Horizon's `horizon-bots`; that part keeps its MIT notice
  (`THIRD_PARTY_NOTICES.md`), which is why `grim-bot` is
  `GPL-3.0-only AND MIT`. MIT code may be combined with GPL code.
* **horizon-mixins** (MIT) is the tool that builds the integration bundle; no
  code of it is copied here.
* Rust crates from crates.io keep their own licenses (see `Cargo.lock`).

## Distributing a built server

A server binary built with `tools/build_server.py` links Horizon (MIT) with
the engine (GPL). If you distribute that binary, the combined program is
under the GPL's terms:

* give recipients the complete corresponding source: this repository, the
  Horizon and horizon-mixins commits it was built from (COMPATIBILITY.md),
  and the build script;
* keep Horizon's MIT notice and this project's GPL notice;
* recipients may modify and redistribute under the same terms.

Running a server for players who connect over the network is not
distribution (the GPL is not the AGPL); publishing the binary is.

## Mojang

Nothing here contains or downloads Minecraft's code or assets. Horizon's
prepared game files come from the server owner's own Minecraft installation
and must not be redistributed. Running a server requires accepting the
Minecraft EULA, which is the server owner's decision.
