// Grim-Horizon: a Rust port of Grim Anticheat (https://github.com/GrimAnticheat/Grim).
// Copyright (C) GrimAC contributors (original Java), Grim-Horizon contributors (port).
// SPDX-License-Identifier: GPL-3.0-only (GNU General Public License, version 3; see LICENSE)

//! Vectors, boxes and Minecraft's math, ported from Grim's `Vector3dm`,
//! `SimpleCollisionBox`, `GrimMath`, `VectorUtils` and `ModernVanillaMath`
//! (the 1.21.11+ sine table that 26.3 clients use).

use std::sync::OnceLock;

/// `SimpleCollisionBox.COLLISION_EPSILON`.
pub const COLLISION_EPSILON: f64 = 1.0e-7;

/// A double-precision vector; method names follow Grim's `Vector3dm`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[allow(clippy::should_implement_trait)]
impl Vec3 {
    pub const ZERO: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    #[inline]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
    #[inline]
    pub fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    #[inline]
    pub fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    #[inline]
    pub fn scale(self, s: f64) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
    #[inline]
    pub fn mul(self, x: f64, y: f64, z: f64) -> Vec3 {
        Vec3::new(self.x * x, self.y * y, self.z * z)
    }
    #[inline]
    pub fn dot(self, o: Vec3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    #[inline]
    pub fn length_squared(self) -> f64 {
        self.dot(self)
    }
    #[inline]
    pub fn length(self) -> f64 {
        self.length_squared().sqrt()
    }
    #[inline]
    pub fn horizontal_length_squared(self) -> f64 {
        self.x * self.x + self.z * self.z
    }
    #[inline]
    pub fn distance_squared(self, o: Vec3) -> f64 {
        self.sub(o).length_squared()
    }
    #[inline]
    pub fn distance(self, o: Vec3) -> f64 {
        self.distance_squared(o).sqrt()
    }
    #[inline]
    pub fn with_y(self, y: f64) -> Vec3 {
        Vec3::new(self.x, y, self.z)
    }
    /// `VectorUtils.normalize` for 1.21.2+ clients (`Vec3.normalize`).
    pub fn normalize(self) -> Vec3 {
        let d = self.length();
        if d < f64::from(1.0e-5_f32) {
            Vec3::ZERO
        } else {
            Vec3::new(self.x / d, self.y / d, self.z / d)
        }
    }
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

/// An axis-aligned box; Grim's `SimpleCollisionBox`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Aabb {
    pub min_x: f64,
    pub min_y: f64,
    pub min_z: f64,
    pub max_x: f64,
    pub max_y: f64,
    pub max_z: f64,
}

impl Aabb {
    #[inline]
    pub const fn new(
        min_x: f64,
        min_y: f64,
        min_z: f64,
        max_x: f64,
        max_y: f64,
        max_z: f64,
    ) -> Self {
        Self {
            min_x,
            min_y,
            min_z,
            max_x,
            max_y,
            max_z,
        }
    }
    pub fn from_points(a: Vec3, b: Vec3) -> Self {
        Self::new(a.x, a.y, a.z, b.x, b.y, b.z).sorted()
    }
    /// `GetBoundingBox.getBoundingBoxFromPosAndSizeRaw`.
    pub fn from_pos_size(x: f64, min_y: f64, z: f64, width: f32, height: f32) -> Self {
        let half = f64::from(width / 2.0);
        let (min_x, max_x) = (x - half, x + half);
        let max_y = min_y + f64::from(height);
        let (min_z, max_z) = (z - half, z + half);
        Self::new(
            min_x.min(max_x),
            min_y.min(max_y),
            min_z.min(max_z),
            min_x.max(max_x),
            min_y.max(max_y),
            min_z.max(max_z),
        )
    }
    pub fn sorted(self) -> Self {
        Self::new(
            self.min_x.min(self.max_x),
            self.min_y.min(self.max_y),
            self.min_z.min(self.max_z),
            self.min_x.max(self.max_x),
            self.min_y.max(self.max_y),
            self.min_z.max(self.max_z),
        )
    }
    #[inline]
    pub fn min(&self) -> Vec3 {
        Vec3::new(self.min_x, self.min_y, self.min_z)
    }
    #[inline]
    pub fn max(&self) -> Vec3 {
        Vec3::new(self.max_x, self.max_y, self.max_z)
    }
    #[inline]
    pub fn offset(self, x: f64, y: f64, z: f64) -> Self {
        Self::new(
            self.min_x + x,
            self.min_y + y,
            self.min_z + z,
            self.max_x + x,
            self.max_y + y,
            self.max_z + z,
        )
    }
    #[inline]
    pub fn offset_vec(self, v: Vec3) -> Self {
        self.offset(v.x, v.y, v.z)
    }
    /// `expand(x, y, z)`: grows (or shrinks with negatives) on both sides.
    #[inline]
    pub fn expand(self, x: f64, y: f64, z: f64) -> Self {
        Self::new(
            self.min_x - x,
            self.min_y - y,
            self.min_z - z,
            self.max_x + x,
            self.max_y + y,
            self.max_z + z,
        )
    }
    #[inline]
    pub fn inflate(self, v: f64) -> Self {
        self.expand(v, v, v)
    }
    /// `expandToCoordinate`: stretches the box towards a movement.
    pub fn expand_towards(mut self, x: f64, y: f64, z: f64) -> Self {
        if x < 0.0 {
            self.min_x += x;
        } else {
            self.max_x += x;
        }
        if y < 0.0 {
            self.min_y += y;
        } else {
            self.max_y += y;
        }
        if z < 0.0 {
            self.min_z += z;
        } else {
            self.max_z += z;
        }
        self
    }
    /// `expandToAbsoluteCoordinates`: includes a point.
    pub fn include_point(mut self, x: f64, y: f64, z: f64) -> Self {
        self.min_x = self.min_x.min(x);
        self.min_y = self.min_y.min(y);
        self.min_z = self.min_z.min(z);
        self.max_x = self.max_x.max(x);
        self.max_y = self.max_y.max(y);
        self.max_z = self.max_z.max(z);
        self
    }
    pub fn union(self, o: Aabb) -> Aabb {
        Aabb::new(
            self.min_x.min(o.min_x),
            self.min_y.min(o.min_y),
            self.min_z.min(o.min_z),
            self.max_x.max(o.max_x),
            self.max_y.max(o.max_y),
            self.max_z.max(o.max_z),
        )
    }
    /// `isIntersected`: open intervals, as vanilla's `AABB.intersects`.
    #[inline]
    pub fn intersects(&self, o: &Aabb) -> bool {
        self.min_x < o.max_x
            && self.max_x > o.min_x
            && self.min_y < o.max_y
            && self.max_y > o.min_y
            && self.min_z < o.max_z
            && self.max_z > o.min_z
    }
    /// `isCollided`: closed intervals.
    #[inline]
    pub fn touches(&self, o: &Aabb) -> bool {
        self.min_x <= o.max_x
            && self.max_x >= o.min_x
            && self.min_y <= o.max_y
            && self.max_y >= o.min_y
            && self.min_z <= o.max_z
            && self.max_z >= o.min_z
    }
    pub fn contains(&self, p: Vec3) -> bool {
        p.x > self.min_x
            && p.x < self.max_x
            && p.y > self.min_y
            && p.y < self.max_y
            && p.z > self.min_z
            && p.z < self.max_z
    }
    pub fn width_x(&self) -> f64 {
        self.max_x - self.min_x
    }
    pub fn width_z(&self) -> f64 {
        self.max_z - self.min_z
    }

    /// `collideX`: how far `moving` may travel along X before it hits this box.
    #[inline]
    pub fn collide_x(&self, moving: &Aabb, offset_x: f64) -> f64 {
        if offset_x != 0.0
            && (moving.min_y - self.max_y) < -COLLISION_EPSILON
            && (moving.max_y - self.min_y) > COLLISION_EPSILON
            && (moving.min_z - self.max_z) < -COLLISION_EPSILON
            && (moving.max_z - self.min_z) > COLLISION_EPSILON
        {
            if offset_x >= 0.0 {
                let max_move = self.min_x - moving.max_x;
                return if max_move < -COLLISION_EPSILON {
                    offset_x
                } else {
                    max_move.min(offset_x)
                };
            } else {
                let max_move = self.max_x - moving.min_x;
                return if max_move > COLLISION_EPSILON {
                    offset_x
                } else {
                    max_move.max(offset_x)
                };
            }
        }
        offset_x
    }
    #[inline]
    pub fn collide_y(&self, moving: &Aabb, offset_y: f64) -> f64 {
        if offset_y != 0.0
            && (moving.min_x - self.max_x) < -COLLISION_EPSILON
            && (moving.max_x - self.min_x) > COLLISION_EPSILON
            && (moving.min_z - self.max_z) < -COLLISION_EPSILON
            && (moving.max_z - self.min_z) > COLLISION_EPSILON
        {
            if offset_y >= 0.0 {
                let max_move = self.min_y - moving.max_y;
                return if max_move < -COLLISION_EPSILON {
                    offset_y
                } else {
                    max_move.min(offset_y)
                };
            } else {
                let max_move = self.max_y - moving.min_y;
                return if max_move > COLLISION_EPSILON {
                    offset_y
                } else {
                    max_move.max(offset_y)
                };
            }
        }
        offset_y
    }
    #[inline]
    pub fn collide_z(&self, moving: &Aabb, offset_z: f64) -> f64 {
        if offset_z != 0.0
            && (moving.min_x - self.max_x) < -COLLISION_EPSILON
            && (moving.max_x - self.min_x) > COLLISION_EPSILON
            && (moving.min_y - self.max_y) < -COLLISION_EPSILON
            && (moving.max_y - self.min_y) > COLLISION_EPSILON
        {
            if offset_z >= 0.0 {
                let max_move = self.min_z - moving.max_z;
                return if max_move < -COLLISION_EPSILON {
                    offset_z
                } else {
                    max_move.min(offset_z)
                };
            } else {
                let max_move = self.max_z - moving.min_z;
                return if max_move > COLLISION_EPSILON {
                    offset_z
                } else {
                    max_move.max(offset_z)
                };
            }
        }
        offset_z
    }
    /// `collidesVertically`, used by NoFall: the boxes overlap on X and Z
    /// and touch on Y.
    pub fn collides_vertically(&self, other: &Aabb) -> bool {
        other.max_x > self.min_x
            && other.min_x < self.max_x
            && other.max_z > self.min_z
            && other.min_z < self.max_z
            && other.max_y >= self.min_y
            && other.min_y <= self.max_y
    }

    /// `ReachUtils.calculateIntercept`: the first point where the segment
    /// from `start` to `end` enters the box.
    pub fn intercept(&self, start: Vec3, end: Vec3) -> Option<Vec3> {
        let mut best: Option<(f64, Vec3)> = None;
        let mut consider = |hit: Option<Vec3>| {
            if let Some(p) = hit {
                let d = start.distance_squared(p);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, p));
                }
            }
        };
        let bx = |p: Option<Vec3>, f: &dyn Fn(Vec3) -> bool| p.filter(|v| f(*v));
        let in_yz = |v: Vec3| {
            v.y >= self.min_y && v.y <= self.max_y && v.z >= self.min_z && v.z <= self.max_z
        };
        let in_xz = |v: Vec3| {
            v.x >= self.min_x && v.x <= self.max_x && v.z >= self.min_z && v.z <= self.max_z
        };
        let in_xy = |v: Vec3| {
            v.x >= self.min_x && v.x <= self.max_x && v.y >= self.min_y && v.y <= self.max_y
        };
        consider(bx(intermediate_x(start, end, self.min_x), &in_yz));
        consider(bx(intermediate_x(start, end, self.max_x), &in_yz));
        consider(bx(intermediate_y(start, end, self.min_y), &in_xz));
        consider(bx(intermediate_y(start, end, self.max_y), &in_xz));
        consider(bx(intermediate_z(start, end, self.min_z), &in_xy));
        consider(bx(intermediate_z(start, end, self.max_z), &in_xy));
        best.map(|(_, p)| p)
    }
}

fn intermediate_x(a: Vec3, b: Vec3, x: f64) -> Option<Vec3> {
    let d = b.x - a.x;
    if d * d < 1.0e-7 {
        return None;
    }
    let t = (x - a.x) / d;
    (0.0..=1.0)
        .contains(&t)
        .then(|| Vec3::new(a.x + d * t, a.y + (b.y - a.y) * t, a.z + (b.z - a.z) * t))
}
fn intermediate_y(a: Vec3, b: Vec3, y: f64) -> Option<Vec3> {
    let d = b.y - a.y;
    if d * d < 1.0e-7 {
        return None;
    }
    let t = (y - a.y) / d;
    (0.0..=1.0)
        .contains(&t)
        .then(|| Vec3::new(a.x + (b.x - a.x) * t, a.y + d * t, a.z + (b.z - a.z) * t))
}
fn intermediate_z(a: Vec3, b: Vec3, z: f64) -> Option<Vec3> {
    let d = b.z - a.z;
    if d * d < 1.0e-7 {
        return None;
    }
    let t = (z - a.z) / d;
    (0.0..=1.0)
        .contains(&t)
        .then(|| Vec3::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t, a.z + d * t))
}

/// `VectorUtils.cutBoxToVector`: the point of `[min, max]` closest to `target`.
#[inline]
pub fn cut_box_to_vector(target: Vec3, min: Vec3, max: Vec3) -> Vec3 {
    Vec3::new(
        target.x.clamp(min.x.min(max.x), min.x.max(max.x)),
        target.y.clamp(min.y.min(max.y), min.y.max(max.y)),
        target.z.clamp(min.z.min(max.z), min.z.max(max.z)),
    )
}

/// `GrimMath.equal`.
#[inline]
pub fn grim_equal(a: f64, b: f64) -> bool {
    (b - a).abs() < f64::from(1.0e-5_f32)
}

/// `Mth.floor` for doubles.
#[inline]
pub fn floor(v: f64) -> i32 {
    v.floor() as i32
}
#[inline]
pub fn ceil(v: f64) -> i32 {
    v.ceil() as i32
}
/// `Mth.lerp`.
#[inline]
pub fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}
#[inline]
pub fn lerp_f32(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

/// `GrimMath.radians`: degrees times `(float) (PI / 180)` in float math.
#[inline]
pub fn radians(degrees: f32) -> f32 {
    degrees * (std::f64::consts::PI / 180.0) as f32
}

/// `Mth.wrapDegrees`.
pub fn wrap_degrees(d: f32) -> f32 {
    let mut f = d % 360.0;
    if f >= 180.0 {
        f -= 360.0;
    }
    if f < -180.0 {
        f += 360.0;
    }
    f
}

fn sin_table() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..65536)
            .map(|i| (f64::from(i) / 10430.378350470453).sin() as f32)
            .collect()
    })
}

/// `ModernVanillaMath.sin` (`Mth.sin` since 1.21.11): a 65536-entry table
/// indexed by the radians as a double.
#[inline]
pub fn mc_sin(value: f64) -> f32 {
    sin_table()[((value * 10430.378350470453) as i64 & 65535) as usize]
}
#[inline]
pub fn mc_cos(value: f64) -> f32 {
    sin_table()[((value * 10430.378350470453 + 16384.0) as i64 & 65535) as usize]
}

/// `ReachUtils.getLook`: the view vector for a rotation (`Entity.calculateViewVector`).
pub fn look_vector(yaw: f32, pitch: f32) -> Vec3 {
    let f = pitch * (std::f64::consts::PI / 180.0) as f32;
    let f1 = -yaw * (std::f64::consts::PI / 180.0) as f32;
    let f2 = mc_cos(f64::from(f1));
    let f3 = mc_sin(f64::from(f1));
    let f4 = mc_cos(f64::from(f));
    let f5 = mc_sin(f64::from(f));
    Vec3::new(f64::from(f3 * f4), f64::from(-f5), f64::from(f2 * f4))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_table_matches_vanilla_points() {
        assert_eq!(mc_sin(0.0), 0.0);
        assert!((mc_sin(std::f64::consts::FRAC_PI_2) - 1.0).abs() < 1e-6);
        assert!((mc_cos(0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn collide_y_stops_on_floor() {
        let floor = Aabb::new(0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
        let player = Aabb::from_pos_size(0.5, 1.2, 0.5, 0.6, 1.8);
        assert!((floor.collide_y(&player, -0.5) - -0.2).abs() < 1e-12);
        // No overlap on X: passes freely.
        let far = player.offset(3.0, 0.0, 0.0);
        assert_eq!(floor.collide_y(&far, -0.5), -0.5);
    }

    #[test]
    fn intercept_hits_front_face() {
        let b = Aabb::new(2.0, 0.0, -0.5, 3.0, 2.0, 0.5);
        let hit = b
            .intercept(Vec3::new(0.0, 1.0, 0.0), Vec3::new(6.0, 1.0, 0.0))
            .unwrap();
        assert!((hit.x - 2.0).abs() < 1e-12);
        assert!(
            b.intercept(Vec3::new(0.0, 5.0, 0.0), Vec3::new(6.0, 5.0, 0.0))
                .is_none()
        );
    }

    #[test]
    fn look_vector_south_is_positive_z() {
        let v = look_vector(0.0, 0.0);
        assert!(v.z > 0.999 && v.x.abs() < 1e-3 && v.y.abs() < 1e-6);
    }
}
