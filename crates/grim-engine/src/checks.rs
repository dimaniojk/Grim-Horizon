// Grim-Horizon: a Rust port of Grim Anticheat (https://github.com/GrimAnticheat/Grim).
// Copyright (C) GrimAC contributors (original Java), Grim-Horizon contributors (port).
// Licensed under the GNU General Public License, version 3 or (at your option) any later version; see LICENSE.

//! The check registry and violation bookkeeping (Grim's `Check`,
//! `@CheckData` and `PunishmentManager`, reduced to what the port uses).
//! Names are Grim's, so alerts and configs read the same.

use crate::config::Config;

/// Every check this port implements, with Grim's metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CheckId {
    Simulation,
    GroundSpoof,
    NoFall,
    Timer,
    TickTimer,
    AntiKB,
    AntiExplosion,
    Reach,
    Hitboxes,
    BadPacketsA,
    BadPacketsD,
    BadPacketsE,
    BadPacketsF,
    BadPacketsI,
    BadPacketsN,
    BadPacketsY,
    CrashA,
    CrashC,
    SelfInteract,
    TransactionOrder,
    AimModulo360,
    AimDuplicateLook,
    PacketOrderB,
}

pub struct CheckInfo {
    /// Grim's display name.
    pub name: &'static str,
    /// Grim's config section name (differs for AntiKB, AntiExplosion and Timer).
    pub config_name: &'static str,
    pub description: &'static str,
    pub decay: f64,
    /// Grim's default `setbackvl` (`f64::MAX` for -1 / never).
    pub setback_vl: f64,
}

impl CheckId {
    pub const ALL: [CheckId; 23] = [
        CheckId::Simulation,
        CheckId::GroundSpoof,
        CheckId::NoFall,
        CheckId::Timer,
        CheckId::TickTimer,
        CheckId::AntiKB,
        CheckId::AntiExplosion,
        CheckId::Reach,
        CheckId::Hitboxes,
        CheckId::BadPacketsA,
        CheckId::BadPacketsD,
        CheckId::BadPacketsE,
        CheckId::BadPacketsF,
        CheckId::BadPacketsI,
        CheckId::BadPacketsN,
        CheckId::BadPacketsY,
        CheckId::CrashA,
        CheckId::CrashC,
        CheckId::SelfInteract,
        CheckId::TransactionOrder,
        CheckId::AimModulo360,
        CheckId::AimDuplicateLook,
        CheckId::PacketOrderB,
    ];

    pub fn info(self) -> CheckInfo {
        // CheckData defaults: decay 0.05, setback 25.
        let i = |name, config_name, description, decay, setback_vl| CheckInfo {
            name,
            config_name,
            description,
            decay,
            setback_vl,
        };
        match self {
            CheckId::Simulation => i(
                "Simulation",
                "Simulation",
                "Moved differently than predicted movement simulation",
                0.02,
                25.0,
            ),
            CheckId::GroundSpoof => i(
                "GroundSpoof",
                "GroundSpoof",
                "Claimed to be on ground when predicted otherwise",
                0.01,
                10.0,
            ),
            CheckId::NoFall => i(
                "NoFall",
                "NoFall",
                "Sent an on-ground packet while not colliding with the ground",
                0.05,
                10.0,
            ),
            CheckId::Timer => i(
                "Timer",
                "TimerA",
                "The players game is running faster than normal",
                0.05,
                10.0,
            ),
            CheckId::TickTimer => i(
                "TickTimer",
                "TickTimer",
                "Did not send client tick end packet",
                0.05,
                1.0,
            ),
            CheckId::AntiKB => i(
                "AntiKB",
                "Knockback",
                "Did not take the expected entity knockback",
                0.025,
                10.0,
            ),
            CheckId::AntiExplosion => i(
                "AntiExplosion",
                "Explosion",
                "Did not take the expected explosion knockback",
                0.05,
                10.0,
            ),
            CheckId::Reach => i(
                "Reach",
                "Reach",
                "Attacked an entity from too far away",
                0.05,
                25.0,
            ),
            CheckId::Hitboxes => i(
                "Hitboxes",
                "Hitboxes",
                "Tried to hit an entity outside its valid hitbox",
                0.05,
                25.0,
            ),
            CheckId::BadPacketsA => i(
                "BadPacketsA",
                "BadPacketsA",
                "Sent duplicate slot id",
                0.05,
                25.0,
            ),
            CheckId::BadPacketsD => i(
                "BadPacketsD",
                "BadPacketsD",
                "Sent an invalid rotation pitch outside the -90 to 90 range",
                0.05,
                25.0,
            ),
            CheckId::BadPacketsE => i(
                "BadPacketsE",
                "BadPacketsE",
                "Sent too many movement packets without updating position",
                0.05,
                25.0,
            ),
            CheckId::BadPacketsF => i(
                "BadPacketsF",
                "BadPacketsF",
                "Sent duplicate sprinting status",
                0.05,
                25.0,
            ),
            CheckId::BadPacketsI => i(
                "BadPacketsI",
                "BadPacketsI",
                "Claimed to be flying while unable to fly",
                0.05,
                25.0,
            ),
            CheckId::BadPacketsN => i(
                "BadPacketsN",
                "BadPacketsN",
                "Ignored or failed to accept a required server teleport",
                0.05,
                25.0,
            ),
            CheckId::BadPacketsY => i(
                "BadPacketsY",
                "BadPacketsY",
                "Sent out of bounds slot id",
                0.05,
                25.0,
            ),
            CheckId::CrashA => i(
                "CrashA",
                "CrashA",
                "Sent a position outside the valid world bounds",
                0.05,
                25.0,
            ),
            CheckId::CrashC => i(
                "CrashC",
                "CrashC",
                "Sent non-finite position or rotation",
                0.05,
                25.0,
            ),
            CheckId::SelfInteract => i(
                "SelfInteract",
                "SelfInteract",
                "Interacted with self",
                0.05,
                25.0,
            ),
            CheckId::TransactionOrder => i(
                "TransactionOrder",
                "TransactionOrder",
                "Sent transaction or ping responses in an invalid order",
                0.05,
                25.0,
            ),
            CheckId::AimModulo360 => i(
                "AimModulo360",
                "AimModulo360",
                "Sent a large yaw snap",
                0.005,
                25.0,
            ),
            CheckId::AimDuplicateLook => i(
                "AimDuplicateLook",
                "AimDuplicateLook",
                "Sent a duplicate rotation update without changing look direction",
                0.05,
                25.0,
            ),
            CheckId::PacketOrderB => i(
                "PacketOrderB",
                "PacketOrderB",
                "Did not swing for attack",
                0.05,
                25.0,
            ),
        }
    }

    pub fn from_name(name: &str) -> Option<CheckId> {
        CheckId::ALL.into_iter().find(|c| {
            c.info().name.eq_ignore_ascii_case(name)
                || c.info().config_name.eq_ignore_ascii_case(name)
        })
    }

    fn index(self) -> usize {
        CheckId::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }
}

/// One flag, before it is turned into alerts and actions.
#[derive(Clone, Debug, PartialEq)]
pub struct Flag {
    pub check: CheckId,
    pub vl: f64,
    pub verbose: String,
    /// The alert should be shown (alert interval).
    pub alert: bool,
    /// The check is above its setback level.
    pub setback: bool,
    /// The check reached its kick level (punish mode only).
    pub kick: bool,
}

#[derive(Clone, Debug)]
pub struct Violations {
    vl: [f64; CheckId::ALL.len()],
    flags: [u32; CheckId::ALL.len()],
}

impl Default for Violations {
    fn default() -> Self {
        Self {
            vl: [0.0; CheckId::ALL.len()],
            flags: [0; CheckId::ALL.len()],
        }
    }
}

impl Violations {
    pub fn vl(&self, c: CheckId) -> f64 {
        self.vl[c.index()]
    }
    /// `Check.flag`: one more violation. `None` when the check is disabled.
    pub fn flag(&mut self, config: &Config, c: CheckId, verbose: String) -> Option<Flag> {
        let info = c.info();
        let cc = config.check(info.config_name);
        if !cc.enabled || !config.enabled {
            return None;
        }
        let i = c.index();
        self.vl[i] += 1.0;
        self.flags[i] += 1;
        let interval = cc.alert_interval.max(1);
        let setback_vl = match cc.setback_vl {
            Some(v) if v < 0.0 => f64::MAX,
            Some(v) => v,
            None => info.setback_vl,
        };
        Some(Flag {
            check: c,
            vl: self.vl[i],
            verbose,
            alert: self.flags[i].is_multiple_of(interval),
            setback: self.vl[i] > setback_vl,
            kick: config.punishments_enabled() && cc.kick_vl > 0.0 && self.vl[i] >= cc.kick_vl,
        })
    }
    /// `Check.reward`: the decay after a passing event.
    pub fn reward(&mut self, config: &Config, c: CheckId) {
        let info = c.info();
        let decay = config.check(info.config_name).decay.unwrap_or(info.decay);
        let i = c.index();
        self.vl[i] = (self.vl[i] - decay).max(0.0);
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// Checks with a violation level, highest first.
    pub fn summary(&self) -> Vec<(CheckId, f64)> {
        let mut v: Vec<_> = CheckId::ALL
            .iter()
            .map(|c| (*c, self.vl(*c)))
            .filter(|(_, vl)| *vl > 0.0)
            .collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_and_decay() {
        let config = Config::default();
        let mut v = Violations::default();
        let f = v
            .flag(&config, CheckId::GroundSpoof, "claimed true".into())
            .unwrap();
        assert_eq!(f.vl, 1.0);
        assert!(f.alert && !f.kick && !f.setback);
        v.reward(&config, CheckId::GroundSpoof);
        assert!((v.vl(CheckId::GroundSpoof) - 0.99).abs() < 1e-12);
    }

    #[test]
    fn names_resolve_both_ways() {
        assert_eq!(CheckId::from_name("Knockback"), Some(CheckId::AntiKB));
        assert_eq!(CheckId::from_name("antikb"), Some(CheckId::AntiKB));
        assert_eq!(CheckId::from_name("TimerA"), Some(CheckId::Timer));
    }

    #[test]
    fn disabled_check_never_flags() {
        let config = Config::parse("[checks.Reach]\nenabled = false\n").unwrap();
        let mut v = Violations::default();
        assert!(v.flag(&config, CheckId::Reach, String::new()).is_none());
    }
}
