// Grim-Horizon: a Rust port of Grim Anticheat (https://github.com/GrimAnticheat/Grim).
// Copyright (C) GrimAC contributors (original Java), Grim-Horizon contributors (port).
// Licensed under the GNU General Public License, version 3 or (at your option) any later version; see LICENSE.

//! Ports of Grim's `KnockbackHandler` (AntiKB) and `ExplosionHandler`.
//!
//! Grim sandwiches each velocity packet between two transactions. Here every
//! server tick ends with one transaction, and a velocity is tagged with the
//! last transaction sent before it: once the client answers that one the
//! velocity *may* have been applied ("first bread"); once it answers a later
//! one the velocity *must* have been applied.

use std::collections::VecDeque;

use crate::math::Vec3;
use crate::player::{Movement, VectorData, VelocityData};

/// Thresholds from Grim's `Knockback.*` / `Explosion.*` config.
#[derive(Clone, Copy, Debug)]
pub struct VelocityConfig {
    pub threshold: f64,
    pub max_advantage: f64,
    pub immediate_setback_threshold: f64,
    pub setback_decay_multiplier: f64,
    pub max_ceiling: f64,
}

impl Default for VelocityConfig {
    fn default() -> Self {
        Self {
            threshold: 0.001,
            max_advantage: 1.0,
            immediate_setback_threshold: 0.1,
            setback_decay_multiplier: 0.999,
            max_ceiling: 4.0,
        }
    }
}

/// What the handler concluded after a prediction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VelocityVerdict {
    /// Nothing to judge.
    None,
    /// The required velocity was taken.
    Taken,
    /// The required velocity was not taken: `offset` is how far off the best
    /// prediction with it applied was (`i32::MAX` when it was ignored).
    Flag {
        offset: f64,
        ignored: bool,
        wants_setback: bool,
    },
}

#[derive(Debug, Default)]
pub struct KnockbackHandler {
    first_bread_map: VecDeque<VelocityData>,
    last_known_taken: VecDeque<VelocityData>,
    first_bread_only: Option<VelocityData>,
    pub point_three: bool,
    threshold: f64,
}

impl KnockbackHandler {
    pub const LIMIT: usize = 64;

    pub fn add(&mut self, vector: Vec3, transaction: u32, is_setback: bool) {
        if self.first_bread_map.len() >= Self::LIMIT {
            self.first_bread_map.pop_front();
        }
        let mut v = VelocityData::new(vector, transaction);
        v.is_setback = is_setback;
        self.first_bread_map.push_back(v);
    }

    fn tick(&mut self, transaction: u32) {
        self.first_bread_only = None;
        while let Some(data) = self.first_bread_map.front().copied() {
            if data.transaction == transaction {
                self.first_bread_only = Some(VelocityData {
                    offset: f64::from(i32::MAX),
                    ..data
                });
                break;
            } else if data.transaction < transaction {
                let mut taken = VelocityData {
                    offset: f64::from(i32::MAX),
                    ..data
                };
                if self.first_bread_only.is_some() {
                    taken.offset = data.offset;
                }
                self.last_known_taken.push_back(taken);
                self.first_bread_only = None;
                self.first_bread_map.pop_front();
            } else {
                break;
            }
        }
    }

    /// `calculateRequiredKB`: the last velocity the client must have applied.
    pub fn calculate_required(
        &mut self,
        transaction: u32,
        just_testing: bool,
    ) -> Option<VelocityData> {
        self.tick(transaction);
        let last = self.last_known_taken.back().copied();
        if !just_testing {
            self.last_known_taken.clear();
        }
        last
    }

    /// `calculateFirstBreadKnockback`.
    pub fn calculate_first_bread(&mut self, transaction: u32) -> Option<VelocityData> {
        self.tick(transaction);
        self.first_bread_only
    }

    pub fn force_exempt(p: &mut Movement) {
        if let Some(k) = p.first_bread_kb.as_mut() {
            k.offset = 0.0;
        }
        if let Some(k) = p.likely_kb.as_mut() {
            k.offset = 0.0;
        }
    }

    pub fn handle_prediction_analysis(p: &mut Movement, offset: f64) {
        if let Some(k) = p.first_bread_kb.as_mut() {
            k.offset = k.offset.min(offset);
        }
        if let Some(k) = p.likely_kb.as_mut() {
            k.offset = k.offset.min(offset);
        }
    }

    pub fn should_ignore_for_prediction(
        p: &Movement,
        data: &VectorData,
        cfg: &VelocityConfig,
    ) -> bool {
        if data.is_knockback() && data.is_first_bread_kb() {
            return p.first_bread_kb.is_some_and(|k| k.offset > cfg.threshold);
        }
        false
    }

    pub fn would_flag(p: &Movement, cfg: &VelocityConfig) -> bool {
        p.likely_kb.is_some_and(|k| k.offset > cfg.threshold)
            || p.first_bread_kb.is_some_and(|k| k.offset > cfg.threshold)
    }

    /// `KnockbackHandler.onPredictionComplete`.
    pub fn on_prediction_complete(
        &mut self,
        p: &mut Movement,
        offset: f64,
        checked: bool,
        teleport: bool,
        cfg: &VelocityConfig,
    ) -> VelocityVerdict {
        if !checked || teleport {
            Self::force_exempt(p);
            return VelocityVerdict::None;
        }
        let was_zero = self.point_three;
        self.point_three = false;
        if p.likely_kb.is_none() && p.first_bread_kb.is_none() {
            return VelocityVerdict::None;
        }
        if p.predicted.is_first_bread_kb() {
            self.first_bread_only = None;
            self.first_bread_map.pop_front();
        }
        if was_zero || p.predicted.is_knockback() {
            Self::handle_prediction_analysis(p, offset);
        }
        if let Some(likely) = p.likely_kb {
            if likely.offset > cfg.threshold {
                self.threshold = (self.threshold + likely.offset).min(cfg.max_ceiling);
                let ignored = likely.offset == f64::from(i32::MAX);
                let wants_setback = likely.is_setback
                    || likely.offset >= cfg.immediate_setback_threshold
                    || self.threshold >= cfg.max_advantage;
                if likely.is_setback {
                    return VelocityVerdict::Flag {
                        offset: likely.offset,
                        ignored,
                        wants_setback: true,
                    };
                }
                return VelocityVerdict::Flag {
                    offset: likely.offset,
                    ignored,
                    wants_setback,
                };
            } else if self.threshold > 0.05 {
                self.threshold *= cfg.setback_decay_multiplier;
            }
            return VelocityVerdict::Taken;
        }
        VelocityVerdict::None
    }

    pub fn pending(&self) -> usize {
        self.first_bread_map.len() + self.last_known_taken.len()
    }
    pub fn clear(&mut self) {
        self.first_bread_map.clear();
        self.last_known_taken.clear();
        self.first_bread_only = None;
    }
}

#[derive(Debug, Default)]
pub struct ExplosionHandler {
    first_bread_map: VecDeque<VelocityData>,
    last_known_taken: Option<VelocityData>,
    first_bread_added: Option<VelocityData>,
    pub point_three: bool,
}

impl ExplosionHandler {
    pub fn add(&mut self, vector: Vec3, transaction: u32) {
        if self.first_bread_map.len() >= KnockbackHandler::LIMIT {
            self.first_bread_map.pop_front();
        }
        self.first_bread_map
            .push_back(VelocityData::new(vector, transaction));
    }

    fn handle_transaction(&mut self, transaction: u32) {
        while let Some(data) = self.first_bread_map.front().copied() {
            if data.transaction == transaction {
                let vector = match self.last_known_taken {
                    Some(l) => l.vector.add(data.vector),
                    None => data.vector,
                };
                self.first_bread_added = Some(VelocityData { vector, ..data });
                break;
            } else if data.transaction < transaction {
                match self.last_known_taken.as_mut() {
                    Some(l) => l.vector = l.vector.add(data.vector),
                    None => self.last_known_taken = Some(data),
                }
                self.first_bread_added = None;
                self.first_bread_map.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn possible(&mut self, transaction: u32, just_testing: bool) -> Option<VelocityData> {
        self.handle_transaction(transaction);
        let r = self.last_known_taken;
        if !just_testing {
            self.last_known_taken = None;
        }
        r
    }

    pub fn first_bread_added(&mut self, transaction: u32) -> Option<VelocityData> {
        self.handle_transaction(transaction);
        self.first_bread_added
    }

    pub fn force_exempt(p: &mut Movement) {
        if let Some(k) = p.first_bread_explosion.as_mut() {
            k.offset = 0.0;
        }
        if let Some(k) = p.likely_explosion.as_mut() {
            k.offset = 0.0;
        }
    }

    pub fn handle_prediction_analysis(p: &mut Movement, offset: f64) {
        if let Some(k) = p.first_bread_explosion.as_mut() {
            k.offset = k.offset.min(offset);
        }
        if let Some(k) = p.likely_explosion.as_mut() {
            k.offset = k.offset.min(offset);
        }
    }

    pub fn should_ignore_for_prediction(p: &Movement, data: &VectorData, threshold: f64) -> bool {
        if data.is_explosion() && data.is_first_bread_explosion() {
            return p
                .first_bread_explosion
                .is_some_and(|k| k.offset > threshold);
        }
        false
    }

    pub fn would_flag(p: &Movement, threshold: f64) -> bool {
        p.likely_explosion.is_some_and(|k| k.offset > threshold)
            || p.first_bread_explosion
                .is_some_and(|k| k.offset > threshold)
    }

    /// `ExplosionHandler.onPredictionComplete`.
    pub fn on_prediction_complete(
        &mut self,
        p: &mut Movement,
        offset: f64,
        threshold: f64,
    ) -> VelocityVerdict {
        let was_zero = self.point_three;
        self.point_three = false;
        if p.likely_explosion.is_none() && p.first_bread_explosion.is_none() {
            self.first_bread_added = None;
            return VelocityVerdict::None;
        }
        let min_trans = p
            .likely_explosion
            .map_or(u32::MAX, |e| e.transaction)
            .min(p.first_bread_explosion.map_or(u32::MAX, |e| e.transaction));
        let kb_trans = p
            .likely_kb
            .map(|k| k.transaction)
            .max(p.first_bread_kb.map(|k| k.transaction));
        if p.predicted.is_first_bread_explosion() {
            self.first_bread_added = None;
            self.first_bread_map.pop_front();
        }
        if was_zero || p.predicted.is_explosion() || kb_trans.is_some_and(|k| min_trans < k) {
            Self::handle_prediction_analysis(p, offset);
        }
        if let Some(likely) = p.likely_explosion
            && !p.is_dead
        {
            if likely.offset > threshold {
                return VelocityVerdict::Flag {
                    offset,
                    ignored: likely.offset == f64::from(i32::MAX),
                    wants_setback: true,
                };
            }
            return VelocityVerdict::Taken;
        }
        VelocityVerdict::None
    }

    pub fn clear(&mut self) {
        self.first_bread_map.clear();
        self.last_known_taken = None;
        self.first_bread_added = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knockback_moves_from_first_bread_to_required() {
        let mut h = KnockbackHandler::default();
        h.add(Vec3::new(0.0, 0.4, 0.3), 5, false);
        // The client has not answered transaction 5 yet.
        assert!(h.calculate_first_bread(4).is_none());
        assert!(h.calculate_required(4, false).is_none());
        // It answered 5: the velocity may have been applied.
        assert!(h.calculate_first_bread(5).is_some());
        assert!(h.calculate_required(5, false).is_none());
        // It answered 6: the velocity must have been applied.
        assert!(h.calculate_required(6, false).is_some());
        assert!(h.calculate_required(6, false).is_none());
    }

    #[test]
    fn explosions_accumulate() {
        let mut h = ExplosionHandler::default();
        h.add(Vec3::new(1.0, 0.0, 0.0), 3);
        h.add(Vec3::new(0.0, 1.0, 0.0), 3);
        let e = h.possible(4, false).unwrap();
        assert_eq!(e.vector, Vec3::new(1.0, 1.0, 0.0));
    }
}
