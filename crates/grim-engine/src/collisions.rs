// Grim-Horizon: a Rust port of Grim Anticheat (https://github.com/GrimAnticheat/Grim).
// Copyright (C) GrimAC contributors (original Java), Grim-Horizon contributors (port).
// Licensed under the GNU General Public License v3.0 only; see LICENSE.

//! Ports of Grim's `Collisions`, `MainSupportingBlockPosFinder`,
//! `BlockProperties` and `StuckSpeed` for 26.3 clients (1.21+ stepping,
//! 26.2+ friction, drag and bounciness attributes).

use crate::math::{Aabb, COLLISION_EPSILON, Vec3, cut_box_to_vector, floor, lerp_f32};
use crate::player::{Attr, Movement, SupportingBlock, VectorData};
use crate::world::{BlockFacts, BlockKind, BlockView};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    X,
    Y,
    Z,
}

/// `nonStupidityCombinations`: the orders vanilla uses (Y first, then the
/// larger horizontal axis first).
const NORMAL_ORDERS: [[Axis; 3]; 2] = [[Axis::Y, Axis::X, Axis::Z], [Axis::Y, Axis::Z, Axis::X]];
/// `allAxisCombinations`, tried for movements hidden by the movement threshold.
const ALL_ORDERS: [[Axis; 3]; 6] = [
    [Axis::Y, Axis::X, Axis::Z],
    [Axis::Y, Axis::Z, Axis::X],
    [Axis::X, Axis::Y, Axis::Z],
    [Axis::X, Axis::Z, Axis::Y],
    [Axis::Z, Axis::X, Axis::Y],
    [Axis::Z, Axis::Y, Axis::X],
];

/// `Collisions.getCollisionBoxes`.
pub fn collision_boxes(world: &dyn BlockView, query: Aabb, out: &mut Vec<Aabb>) {
    world.collision_boxes(query, out);
}

/// `Collisions.isEmpty`: no collision box touches `bb`.
pub fn is_empty(world: &dyn BlockView, bb: Aabb) -> bool {
    let mut boxes = Vec::new();
    world.collision_boxes(bb, &mut boxes);
    !boxes.iter().any(|b| b.touches(&bb))
}

/// `Collisions.collideBoundingBoxLegacy`.
pub fn collide_bounding_box(movement: Vec3, bb: Aabb, boxes: &[Aabb], order: &[Axis; 3]) -> Vec3 {
    let (mut x, mut y, mut z) = (movement.x, movement.y, movement.z);
    let mut set_bb = bb;
    for axis in order {
        match axis {
            Axis::X => {
                for b in boxes {
                    x = b.collide_x(&set_bb, x);
                }
                set_bb = set_bb.offset(x, 0.0, 0.0);
            }
            Axis::Y => {
                for b in boxes {
                    y = b.collide_y(&set_bb, y);
                }
                set_bb = set_bb.offset(0.0, y, 0.0);
            }
            Axis::Z => {
                for b in boxes {
                    z = b.collide_z(&set_bb, z);
                }
                set_bb = set_bb.offset(0.0, 0.0, z);
            }
        }
    }
    Vec3::new(x, y, z)
}

/// `Collisions.collectStepHeights`.
fn collect_step_heights(bb: Aabb, boxes: &[Aabb], step_height: f32, collide_y: f32) -> Vec<f32> {
    let mut out: Vec<f32> = Vec::with_capacity(4);
    for b in boxes {
        for possible in [b.min_y, b.max_y] {
            let y_diff = (possible - bb.min_y) as f32;
            if !(y_diff < 0.0) && y_diff != collide_y {
                if y_diff > step_height {
                    break;
                }
                if !out.contains(&y_diff) {
                    out.push(y_diff);
                }
            }
        }
    }
    out.sort_by(|a, b| a.total_cmp(b));
    out
}

pub fn horizontal_distance_sqr(v: Vec3) -> f64 {
    v.x * v.x + v.z * v.z
}

/// `Collisions.collide(player, x, y, z)` when there is no Y uncertainty.
pub fn collide_simple(p: &mut Movement, world: &dyn BlockView, x: f64, y: f64, z: f64) -> Vec3 {
    collide(p, world, x, y, z, y, None, false)
}

/// `Collisions.collide`: vanilla's `Entity.collide` with step-up, trying
/// the axis orders the client could have used and keeping the result
/// closest to the player's real movement.
#[allow(clippy::too_many_arguments)]
pub fn collide(
    p: &mut Movement,
    world: &dyn BlockView,
    desired_x: f64,
    desired_y: f64,
    desired_z: f64,
    client_vel_y: f64,
    data: Option<&VectorData>,
    disallow_stepping: bool,
) -> Vec3 {
    if desired_x == 0.0 && desired_y == 0.0 && desired_z == 0.0 {
        return Vec3::ZERO;
    }
    let bb = p.bounding_box;
    let step_up = p.max_up_step();
    let mut grab = bb;
    if desired_x == 0.0 && desired_z == 0.0 {
        if desired_y > 0.0 {
            grab.max_y += desired_y;
        } else {
            grab.min_y += desired_y;
        }
    } else if step_up > 0.0 && (p.last_on_ground || desired_y < 0.0 || client_vel_y < 0.0) {
        if desired_y <= 0.0 {
            grab = grab.expand_towards(desired_x, desired_y, desired_z);
            grab.max_y += step_up;
        } else {
            grab = grab.expand_towards(desired_x, step_up.max(desired_y), desired_z);
        }
    } else {
        grab = grab.expand_towards(desired_x, desired_y, desired_z);
    }

    let mut boxes = Vec::new();
    world.collision_boxes(grab, &mut boxes);

    let theoretical_box = Aabb::new(0.0, desired_y.min(0.0), 0.0, desired_x, step_up.max(desired_y), desired_z).sorted();
    let best_theoretical = cut_box_to_vector(p.actual_movement, theoretical_box.min(), theoretical_box.max());
    let zero_count = usize::from(desired_x == 0.0) + usize::from(desired_y == 0.0) + usize::from(desired_z == 0.0);

    let orders: &[[Axis; 3]] = if data.is_some_and(|d| d.is_zero_point_zero_three()) { &ALL_ORDERS } else { &NORMAL_ORDERS };
    let desired = Vec3::new(desired_x, desired_y, desired_z);
    let mut best_input = f64::MAX;
    let mut best: Option<Vec3> = None;
    let mut step_boxes = Vec::new();

    for order in orders {
        let mut result = collide_bounding_box(desired, bb, &boxes, order);
        // closeEnoughToGroundToStepWithPointThree is false for clients without 0.03.
        let moving_into_ground_real = result.y != desired_y && (desired_y < 0.0 || client_vel_y < 0.0);
        let moving_into_ground = p.last_on_ground || moving_into_ground_real;

        if !disallow_stepping
            && step_up > 0.0
            && moving_into_ground
            && (result.x != desired_x || result.z != desired_z)
        {
            p.uncertainty.is_step_movement = true;
            // The 1.21 stepping algorithm.
            let starting = if moving_into_ground_real { bb.offset(0.0, result.y, 0.0) } else { bb };
            let mut step_box = starting.expand_towards(desired_x, step_up, desired_z);
            if !moving_into_ground_real {
                step_box = step_box.expand_towards(0.0, -f64::from(1.0e-5_f32), 0.0);
            }
            step_boxes.clear();
            world.collision_boxes(step_box, &mut step_boxes);
            let heights = collect_step_heights(starting, &step_boxes, step_up as f32, result.y as f32);
            for h in heights {
                let v = collide_bounding_box(Vec3::new(desired_x, f64::from(h), desired_z), starting, &step_boxes, order);
                if horizontal_distance_sqr(v) > horizontal_distance_sqr(result) {
                    let d = bb.min_y - starting.min_y;
                    result = v.add(Vec3::new(0.0, -d, 0.0));
                    break;
                }
            }
        }

        let mut accuracy = result.distance_squared(best_theoretical);
        if p.would_collision_result_flag_ground_spoof(desired_y, result.y) {
            accuracy += 1.0;
        }
        if accuracy < best_input {
            best = Some(result);
            best_input = accuracy;
            if accuracy < 0.00001 * 0.00001 || zero_count >= 2 {
                break;
            }
        }
    }
    best.unwrap_or(Vec3::ZERO)
}

/// `Collisions.isAboveGround`.
pub fn is_above_ground(p: &Movement, world: &dyn BlockView) -> bool {
    let step = p.max_up_step();
    p.last_on_ground
        || (p.fall_distance < step && !is_empty(world, p.bounding_box.offset(0.0, p.fall_distance - step, 0.0)))
}

/// `Collisions.maybeBackOffFromEdge`: sneaking players do not walk off edges.
pub fn maybe_back_off_from_edge(v: Vec3, p: &Movement, world: &dyn BlockView) -> Vec3 {
    if p.is_flying || !p.is_sneaking || !is_above_ground(p, world) {
        return v;
    }
    let (mut x, mut z) = (v.x, v.z);
    let max_step_down = -p.max_up_step();
    let step = |a: &mut f64| {
        if *a < 0.05 && *a >= -0.05 {
            *a = 0.0;
        } else if *a > 0.0 {
            *a -= 0.05;
        } else {
            *a += 0.05;
        }
    };
    let mut guard = 0;
    while x != 0.0 && is_empty(world, p.bounding_box.offset(x, max_step_down, 0.0)) && guard < 400 {
        step(&mut x);
        guard += 1;
    }
    while z != 0.0 && is_empty(world, p.bounding_box.offset(0.0, max_step_down, z)) && guard < 800 {
        step(&mut z);
        guard += 1;
    }
    while x != 0.0 && z != 0.0 && is_empty(world, p.bounding_box.offset(x, max_step_down, z)) && guard < 1200 {
        step(&mut x);
        step(&mut z);
        guard += 1;
    }
    Vec3::new(x, v.y, z)
}

/// `Collisions.slowCouldPointThreeHitGround`.
pub fn slow_could_point_three_hit_ground(p: &mut Movement, world: &dyn BlockView, x: f64, y: f64, z: f64) -> bool {
    let old = p.bounding_box;
    p.bounding_box = p.box_at(x, y, z, 0.6, 0.06);
    let t = crate::player::MOVEMENT_THRESHOLD;
    let a = collide_simple(p, world, t, -t, t).y;
    let b = collide_simple(p, world, -t, -t, -t).y;
    let c = collide_simple(p, world, t, -t, -t).y;
    let d = collide_simple(p, world, -t, -t, t).y;
    p.bounding_box = old;
    b != -t || c != -t || a != -t || d != -t
}

/// `Collisions.onClimbable`.
pub fn on_climbable(world: &dyn BlockView, x: f64, y: f64, z: f64) -> bool {
    world.block(floor(x), floor(y), floor(z)).climbable
}

/// `Collisions.doesBlockSuffocate` / `suffocatesAt` for 1.16+ clients.
pub fn suffocates_at(world: &dyn BlockView, bb: Aabb) -> bool {
    let mut boxes = Vec::new();
    for y in floor(bb.min_y)..(bb.max_y.ceil() as i32) {
        for z in floor(bb.min_z)..(bb.max_z.ceil() as i32) {
            for x in floor(bb.min_x)..(bb.max_x.ceil() as i32) {
                if world.block(x, y, z).suffocates {
                    boxes.clear();
                    world.block_collision_boxes(x, y, z, &mut boxes);
                    let intersects = boxes.iter().any(|b| {
                        b.max_x - COLLISION_EPSILON > bb.min_x
                            && b.min_x + COLLISION_EPSILON < bb.max_x
                            && b.max_y - COLLISION_EPSILON > bb.min_y
                            && b.min_y + COLLISION_EPSILON < bb.max_y
                            && b.max_z - COLLISION_EPSILON > bb.min_z
                            && b.min_z + COLLISION_EPSILON < bb.max_z
                    });
                    if intersects {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Every non-air block position whose cell overlaps `bb` (`hasMaterial`).
pub fn for_each_block(world: &dyn BlockView, bb: Aabb, mut f: impl FnMut(i32, i32, i32, BlockFacts)) {
    let min_y = floor(bb.min_y).max(world.min_y());
    let max_y = floor(bb.max_y).min(world.max_y() - 1);
    for x in floor(bb.min_x)..=floor(bb.max_x) {
        for y in min_y..=max_y {
            for z in floor(bb.min_z)..=floor(bb.max_z) {
                let b = world.block(x, y, z);
                if !b.air {
                    f(x, y, z, b);
                }
            }
        }
    }
}

/// `Collisions.forEachCollisionBox`: blocks with a collision box that
/// intersects `bb`, by block position.
pub fn for_each_colliding_block(world: &dyn BlockView, bb: Aabb, mut f: impl FnMut(i32, i32, i32, BlockFacts)) {
    let mut boxes = Vec::new();
    // Fences and walls reach half a block above their cell.
    let search = Aabb::new(bb.min_x, bb.min_y - 0.5, bb.min_z, bb.max_x, bb.max_y, bb.max_z);
    for_each_block(world, search, |x, y, z, b| {
        boxes.clear();
        world.block_collision_boxes(x, y, z, &mut boxes);
        if boxes.iter().any(|c| c.intersects(&bb)) {
            f(x, y, z, b);
        }
    });
}

/// `MainSupportingBlockPosFinder.findMainSupportingBlockPos`.
pub fn find_main_supporting_block(
    p: &Movement,
    world: &dyn BlockView,
    last: SupportingBlock,
    last_movement: Vec3,
    bb: Aabb,
    on_ground: bool,
) -> SupportingBlock {
    if !on_ground {
        return SupportingBlock { pos: None, on_ground: false };
    }
    let below = Aabb::new(bb.min_x, bb.min_y - 1.0e-6, bb.min_z, bb.max_x, bb.min_y, bb.max_z);
    let found = find_supporting_block(p, world, below);
    if found.is_none() && !last.last_on_ground_and_no_block() {
        let moved = below.offset(-last_movement.x, 0.0, -last_movement.z);
        return SupportingBlock { pos: find_supporting_block(p, world, moved), on_ground: true };
    } else if found.is_some() {
        return SupportingBlock { pos: found, on_ground: true };
    }
    SupportingBlock { pos: None, on_ground: true }
}

fn find_supporting_block(p: &Movement, world: &dyn BlockView, search: Aabb) -> Option<[i32; 3]> {
    let pos = p.position();
    let mut best: Option<[i32; 3]> = None;
    let mut best_distance = f64::MAX;
    for_each_colliding_block(world, search, |x, y, z, _| {
        let center = Vec3::new(f64::from(x) + 0.5, f64::from(y) + 0.5, f64::from(z) + 0.5);
        let d = pos.distance_squared(center);
        if d < best_distance || (d == best_distance && best.is_none_or(|b| first_has_priority(x, y, z, b))) {
            best = Some([x, y, z]);
            best_distance = d;
        }
    });
    best
}

fn first_has_priority(x: i32, y: i32, z: i32, second: [i32; 3]) -> bool {
    if y < second[1] {
        return true;
    }
    let sum_x = f64::from(second[0] - x);
    let sum_z = f64::from(second[2] - z);
    let total = sum_x + sum_z;
    if total == 0.0 {
        return sum_x < 0.0;
    }
    total < 0.0
}

/// `BlockProperties.getOnPos`.
fn on_pos(p: &Movement, world: &dyn BlockView, pos: Vec3, support: SupportingBlock, below: f32) -> [i32; 3] {
    if let Some(main) = support.pos {
        let kind = world.block(main[0], main[1], main[2]).kind;
        let should_return = (!(f64::from(below) <= 0.5) || kind != BlockKind::FenceLike) && kind != BlockKind::FenceLike;
        let _ = p;
        if should_return { [main[0], floor(pos.y - f64::from(below)), main[2]] } else { main }
    } else {
        [floor(pos.x), floor(pos.y - f64::from(below)), floor(pos.z)]
    }
}

/// `getBlockPosBelowThatAffectsMyMovement`.
pub fn block_below_affecting_movement(p: &Movement, world: &dyn BlockView, support: SupportingBlock, pos: Vec3) -> BlockFacts {
    let b = on_pos(p, world, pos, support, 0.500001);
    world.block(b[0], b[1], b[2])
}

/// `BlockProperties.getOnPos(player, data, pos)` with 0.2 below: the block
/// landing bounces on.
pub fn on_block(p: &Movement, world: &dyn BlockView, support: SupportingBlock, pos: Vec3) -> BlockFacts {
    let b = on_pos(p, world, pos, support, 0.2);
    world.block(b[0], b[1], b[2])
}

/// `BlockProperties.getFriction`.
pub fn friction(p: &Movement, world: &dyn BlockView, support: SupportingBlock, pos: Vec3) -> f32 {
    block_below_affecting_movement(p, world, support, pos).friction
}

/// `BlockProperties.getModifiedFriction` (26.2+ `friction_modifier`).
pub fn modified_friction(friction: f32, p: &Movement) -> f32 {
    (1.0 - (1.0 - friction) * p.attr(Attr::FrictionModifier) as f32).clamp(0.0, 1.0)
}

/// `BlockProperties.getModifiedAirDrag` (26.2+ `air_drag_modifier`).
pub fn modified_air_drag(drag: f32, p: &Movement) -> f32 {
    (1.0 - (1.0 - drag) * p.attr(Attr::AirDragModifier) as f32).clamp(0.0, 1.0)
}

/// `BlockProperties.getFrictionInfluencedSpeed` for a player on foot.
pub fn friction_influenced_speed(block_friction: f32, p: &Movement) -> f32 {
    let movement_speed = p.speed as f32;
    if p.last_on_ground {
        let f = modified_friction(block_friction, p);
        if f <= 0.6 {
            return movement_speed;
        }
        return movement_speed * (0.21600002_f32 / (f * f * f));
    }
    if p.is_flying {
        return p.fly_speed * 20.0 * if p.is_sprinting { 0.1 } else { 0.05 };
    }
    if p.is_sprinting { 0.025999999 } else { 0.02 }
}

/// `BlockProperties.getBlockSpeedFactor` for 1.21+ clients.
pub fn block_speed_factor(p: &Movement, world: &dyn BlockView, support: SupportingBlock, pos: Vec3) -> f32 {
    if p.is_gliding || p.is_flying {
        return 1.0;
    }
    let inside = world.block(floor(pos.x), floor(pos.y), floor(pos.z));
    let efficiency = p.attr(Attr::MovementEfficiency) as f32;
    if inside.speed_factor != 1.0 || inside.kind == BlockKind::Water || inside.kind == BlockKind::BubbleColumn {
        return lerp_f32(efficiency, inside.speed_factor, 1.0);
    }
    let under = block_below_affecting_movement(p, world, support, pos);
    lerp_f32(efficiency, under.speed_factor, 1.0)
}

/// `BlockProperties.onHoneyBlock`.
pub fn on_honey_block(p: &Movement, world: &dyn BlockView, support: SupportingBlock, pos: Vec3) -> bool {
    world.block(floor(pos.x), floor(pos.y), floor(pos.z)).kind == BlockKind::HoneyBlock
        || block_below_affecting_movement(p, world, support, pos).kind == BlockKind::HoneyBlock
}

/// `BlockProperties.getEntityBounciness`.
pub fn entity_bounciness(p: &Movement) -> f32 {
    if p.is_sneaking {
        return 0.0;
    }
    p.attr(Attr::Bounciness) as f32
}

/// `BlockProperties.getVelocityAfterHorizontalCollision`.
pub fn velocity_after_horizontal_collision(p: &Movement, velocity: f64) -> f64 {
    -velocity * f64::from(entity_bounciness(p))
}

/// `BlockProperties.getEffectiveGravity`.
pub fn effective_gravity(p: &Movement, delta_y: f64) -> f64 {
    let gravity = p.attr(Attr::Gravity);
    if delta_y <= 0.0 && p.slow_falling() {
        return gravity.min(0.01);
    }
    gravity
}

/// `BlockProperties.getVelocityAfterVerticalCollision` (26.3).
pub fn velocity_after_vertical_collision(p: &Movement, velocity: f64, movement_y: f64, restitution: f64) -> f64 {
    let gravity = effective_gravity(p, velocity);
    let correct_gravity = -velocity <= gravity;
    if velocity < 0.0 && correct_gravity {
        return 0.0;
    }
    if restitution <= 0.0 {
        return 0.0;
    }
    let portion = if velocity == 0.0 { 0.0 } else { movement_y / velocity };
    let gravity_compensation = portion * gravity;
    let effective_drag = crate::math::lerp(portion, 1.0, f64::from(modified_air_drag(0.98, p)));
    (gravity_compensation - velocity) * effective_drag * restitution
}

/// `StuckSpeed.checkStuckSpeed`: the stuck-speed blocks the player's box
/// (grown by `expand`) touches, as a bit mask (1 cobweb, 4 berry bush,
/// 8 powder snow).
pub fn check_stuck_speed(p: &Movement, world: &dyn BlockView, expand: f64) -> u32 {
    let bb = p.pose_box_at(p.x, p.y, p.z).inflate(expand);
    let (x0, y0, z0, x1, y1, z1) = (floor(bb.min_x), floor(bb.min_y), floor(bb.min_z), floor(bb.max_x), floor(bb.max_y), floor(bb.max_z));
    if !world.are_chunks_loaded(x0, z0, x1, z1) {
        return 0;
    }
    let mut mask = 0;
    for x in x0..=x1 {
        for y in y0..=y1 {
            for z in z0..=z1 {
                match world.block(x, y, z).kind {
                    BlockKind::Cobweb => mask |= 1,
                    BlockKind::SweetBerryBush => mask |= 4,
                    BlockKind::PowderSnow if x == floor(p.x) && y == floor(p.y) && z == floor(p.z) => mask |= 8,
                    _ => {}
                }
            }
        }
    }
    mask
}

/// The multiplier for a stuck-speed bit.
pub fn stuck_speed_of(bit: u32) -> Vec3 {
    match bit {
        1 => Vec3::new(0.25, f64::from(0.05_f32), 0.25),
        2 => Vec3::new(0.5, 0.25, 0.5),
        4 => Vec3::new(f64::from(0.8_f32), 0.75, f64::from(0.8_f32)),
        _ => Vec3::new(f64::from(0.9_f32), 1.5, f64::from(0.9_f32)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::TestWorld;

    fn player_at(x: f64, y: f64, z: f64) -> Movement {
        let mut p = Movement::default();
        (p.x, p.y, p.z) = (x, y, z);
        (p.last_x, p.last_y, p.last_z) = (x, y, z);
        p.bounding_box = p.pose_box_at(x, y, z);
        p
    }

    #[test]
    fn falling_onto_floor_stops_at_surface() {
        let mut w = TestWorld::new();
        w.floor(-3, 3, 63, -3, 3, BlockFacts::SOLID);
        let mut p = player_at(0.5, 64.3, 0.5);
        p.actual_movement = Vec3::new(0.0, -0.3, 0.0);
        let r = collide_simple(&mut p, &w, 0.0, -0.5, 0.0);
        assert!((r.y - -0.3).abs() < 1e-9, "{r:?}");
    }

    #[test]
    fn walking_into_slab_steps_up() {
        let mut w = TestWorld::new();
        w.floor(-3, 6, 63, -3, 3, BlockFacts::SOLID);
        // A half slab at x = 1.
        w.set_shape(1, 64, 0, BlockFacts::SOLID, &[Aabb::new(0.0, 0.0, 0.0, 1.0, 0.5, 1.0)]);
        let mut p = player_at(0.5, 64.0, 0.5);
        p.last_on_ground = true;
        p.on_ground = true;
        p.actual_movement = Vec3::new(0.4, 0.5, 0.0);
        let r = collide(&mut p, &w, 0.4, -0.0784, 0.0, -0.0784, None, false);
        assert!(p.uncertainty.is_step_movement);
        assert!((r.y - 0.5).abs() < 1e-6, "{r:?}");
        assert!((r.x - 0.4).abs() < 1e-9, "{r:?}");
    }

    #[test]
    fn sneaking_backs_off_edge() {
        let mut w = TestWorld::new();
        w.set_full(0, 63, 0, BlockFacts::SOLID);
        let mut p = player_at(0.5, 64.0, 0.5);
        p.is_sneaking = true;
        p.last_on_ground = true;
        let v = maybe_back_off_from_edge(Vec3::new(2.0, 0.0, 0.0), &p, &w);
        assert!(v.x < 0.9, "{v:?}");
    }

    #[test]
    fn ice_speed_factor_and_friction() {
        let mut w = TestWorld::new();
        let ice = BlockFacts { friction: 0.98, kind: BlockKind::Ice, ..BlockFacts::SOLID };
        w.floor(-2, 2, 63, -2, 2, ice);
        let p = player_at(0.5, 64.0, 0.5);
        let support = SupportingBlock { pos: Some([0, 63, 0]), on_ground: true };
        assert_eq!(friction(&p, &w, support, p.position()), 0.98);
    }
}
