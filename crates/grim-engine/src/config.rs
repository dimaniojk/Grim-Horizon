// Grim-Horizon: a Rust port of Grim Anticheat (https://github.com/GrimAnticheat/Grim).
// Copyright (C) GrimAC contributors (original Java), Grim-Horizon contributors (port).
// Licensed under the GNU General Public License v3.0 only; see LICENSE.

//! Configuration, read from `grim-horizon.toml`. Defaults follow Grim's
//! `config.yml`, except that the anti-cheat starts in alert-only mode: no
//! setbacks and no kicks until the owner turns them on.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::velocity::VelocityConfig;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Master switch.
    pub enabled: bool,
    /// `alert-only`: flag and alert, never touch the player.
    /// `setback`: also teleport players back on movement violations.
    /// `punish`: also run the per-check punishments (kicks).
    pub mode: Mode,
    pub alerts: AlertsConfig,
    /// Operator level needed for `/grim` commands and for receiving alerts.
    pub permission_level: u8,
    /// Players (by name, lower case) that are never checked.
    pub exempt_players: Vec<String>,
    /// How long a player may leave transactions unanswered before they are
    /// kicked for timing out (0 turns it off). Grim's `max-transaction-time`.
    pub max_transaction_time_seconds: u64,
    pub simulation: SimulationConfig,
    pub knockback: KnockbackConfig,
    pub explosion: ExplosionConfig,
    pub timer: TimerConfig,
    pub reach: ReachConfig,
    /// Per-check settings, by Grim's check name (`Simulation`, `GroundSpoof`, ...).
    pub checks: BTreeMap<String, CheckConfig>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    AlertOnly,
    Setback,
    Punish,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AlertsConfig {
    /// Staff get alerts as soon as they join.
    pub enabled_on_join: bool,
    /// Write every alert to the server log.
    pub print_to_console: bool,
    /// Alert format: `%player%`, `%check%`, `%vl%`, `%verbose%`, `%description%`.
    pub format: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SimulationConfig {
    pub threshold: f64,
    pub immediate_setback_threshold: f64,
    pub max_advantage: f64,
    pub max_ceiling: f64,
    pub setback_decay_multiplier: f64,
    pub setback_violation_threshold: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct KnockbackConfig {
    pub threshold: f64,
    pub max_advantage: f64,
    pub immediate_setback_threshold: f64,
    pub setback_decay_multiplier: f64,
    pub max_ceiling: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ExplosionConfig {
    pub threshold: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct TimerConfig {
    /// How far (milliseconds) the player may fall behind its ping (`TimerA.drift`).
    pub drift_ms: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ReachConfig {
    /// Extra hitbox margin, blocks (`Reach.threshold`).
    pub threshold: f64,
    /// Ticks of entity history before the last acknowledged tick that may
    /// still be on the client's screen (interpolation).
    pub interpolation_ticks: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct CheckConfig {
    pub enabled: bool,
    /// Violations removed per passing event (Grim's `decay`).
    pub decay: Option<f64>,
    /// Violation level above which the check sets back (`setbackvl`, -1: never).
    pub setback_vl: Option<f64>,
    /// Alert on every Nth flag (1: every flag).
    pub alert_interval: u32,
    /// Kick at this violation level in `punish` mode (0: never).
    pub kick_vl: f64,
}

impl Default for CheckConfig {
    fn default() -> Self {
        Self { enabled: true, decay: None, setback_vl: None, alert_interval: 1, kick_vl: 0.0 }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: Mode::AlertOnly,
            alerts: AlertsConfig::default(),
            permission_level: 2,
            exempt_players: Vec::new(),
            max_transaction_time_seconds: 60,
            simulation: SimulationConfig::default(),
            knockback: KnockbackConfig::default(),
            explosion: ExplosionConfig::default(),
            timer: TimerConfig::default(),
            reach: ReachConfig::default(),
            checks: BTreeMap::new(),
        }
    }
}

impl Default for AlertsConfig {
    fn default() -> Self {
        Self {
            enabled_on_join: true,
            print_to_console: true,
            format: "[Grim] %player% failed %check% (x%vl%) %verbose%".to_owned(),
        }
    }
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            threshold: 0.001,
            immediate_setback_threshold: 0.1,
            max_advantage: 1.0,
            max_ceiling: 4.0,
            setback_decay_multiplier: 0.999,
            setback_violation_threshold: 1.0,
        }
    }
}

impl Default for KnockbackConfig {
    fn default() -> Self {
        Self { threshold: 0.001, max_advantage: 1.0, immediate_setback_threshold: 0.1, setback_decay_multiplier: 0.999, max_ceiling: 4.0 }
    }
}

impl Default for ExplosionConfig {
    fn default() -> Self {
        Self { threshold: 0.00001 }
    }
}

impl Default for TimerConfig {
    fn default() -> Self {
        Self { drift_ms: 120.0 }
    }
}

impl Default for ReachConfig {
    fn default() -> Self {
        Self { threshold: 0.0005, interpolation_ticks: 4 }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        let mut c: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        c.normalise();
        Ok(c)
    }
    fn normalise(&mut self) {
        if self.simulation.max_advantage < 0.0 {
            self.simulation.max_advantage = f64::MAX;
        }
        if self.simulation.immediate_setback_threshold < 0.0 {
            self.simulation.immediate_setback_threshold = f64::MAX;
        }
        if self.knockback.max_advantage < 0.0 {
            self.knockback.max_advantage = f64::MAX;
        }
        if self.knockback.immediate_setback_threshold < 0.0 {
            self.knockback.immediate_setback_threshold = f64::MAX;
        }
    }
    pub fn check(&self, name: &str) -> CheckConfig {
        self.checks.get(name).cloned().unwrap_or_default()
    }
    pub fn velocity(&self) -> VelocityConfig {
        VelocityConfig {
            threshold: self.knockback.threshold,
            max_advantage: self.knockback.max_advantage,
            immediate_setback_threshold: self.knockback.immediate_setback_threshold,
            setback_decay_multiplier: self.knockback.setback_decay_multiplier,
            max_ceiling: self.knockback.max_ceiling,
        }
    }
    pub fn setbacks_enabled(&self) -> bool {
        matches!(self.mode, Mode::Setback | Mode::Punish)
    }
    pub fn punishments_enabled(&self) -> bool {
        self.mode == Mode::Punish
    }
    pub fn is_exempt(&self, name: &str) -> bool {
        self.exempt_players.iter().any(|n| n.eq_ignore_ascii_case(name))
    }
}

/// The file written on the first start.
pub const DEFAULT_CONFIG: &str = r#"# Grim-Horizon: a native Rust port of Grim Anticheat for Horizon servers.
# Changes apply on `/grim reload`.

enabled = true
# alert-only: flag and alert staff, never touch the player (the default until
#             the checks have been validated on your server)
# setback:    also teleport players back after movement violations
# punish:     also kick at each check's kick_vl
mode = "alert-only"
# Operator level for /grim commands and alerts.
permission_level = 2
# Never checked (names, any case).
exempt_players = []
# Kick a player who leaves our ping transactions unanswered this long (0: off).
max_transaction_time_seconds = 60

[alerts]
enabled_on_join = true
print_to_console = true
format = "[Grim] %player% failed %check% (x%vl%) %verbose%"

[simulation]
threshold = 0.001
immediate_setback_threshold = 0.1
max_advantage = 1.0
max_ceiling = 4.0
setback_decay_multiplier = 0.999
setback_violation_threshold = 1.0

[knockback]
threshold = 0.001
max_advantage = 1.0
immediate_setback_threshold = 0.1
setback_decay_multiplier = 0.999
max_ceiling = 4.0

[explosion]
threshold = 0.00001

[timer]
drift_ms = 120.0

[reach]
threshold = 0.0005
interpolation_ticks = 4

# Per-check overrides, by Grim's names. Example:
# [checks.Reach]
# enabled = true
# alert_interval = 1
# kick_vl = 0.0
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_parses_to_defaults() {
        let c = Config::parse(DEFAULT_CONFIG).unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(c.mode, Mode::AlertOnly);
        assert!(!c.setbacks_enabled());
    }

    #[test]
    fn unknown_keys_are_errors() {
        assert!(Config::parse("bogus = 1").is_err());
    }

    #[test]
    fn per_check_overrides() {
        let c = Config::parse("mode = \"punish\"\n[checks.Reach]\nenabled = false\nkick_vl = 10.0\n").unwrap();
        assert!(!c.check("Reach").enabled);
        assert_eq!(c.check("Reach").kick_vl, 10.0);
        assert!(c.check("Simulation").enabled);
        assert!(c.punishments_enabled());
    }
}
