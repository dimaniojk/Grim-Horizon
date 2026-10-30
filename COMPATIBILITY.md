# Compatibility report

What Grim-Horizon was built against, where it hooks into Horizon, and why it
integrates the way it does. Everything here was read from the source at the
commits below; no Horizon API was assumed.

## Upstream commits (inspected 2026-10-30)

| Repository | Commit | Notes |
|---|---|---|
| [GrimAnticheat/Grim](https://github.com/GrimAnticheat/Grim) | `c12f1071055ae6f55abd890eaf96fda11e58317b` (2026-10-28) | GPLv3. Already supports 26.3 clients (`ClientVersion.V_26_3`). The supplied `grimac-bukkit-2.3.74.jar` is a build of the same project; the port follows the source. |
| [horizon-rust/horizon](https://github.com/horizon-rust/horizon) | `c5410785c0a0bc9b17eefa85fb108793e57d1d38` (2026-10-30) | MIT. Minecraft Java 26.3, protocol 777. The bundle pins this commit. |
| [horizon-rust/horizon-native-mod-template](https://github.com/horizon-rust/horizon-native-mod-template) | `c8bfba72fb3c1f495d491d580e72cca7ae81de68` | Pins an older Horizon (`028f6088`) and horizon-mixins 0.5.0; used as a reference only. |
| [horizon-rust/horizon-mixins](https://github.com/horizon-rust/horizon-mixins) | `cb1ad23022392d88f006cf94cbf6552a73f5df12` (0.6.0) | Bundle crates (`[[crate]]`), named hooks, support modules. |

Toolchain used: Rust 1.99.1 (the version Horizon's launcher pins for native builds), on macOS arm64.

## Target game version

Horizon implements Minecraft Java **26.3**, so the port targets 26.3 clients
only and takes Grim's 26.3 code path everywhere: known inputs from
`ServerboundPlayerInputPacket` (1.21.2+), `ServerboundClientTickEndPacket`,
the 2e-4 movement threshold (1.18.2+, no "0.03"), the 1.21 step-up
algorithm, 1.21.5+ input normalisation and tiny-velocity zeroing, 1.21.11+
`Mth.sin` table, 26.2+ `friction_modifier`/`air_drag_modifier`/`bounciness`
attributes, the 26.3 `EntityFluidInteraction` and teleport-accept packets that
carry the position. Grim's version branches for older clients were not
ported; ViaVersion is irrelevant (Horizon speaks one protocol).

## Horizon's server architecture, as found

The dedicated server is the game binary run headless
(`horizon-client --dedicated`, `crates/horizon-client/src/game/dedicated.rs`).
Its players are handled by the same play hub as Open-to-LAN guests
(`crates/horizon-client/src/lan/`). The answers to the ten questions:

1. **Where inbound movement packets are decoded.** On each connection's reader
   thread: `lan/conn.rs` `read_loop` decodes with `play::Serverbound::decode`
   (`horizon-protocol/src/packets/play.rs`) and sends `NetEvent::Packet` to the
   game thread. `Hub::poll` (`lan/hub.rs`) pushes each packet into the guest's
   `Guest::inbox` (`lan/guest.rs`), a `VecDeque<play::Serverbound>`.
2. **Where movement is validated and applied.** `Hub::process_inbox` →
   `Hub::move_player` → `Hub::position_change` → `Hub::collide`
   (`lan/hub.rs`), a port of vanilla's `handleMovePlayer`: invalid values kick,
   "moved too quickly" (100/300 blocks²) and "moved wrongly" teleport back.
3. **How players are identified and tracked.** `Guest` (`lan/guest.rs`):
   `conn: u32` (connection id), `uuid`, `name`, `entity_id`, `permission`
   (operator level), `dimension`, `position`, `inbox`, `sender`. `Hub::guests`
   holds them; `Hub::guest(conn)` finds one.
4. **Where attacks and interactions are processed.** `process_inbox` forwards
   `S::Attack`/`S::Interact` as `HostEvent::Packet`; the game handles them in
   `game/lan_actions.rs` (`Game::guest_attack`, the `S::Interact` arm).
5. **Where server ticks occur.** `game/levels.rs` `Game::tick_server` runs
   each level; the hub's parts are `Hub::poll` (once), `Hub::tick_level` (per
   level: `process_inbox` for the level's guests, chunks, blocks, entities)
   and `Hub::end_tick` (once, called from `game/lan.rs`).
6. **Velocity, attributes, effects.** Server-side motion in `Guest::motion`
   (`PlayerMotion`), effects in `Guest::effects`; clients learn them through
   `ClientboundSetEntityMotion` (`game/lan_mobs.rs` `send_guest_motion`,
   `game/lan_actions.rs`), `UpdateAttributes`, `UpdateMobEffect`.
7. **Block collisions and bounding boxes.** `horizon-client-render/src/collisions.rs`
   `SceneCollisions` over `horizon-world/src/block_collision_scan.rs`
   `BlockCollisionScan` (vanilla `BlockCollisions`); `Guest::dimensions`/`bounds`.
8. **Teleport confirmations, ping, latency.** `Guest::awaiting_teleport`,
   `Hub::teleport`/`teleport_relative` (sends `PlayerPosition`, ignores moves
   until `AcceptTeleportation`, resends after 20 ticks). Keep-alives are
   answered on the network thread (`lan/conn.rs`, `Guest::latency`). Horizon
   sends no `ClientboundPing` in play, so the anti-cheat owns ping/pong
   transactions.
9. **Whether outgoing packets can be observed.** Yes: `PlayerSender::send`
   (`lan/conn.rs`) carries every `Guest::send` and every broadcast. Only
   `send_many` (chunk batches and the entity tracker's per-tick batches)
   bypasses it; neither carries the player's own movement state.
10. **Where movement or attacks can be cancelled or corrected.** Before
    `process_inbox` handles them, packets are still in `Guest::inbox` and can be
    removed or have their on-ground flag changed; `Hub::teleport` sets a player
    back; `Hub::kick` disconnects.

## Integration strategy

| Option | Verdict |
|---|---|
| **A. Official native server-side mod** (named hook points, `native-hooks.json`) | Not sufficient. The 12 named hooks (`tick`, `frame`, `hud`, `key`, `command`...) are client/UI hooks; none sees server packets. `docs/native-mods.md` says dedicated-server native builds are "not built yet" by the launcher. |
| **B. horizon-mixins bundle** | **Chosen.** horizon-mixins 0.6.0 can hook any non-generic function of Horizon's source with HEAD hooks, ship the engine as a bundle crate built against Horizon's own `Cargo.lock`, and add a support module. The same staged source builds the dedicated server (`cargo build -p horizon-client --no-default-features`). Horizon's checkout is never modified. |
| C. Patch Horizon + separate crate | Not needed. The bundle reaches every point listed above without a fork. Upstreaming named *server* hook points (see below) would make B independent of Horizon's internals. |

### The hooks (all HEAD; nothing in Horizon's bodies is rewritten)

| Hook | Target | What it does |
|---|---|---|
| `inbox` | `horizon_client::lan::hub::Hub::process_inbox` | Gives the guest's new packets to the engine in client order with the level's world, applies verdicts to the inbox (cancel, on-ground correction), runs `/grim` commands, performs setbacks/kicks/messages. |
| `inbox-done` | `Hub::sync_world` | After every guest of a level was handled: what is left in each inbox was already seen (packets Horizon defers to the next tick). |
| `level` | `Hub::tick_level` | Records every entity's and player's position for Reach/Hitboxes. |
| `end-tick` | `Hub::end_tick` | One ping transaction per player per tick, timeouts, forgets players who left. |
| `send` | `horizon_client::lan::conn::PlayerSender::send` | Decodes the few outgoing packets the engine needs (velocity, teleports, attributes, effects, abilities, game mode, respawn, health, block changes, passengers, its own pings). Only a separate outbox mutex is touched here, so Horizon calls made while the engine is locked can never deadlock. |
| `console` | `horizon_client::game::dedicated::Game<'_>::console_command` (cancellable) | `grim ...` on the server console. |
| `tick` | named hook `tick` v1 (`Game::native_tick`) | Registers entity sizes from Horizon's own entity table once; prints the adapter's log lines through the dedicated server's logger. |

### Upstream proposal (not opened: no remote changes were authorised)

Named, versioned server hook points in `native_hooks.rs`, e.g.
`native_server_packet(conn, &mut Serverbound, &mut Verdict)`,
`native_server_send(conn, &Clientbound)`, `native_server_tick_end()`, would
let any server mod (anti-cheats, logging, protection plugins) do what this
bundle does without depending on Horizon's private hub internals.
