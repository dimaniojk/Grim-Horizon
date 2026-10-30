// SPDX-License-Identifier: GPL-3.0-only AND MIT (MIT parts: see THIRD_PARTY_NOTICES.md)
//! grim-bot: a player for Grim-Horizon's end-to-end test (tools/smoke_test.py).
//!
//! It joins a Horizon server offline, then moves with the vanilla-model
//! client the engine's own tests use (crates/grim-engine/tests/common), so a
//! `legit` bot is exactly as legitimate as the test suite's players, and a
//! `speed` bot is that player with 0.3 blocks a tick added. The model knows no
//! terrain, so the server must be a superflat overworld (top at y = -60) and
//! the bot keeps near its spawn; in the Nether it walks on the bedrock roof
//! (top at y = 128). It answers keep-alives, pings and teleports, respawns
//! when it dies, and prints one line per event for the test to read:
//!
//!   SPAWNED x y z | TELEPORT n x y z | RESPAWN dimension | CHAT text |
//!   DISCONNECTED reason | DONE
//!
//! The handshake, login and configuration code is adapted from Horizon's
//! horizon-bots (crates/horizon-bots, MIT License, Copyright (c) 2026 Ludovic
//! Marie and the Horizon contributors).

#[path = "../../../crates/grim-engine/tests/common/mod.rs"]
mod common;

use common::{Client, Keys};
use grim_horizon_engine::engine::Serverbound as GS;
use grim_horizon_engine::math::Vec3;
use grim_horizon_engine::world::{BlockFacts, TestWorld};
use horizon_protocol::{
    PROTOCOL_VERSION,
    buf::{Reader, Writer},
    connection::{ConnWriter, Connection},
    crypto::offline_uuid,
    ids,
    packets::{
        Empty, configuration,
        handshake::{self, Intention},
        login::{self, ServerHello},
        play,
    },
};
use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

enum Event {
    Login {
        entity: i32,
    },
    Teleport {
        id: i32,
        pos: [f64; 3],
        relatives: i32,
        yaw: f32,
        pitch: f32,
    },
    Respawn {
        dimension: String,
    },
    Health(f32),
    Ping(i32),
    Motion {
        entity: i32,
        velocity: [f64; 3],
    },
    Chat(String),
    Gone(String),
}

struct Options {
    host: String,
    port: u16,
    name: String,
    mode: String,
    seconds: u64,
}

fn options() -> Result<Options, String> {
    let mut o = Options {
        host: "127.0.0.1".into(),
        port: 25565,
        name: "GrimBot".into(),
        mode: "legit".into(),
        seconds: 30,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let v = args.next().ok_or(format!("{a} needs a value"))?;
        match a.as_str() {
            "--server" => {
                let (h, p) = v.rsplit_once(':').ok_or("--server HOST:PORT")?;
                o.host = h.into();
                o.port = p.parse().map_err(|_| "bad port")?;
            }
            "--name" => o.name = v,
            "--mode" => o.mode = v,
            "--seconds" => o.seconds = v.parse().map_err(|_| "bad seconds")?,
            _ => {
                return Err(format!(
                    "unknown option {a}; use --server --name --mode legit|speed|stand --seconds"
                ));
            }
        }
    }
    if !matches!(o.mode.as_str(), "legit" | "speed" | "stand") {
        return Err("--mode is legit, speed or stand".into());
    }
    Ok(o)
}

fn raw(id: i32, write: impl FnOnce(&mut Writer)) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(id);
    write(&mut w);
    w.data
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Handshake, offline login and configuration (from horizon-bots).
fn join(o: &Options) -> Result<Connection, String> {
    let mut c = Connection::connect(&o.host, o.port, Duration::from_secs(10)).map_err(err)?;
    c.send(
        &handshake::Serverbound::Intention(Intention {
            protocol: PROTOCOL_VERSION,
            host: o.host.clone(),
            port: o.port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )
    .map_err(err)?;
    c.send(
        &login::Serverbound::Hello(ServerHello {
            name: o.name.clone(),
            uuid: offline_uuid(&o.name),
        })
        .encode(),
    )
    .map_err(err)?;
    loop {
        let (id, body) = c.recv().map_err(err)?;
        match login::Clientbound::decode(id, &body).map_err(err)? {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(_) => {
                c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
                    .map_err(err)?;
                break;
            }
            login::Clientbound::Disconnect(p) => return Err(format!("refused: {}", p.reason_json)),
            login::Clientbound::Hello(_) => {
                return Err("the server is in online mode; the bot needs online-mode=false".into());
            }
            login::Clientbound::CustomQuery(p) => c
                .send(
                    &login::Serverbound::CustomQueryAnswer(login::CustomQueryAnswer {
                        transaction: p.transaction,
                        payload: None,
                    })
                    .encode(),
                )
                .map_err(err)?,
            login::Clientbound::CookieRequest(p) => c
                .send(
                    &login::Serverbound::CookieResponse(login::CookieResponse {
                        key: p.key,
                        payload: None,
                    })
                    .encode(),
                )
                .map_err(err)?,
            login::Clientbound::Unknown(..) => {}
        }
    }
    use ids::configuration::clientbound as cb;
    c.send(
        &configuration::Serverbound::ClientInformation(configuration::ClientInformation::new(4))
            .encode(),
    )
    .map_err(err)?;
    loop {
        let (id, body) = c.recv().map_err(err)?;
        if !matches!(
            id,
            cb::SELECT_KNOWN_PACKS
                | cb::KEEP_ALIVE
                | cb::PING
                | cb::CODE_OF_CONDUCT
                | cb::COOKIE_REQUEST
                | cb::DISCONNECT
                | cb::FINISH_CONFIGURATION
        ) {
            continue;
        }
        let reply = match configuration::Clientbound::decode(id, &body).map_err(err)? {
            configuration::Clientbound::SelectKnownPacks(p) => {
                configuration::Serverbound::SelectKnownPacks(p)
            }
            configuration::Clientbound::KeepAlive(p) => configuration::Serverbound::KeepAlive(p),
            configuration::Clientbound::Ping(p) => configuration::Serverbound::Pong(p),
            configuration::Clientbound::CodeOfConduct(_) => {
                configuration::Serverbound::AcceptCodeOfConduct(Empty)
            }
            configuration::Clientbound::CookieRequest(p) => {
                configuration::Serverbound::CookieResponse(configuration::CookieResponse {
                    key: p.key,
                    payload: None,
                })
            }
            configuration::Clientbound::Disconnect(_) => {
                return Err("disconnected during configuration".into());
            }
            configuration::Clientbound::FinishConfiguration(_) => {
                c.send(&configuration::Serverbound::FinishConfiguration(Empty).encode())
                    .map_err(err)?;
                return Ok(c);
            }
            _ => continue,
        };
        c.send(&reply.encode()).map_err(err)?;
    }
}

/// Answers what a vanilla client answers at once (keep-alives, pings, chunk
/// batches) and hands the rest to the movement loop, in order.
fn read(
    mut reader: horizon_protocol::connection::ConnReader,
    writer: Arc<Mutex<ConnWriter>>,
    events: mpsc::Sender<Event>,
) {
    use ids::play::clientbound as cb;
    use ids::play::serverbound as sb;
    let send = |body: Vec<u8>| writer.lock().unwrap_or_else(|e| e.into_inner()).send(&body);
    loop {
        let (id, body) = match reader.recv() {
            Ok(p) => p,
            Err(e) => {
                let _ = events.send(Event::Gone(e.to_string()));
                return;
            }
        };
        let mut r = Reader::new(&body);
        let sent = match id {
            cb::KEEP_ALIVE => r
                .i64()
                .map_or(Ok(()), |v| send(raw(sb::KEEP_ALIVE, |w| w.i64(v)))),
            // The anti-cheat's transactions: the vanilla client answers them on
            // its main thread, in order with what it applies (below).
            cb::PING => {
                if let Ok(v) = r.i32() {
                    let _ = events.send(Event::Ping(v));
                }
                Ok(())
            }
            cb::CHUNK_BATCH_FINISHED => send(
                play::Serverbound::ChunkBatchReceived(play::ChunkBatchReceived {
                    chunks_per_tick: 64.0,
                })
                .encode(),
            ),
            cb::DISCONNECT | cb::START_CONFIGURATION => {
                let reason = String::from_utf8_lossy(&body)
                    .chars()
                    .filter(|c| !c.is_control())
                    .collect();
                let _ = events.send(Event::Gone(reason));
                return;
            }
            _ => {
                let event = match play::Clientbound::decode(id, &body) {
                    Ok(play::Clientbound::Login(p)) => Some(Event::Login {
                        entity: p.player_id,
                    }),
                    Ok(play::Clientbound::PlayerPosition(p)) => Some(Event::Teleport {
                        id: p.teleport,
                        pos: p.position,
                        relatives: p.relatives,
                        yaw: p.yaw,
                        pitch: p.pitch,
                    }),
                    Ok(play::Clientbound::Respawn(p)) => Some(Event::Respawn {
                        dimension: p.spawn.dimension,
                    }),
                    Ok(play::Clientbound::SetHealth(p)) => Some(Event::Health(p.health)),
                    Ok(play::Clientbound::SystemChat(p)) => {
                        Some(Event::Chat(format!("{:?}", p.content)))
                    }
                    Ok(play::Clientbound::SetEntityMotion(p)) => Some(Event::Motion {
                        entity: p.id,
                        velocity: p.velocity,
                    }),
                    _ => None,
                };
                if let Some(e) = event {
                    let _ = events.send(e);
                }
                Ok(())
            }
        };
        if sent.is_err() {
            let _ = events.send(Event::Gone("write failed".into()));
            return;
        }
    }
}

/// The terrain the model knows: a flat floor whose top is at `top`.
fn floor_world(top: i32, around: Vec3) -> TestWorld {
    let mut w = TestWorld::new();
    w.min_y = -64;
    w.max_y = 320;
    let (x, z) = (around.x.floor() as i32, around.z.floor() as i32);
    w.floor(x - 64, x + 64, top - 1, z - 64, z + 64, BlockFacts::SOLID);
    w
}

fn surface(dimension: &str) -> Option<i32> {
    match dimension {
        "minecraft:overworld" => Some(-60),
        "minecraft:the_nether" => Some(128),
        _ => None,
    }
}

/// The model's packets in the protocol's form.
fn encode(p: &GS, entity: i32) -> Option<Vec<u8>> {
    let flags = |on_ground: bool| u8::from(on_ground);
    Some(match *p {
        GS::Move {
            pos: Some(v),
            rot: Some((yaw, pitch)),
            on_ground,
            ..
        } => play::Serverbound::MovePlayerPosRot(play::MovePlayerPosRot {
            x: v.x,
            y: v.y,
            z: v.z,
            yaw,
            pitch,
            flags: flags(on_ground),
        })
        .encode(),
        GS::Move {
            pos: Some(v),
            rot: None,
            on_ground,
            ..
        } => play::Serverbound::MovePlayerPos(play::MovePlayerPos {
            x: v.x,
            y: v.y,
            z: v.z,
            flags: flags(on_ground),
        })
        .encode(),
        GS::Move {
            pos: None,
            rot: Some((yaw, pitch)),
            on_ground,
            ..
        } => play::Serverbound::MovePlayerRot(play::MovePlayerRot {
            yaw,
            pitch,
            flags: flags(on_ground),
        })
        .encode(),
        GS::Move {
            pos: None,
            rot: None,
            on_ground,
            ..
        } => play::Serverbound::MovePlayerStatus(play::MovePlayerStatus {
            flags: flags(on_ground),
        })
        .encode(),
        GS::Input { flags } => play::Serverbound::PlayerInput(play::PlayerInput { flags }).encode(),
        GS::PlayerCommand { action } => play::Serverbound::PlayerCommand(play::PlayerCommand {
            id: entity,
            action,
            data: 0,
        })
        .encode(),
        GS::TickEnd => play::Serverbound::ClientTickEnd(Empty).encode(),
        _ => return None,
    })
}

/// A deterministic walk that stays within 12 blocks of `home`.
struct Walker {
    seed: u64,
    keys: Keys,
}

impl Walker {
    fn next(&mut self) -> u32 {
        self.seed = self
            .seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.seed >> 33) as u32
    }
    fn chance(&mut self, percent: u32) -> bool {
        self.next() % 100 < percent
    }
    fn tick(&mut self, c: &mut Client, home: Vec3) -> Keys {
        if self.chance(8) {
            self.keys = Keys {
                forward: self.chance(80),
                backward: false,
                left: self.chance(15),
                right: self.chance(15),
                jump: self.chance(20),
                sneak: self.chance(10),
                sprint: self.chance(50),
            };
        }
        let away = c.pos.sub(home);
        if away.x.hypot(away.z) > 12.0 {
            // Turn toward home (yaw 0 is +z) by the short way, as a mouse does:
            // a 360-degree jump is AimModulo360's cheat signature.
            let target = away.x.atan2(-away.z).to_degrees() as f32;
            c.yaw += ((target - c.yaw + 540.0).rem_euclid(360.0) - 180.0).clamp(-30.0, 30.0);
            self.keys.forward = true;
        } else if self.chance(20) {
            c.yaw += (self.next() % 31) as f32 - 15.0;
        }
        self.keys
    }
}

fn main() {
    let o = match options() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let connection = match join(&o) {
        Ok(c) => c,
        Err(e) => {
            println!("DISCONNECTED {e}");
            std::process::exit(1);
        }
    };
    let (reader, writer) = connection.split();
    let writer = Arc::new(Mutex::new(writer));
    let (tx, events) = mpsc::channel();
    {
        let writer = writer.clone();
        thread::spawn(move || read(reader, writer, tx));
    }
    let send = |body: Vec<u8>| {
        writer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(&body)
            .is_ok()
    };

    let mut client = Client::new(Vec3::ZERO);
    let mut world = TestWorld::new();
    let mut dimension = "minecraft:overworld".to_owned();
    let mut entity = 0;
    let mut spawned = false;
    let mut home = Vec3::ZERO;
    let mut teleports = 0;
    let mut walker = Walker {
        seed: offline_uuid(&o.name) as u64,
        keys: Keys::default(),
    };
    let until = Instant::now() + Duration::from_secs(o.seconds);
    let mut next = Instant::now();
    let mut code = 0;
    'run: while Instant::now() < until {
        while let Ok(e) = events.try_recv() {
            match e {
                Event::Login { entity: id } => entity = id,
                Event::Teleport {
                    id,
                    pos,
                    relatives,
                    yaw,
                    pitch,
                } => {
                    let base = [client.pos.x, client.pos.y, client.pos.z];
                    let p: [f64; 3] = std::array::from_fn(|i| {
                        if relatives & (1 << i) != 0 {
                            base[i] + pos[i]
                        } else {
                            pos[i]
                        }
                    });
                    client.pos = Vec3::new(p[0], p[1], p[2]);
                    // Horizon's teleports send no motion: DELTA relatives keep it.
                    if relatives & (32 | 64 | 128) == 0 {
                        client.vel = Vec3::ZERO;
                    }
                    client.yaw = if relatives & 8 != 0 {
                        client.yaw + yaw
                    } else {
                        yaw
                    };
                    client.pitch = if relatives & 16 != 0 {
                        client.pitch + pitch
                    } else {
                        pitch
                    }
                    .clamp(-90.0, 90.0);
                    teleports += 1;
                    println!("TELEPORT {teleports} {:.3} {:.3} {:.3}", p[0], p[1], p[2]);
                    send(
                        play::Serverbound::AcceptTeleportation(play::AcceptTeleportation {
                            id,
                            x: p[0],
                            y: p[1],
                            z: p[2],
                            yaw: client.yaw,
                            pitch: client.pitch,
                        })
                        .encode(),
                    );
                    if let Some(top) = surface(&dimension) {
                        world = floor_world(top, client.pos);
                    }
                    if !spawned {
                        spawned = true;
                        home = client.pos;
                        send(raw(ids::play::serverbound::PLAYER_LOADED, |_| {}));
                        println!("SPAWNED {:.3} {:.3} {:.3}", p[0], p[1], p[2]);
                    }
                }
                Event::Respawn { dimension: d } => {
                    println!("RESPAWN {d}");
                    dimension = d;
                    home = Vec3::ZERO;
                    spawned = false;
                }
                Event::Health(h) if h <= 0.0 => {
                    send(
                        play::Serverbound::ClientCommand(play::ClientCommand { action: 0 })
                            .encode(),
                    );
                }
                Event::Health(_) => {}
                Event::Ping(id) => {
                    send(raw(ids::play::serverbound::PONG, |w| w.i32(id)));
                }
                Event::Motion {
                    entity: id,
                    velocity,
                } if id == entity => {
                    client.vel = Vec3::new(velocity[0], velocity[1], velocity[2]);
                }
                Event::Motion { .. } => {}
                Event::Chat(text) => println!("CHAT {text}"),
                Event::Gone(reason) => {
                    println!("DISCONNECTED {reason}");
                    code = 1;
                    break 'run;
                }
            }
        }
        if spawned && home == Vec3::ZERO {
            home = client.pos;
        }
        if spawned {
            let packets = if surface(&dimension).is_none() || o.mode == "stand" {
                // A terrain the model does not know (or a staff bot): stand.
                client.packets(&world, Keys::default(), |_| {})
            } else {
                let keys = walker.tick(&mut client, home);
                let cheat = o.mode == "speed";
                client.packets(&world, keys, |c| {
                    if cheat {
                        c.pos.z += 0.3;
                    }
                })
            };
            for p in &packets {
                if let Some(body) = encode(p, entity)
                    && !send(body)
                {
                    println!("DISCONNECTED write failed");
                    code = 1;
                    break 'run;
                }
            }
        } else {
            send(play::Serverbound::ClientTickEnd(Empty).encode());
        }
        next += Duration::from_millis(50);
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        } else if now - next > Duration::from_secs(1) {
            next = now;
        }
    }
    writer.lock().unwrap_or_else(|e| e.into_inner()).shutdown();
    println!("DONE");
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::Harness;
    use grim_horizon_engine::CheckId;

    /// The bot's walk, against the engine in process, on its flat floor.
    fn walk(cheat: bool, ticks: usize) -> Harness {
        let spawn = Vec3::new(3.5, -60.0, -2.5);
        let mut h = Harness::new(floor_world(-60, spawn), spawn);
        let mut walker = Walker {
            seed: 7,
            keys: Keys::default(),
        };
        for _ in 0..ticks {
            let keys = walker.tick(&mut h.client, spawn);
            h.client_tick(keys, |c| {
                if cheat {
                    c.pos.z += 0.3;
                }
            });
        }
        h
    }

    #[test]
    fn the_legit_walk_is_clean_and_stays_home() {
        let h = walk(false, 6000);
        assert!(h.flags.is_empty(), "{:?}", &h.flags[..h.flags.len().min(5)]);
        let away = h.client.pos.sub(Vec3::new(3.5, -60.0, -2.5));
        assert!(
            away.x.hypot(away.z) < 40.0,
            "wandered to {:?}",
            h.client.pos
        );
    }

    #[test]
    fn the_speed_walk_flags() {
        let h = walk(true, 40);
        assert!(h.flagged(CheckId::Simulation) > 0);
    }
}
