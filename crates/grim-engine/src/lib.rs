// Grim-Horizon: a Rust port of Grim Anticheat (https://github.com/GrimAnticheat/Grim).
// Copyright (C) GrimAC contributors (original Java), Grim-Horizon contributors (port).
// Licensed under the GNU General Public License v3.0 only; see LICENSE.

//! A native Rust port of Grim Anticheat's predictive movement engine and
//! checks, for Minecraft 26.3 clients. The engine is independent of any
//! server: an adapter feeds it packets in client order, tells it what the
//! server sent, gives it a [`world::BlockView`] of the player's level, and
//! applies the [`engine::Action`]s and [`engine::Verdict`]s it returns.

pub mod checks;
pub mod collisions;
pub mod commands;
pub mod config;
pub mod engine;
pub mod entities;
pub mod fluid;
pub mod math;
pub mod player;
pub mod prediction;
pub mod velocity;
pub mod world;

pub use checks::CheckId;
pub use config::Config;
pub use engine::{Action, Clientbound, Engine, PING_MARK, PlayerKey, Serverbound, Verdict};
pub use math::{Aabb, Vec3};
