// Grim-Horizon: a Rust port of Grim Anticheat (https://github.com/GrimAnticheat/Grim).
// Copyright (C) GrimAC contributors (original Java), Grim-Horizon contributors (port).
// SPDX-License-Identifier: GPL-3.0-only (GNU General Public License, version 3; see LICENSE)

//! Entity positions for Reach and Hitboxes.
//!
//! Grim replays every entity packet it sends into `CompensatedEntities` and
//! knows, through transactions, which positions the client may be drawing
//! (`getPossibleCollisionBoxes`). This port records the server's position of
//! every entity at each server tick instead, and maps transactions to ticks:
//! once the client answered the transaction sent at the end of tick T, it has
//! received every position up to T, and may still be interpolating towards
//! positions from a few ticks before. The union of the boxes over that window
//! is the target, which is never smaller than what the client could see.

use std::collections::{HashMap, VecDeque};

use crate::math::{Aabb, Vec3};

/// An entity's state at one server tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntitySnapshot {
    pub id: i32,
    pub pos: Vec3,
    pub width: f32,
    pub height: f32,
    /// `isLivingEntity`, or an end crystal: Grim only flags these.
    pub checkable: bool,
    /// Riding another entity (Grim exempts those).
    pub riding: bool,
}

impl EntitySnapshot {
    pub fn bounding_box(&self) -> Aabb {
        Aabb::from_pos_size(self.pos.x, self.pos.y, self.pos.z, self.width, self.height)
    }
}

/// Vanilla 26.3 sizes of common living entities, used when the adapter has
/// not registered the server's own table. Unknown types are never flagged.
pub fn builtin_size(kind: &str) -> Option<(f32, f32)> {
    let kind = kind.strip_prefix("minecraft:").unwrap_or(kind);
    Some(match kind {
        "player" => (0.6, 1.8),
        "zombie" | "husk" | "drowned" | "zombie_villager" | "villager" | "witch" | "pillager"
        | "vindicator" | "evoker" | "illusioner" | "wandering_trader" | "piglin"
        | "piglin_brute" | "zombified_piglin" => (0.6, 1.95),
        "skeleton" | "stray" | "bogged" => (0.6, 1.99),
        "wither_skeleton" => (0.7, 2.4),
        "creeper" => (0.6, 1.7),
        "spider" => (1.4, 0.9),
        "cave_spider" => (0.7, 0.5),
        "enderman" => (0.6, 2.9),
        "cow" | "mooshroom" => (0.9, 1.4),
        "pig" => (0.9, 0.9),
        "sheep" => (0.9, 1.3),
        "chicken" => (0.4, 0.7),
        "iron_golem" => (1.4, 2.7),
        "snow_golem" => (0.7, 1.9),
        "wolf" => (0.6, 0.85),
        "cat" | "ocelot" => (0.6, 0.7),
        "fox" => (0.6, 0.7),
        "rabbit" => (0.4, 0.5),
        "horse" | "donkey" | "mule" | "skeleton_horse" | "zombie_horse" => (1.3964844, 1.6),
        "llama" | "trader_llama" => (0.9, 1.87),
        "blaze" => (0.6, 1.8),
        "end_crystal" => (2.0, 2.0),
        "silverfish" => (0.4, 0.3),
        "endermite" => (0.4, 0.3),
        "guardian" => (0.85, 0.85),
        "elder_guardian" => (1.9975, 1.9975),
        "ravager" => (1.95, 2.2),
        "hoglin" | "zoglin" => (1.3964844, 1.4),
        "vex" => (0.4, 0.8),
        "bee" => (0.7, 0.6),
        "goat" => (0.9, 1.3),
        "axolotl" => (0.75, 0.42),
        "frog" => (0.5, 0.5),
        "warden" => (0.9, 2.9),
        "allay" => (0.35, 0.6),
        "sniffer" => (1.9, 1.75),
        "camel" => (1.7, 2.375),
        "armadillo" => (0.7, 0.65),
        "breeze" => (0.6, 1.77),
        "turtle" => (1.2, 0.4),
        "polar_bear" => (1.4, 1.4),
        "panda" => (1.3, 1.25),
        "parrot" => (0.5, 0.9),
        "squid" | "glow_squid" => (0.8, 0.8),
        "dolphin" => (0.9, 0.6),
        "strider" => (0.9, 1.7),
        "phantom" => (0.9, 0.5),
        "shulker" => (1.0, 1.0),
        _ => return None,
    })
}

/// The kinds Grim never checks (`Reach.blacklisted`).
pub fn is_reach_exempt_kind(kind: &str) -> bool {
    let kind = kind.strip_prefix("minecraft:").unwrap_or(kind);
    kind.ends_with("boat") || kind.ends_with("raft") || kind == "shulker"
}

const HISTORY_TICKS: usize = 40;

/// Entity positions by server tick, for one level.
#[derive(Debug, Default)]
pub struct EntityHistory {
    /// (server tick, entities at the start of that tick).
    ticks: VecDeque<(u64, HashMap<i32, EntitySnapshot>)>,
}

impl EntityHistory {
    pub fn record(&mut self, tick: u64, entities: impl IntoIterator<Item = EntitySnapshot>) {
        if self.ticks.back().is_some_and(|(t, _)| *t == tick) {
            let (_, map) = self.ticks.back_mut().unwrap();
            for e in entities {
                map.insert(e.id, e);
            }
            return;
        }
        if self.ticks.len() >= HISTORY_TICKS {
            self.ticks.pop_front();
        }
        self.ticks
            .push_back((tick, entities.into_iter().map(|e| (e.id, e)).collect()));
    }

    pub fn latest(&self, id: i32) -> Option<&EntitySnapshot> {
        self.ticks.back().and_then(|(_, m)| m.get(&id))
    }

    /// The union of `id`'s boxes over server ticks `[from, to]`, and whether
    /// the entity was checkable and not riding throughout. `None` when the
    /// window has no record of it.
    pub fn possible_box(&self, id: i32, from: u64, to: u64) -> Option<(Aabb, EntitySnapshot)> {
        let mut out: Option<(Aabb, EntitySnapshot)> = None;
        for (tick, map) in &self.ticks {
            if *tick < from || *tick > to {
                continue;
            }
            if let Some(e) = map.get(&id) {
                let b = e.bounding_box();
                out = Some(match out {
                    None => (b, *e),
                    Some((u, first)) => {
                        let merged = EntitySnapshot {
                            checkable: first.checkable && e.checkable,
                            riding: first.riding || e.riding,
                            ..first
                        };
                        (u.union(b), merged)
                    }
                });
            }
        }
        out
    }

    /// Whether the history covers `tick` (older ticks were dropped).
    pub fn covers(&self, tick: u64) -> bool {
        self.ticks.front().is_some_and(|(t, _)| *t <= tick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zombie(id: i32, x: f64) -> EntitySnapshot {
        EntitySnapshot {
            id,
            pos: Vec3::new(x, 64.0, 0.0),
            width: 0.6,
            height: 1.95,
            checkable: true,
            riding: false,
        }
    }

    #[test]
    fn union_over_window() {
        let mut h = EntityHistory::default();
        h.record(1, [zombie(5, 0.0)]);
        h.record(2, [zombie(5, 1.0)]);
        h.record(3, [zombie(5, 2.0)]);
        let (b, _) = h.possible_box(5, 2, 3).unwrap();
        assert!((b.min_x - 0.7).abs() < 1e-6 && (b.max_x - 2.3).abs() < 1e-6);
        assert!(h.possible_box(6, 1, 3).is_none());
    }

    #[test]
    fn builtin_sizes() {
        assert_eq!(builtin_size("minecraft:player"), Some((0.6, 1.8)));
        assert!(builtin_size("minecraft:item").is_none());
        assert!(is_reach_exempt_kind("minecraft:oak_boat"));
    }
}
