// Grim-Horizon: a Rust port of Grim Anticheat (https://github.com/GrimAnticheat/Grim).
// Copyright (C) GrimAC contributors (original Java), Grim-Horizon contributors (port).
// Licensed under the GNU General Public License, version 3 or (at your option) any later version; see LICENSE.

//! Ports of `EntityFluidInteraction` (the 26.3 path) and `FluidTypeFlowing`.

use crate::math::{Aabb, Vec3, ceil, floor};
use crate::player::Movement;
use crate::world::{BlockView, FluidKind};

/// What one `EntityFluidInteraction.update` found.
#[derive(Clone, Copy, Debug, Default)]
pub struct FluidScan {
    pub water_height: f64,
    pub lava_height: f64,
    pub eye_in_water: bool,
    pub eye_in_lava: bool,
    water_current: Vec3,
    water_count: u32,
    lava_current: Vec3,
    lava_count: u32,
    water_acc_height: f64,
    lava_acc_height: f64,
}

/// `FluidTypeFlowing.ownHeight`: the cell's own fluid height, ignoring the
/// fluid above it.
fn own_height(world: &dyn BlockView, x: i32, y: i32, z: i32, kind: FluidKind) -> f32 {
    let f = world.fluid(x, y, z);
    if f.kind != kind || f.kind == FluidKind::None {
        return 0.0;
    }
    f32::from(f.amount) / 9.0
}

fn affects_flow(world: &dyn BlockView, kind: FluidKind, x: i32, y: i32, z: i32) -> bool {
    let f = world.fluid(x, y, z);
    f.kind == FluidKind::None || f.kind == kind
}

/// `FluidTypeFlowing.getFlow` for 1.13+ clients.
pub fn flow(world: &dyn BlockView, x: i32, y: i32, z: i32) -> Vec3 {
    let here = world.fluid(x, y, z);
    if here.kind == FluidKind::None {
        return Vec3::ZERO;
    }
    let level = own_height(world, x, y, z, here.kind);
    if level == 0.0 {
        return Vec3::ZERO;
    }
    let (mut mod_x, mut mod_z) = (0.0, 0.0);
    // NORTH, EAST, SOUTH, WEST.
    for (dx, dz) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
        let (nx, nz) = (x + dx, z + dz);
        if affects_flow(world, here.kind, nx, y, nz) {
            let mut adjacent = own_height(world, nx, y, nz, here.kind);
            let mut flow = 0.0_f32;
            if adjacent == 0.0 {
                if world.block(nx, y, nz).blocks_motion {
                    // A solid neighbour: compare with the cell below it.
                } else if affects_flow(world, here.kind, nx, y - 1, nz) {
                    adjacent = own_height(world, nx, y - 1, nz, here.kind);
                    if adjacent > 0.0 {
                        flow = level - (adjacent - 0.8888889);
                    }
                }
            } else if adjacent > 0.0 {
                flow = level - adjacent;
            }
            if flow != 0.0 {
                mod_x += f64::from(dx) * f64::from(flow);
                mod_z += f64::from(dz) * f64::from(flow);
            }
        }
    }
    let mut v = Vec3::new(mod_x, 0.0, mod_z);
    if here.falling {
        for (dx, dz) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
            if world.is_solid_face(x + dx, y, z + dz) || world.is_solid_face(x + dx, y + 1, z + dz)
            {
                v = v.normalize().add(Vec3::new(0.0, -6.0, 0.0));
                break;
            }
        }
    }
    v.normalize()
}

fn has_fluid_and_loaded(world: &dyn BlockView, x0: i32, z0: i32, x1: i32, z1: i32) -> bool {
    world.are_chunks_loaded(x0, z0, x1, z1)
}

/// `EntityFluidInteraction.update` for a 26.3 client. `ignore_current` is
/// `!isPushedByFluid()`.
pub fn scan(p: &Movement, world: &dyn BlockView, ignore_current: bool) -> FluidScan {
    let mut out = FluidScan::default();
    let aabb = p.bounding_box.inflate(-0.001);
    let (min_x, min_y, min_z) = (floor(aabb.min_x), floor(aabb.min_y), floor(aabb.min_z));
    let (max_x, max_y, max_z) = (
        ceil(aabb.max_x) - 1,
        ceil(aabb.max_y) - 1,
        ceil(aabb.max_z) - 1,
    );
    if !has_fluid_and_loaded(world, min_x - 1, min_z - 1, max_x + 1, max_z + 1) {
        return out;
    }
    let aabb_min_y = p.bounding_box.min_y;
    let player_x = floor(p.last_x);
    let player_eye_y = p.last_y + p.eye_height() - 0.1111111119389534;
    let player_z = floor(p.last_z);

    for x in min_x..=max_x {
        for y in min_y..=max_y {
            for z in min_z..=max_z {
                let f = world.fluid(x, y, z);
                if f.kind == FluidKind::None || f.height == 0.0 {
                    continue;
                }
                let top = f64::from(y) + f64::from(f.height);
                if top < aabb.min_y {
                    continue;
                }
                let water = f.kind == FluidKind::Water;
                if x == player_x && z == player_z && player_eye_y >= f64::from(y) {
                    let camera_top = if f.source && world.is_solid_face(x, y + 1, z) {
                        f64::from(y) + 1.0
                    } else {
                        top
                    };
                    if player_eye_y <= camera_top {
                        if water {
                            out.eye_in_water = true;
                        } else {
                            out.eye_in_lava = true;
                        }
                    }
                }
                let height = if water {
                    &mut out.water_height
                } else {
                    &mut out.lava_height
                };
                *height = height.max(top - aabb_min_y);
                let h = *height;
                if !ignore_current {
                    let mut current = flow(world, x, y, z);
                    let (acc_height, acc, count) = if water {
                        (
                            &mut out.water_acc_height,
                            &mut out.water_current,
                            &mut out.water_count,
                        )
                    } else {
                        (
                            &mut out.lava_acc_height,
                            &mut out.lava_current,
                            &mut out.lava_count,
                        )
                    };
                    *acc_height = acc_height.max(h);
                    if *acc_height < 0.4 {
                        current = current.scale(*acc_height);
                    }
                    *acc = acc.add(current);
                    *count += 1;
                }
            }
        }
    }
    out
}

impl FluidScan {
    /// `CurrentAccumulator.applyCurrentTo`: returns (the push before the
    /// 0.003 rule, the push actually added).
    pub fn current(&self, water: bool, scale: f64, client_velocity: Vec3) -> Option<(Vec3, Vec3)> {
        let (acc, count) = if water {
            (self.water_current, self.water_count)
        } else {
            (self.lava_current, self.lava_count)
        };
        if count == 0 || acc.length_squared() < f64::from(1.0e-5_f32) {
            return None;
        }
        let current = acc.scale(1.0 / f64::from(count)).scale(scale);
        let mut added = current;
        if client_velocity.x.abs() < 0.003
            && client_velocity.z.abs() < 0.003
            && current.length() < 0.0045000000000000005
        {
            added = current.normalize().scale(0.0045000000000000005);
        }
        Some((current, added))
    }
}

/// Whether there is any liquid in `bb` (`CompensatedWorld.containsLiquid`).
pub fn contains_liquid(world: &dyn BlockView, bb: Aabb) -> bool {
    for x in floor(bb.min_x)..=floor(bb.max_x) {
        for y in floor(bb.min_y)..=floor(bb.max_y) {
            for z in floor(bb.min_z)..=floor(bb.max_z) {
                let f = world.fluid(x, y, z);
                if f.kind != FluidKind::None && f64::from(y) + f64::from(f.height) > bb.min_y {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{FluidFacts, TestWorld};

    #[test]
    fn still_pool_has_no_flow_and_full_height() {
        let mut w = TestWorld::new();
        w.water_pool(-2, 2, 60, 62, -2, 2);
        assert_eq!(flow(&w, 0, 61, 0), Vec3::ZERO);
        let mut p = Movement::default();
        (p.last_x, p.last_y, p.last_z) = (0.5, 61.0, 0.5);
        p.bounding_box = p.pose_box_at(0.5, 61.0, 0.5);
        let s = scan(&p, &w, false);
        assert!(s.water_height > 1.5);
        assert!(s.eye_in_water);
    }

    #[test]
    fn flowing_water_pushes_downhill() {
        let mut w = TestWorld::new();
        w.set_fluid(
            0,
            64,
            0,
            FluidFacts {
                kind: FluidKind::Water,
                amount: 7,
                source: false,
                falling: false,
                height: 7.0 / 9.0,
            },
        );
        w.set_fluid(
            1,
            64,
            0,
            FluidFacts {
                kind: FluidKind::Water,
                amount: 6,
                source: false,
                falling: false,
                height: 6.0 / 9.0,
            },
        );
        let v = flow(&w, 0, 64, 0);
        assert!(v.x > 0.0, "{v:?}");
    }
}
