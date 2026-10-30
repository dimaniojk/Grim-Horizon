// Grim-Horizon: a Rust port of Grim Anticheat (https://github.com/GrimAnticheat/Grim).
// Copyright (C) GrimAC contributors (original Java), Grim-Horizon contributors (port).
// Licensed under the GNU General Public License, version 3 or (at your option) any later version; see LICENSE.

//! What to do after the engine panicked inside a server hook.
//!
//! A caught panic leaves the state it was changing unknown, so nothing
//! resumes on it:
//!
//! * a panic while handling one player's packets loses only that player's
//!   state; the player is kicked (a rejoin resends position, attributes and
//!   effects, so the new state is complete) and nobody else is affected;
//! * a panic in a hook shared by everyone rebuilds the whole engine; players
//!   already online have incomplete state and stay unchecked until they
//!   rejoin, which staff are told;
//! * if shared hooks keep failing, the anti-cheat turns off and keeps telling
//!   staff so, rather than crash the server or fail silently.

use std::collections::VecDeque;

/// Shared-hook failures within this many seconds before turning off.
pub const GLOBAL_WINDOW_SECONDS: u64 = 600;
pub const GLOBAL_LIMIT: usize = 3;
/// How often staff are reminded that the anti-cheat is off.
pub const OFF_REMINDER_SECONDS: u64 = 60;

#[derive(Debug, PartialEq)]
pub enum GlobalOutcome {
    /// Build a fresh engine; existing players unchecked until they rejoin.
    Rebuild,
    /// Too many failures: stay off until the server restarts.
    Off,
}

#[derive(Debug, Default)]
pub struct Recovery {
    global: VecDeque<u64>,
    off: bool,
    last_reminder: Option<u64>,
    /// Panics caused while handling players' packets, ever.
    pub player_panics: u64,
}

impl Recovery {
    pub fn is_off(&self) -> bool {
        self.off
    }

    /// One player's packet handling panicked: the line for staff and the log.
    /// The caller drops that player's state and kicks them.
    pub fn player_panic(&mut self, name: &str, what: &str) -> String {
        self.player_panics += 1;
        format!(
            "[Grim] internal error while checking {name} ({what}); {name} was disconnected to reset their state. Please report this."
        )
    }

    /// A shared hook panicked at `now` (seconds).
    pub fn global_panic(&mut self, now: u64, what: &str) -> (GlobalOutcome, String) {
        while self
            .global
            .front()
            .is_some_and(|&t| now.saturating_sub(t) > GLOBAL_WINDOW_SECONDS)
        {
            self.global.pop_front();
        }
        self.global.push_back(now);
        if self.global.len() >= GLOBAL_LIMIT {
            self.off = true;
            self.last_reminder = Some(now);
            (
                GlobalOutcome::Off,
                format!(
                    "[Grim] internal error ({what}), {} times in {GLOBAL_WINDOW_SECONDS} s: Grim-Horizon is OFF until the server restarts. Nobody is being checked. Please report this.",
                    self.global.len()
                ),
            )
        } else {
            (
                GlobalOutcome::Rebuild,
                format!(
                    "[Grim] internal error ({what}); Grim-Horizon restarted. Players online now are not checked until they rejoin. Please report this."
                ),
            )
        }
    }

    /// While off: a reminder for staff at most once a minute.
    pub fn reminder(&mut self, now: u64) -> Option<String> {
        if !self.off
            || self
                .last_reminder
                .is_some_and(|t| now.saturating_sub(t) < OFF_REMINDER_SECONDS)
        {
            return None;
        }
        self.last_reminder = Some(now);
        Some("[Grim] Grim-Horizon is OFF after repeated internal errors; nobody is being checked until the server restarts.".into())
    }
}

/// The message of a caught panic.
pub fn panic_message(e: &(dyn std::any::Any + Send)) -> String {
    e.downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| e.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_failures_never_turn_the_anti_cheat_off() {
        let mut r = Recovery::default();
        for _ in 0..100 {
            let line = r.player_panic("Mallory", "boom");
            assert!(line.contains("Mallory was disconnected"));
        }
        assert!(!r.is_off());
        assert_eq!(r.player_panics, 100);
    }

    #[test]
    fn repeated_shared_failures_turn_it_off_loudly() {
        let mut r = Recovery::default();
        assert_eq!(r.global_panic(0, "a").0, GlobalOutcome::Rebuild);
        // Failures far apart are each recovered from.
        assert_eq!(r.global_panic(1000, "b").0, GlobalOutcome::Rebuild);
        assert_eq!(r.global_panic(2000, "c").0, GlobalOutcome::Rebuild);
        assert!(!r.is_off());
        // Three within ten minutes: off, and staff keep hearing about it.
        assert_eq!(r.global_panic(2100, "d").0, GlobalOutcome::Rebuild);
        let (outcome, line) = r.global_panic(2200, "e");
        assert_eq!(outcome, GlobalOutcome::Off);
        assert!(line.contains("OFF"));
        assert!(r.reminder(2230).is_none(), "at most once a minute");
        assert!(r.reminder(2261).is_some());
        assert!(r.reminder(2262).is_none());
    }
}
