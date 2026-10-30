// Grim-Horizon: a Rust port of Grim Anticheat (https://github.com/GrimAnticheat/Grim).
// Copyright (C) GrimAC contributors (original Java), Grim-Horizon contributors (port).
// Licensed under the GNU General Public License v3.0 only; see LICENSE.

//! `/grim` commands (Grim's `GrimAlerts`, `GrimVerbose`, `GrimProfile`,
//! `GrimReload`, `GrimDebug`, ...), answered as plain text lines.

use crate::checks::CheckId;
use crate::config::Config;
use crate::engine::{Engine, PlayerKey, format_offset};

/// What a command asks of the adapter besides the reply.
#[derive(Debug, Default, PartialEq)]
pub struct CommandOutcome {
    pub lines: Vec<String>,
    /// Re-read the config file and pass it to [`Engine::reload`].
    pub reload: bool,
}

impl CommandOutcome {
    fn line(mut self, s: impl Into<String>) -> Self {
        self.lines.push(s.into());
        self
    }
}

const HELP: &[&str] = &[
    "/grim alerts - toggle alerts",
    "/grim verbose - toggle every flag, not only alerts",
    "/grim profile <player> - violation levels and prediction stats",
    "/grim debug <player> - toggle per-movement prediction output",
    "/grim checks - list checks and their state",
    "/grim exempt <player> - toggle a player's exemption until they leave",
    "/grim reload - reload grim-horizon.toml",
    "/grim stats - engine statistics",
];

impl Engine {
    /// Runs `/grim <args>`. `sender` is the player who typed it, or `None`
    /// for the server console (which may do everything except toggling its
    /// own alerts). `now` is the engine clock in nanoseconds.
    pub fn command(&mut self, sender: Option<PlayerKey>, args: &str, now: u64) -> CommandOutcome {
        let out = CommandOutcome::default();
        if let Some(key) = sender {
            let level = self.config.permission_level;
            match self.player(key) {
                Some(p) if p.permission >= level => {}
                _ => return out.line("You do not have permission to use /grim."),
            }
        }
        let mut words = args.split_whitespace();
        let sub = words.next().unwrap_or("help").to_ascii_lowercase();
        let arg = words.next();
        match sub.as_str() {
            "alerts" => {
                let Some(key) = sender else { return out.line("The console always receives alerts in the log.") };
                let p = self.player_mut(key).expect("checked above");
                p.alerts = !p.alerts;
                let on = p.alerts;
                out.line(if on { "Alerts enabled" } else { "Alerts disabled" })
            }
            "verbose" => {
                let Some(key) = sender else { return out.line("Verbose output is only for players.") };
                let p = self.player_mut(key).expect("checked above");
                p.verbose = !p.verbose;
                let on = p.verbose;
                out.line(if on { "Verbose enabled" } else { "Verbose disabled" })
            }
            "debug" => {
                let Some(key) = sender else { return out.line("Debug output is only for players.") };
                let Some(name) = arg else { return out.line("Usage: /grim debug <player>") };
                let Some(target) = self.find_player(name) else { return out.line(format!("{name} is not online")) };
                let p = self.player_mut(key).expect("checked above");
                let on = if p.debug_targets.remove(&target) {
                    false
                } else {
                    p.debug_targets.insert(target);
                    true
                };
                out.line(format!("Debug for {name} {}", if on { "enabled" } else { "disabled" }))
            }
            "exempt" => {
                let Some(name) = arg else { return out.line("Usage: /grim exempt <player>") };
                let Some(target) = self.find_player(name) else { return out.line(format!("{name} is not online")) };
                let p = self.player_mut(target).expect("found");
                p.exempt = !p.exempt;
                let on = p.exempt;
                out.line(format!("{name} is {} exempt", if on { "now" } else { "no longer" }))
            }
            "profile" => {
                let Some(name) = arg else { return out.line("Usage: /grim profile <player>") };
                let Some(target) = self.find_player(name) else { return out.line(format!("{name} is not online")) };
                let p = self.player(target).expect("found");
                let m = &p.movement;
                let mut o = out.line(format!("--- Grim profile: {} ---", p.name));
                o = o.line(format!(
                    "Ping (transactions): {}",
                    p.transaction_ping_ms(now).map_or("unknown".to_owned(), |v| format!("{v:.0} ms"))
                ));
                o = o.line(format!(
                    "Predictions: {} ({} not checked{}), last offset {}",
                    p.predictions,
                    p.skipped,
                    p.last_skip_reason.map(|r| format!(", last: {r}")).unwrap_or_default(),
                    format_offset(p.last_offset)
                ));
                o = o.line(format!(
                    "Position {:.3}, {:.3}, {:.3}  on ground {}  sprinting {}  sneaking {}  mode {:?}",
                    m.x, m.y, m.z, m.on_ground, m.is_sprinting, m.is_sneaking, m.game_mode
                ));
                let summary = p.violations.summary();
                if summary.is_empty() {
                    o = o.line("No violations");
                }
                for (c, vl) in summary {
                    o = o.line(format!("  {}: {:.2}", c.info().name, vl));
                }
                if p.exempt {
                    o = o.line("Exempt (runtime)");
                }
                o
            }
            "checks" => {
                let mut o = out.line(format!("Mode: {:?}", self.config.mode));
                for c in CheckId::ALL {
                    let info = c.info();
                    let enabled = self.config.check(info.config_name).enabled;
                    o = o.line(format!("  {} [{}] - {}", info.name, if enabled { "on" } else { "off" }, info.description));
                }
                o
            }
            "stats" => {
                let s = self.stats;
                let avg = if s.predictions == 0 { 0.0 } else { s.prediction_nanos as f64 / s.predictions as f64 / 1000.0 };
                out.line(format!(
                    "{} players, {} packets, {} predictions, {:.1} µs per prediction",
                    self.players().count(),
                    s.packets,
                    s.predictions,
                    avg
                ))
            }
            "reload" => CommandOutcome { lines: Vec::new(), reload: true },
            _ => {
                let mut o = out;
                for l in HELP {
                    o = o.line(*l);
                }
                o
            }
        }
    }

    /// Replaces the configuration (after `/grim reload`).
    pub fn reload(&mut self, text: &str) -> Result<(), String> {
        let c = Config::parse(text)?;
        self.config = c;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_is_required() {
        let mut e = Engine::new(Config::default());
        e.add_player(1, 1, "Guest", 0);
        e.add_player(2, 2, "Admin", 4);
        assert_eq!(e.command(Some(1), "alerts", 0).lines, vec!["You do not have permission to use /grim."]);
        assert_eq!(e.command(Some(2), "alerts", 0).lines, vec!["Alerts disabled"]);
        assert_eq!(e.command(Some(2), "alerts", 0).lines, vec!["Alerts enabled"]);
        assert!(e.command(None, "reload", 0).reload);
        assert!(e.command(None, "profile Guest", 0).lines[0].contains("Guest"));
    }
}
