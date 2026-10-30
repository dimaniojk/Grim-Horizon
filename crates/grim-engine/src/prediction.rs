// Grim-Horizon: a Rust port of Grim Anticheat (https://github.com/GrimAnticheat/Grim).
// Copyright (C) GrimAC contributors (original Java), Grim-Horizon contributors (port).
// Licensed under the GNU General Public License v3.0 only; see LICENSE.

//! The movement prediction engine: Grim's `MovementCheckRunner.check`,
//! `PlayerBaseTick`, `MovementTicker`/`MovementTickerPlayer`,
//! `PredictionEngine`, `PredictionEngineNormal`, `PredictionEngineWater`,
//! `PredictionEngineLava`, the parts of `PointThreeEstimator` and
//! `UncertaintyHandler` they use, and `JumpPower`, on the 26.3 code path
//! (known inputs from `ServerboundPlayerInputPacket`, 2e-4 movement
//! threshold, 1.21 stepping, 26.2 friction/drag/bounciness attributes).
//!
//! Not modelled (the movement is still simulated, but these extras are not):
//! vehicles, elytra gliding, riptide, pistons, fireworks, fishing rods,
//! entity pushing, shulkers, honey-wall sliding and the potent sulfur geyser.
//! The runner refuses to judge movement affected by them (`checked = false`).

use crate::collisions;
use crate::fluid;
use crate::math::{Aabb, COLLISION_EPSILON, Vec3, cut_box_to_vector, floor, grim_equal, mc_cos, mc_sin, radians};
use crate::player::{Attr, Effect, KnownInput, MOVEMENT_THRESHOLD, Movement, Pose, VectorData, vt};
use crate::velocity::{ExplosionHandler, KnockbackHandler, VelocityConfig};
use crate::world::{BlockKind, BlockView};

/// What the engine needs besides the player and the world.
pub struct Context<'a> {
    pub world: &'a dyn BlockView,
    pub kb: &'a mut KnockbackHandler,
    pub explosion: &'a mut ExplosionHandler,
    pub kb_cfg: VelocityConfig,
    pub explosion_threshold: f64,
    /// A setback is being completed (`getRequiredSetBack().getTicksComplete() == 1`):
    /// stepping is not allowed.
    pub disallow_stepping: bool,
}

/// A movement packet the runner simulates (`PositionUpdate`).
#[derive(Clone, Copy, Debug)]
pub struct PositionUpdate {
    pub to: Vec3,
    pub on_ground: bool,
    pub is_teleport: bool,
    /// The velocity a teleport sets (`PlayerPosition.delta` with relative flags).
    pub teleport_velocity: Option<Vec3>,
}

/// `PredictionComplete`.
#[derive(Clone, Copy, Debug)]
pub struct PredictionComplete {
    pub offset: f64,
    pub checked: bool,
    pub teleport: bool,
    /// Why the prediction was not checked, for debugging.
    pub skipped: Option<&'static str>,
}

#[derive(Clone, Copy, PartialEq)]
enum EngineKind {
    Normal,
    Water { gravity: f64, falling: bool, friction: f32 },
    Lava,
}

/// `MovementCheckRunner.check`: simulates one movement packet.
pub fn check(p: &mut Movement, cx: &mut Context<'_>, update: PositionUpdate) -> PredictionComplete {
    if update.is_teleport {
        handle_teleport(p, cx, update);
        return PredictionComplete { offset: 0.0, checked: true, teleport: true, skipped: None };
    }
    p.uncertainty.tick();
    p.on_ground = update.on_ground;

    // Sneaking near an edge: remember which sides held the player back.
    // Grim probes one step of the last movement plus 0.05 each way; a short
    // 0.05 probe is added so that a hole narrower than that step (where the
    // long probe lands on the far side) still counts.
    if !p.is_flying && p.is_sneaking && collisions::is_above_ground(p, cx.world) {
        let m = p.actual_movement;
        let long = (
            (m.x.clamp(-16.0, 16.0) + 0.05).max(0.05),
            (m.z.clamp(-16.0, 16.0) + 0.05).max(0.05),
            (m.x.clamp(-16.0, 16.0) - 0.05).min(-0.05),
            (m.z.clamp(-16.0, 16.0) - 0.05).min(-0.05),
        );
        // A third probe reaches as far as this tick's movement can: the
        // carried velocity plus a full input step (0.15 covers sprinting).
        let v = p.client_velocity;
        let reach = (v.x.abs().min(16.0) + 0.15, v.z.abs().min(16.0) + 0.15);
        let fast = (reach.0, reach.1, -reach.0, -reach.1);
        let (mut east, mut west, mut north, mut south) = (false, false, false, false);
        for (pos_x, pos_z, neg_x, neg_z) in [long, (0.05, 0.05, -0.05, -0.05), fast] {
            let ne = collisions::maybe_back_off_from_edge(Vec3::new(pos_x, 0.0, neg_z), p, cx.world);
            let nw = collisions::maybe_back_off_from_edge(Vec3::new(neg_x, 0.0, neg_z), p, cx.world);
            let se = collisions::maybe_back_off_from_edge(Vec3::new(pos_x, 0.0, pos_z), p, cx.world);
            let sw = collisions::maybe_back_off_from_edge(Vec3::new(neg_x, 0.0, pos_z), p, cx.world);
            east |= ne.x != pos_x || se.x != pos_x;
            west |= nw.x != neg_x || sw.x != neg_x;
            north |= ne.z != neg_z || nw.z != neg_z;
            south |= se.z != pos_z || sw.z != pos_z;
        }
        let u = &mut p.uncertainty;
        if east {
            u.last_stuck_east.reset();
        }
        if west {
            u.last_stuck_west.reset();
        }
        if north {
            u.last_stuck_north.reset();
        }
        if south {
            u.last_stuck_south.reset();
        }
        if east || west || north || south {
            u.stuck_on_edge.reset();
        }
    }

    if p.in_bed {
        return PredictionComplete { offset: 0.0, checked: false, teleport: false, skipped: Some("sleeping") };
    }

    // The client keeps its ground flag through a teleport, so a sneaking
    // player can be held at an "edge" in mid-air for a tick that sends no
    // position while its velocity grows. Give the first prediction after a
    // teleport Grim's minimal sneaking lenience (SneakingEstimator's 0.15).
    if p.uncertainty.last_teleport_ticks.has_occurred_since(0) && p.is_sneaking && !p.did_last_movement_include_position {
        p.sneaking_hidden = Aabb::new(-0.15, 0.0, -0.15, 0.15, 0.0, 0.15);
    }

    p.speed = p.attr(Attr::MovementSpeed);
    p.gravity = p.attr(Attr::Gravity);

    if p.is_flying {
        p.fall_distance = 0.0;
        p.uncertainty.last_flying_ticks.reset();
    }
    p.is_climbing = collisions::on_climbable(cx.world, p.last_x, p.last_y, p.last_z);
    p.actual_movement = p.position().sub(p.last_position());
    p.is_jumping = p.known_input.jump;

    if p.is_sprinting {
        let s = p.speed;
        p.speed = f64::from((s + s * f64::from(0.3_f32)) as f32);
    }

    // Blocks below that the client may be bouncing or sliding on.
    let stepping = p
        .pose_box_at(p.last_x, p.last_y, p.last_z)
        .inflate(MOVEMENT_THRESHOLD)
        .offset(0.0, -0.2, 0.0);
    let (last_y, world) = (p.last_y, cx.world);
    let mut flags = (false, false, false, false, false, false);
    collisions::for_each_colliding_block(world, stepping, |x, y, z, b| match b.kind {
        BlockKind::SlimeBlock => {
            let above = block_collision_height(world, x, y + 1, z);
            let height = if above > 0.0 && above < 1.0 { 1.0 + above } else { 1.0 };
            if (f64::from(y) + height - last_y).abs() <= MOVEMENT_THRESHOLD {
                flags.0 = true;
                flags.1 = true;
            }
        }
        BlockKind::HoneyBlock => flags.2 = true,
        BlockKind::Bed => {
            if (f64::from(y) + 0.5625 - last_y).abs() <= MOVEMENT_THRESHOLD {
                flags.1 = true;
            }
        }
        BlockKind::Ice => flags.3 = true,
        BlockKind::BubbleColumn => flags.4 = true,
        BlockKind::Scaffolding => flags.5 = true,
        _ => {}
    });
    let u = &mut p.uncertainty;
    u.is_stepping_on_slime |= flags.0;
    u.is_stepping_on_bouncy |= flags.1;
    u.is_stepping_on_honey |= flags.2;
    u.is_stepping_on_ice |= flags.3;
    u.is_stepping_near_bubble_column |= flags.4;
    u.is_stepping_near_scaffolding |= flags.5;
    u.this_tick_slime_uncertainty = u.next_tick_slime_uncertainty;
    u.next_tick_slime_uncertainty = 0.0;

    if p.is_flying != p.was_flying {
        p.uncertainty.last_flying_status_change.reset();
    }

    // Stuck speed (cobwebs, berry bushes, powder snow).
    let max_stuck = collisions::check_stuck_speed(p, cx.world, MOVEMENT_THRESHOLD);
    let min_stuck = collisions::check_stuck_speed(p, cx.world, -MOVEMENT_THRESHOLD);
    let last_stuck_bit = p.last_stuck_speed.map(stuck_bit).unwrap_or(0);
    p.uncertainty.should_simulate_stuck_speed = max_stuck != min_stuck
        || max_stuck.count_ones() > 1
        || (min_stuck & last_stuck_bit == 0 && last_stuck_bit != 0);
    if max_stuck != 0 {
        p.uncertainty.last_stuck_speed_multiplier.reset();
    }
    let stuck_mask = max_stuck;

    p.start_tick_client_vel = p.client_velocity;
    let mut checked = false;
    let mut skipped = None;

    if p.is_dead {
        p.predicted = VectorData::new(Vec3::ZERO, vt::DEAD);
        p.client_velocity = Vec3::ZERO;
        skipped = Some("dead");
    } else if p.game_mode == crate::player::GameMode::Spectator || p.is_flying || p.is_gliding || p.in_vehicle {
        // Grim does not predict flight, exempt elytra gliding is the default
        // here, and vehicles are not modelled: follow the client.
        p.predicted = VectorData::new(p.actual_movement, vt::SPECTATOR);
        p.client_velocity = p.actual_movement;
        p.gravity = 0.0;
        p.friction = collisions::modified_air_drag(0.91, p);
        static_vector_end_of_tick_normal(p);
        skipped = Some(if p.in_vehicle {
            "vehicle"
        } else if p.is_gliding {
            "gliding"
        } else {
            "flying"
        });
    } else {
        checked = true;
        p.depth_strider_level = p.attr(Attr::WaterMovementEfficiency) as f32;
        p.sneaking_speed_multiplier = p.attr(Attr::SneakingSpeed) as f32;
        p.vertical_collision = false;
        do_base_tick(p, cx.world);
        living_entity_ai_step(p, cx, stuck_mask);
        update_player_pose(p, cx.world);
    }

    let mut offset = p.predicted.vector.distance(p.actual_movement);
    offset = reduce_offset(p, offset);
    end_of_tick_point_three(p, cx.world);

    if p.skipped_tick_in_actual_movement || !checked {
        p.uncertainty.last_point_three.reset();
    }

    if !checked {
        ExplosionHandler::force_exempt(p);
        KnockbackHandler::force_exempt(p);
    }

    PredictionComplete { offset, checked, teleport: false, skipped }
}

/// `SneakingEstimator.onPredictionComplete`: a player held back by a sneak
/// edge keeps velocity the server never sees; give that much lenience.
fn update_sneaking_estimator(p: &mut Movement) {
    let mut friction = f64::from(p.friction);
    if p.was_touching_lava {
        friction = 0.5;
    }
    if p.was_touching_water {
        friction = 0.96;
    }
    if p.is_gliding {
        friction = 0.99;
    }
    if !p.uncertainty.stuck_on_edge.has_occurred_since(0) {
        p.sneaking_hidden = Aabb::default();
        return;
    }
    let u = &p.uncertainty;
    let mut b = p.sneaking_hidden;
    for d in &p.last_possible {
        if d.is_jump() == p.predicted.is_jump() && d.is_knockback() == p.predicted.is_knockback() && d.is_explosion() == p.predicted.is_explosion() {
            if u.last_stuck_west.has_occurred_since(0) || u.last_stuck_north.has_occurred_since(0) {
                b.min_x = b.min_x.min(d.vector.x);
                b.min_z = b.min_z.min(d.vector.z);
            }
            if u.last_stuck_east.has_occurred_since(0) || u.last_stuck_south.has_occurred_since(0) {
                b.max_x = b.max_x.max(d.vector.x);
                b.max_z = b.max_z.max(d.vector.z);
            }
        }
    }
    b.min_x = (b.min_x * friction).min(-0.15);
    b.min_z = (b.min_z * friction).min(-0.15);
    b.max_x = (b.max_x * friction).max(0.15);
    b.max_z = (b.max_z * friction).max(0.15);
    if !u.last_stuck_east.has_occurred_since(0) {
        b.max_x = 0.0;
    }
    if !u.last_stuck_west.has_occurred_since(0) {
        b.min_x = 0.0;
    }
    if !u.last_stuck_north.has_occurred_since(0) {
        b.min_z = 0.0;
    }
    if !u.last_stuck_south.has_occurred_since(0) {
        b.max_z = 0.0;
    }
    p.sneaking_hidden = b;
}

/// Bookkeeping after the checks have seen a prediction (the tail of
/// `MovementCheckRunner.check`).
pub fn finish(p: &mut Movement, offset: f64, checked: bool) {
    if checked {
        update_sneaking_estimator(p);
    }
    p.last_on_ground = p.on_ground;
    p.last_sprinting = p.is_sprinting;
    p.was_flying = p.is_flying;
    p.was_gliding = p.is_gliding;
    p.was_swimming = p.is_swimming;
    p.was_sneaking = p.is_sneaking;
    p.uncertainty.last_movement_was_003 = p.skipped_tick_in_actual_movement;
    p.uncertainty.last_movement_was_unknown_003_reset = p.could_skip_tick && p.predicted.is_knockback();
    p.could_skip_tick = false;
    p.uncertainty.was_003_vertically = (p.uncertainty.last_movement_was_003 && controls_vertical_movement(p))
        || !p.point_three.can_predict_next_vertical_movement()
        || !p.point_three.was_always_certain;
    p.uncertainty.last_packet_was_ground_packet = p.uncertainty.on_ground_uncertain;
    p.uncertainty.on_ground_uncertain = false;
    p.min_attack_slow = 0;
    p.max_attack_slow = 0;
    p.likely_kb = None;
    p.first_bread_kb = None;
    p.first_bread_explosion = None;
    p.likely_explosion = None;
    let _ = offset;
    p.uncertainty.tick_instances();
}

fn stuck_bit(v: Vec3) -> u32 {
    if v.x == 0.25 {
        1
    } else if v.x == 0.5 {
        2
    } else if v.x == f64::from(0.8_f32) {
        4
    } else if v.x == f64::from(0.9_f32) {
        8
    } else {
        0
    }
}

fn block_collision_height(world: &dyn BlockView, x: i32, y: i32, z: i32) -> f64 {
    if world.block(x, y, z).air {
        return 0.0;
    }
    let mut boxes = Vec::new();
    world.block_collision_boxes(x, y, z, &mut boxes);
    boxes.iter().map(|b| b.max_y - f64::from(y)).fold(0.0, f64::max)
}

/// `MovementCheckRunner.handleTeleport`.
pub fn handle_teleport(p: &mut Movement, cx: &mut Context<'_>, update: PositionUpdate) {
    p.last_x = p.x;
    p.last_y = p.y;
    p.last_z = p.z;
    (p.x, p.y, p.z) = (update.to.x, update.to.y, update.to.z);
    p.last_x = p.x;
    p.last_y = p.y;
    p.last_z = p.z;
    if !p.in_vehicle {
        match update.teleport_velocity {
            None => {
                p.client_velocity = Vec3::ZERO;
                p.last_was_climbing = 0.0;
                p.can_swim_hop = false;
            }
            Some(v) => p.client_velocity = v,
        }
    }
    p.uncertainty.last_teleport_ticks.reset();
    ExplosionHandler::force_exempt(p);
    KnockbackHandler::force_exempt(p);
    p.bounding_box = p.pose_box_at(p.x, p.y, p.z);
    p.uncertainty.last_horizontal_offset = 0.0;
    p.uncertainty.last_vertical_offset = 0.0;
    p.fall_distance = 0.0;
    let _ = cx;
}

/// `UncertaintyHandler.reduceOffset`.
fn reduce_offset(p: &Movement, mut offset: f64) -> f64 {
    if p.uncertainty.last_hard_colliding_entity.has_occurred_since(3) {
        offset -= 1.2;
    }
    if p.uncertainty.influenced_by_bouncy_block() {
        offset -= 0.03;
    }
    offset.max(0.0)
}

// ---------------------------------------------------------------------------
// PlayerBaseTick

fn can_enter_pose(p: &Movement, world: &dyn BlockView, pose: Pose, x: f64, y: f64, z: f64) -> bool {
    let bb = p.box_at(x, y, z, pose.width(), pose.height()).inflate(-1.0e-7);
    collisions::is_empty(world, bb)
}

/// `PlayerBaseTick.doBaseTick` (26.3: fluid interaction, swimming,
/// sneaking in water, the crouch slowdown and suffocation pushing).
fn do_base_tick(p: &mut Movement, world: &dyn BlockView) {
    p.base_tick_addition = Vec3::ZERO;
    p.base_tick_water_pushing = Vec3::ZERO;
    if p.is_flying && p.is_sneaking {
        let shift = Vec3::new(0.0, f64::from(p.fly_speed) * -3.0, 0.0);
        p.client_velocity = p.client_velocity.add(shift);
        p.base_tick_addition = p.base_tick_addition.add(shift);
    }

    // updateFluidInteraction (26.3+): the eye test uses last tick's scan.
    p.was_eye_in_water = p.eye_in_water;
    update_fluid_interaction(p, world);
    update_swimming(p, world);

    if p.was_touching_lava {
        p.fall_distance *= 0.5;
    }
    if p.was_touching_water && p.is_sneaking && !p.is_flying {
        let push = Vec3::new(0.0, f64::from(-0.04_f32), 0.0);
        p.client_velocity = p.client_velocity.add(push);
        p.base_tick_addition = p.base_tick_addition.add(push);
    }
    p.last_pose = p.pose;

    p.is_slow_movement = (!p.was_flying
        && !p.is_swimming
        && can_enter_pose(p, world, Pose::Crouching, p.last_x, p.last_y, p.last_z)
        && (p.was_sneaking || !p.in_bed && !can_enter_pose(p, world, Pose::Standing, p.last_x, p.last_y, p.last_z)))
        || ((p.pose == Pose::Swimming || (!p.is_gliding && p.pose == Pose::FallFlying)) && !p.was_touching_water);

    let w = p.bounding_box.width_x();
    let d = p.bounding_box.width_z();
    for (sx, sz) in [(-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0), (1.0, 1.0)] {
        move_towards_closest_space(p, world, p.last_x + sx * w * 0.35, p.last_z + sz * d * 0.35);
    }
}

/// A client tick that sent no movement packet (it moved less than 2e-4):
/// the state the next tick reads still advances. Grim updates only the pose
/// and `wasSneaking` here; the swimming state and the sprint history also
/// change on such ticks (a swimmer can start in place), so they follow too.
pub fn idle_tick(p: &mut Movement, world: &dyn BlockView) {
    if p.in_vehicle || p.is_dead {
        return;
    }
    p.was_eye_in_water = p.eye_in_water;
    let s = fluid::scan(p, world, true);
    p.was_was_touching_water = p.was_touching_water;
    p.was_touching_water = s.water_height > 0.0;
    p.was_touching_lava = s.lava_height > 0.0;
    p.water_height = s.water_height;
    p.lava_height = s.lava_height;
    p.eye_in_water = s.eye_in_water;
    p.eye_in_lava = s.eye_in_lava;
    update_swimming(p, world);
    update_player_pose(p, world);
    p.last_sprinting = p.is_sprinting;
    p.was_sneaking = p.is_sneaking;
}

/// `PlayerBaseTick.updateFluidInteraction`.
pub fn update_fluid_interaction(p: &mut Movement, world: &dyn BlockView) {
    let ignore_current = !p.is_pushed_by_fluid();
    let s = fluid::scan(p, world, ignore_current);
    p.was_was_touching_water = p.was_touching_water;
    p.was_touching_water = s.water_height > 0.0;
    p.was_touching_lava = s.lava_height > 0.0;
    p.water_height = s.water_height;
    p.lava_height = s.lava_height;
    p.eye_in_water = s.eye_in_water;
    p.eye_in_lava = s.eye_in_lava;
    if p.is_pushed_by_fluid() {
        if p.was_touching_water
            && let Some((raw, added)) = s.current(true, 0.014, p.client_velocity)
        {
            p.base_tick_water_pushing = p.base_tick_water_pushing.add(raw);
            p.client_velocity = p.client_velocity.add(added);
        }
        if p.was_touching_lava
            && let Some((raw, added)) = s.current(false, 0.0023333333333333335, p.client_velocity)
        {
            p.base_tick_water_pushing = p.base_tick_water_pushing.add(raw);
            p.client_velocity = p.client_velocity.add(added);
        }
    }
}

fn update_swimming(p: &mut Movement, world: &dyn BlockView) {
    if p.is_flying || p.in_vehicle {
        p.is_swimming = false;
    } else if p.is_swimming {
        p.is_swimming = p.last_sprinting && p.was_touching_water;
    } else {
        let feet_in_water = world.water_height(floor(p.last_x), floor(p.last_y), floor(p.last_z)) > 0.0;
        p.is_swimming = p.last_sprinting && p.was_eye_in_water && p.was_touching_water && feet_in_water;
    }
}

fn move_towards_closest_space(p: &mut Movement, world: &dyn BlockView, x: f64, z: f64) {
    let t = MOVEMENT_THRESHOLD;
    p.bounding_box = p.bounding_box.expand(t, 0.0, t);
    let block_x = x.floor() as i32;
    let block_z = z.floor() as i32;
    let suffocates = |p: &Movement, bx: i32, bz: i32| {
        let bb = Aabb::new(f64::from(bx), p.bounding_box.min_y, f64::from(bz), f64::from(bx) + 1.0, p.bounding_box.max_y, f64::from(bz) + 1.0)
            .inflate(-1.0e-7);
        collisions::suffocates_at(world, bb)
    };
    if suffocates(p, block_x, block_z) {
        let rel_x = x - f64::from(block_x);
        let rel_z = z - f64::from(block_z);
        let mut best: Option<u8> = None;
        let mut lowest = f64::MAX;
        // WEST, EAST, NORTH, SOUTH.
        for dir in 0..4u8 {
            let d7 = if dir < 2 { rel_x } else { rel_z };
            let d6 = if dir == 1 || dir == 3 { 1.0 - d7 } else { d7 };
            let blocked = match dir {
                0 => suffocates(p, block_x - 1, block_z),
                1 => suffocates(p, block_x + 1, block_z),
                2 => suffocates(p, block_x, block_z - 1),
                _ => suffocates(p, block_x, block_z + 1),
            };
            if d6 >= lowest || blocked {
                continue;
            }
            lowest = d6;
            best = Some(dir);
        }
        match best {
            Some(0 | 1) => {
                p.uncertainty.x_pos += 0.15;
                p.uncertainty.x_neg -= 0.15;
                p.point_three.is_pushing = true;
            }
            Some(_) => {
                p.uncertainty.z_pos += 0.15;
                p.uncertainty.z_neg -= 0.15;
                p.point_three.is_pushing = true;
            }
            None => {}
        }
    }
    p.bounding_box = p.bounding_box.expand(-t, 0.0, -t);
}

/// `PlayerBaseTick.updatePlayerPose`.
pub fn update_player_pose(p: &mut Movement, world: &dyn BlockView) {
    if can_enter_pose(p, world, Pose::Swimming, p.x, p.y, p.z) {
        let mut pose = if p.is_gliding {
            Pose::FallFlying
        } else if p.in_bed {
            Pose::Sleeping
        } else if p.is_swimming {
            Pose::Swimming
        } else if p.is_sneaking && !p.is_flying {
            Pose::Crouching
        } else {
            Pose::Standing
        };
        if !p.in_vehicle && !can_enter_pose(p, world, pose, p.x, p.y, p.z) {
            pose = if can_enter_pose(p, world, Pose::Crouching, p.x, p.y, p.z) { Pose::Crouching } else { Pose::Swimming };
        }
        p.pose = pose;
        p.bounding_box = p.pose_box_at(p.x, p.y, p.z);
    }
}

// ---------------------------------------------------------------------------
// MovementTicker

fn living_entity_ai_step(p: &mut Movement, cx: &mut Context<'_>, stuck_mask: u32) {
    p.uncertainty.push_colliding_entities(0);
    living_entity_travel(p, cx, stuck_mask);
    let u = &mut p.uncertainty;
    (u.x_neg, u.x_pos, u.y_neg, u.y_pos, u.z_neg, u.z_pos) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    if p.uncertainty.last_teleport_ticks.has_occurred_since(0) {
        p.uncertainty.y_neg -= 0.02;
    }
}

fn living_entity_travel(p: &mut Movement, cx: &mut Context<'_>, stuck_mask: u32) {
    let mut gravity = p.attr(Attr::Gravity);
    let is_falling = p.actual_movement.y <= 0.0;
    if is_falling && p.slow_falling() {
        gravity = gravity.min(0.01);
        p.fall_distance = 0.0;
    }
    p.gravity = gravity;

    if p.was_touching_water && !p.is_flying {
        let mut swim_friction: f32 = if p.is_sprinting { 0.9 } else { 0.8 };
        let mut swim_speed: f32 = 0.02;
        if !p.last_on_ground {
            p.depth_strider_level *= 0.5;
        }
        if p.depth_strider_level > 0.0 {
            swim_friction += (0.54600006 - swim_friction) * p.depth_strider_level;
            swim_speed += (p.speed as f32 - swim_speed) * p.depth_strider_level;
        }
        if p.effects.level(Effect::DolphinsGrace).is_some() {
            swim_friction = 0.96;
        }
        p.friction = swim_friction;
        guess_best_movement(p, cx, swim_speed, EngineKind::Water { gravity, falling: is_falling, friction: swim_friction }, stuck_mask);
        p.is_climbing = collisions::on_climbable(cx.world, p.x, p.y, p.z);
        if p.is_climbing {
            let v = fluid_falling_adjusted(p, gravity, is_falling, p.client_velocity.with_y(0.2 * f64::from(0.8_f32)));
            p.last_was_climbing = v.y;
        }
    } else if p.was_touching_lava && !p.is_flying {
        p.friction = 0.5;
        guess_best_movement(p, cx, 0.02, EngineKind::Lava, stuck_mask);
        if p.lava_height <= 0.4 {
            p.client_velocity = p.client_velocity.mul(0.5, 0.800000011920929, 0.5);
            p.client_velocity = fluid_falling_adjusted(p, gravity, is_falling, p.client_velocity);
        } else {
            p.client_velocity = p.client_velocity.scale(0.5);
        }
        if p.has_gravity {
            p.client_velocity = p.client_velocity.add(Vec3::new(0.0, -gravity / 4.0, 0.0));
        }
    } else {
        let block_friction = collisions::friction(p, cx.world, p.supporting, p.last_position());
        let air_drag = collisions::modified_air_drag(0.91, p);
        p.friction = if p.last_on_ground { collisions::modified_friction(block_friction, p) * air_drag } else { air_drag };
        let speed = collisions::friction_influenced_speed(block_friction, p);
        guess_best_movement(p, cx, speed, EngineKind::Normal, stuck_mask);
    }
    apply_effects_from_blocks(p, cx.world);
}

/// `FluidFallingAdjustedMovement.getFluidFallingAdjustedMovement`.
fn fluid_falling_adjusted(p: &Movement, gravity: f64, is_falling: bool, v: Vec3) -> Vec3 {
    if !p.has_gravity || p.is_sprinting {
        return v;
    }
    let new_y = if is_falling && (v.y - 0.005).abs() >= 0.003 && (v.y - gravity / 16.0).abs() < 0.003 { -0.003 } else { v.y - gravity / 16.0 };
    v.with_y(new_y)
}

/// `Collisions.applyEffectsFromBlocks` for 1.21.2+: cobwebs, berry bushes,
/// powder snow and bubble columns along the move (blocks touched by the box
/// at the start or end of the movement).
fn apply_effects_from_blocks(p: &mut Movement, world: &dyn BlockView) {
    if p.stuck_speed.is_some_and(|s| s.x < 0.99) {
        p.uncertainty.last_stuck_speed_multiplier.reset();
    }
    p.last_stuck_speed = p.stuck_speed;
    p.stuck_speed = None;
    let start = p.pose_box_at(p.last_x, p.last_y, p.last_z).inflate(-f64::from(1.0e-5_f32));
    let end = p.pose_box_at(p.x, p.y, p.z).inflate(-f64::from(1.0e-5_f32));
    let area = start.union(end);
    let (px, py, pz) = (floor(p.x), floor(p.y), floor(p.z));
    let mut velocity = p.client_velocity;
    let mut stuck = None;
    let mut reset_fall = false;
    collisions::for_each_block(world, area, |x, y, z, b| match b.kind {
        BlockKind::Cobweb => stuck = Some(collisions::stuck_speed_of(1)),
        BlockKind::SweetBerryBush => stuck = Some(collisions::stuck_speed_of(4)),
        BlockKind::PowderSnow if x == px && y == py && z == pz => stuck = Some(collisions::stuck_speed_of(8)),
        BlockKind::BubbleColumn => {
            let cell = Aabb::new(f64::from(x), f64::from(y), f64::from(z), f64::from(x + 1), f64::from(y + 1), f64::from(z + 1));
            if end.intersects(&cell) {
                let above_air = world.block(x, y + 1, z).air;
                velocity.y = match (above_air, b.drag) {
                    (true, true) => (velocity.y - 0.03).max(-0.9),
                    (true, false) => (velocity.y + 0.1).min(1.8),
                    (false, true) => (velocity.y - 0.03).max(-0.3),
                    (false, false) => (velocity.y + 0.06).min(0.7),
                };
                reset_fall = true;
            }
        }
        _ => {}
    });
    p.client_velocity = velocity;
    p.stuck_speed = stuck;
    if reset_fall || p.stuck_speed.is_some_and(|s| s.x < 0.9) {
        p.fall_distance = 0.0;
    }
    if p.is_flying {
        p.stuck_speed = None;
    }
}

/// `MovementTicker.move`: applies the chosen movement's collisions.
fn tick_move(p: &mut Movement, cx: &mut Context<'_>, input_vel: Vec3, collide: Vec3) {
    if p.stuck_speed.is_some_and(|s| s.x < 0.99) {
        p.client_velocity = Vec3::ZERO;
    }
    let x_axis = !grim_equal(input_vel.x, collide.x);
    let z_axis = !grim_equal(input_vel.z, collide.z);
    p.horizontal_collision = x_axis || z_axis;
    p.vertical_collision = input_vel.y != collide.y;

    let mut calculated_on_ground = p.vertical_collision && input_vel.y < 0.0;
    if input_vel.y == -COLLISION_EPSILON && collide.y > -COLLISION_EPSILON && collide.y <= 0.0 {
        calculated_on_ground = p.on_ground;
    }
    p.client_claims_last_on_ground = p.on_ground;
    if !p.exempt_on_ground() {
        p.on_ground = calculated_on_ground;
    }

    p.bounding_box = p.pose_box_at(p.x, p.y, p.z);
    if !p.was_touching_water {
        update_fluid_interaction(p, cx.world);
    }
    if p.on_ground {
        p.fall_distance = 0.0;
    } else if collide.y < 0.0 {
        p.fall_distance -= collide.y;
    }
    if x_axis {
        p.client_velocity.x = collisions::velocity_after_horizontal_collision(p, p.client_velocity.x);
    }
    if z_axis {
        p.client_velocity.z = collisions::velocity_after_horizontal_collision(p, p.client_velocity.z);
    }

    p.supporting = collisions::find_main_supporting_block(p, cx.world, p.supporting, collide, p.bounding_box, p.on_ground);
    let on_block = collisions::on_block(p, cx.world, p.supporting, p.position());

    if input_vel.y != collide.y {
        let restitution = on_block.bounce;
        let suppresses = on_block.kind == BlockKind::SuppressesBounce;
        if restitution > 0.0 || (!suppresses && collisions::entity_bounciness(p) > 0.0) {
            apply_block_bounce(p, restitution, collide);
        } else {
            p.client_velocity.y = 0.0;
        }
    }

    let mut collide = collide;
    if collide.length_squared() <= 1e-7 && input_vel.length_squared() - collide.length_squared() >= 1e-7 {
        collide = Vec3::ZERO;
    }
    p.predicted = VectorData { vector: collide, ..p.predicted };
    let f = f64::from(collisions::block_speed_factor(p, cx.world, p.supporting, p.position()));
    p.client_velocity = p.client_velocity.mul(f, 1.0, f);
}

fn apply_block_bounce(p: &mut Movement, restitution: f32, movement: Vec3) {
    if p.is_sneaking {
        p.client_velocity.y = 0.0;
        return;
    }
    let current_y = p.client_velocity.y;
    if current_y >= 0.0 {
        return;
    }
    let restitution = f64::from(restitution).max(f64::from(collisions::entity_bounciness(p)));
    p.client_velocity.y = collisions::velocity_after_vertical_collision(p, current_y, movement.y, restitution);
}

// ---------------------------------------------------------------------------
// PredictionEngine

fn guess_best_movement(p: &mut Movement, cx: &mut Context<'_>, speed: f32, kind: EngineKind, stuck_mask: u32) {
    let init = fetch_possible_start_tick_vectors(p, cx, kind);

    if p.uncertainty.influenced_by_bouncy_block() {
        for data in &init {
            let to_zero = handle_starting_velocity_uncertainty(p, cx, data, Vec3::new(0.0, -1_000_000_000.0, 0.0));
            p.uncertainty.next_tick_slime_uncertainty = to_zero.y.abs().max(p.uncertainty.next_tick_slime_uncertainty);
        }
    }

    update_velocity_movement_skipping(p, cx);
    p.could_skip_tick = p.could_skip_tick || determine_can_skip_tick(p, cx, speed, &init);

    let mut possible = apply_inputs(p, cx, &init, speed, kind, stuck_mask);
    if p.could_skip_tick {
        add_zero_point_three(p, cx, speed, kind, stuck_mask, &mut possible);
    }

    let before_collision = do_predictions(p, cx, &mut possible, speed);
    let collide = p.predicted.vector;
    tick_move(p, cx, before_collision, collide);
    end_of_tick(p, cx.world, kind);
}

/// `getPossibleVelocities`: knockback first, then the carried velocity,
/// a swim hop and climbing.
fn possible_velocities(p: &Movement, cx: &Context<'_>, with_knockback: bool) -> Vec<VectorData> {
    let mut set = Vec::with_capacity(6);
    if with_knockback {
        if let Some(kb) = p.first_bread_kb {
            set.push(VectorData::new(kb.vector, vt::KNOCKBACK | vt::FIRST_BREAD_KB));
        }
        if let Some(kb) = p.likely_kb {
            set.push(VectorData::new(kb.vector, vt::KNOCKBACK));
        }
    }
    set.push(VectorData::new(p.client_velocity, vt::NORMAL));
    if p.can_swim_hop && !p.on_ground {
        set.push(VectorData::new(p.client_velocity.with_y(0.30000001192092896), vt::SWIM_HOP));
    }
    if p.last_was_climbing != 0.0 {
        set.push(VectorData::new(p.client_velocity.with_y(p.last_was_climbing + p.base_tick_addition.y), vt::CLIMBABLE));
    }
    let _ = cx;
    set
}

fn fetch_possible_start_tick_vectors(p: &mut Movement, cx: &mut Context<'_>, kind: EngineKind) -> Vec<VectorData> {
    if let EngineKind::Water { gravity, falling, .. } = kind
        && p.last_was_climbing == 0.0
        && p.point_three.is_near_climbable
    {
        p.last_was_climbing = fluid_falling_adjusted(p, gravity, falling, p.client_velocity.with_y(0.2 * f64::from(0.8_f32))).y;
    }

    let mut v = possible_velocities(p, cx, true);
    // The vector that aliases the player's own velocity in Grim.
    let normal_index = v.iter().position(|d| d.types == vt::NORMAL).unwrap_or(0);

    add_explosion(p, &mut v);
    // Fluid pushing on knockback.
    for d in v.iter_mut() {
        if d.is_knockback() && p.base_tick_addition.length_squared() != 0.0 {
            d.vector = d.vector.add(p.base_tick_addition);
        }
        if d.is_knockback() && p.base_tick_water_pushing.length_squared() != 0.0 {
            let mut push = p.base_tick_water_pushing;
            if d.vector.x.abs() < 0.003 && d.vector.z.abs() < 0.003 && push.length() < 0.0045000000000000005 {
                push = push.normalize().scale(0.0045000000000000005);
            }
            d.vector = d.vector.add(push);
        }
    }
    // Attack slowdown (Player.attack multiplies by 0.6 per sprint hit).
    for _ in 1..=p.max_attack_slow.min(5) {
        let snapshot = v.clone();
        if p.min_attack_slow > 0 {
            for d in v.iter_mut() {
                d.vector = d.vector.mul(0.6, 1.0, 0.6);
                d.types |= vt::ATTACK_SLOW;
            }
        } else {
            for d in snapshot {
                v.push(d.modified(d.vector.mul(0.6, 1.0, 0.6), vt::ATTACK_SLOW));
            }
        }
        if p.min_attack_slow > 0 {
            p.min_attack_slow -= 1;
        }
    }
    apply_movement_threshold(p, &mut v);
    if let Some(n) = v.get(normal_index) {
        p.client_velocity = n.vector;
    }
    add_jumps(p, cx, &mut v, kind);

    if matches!(kind, EngineKind::Water { .. }) {
        v = transform_swimming_vectors(p, v);
    }
    v
}

fn add_explosion(p: &Movement, v: &mut Vec<VectorData>) {
    if p.likely_explosion.is_none() && p.first_bread_explosion.is_none() {
        return;
    }
    let base = v.clone();
    for d in base {
        if let Some(e) = p.likely_explosion {
            v.push(d.modified(d.vector.add(e.vector), vt::EXPLOSION));
        }
        if let Some(e) = p.first_bread_explosion {
            v.push(d.modified(d.vector.add(e.vector), vt::EXPLOSION | vt::FIRST_BREAD_EXPLOSION));
        }
    }
}

/// `applyMovementThreshold` for 1.21.5+ clients ("stupid vectors").
fn apply_movement_threshold(p: &Movement, v: &mut Vec<VectorData>) {
    let minimum = 0.003;
    let stuck_on_edge = p.uncertainty.stuck_on_edge.has_occurred_since(2);
    let mut extra = Vec::new();
    for d in v.iter_mut() {
        // Near the 9e-6 cut the carried velocity's last bits decide; keep
        // the side the client may be on as well.
        let h = collisions::horizontal_distance_sqr(d.vector);
        if (9.0e-6 * 0.8..9.0e-6 * 1.25).contains(&h) {
            let mut other = *d;
            if h < 9.0e-6 {
                if other.vector.y.abs() < minimum {
                    other.vector.y = 0.0;
                }
            } else {
                other.vector.x = 0.0;
                other.vector.z = 0.0;
                if other.vector.y.abs() < minimum {
                    other.vector.y = 0.0;
                }
            }
            extra.push(other);
        }
        if collisions::horizontal_distance_sqr(d.vector) < 9.0e-6 {
            if stuck_on_edge {
                let mut edge = *d;
                if edge.vector.y.abs() < minimum {
                    edge.vector.y = 0.0;
                }
                extra.push(edge);
            }
            d.vector.x = 0.0;
            d.vector.z = 0.0;
        }
        if d.vector.y.abs() < minimum {
            d.vector.y = 0.0;
        }
    }
    v.extend(extra);
}

/// `JumpPower.jumpFromGround` (1.21.2+).
fn jump_from_ground(p: &Movement, world: &dyn BlockView, v: &mut Vec3) {
    let mut jump_power = jump_power(p, world);
    if let Some(level) = p.effects.level(Effect::JumpBoost) {
        jump_power += 0.1 * (level + 1) as f32;
    }
    if jump_power <= 1.0e-5 {
        return;
    }
    v.y = f64::from(jump_power).max(v.y);
    if p.is_sprinting {
        let r = radians(p.yaw);
        v.x += -f64::from(mc_sin(f64::from(r))) * 0.2;
        v.z += f64::from(mc_cos(f64::from(r))) * 0.2;
    }
}

/// `JumpPower.getJumpPower`: the jump strength times the honey factor.
fn jump_power(p: &Movement, world: &dyn BlockView) -> f32 {
    let strength = p.attr(Attr::JumpStrength) as f32;
    let factor = if collisions::on_honey_block(p, world, p.supporting, p.last_position()) { 0.5 } else { 1.0 };
    strength * factor
}

fn add_jumps(p: &Movement, cx: &Context<'_>, v: &mut Vec<VectorData>, kind: EngineKind) {
    // Water and normal movement read the known jump key; lava does not.
    if !p.known_input.jump && !matches!(kind, EngineKind::Lava) {
        return;
    }
    match kind {
        EngineKind::Normal => {
            let base = v.clone();
            for d in base {
                let mut jump = d.vector;
                if !p.is_flying {
                    let boost = p.effects.level(Effect::JumpBoost);
                    if ((boost.is_none() || boost.is_some_and(|b| b >= 0)) && p.on_ground) || !p.last_on_ground {
                        return;
                    }
                    jump_from_ground(p, cx.world, &mut jump);
                } else {
                    jump.y += f64::from(p.fly_speed) * 3.0;
                }
                v.push(d.modified(jump, vt::JUMP));
            }
        }
        EngineKind::Water { .. } | EngineKind::Lava => {
            let fluid_height = if matches!(kind, EngineKind::Lava) { p.lava_height } else { p.water_height };
            let base = v.clone();
            for d in base {
                if p.could_skip_tick && d.is_zero_point_zero_three() {
                    let y = p.actual_movement.y.clamp(d.vector.y, d.vector.y + f64::from(0.05_f32));
                    v.push(d.modified(d.vector.with_y(y), vt::JUMP));
                } else {
                    v.push(d.modified(d.vector.add(Vec3::new(0.0, f64::from(0.04_f32), 0.0)), vt::JUMP));
                }
                if fluid_height <= 0.4 && p.last_on_ground && !p.on_ground {
                    let mut with_jump = d.vector;
                    if p.last_on_ground && !p.on_ground {
                        jump_from_ground(p, cx.world, &mut with_jump);
                    }
                    v.push(d.modified(with_jump, vt::JUMP));
                }
            }
        }
    }
}

/// `PredictionEngineWater.transformSwimmingVectors`.
fn transform_swimming_vectors(p: &Movement, base: Vec<VectorData>) -> Vec<VectorData> {
    if (p.was_eye_in_water || p.eye_in_water || p.is_swimming || p.was_swimming) && !p.in_vehicle {
        let look_y = crate::math::look_vector(p.yaw, p.pitch).y;
        let scalar = if look_y < -0.2 { 0.085 } else { 0.06 };
        let mut out = Vec::with_capacity(base.len() * 2);
        for d in base {
            out.push(d.modified(Vec3::new(d.vector.x, d.vector.y + (look_y - d.vector.y) * scalar, d.vector.z), vt::SWIMMING_SPACE));
            out.push(d.modified(d.vector, vt::SURFACE_SWIMMING));
        }
        return out;
    }
    base
}

/// `ModernInputTransformer.transformInputsToVector` (1.21.5+).
fn transform_input(p: &Movement, sideways: i32, forward: i32) -> Vec3 {
    let (sx, fz) = (sideways as f32, forward as f32);
    let len = (sx * sx + fz * fz).sqrt();
    let (mut x, mut z) = if len < 1.0e-4 { (0.0, 0.0) } else { (sx / len, fz / len) };
    if x * x + z * z == 0.0 {
        return Vec3::ZERO;
    }
    x *= 0.98;
    z *= 0.98;
    if p.slowed_by_using_item && !p.in_vehicle {
        x *= 0.2;
        z *= 0.2;
    }
    if p.is_slow_movement {
        x *= p.sneaking_speed_multiplier;
        z *= p.sneaking_speed_multiplier;
    }
    // modifyInputSpeedForSquareMovement.
    let length = (x * x + z * z).sqrt();
    if length > 0.0 {
        let (mx, mz) = (x * (1.0 / length), z * (1.0 / length));
        let (ax, az) = (mx.abs(), mz.abs());
        let additional = if az > ax { ax / az } else { az / ax };
        let distance = (1.0 + additional * additional).sqrt();
        let min = (length * distance).min(1.0);
        x = mx * min;
        z = mz * min;
    }
    Vec3::new(f64::from(x), 0.0, f64::from(z))
}

/// `DoubleInputTransformer.getMovementResultFromInput`.
fn movement_from_input(input: Vec3, speed: f32, yaw: f32) -> Vec3 {
    let r = radians(yaw);
    let sin = f64::from(mc_sin(f64::from(r)));
    let cos = f64::from(mc_cos(f64::from(r)));
    let x = input.x * cos - input.z * sin;
    let z = input.z * cos + input.x * sin;
    let s = f64::from(speed);
    Vec3::new(x * s, input.y * s, z * s)
}

fn input_ranges(input: KnownInput, sprinting: bool, swimming: bool) -> (i32, i32) {
    let mut forward = 0;
    if input.forward || (sprinting && !swimming) {
        forward += 1;
    }
    if input.backward && (!sprinting || swimming) {
        forward -= 1;
    }
    let mut strafe = 0;
    if input.left {
        strafe += 1;
    }
    if input.right {
        strafe -= 1;
    }
    (forward, strafe)
}

/// `loopVectors` with the client's known inputs: one movement input, and
/// both states of the crouch slowdown and of item use.
fn apply_inputs(p: &mut Movement, cx: &Context<'_>, init: &[VectorData], speed: f32, kind: EngineKind, stuck_mask: u32) -> Vec<VectorData> {
    let (forward, strafe) = input_ranges(p.known_input, p.is_sprinting, p.is_swimming);
    let mut out = Vec::with_capacity(init.len() * 4);
    for loop_slowed in 0..=1 {
        for loop_using_item in 0..=1 {
            for d in init {
                if loop_slowed == 1 && !d.is_zero_point_zero_three() {
                    continue;
                }
                let input = transform_input(p, strafe, forward);
                let mut result = d.modified(d.vector.add(movement_from_input(input, speed, p.yaw)), vt::NORMAL);
                result.input = Some(input);
                if p.uncertainty.should_simulate_stuck_speed {
                    if stuck_mask == 0 {
                        out.push(stuck_result(p, cx, result, None, loop_using_item == 1, kind));
                    }
                    out.push(stuck_result(p, cx, result, p.stuck_speed, loop_using_item == 1, kind));
                    for bit in [1u32, 2, 4, 8] {
                        let m = collisions::stuck_speed_of(bit);
                        if stuck_mask & bit != 0 && p.stuck_speed != Some(m) {
                            out.push(stuck_result(p, cx, result, Some(m), loop_using_item == 1, kind));
                        }
                    }
                } else {
                    out.push(stuck_result(p, cx, result, p.stuck_speed, loop_using_item == 1, kind));
                }
            }
            p.slowed_by_using_item = !p.slowed_by_using_item;
        }
        p.is_slow_movement = !p.is_slow_movement;
    }
    out
}

fn stuck_result(p: &Movement, cx: &Context<'_>, mut result: VectorData, stuck: Option<Vec3>, flip_item: bool, kind: EngineKind) -> VectorData {
    if let Some(m) = stuck {
        result = result.modified(result.vector.mul(m.x, m.y, m.z), vt::STUCK);
    }
    result.stuck = stuck;
    let climbed = if kind == EngineKind::Normal { handle_on_climbable(p, cx, result.vector) } else { result.vector };
    result = result.modified(climbed, vt::CLIMBABLE);
    if flip_item {
        result.types |= vt::FLIP_ITEM;
    }
    result
}

/// `PredictionEngineNormal.handleOnClimbable`.
fn handle_on_climbable(p: &Movement, cx: &Context<'_>, mut v: Vec3) -> Vec3 {
    if p.is_climbing {
        v.x = v.x.clamp(-f64::from(0.15_f32), f64::from(0.15_f32));
        v.z = v.z.clamp(-f64::from(0.15_f32), f64::from(0.15_f32));
        v.y = v.y.max(-f64::from(0.15_f32));
        if v.y < 0.0 && cx.world.block_at(p.last_x, p.last_y, p.last_z).kind != BlockKind::Scaffolding && p.is_sneaking && !p.is_flying {
            v.y = 0.0;
        }
    }
    v
}

fn controls_vertical_movement(p: &Movement) -> bool {
    let t = &p.point_three;
    t.is_near_fluid
        || t.is_near_climbable
        || t.near_horizontal_flow
        || t.near_vertical_flow
        || t.near_bubble_column
        || t.is_gliding
        || p.uncertainty.influenced_by_bouncy_block()
}

/// `PointThreeEstimator.determineCanSkipTick` for 1.21.2+ clients: only
/// after a tick that sent no position.
fn determine_can_skip_tick(p: &mut Movement, cx: &mut Context<'_>, speed: f32, init: &[VectorData]) -> bool {
    if p.did_last_movement_include_position && !p.uncertainty.is_stepping_on_slime {
        return false;
    }
    if (p.is_gliding || p.was_gliding) && !p.did_last_movement_include_position {
        return true;
    }
    if p.point_three.is_near_climbable || p.point_three.is_pushing {
        return true;
    }
    let mut minimum = f64::MAX;
    for d in init {
        let to_zero = handle_starting_velocity_uncertainty(p, cx, d, Vec3::ZERO);
        let r = collisions::collide(p, cx.world, to_zero.x, to_zero.y, to_zero.z, f64::from(i32::MIN), None, cx.disallow_stepping);
        let min_horiz = (r.x.hypot(r.z) - f64::from(speed)).max(0.0);
        let forced_no_003 = d.is_explosion() || d.is_knockback();
        let vertical = if (!forced_no_003 && p.last_on_ground) || controls_vertical_movement(p) { 0.0 } else { r.y.abs() };
        let length = vertical.hypot(min_horiz);
        minimum = minimum.min(length);
        if minimum < MOVEMENT_THRESHOLD {
            break;
        }
    }
    minimum < MOVEMENT_THRESHOLD
}

/// `GrimPlayer.updateVelocityMovementSkipping`.
fn update_velocity_movement_skipping(p: &mut Movement, cx: &mut Context<'_>) {
    let block_friction = collisions::friction(p, cx.world, p.supporting, p.last_position());
    let saved = p.speed;
    p.speed = saved * if p.is_sprinting { 1.3 } else { 1.0 };
    let speed = collisions::friction_influenced_speed(block_friction, p);
    p.speed = saved;
    if !p.could_skip_tick {
        let minus_kb = possible_velocities(p, cx, false);
        p.could_skip_tick = determine_can_skip_tick(p, cx, speed, &minus_kb);
    }
    let kb: Vec<VectorData> = [p.first_bread_kb, p.likely_kb].iter().flatten().map(|k| VectorData::new(k.vector, vt::KNOCKBACK)).collect();
    let kb_003 = !kb.is_empty() && determine_can_skip_tick(p, cx, speed, &kb);
    cx.kb.point_three |= kb_003;
    let ex: Vec<VectorData> = [p.first_bread_explosion, p.likely_explosion].iter().flatten().map(|k| VectorData::new(k.vector, vt::EXPLOSION)).collect();
    let ex_003 = !ex.is_empty() && determine_can_skip_tick(p, cx, speed, &ex);
    cx.explosion.point_three |= ex_003;
    if kb_003 || ex_003 {
        p.uncertainty.last_point_three.reset();
    }
}

/// `addZeroPointThreeToPossibilities`.
fn add_zero_point_three(p: &mut Movement, cx: &mut Context<'_>, speed: f32, kind: EngineKind, stuck_mask: u32, possible: &mut Vec<VectorData>) {
    let mut set = Vec::new();
    let mut base = Vec3::ZERO;
    if !controls_vertical_movement(p) {
        base.y = p.client_velocity.y;
    } else {
        set.push(VectorData::new(Vec3::new(0.0, p.client_velocity.y, 0.0), vt::ZERO_POINT_ZERO_THREE));
    }
    set.push(VectorData::new(base, vt::ZERO_POINT_ZERO_THREE));
    if p.point_three.is_near_fluid
        && !collisions::is_empty(cx.world, p.bounding_box.expand(0.4, 0.0, 0.4))
        && !p.on_ground
    {
        set.push(VectorData::new(Vec3::new(0.0, 0.3, 0.0), vt::ZERO_POINT_ZERO_THREE));
    }
    if p.is_swimming {
        set = transform_swimming_vectors(p, set);
    }
    if p.point_three.is_near_climbable {
        let mut climb = Vec3::new(0.0, 0.2, 0.0);
        static_vector_end_of_tick(p, &mut climb);
        set.push(VectorData::new(climb, vt::ZERO_POINT_ZERO_THREE));
    }
    add_jumps(p, cx, &mut set, kind);
    add_explosion(p, &mut set);
    possible.extend(apply_inputs(p, cx, &set, speed, kind, stuck_mask));
}

/// `sortVectorData`: knockback and explosions first, flip-item last, then
/// by distance to the real movement.
fn sort_key(p: &Movement, d: &VectorData) -> (i32, f64) {
    let mut score = 0;
    if d.is_explosion() {
        score -= 5;
    }
    if d.is_knockback() {
        score -= 5;
    }
    if d.is_first_bread_explosion() {
        score += 1;
    }
    if d.is_first_bread_kb() {
        score += 1;
    }
    if d.is_flip_item() {
        score += 3;
    }
    if d.is_zero_point_zero_three() {
        score -= 1;
    }
    if p.on_ground && d.vector.y >= 0.0 {
        score += 2;
    }
    (score, d.vector.distance_squared(p.actual_movement))
}

fn do_predictions(p: &mut Movement, cx: &mut Context<'_>, possible: &mut [VectorData], speed: f32) -> Vec3 {
    possible.sort_by(|a, b| {
        let (sa, da) = sort_key(p, a);
        let (sb, db) = sort_key(p, b);
        sa.cmp(&sb).then(da.total_cmp(&db))
    });
    p.last_possible.clear();
    p.last_possible.extend_from_slice(possible);

    let mut best_input = f64::MAX;
    let mut best: Option<VectorData> = None;
    let mut before_collision = Vec3::ZERO;
    let mut real_before_collision = Vec3::ZERO;
    let original_client_vel = p.client_velocity;
    let original_bb = p.bounding_box;
    let point_three_box = p.box_at(p.last_x, p.last_y, p.last_z, 0.6, 0.6);
    p.skipped_tick_in_actual_movement = false;

    for d in possible.iter() {
        let primary = handle_starting_velocity_uncertainty(p, cx, d, p.actual_movement);
        let theoretical_box = Aabb::new(0.0, primary.y.min(0.0), 0.0, primary.x, primary.y.max(0.6), primary.z).sorted();
        let theoretical = cut_box_to_vector(p.actual_movement, theoretical_box.min(), theoretical_box.max());
        if theoretical.distance_squared(p.actual_movement) > best_input && !d.is_knockback() && !d.is_explosion() {
            continue;
        }
        p.bounding_box = if d.is_zero_point_zero_three() { point_three_box } else { original_bb };

        let (mut test_primary, mut output, mut real_primary) = seek_wall_collisions(p, cx, primary, original_client_vel, d);
        let mut accuracy = output.distance_squared(p.actual_movement);
        // Grim moves 0.03 candidates with a 0.6 x 0.6 box because the pose is
        // unknown after ticks without a position; the real pose is just as
        // possible (it stops at a ledge the small box slips under).
        if d.is_zero_point_zero_three() && original_bb != point_three_box {
            p.bounding_box = original_bb;
            let (t, o, r) = seek_wall_collisions(p, cx, primary, original_client_vel, d);
            let a = o.distance_squared(p.actual_movement);
            if a < accuracy {
                (test_primary, output, real_primary, accuracy) = (t, o, r, a);
            }
        }

        if d.is_zero_point_zero_three() && accuracy < 0.001 * 0.001 {
            p.skipped_tick_in_actual_movement = true;
        }
        if d.is_knockback() {
            KnockbackHandler::handle_prediction_analysis(p, reduce_offset(p, accuracy).sqrt());
        }
        if d.is_explosion() {
            ExplosionHandler::handle_prediction_analysis(p, reduce_offset(p, accuracy).sqrt());
        }
        if (d.is_knockback() || d.is_explosion()) && !d.is_zero_point_zero_three() {
            let one = [*d];
            let was_003 = determine_can_skip_tick(p, cx, speed, &one);
            if d.is_knockback() {
                cx.kb.point_three |= was_003;
            }
            if d.is_explosion() {
                cx.explosion.point_three |= was_003;
            }
        }
        if KnockbackHandler::should_ignore_for_prediction(p, d, &cx.kb_cfg)
            || ExplosionHandler::should_ignore_for_prediction(p, d, cx.explosion_threshold)
        {
            continue;
        }
        if accuracy < best_input {
            let mut picked = d.modified(output, vt::BEST_VEL_PICKED);
            picked.types |= d.types & !vt::STICKY;
            best = Some(picked);
            before_collision = test_primary;
            real_before_collision = real_primary;
            if p.would_collision_result_flag_ground_spoof(test_primary.y, output.y) {
                accuracy += 0.0001 * 0.0001;
            }
            best_input = accuracy;
        }
        if best_input < 1e-5 * 1e-5
            && !KnockbackHandler::would_flag(p, &cx.kb_cfg)
            && !ExplosionHandler::would_flag(p, cx.explosion_threshold)
        {
            break;
        }
    }

    let best = best.unwrap_or_else(|| VectorData::new(Vec3::ZERO, vt::NORMAL));
    p.client_velocity = real_before_collision;
    p.predicted = best;
    p.stuck_speed = best.stuck;
    p.bounding_box = original_bb;
    if p.predicted.is_zero_point_zero_three() {
        p.skipped_tick_in_actual_movement = true;
    }
    before_collision
}

fn seek_wall_collisions(p: &mut Movement, cx: &mut Context<'_>, primary: Vec3, original_client_vel: Vec3, d: &VectorData) -> (Vec3, Vec3, Vec3) {
    let x_add = primary.x.signum() * COLLISION_EPSILON * f64::from(u8::from(primary.x != 0.0));
    let y_add = if primary.y > 0.0 { 1.0 } else { -1.0 } * COLLISION_EPSILON;
    let z_add = primary.z.signum() * COLLISION_EPSILON * f64::from(u8::from(primary.z != 0.0));
    let mut test = Vec3::new(primary.x + x_add, primary.y + y_add, primary.z + z_add);
    let mut out = collisions::collide(p, cx.world, test.x, test.y, test.z, original_client_vel.y, Some(d), cx.disallow_stepping);
    if test.x == out.x {
        test.x -= x_add;
        out.x -= x_add;
    }
    if test.y == out.y {
        test.y -= y_add;
        out.y -= y_add;
    }
    if test.z == out.z {
        test.z -= z_add;
        out.z -= z_add;
    }
    (test, out, primary)
}

/// `UncertaintyHandler.getOffsetHorizontal`.
fn offset_horizontal(p: &Movement, d: &VectorData) -> f64 {
    let t = MOVEMENT_THRESHOLD;
    let new_vector_003 = p.could_skip_tick && d.is_knockback();
    let explicit_003 = d.is_zero_point_zero_three() || p.uncertainty.last_movement_was_003;
    let either = new_vector_003 || explicit_003;
    let mut point_three = if new_vector_003 || p.uncertainty.last_movement_was_unknown_003_reset { t } else { 0.0 };
    if explicit_003 {
        point_three = 0.91 * 0.6 * (t * 2.0) + t;
    }
    if either && (p.uncertainty.influenced_by_bouncy_block() || p.uncertainty.is_stepping_on_honey) {
        point_three = 0.91 * 0.8 * (t * 2.0) + t;
    }
    if either && p.uncertainty.is_stepping_on_ice {
        point_three = 0.91 * 0.989 * (t * 2.0) + t;
    }
    if point_three > t {
        point_three *= 0.91 * 0.989;
    }
    if either && (p.last_on_ground || p.is_flying) {
        point_three = 0.91 * (t * 2.0) + t;
    }
    if either && (p.is_gliding || p.was_gliding) {
        point_three = 0.99 * (t * 2.0) + t;
    }
    point_three
}

/// `UncertaintyHandler.getVerticalOffset`.
fn offset_vertical(p: &Movement, d: &VectorData) -> f64 {
    let u = &p.uncertainty;
    if u.was_stepping_on_bouncy && (p.was_touching_water || p.was_touching_lava) {
        return 0.06;
    }
    if u.last_flying_ticks.has_occurred_since(5) && d.vector.y.abs() < 4.5 * f64::from(p.fly_speed) - 0.25 {
        return 0.06;
    }
    let t = MOVEMENT_THRESHOLD;
    if d.is_trident() {
        return t * 2.0;
    }
    if p.could_skip_tick && (d.is_knockback() || p.is_climbing) && !d.is_zero_point_zero_three() {
        return t;
    }
    if controls_vertical_movement(p) && (d.is_zero_point_zero_three() || u.last_movement_was_003) {
        return t * 2.0;
    }
    if u.was_003_vertically || u.on_ground_uncertain || u.last_packet_was_ground_packet {
        return t;
    }
    0.0
}

/// `PointThreeEstimator.getAdditionalVerticalUncertainty`.
fn additional_vertical_uncertainty(p: &mut Movement, d: &VectorData) -> f64 {
    let fluid_addition = if d.is_zero_point_zero_three() { 0.014 } else { 0.0 };
    if p.point_three.head_hitter {
        p.point_three.was_always_certain = false;
        return -d.vector.y.max(0.0) - 0.1 - fluid_addition;
    }
    if !d.is_zero_point_zero_three() {
        return 0.0;
    }
    let min_movement = 0.003;
    let mut y_vel = d.vector.y;
    let mut max_y_traveled = 0.0;
    let mut first = true;
    let mut guard = 0;
    loop {
        if y_vel.abs() < min_movement {
            y_vel = 0.0;
        }
        if !first {
            max_y_traveled += y_vel;
        }
        if !first && y_vel == 0.0 {
            break;
        }
        first = false;
        if let Some(level) = p.effects.level(Effect::Levitation) {
            y_vel += (0.05 * f64::from(level + 1) - y_vel) * 0.2;
        } else if p.has_gravity {
            y_vel -= p.gravity;
        }
        y_vel *= f64::from(collisions::modified_air_drag(0.98, p));
        guard += 1;
        if y_vel == 0.0 || guard > 256 || (max_y_traveled + d.vector.y).abs() >= MOVEMENT_THRESHOLD {
            break;
        }
    }
    if max_y_traveled != 0.0 {
        p.point_three.was_always_certain = false;
    }
    max_y_traveled
}

/// `PredictionEngine.handleStartingVelocityUncertainty`: grows the vector
/// into a box of what the client could have had, and returns the point of
/// it closest to `target`.
fn handle_starting_velocity_uncertainty(p: &mut Movement, cx: &mut Context<'_>, d: &VectorData, target: Vec3) -> Vec3 {
    let avg_colliding = f64::from(p.uncertainty.max_colliding_entities());
    let mut add_h = offset_horizontal(p, d) + p.uncertainty.last_horizontal_offset;
    let add_v = offset_vertical(p, d) + p.uncertainty.last_vertical_offset;
    let mut bonus_y = 0.0;
    if p.uncertainty.last_flying_status_change.has_occurred_since(4) {
        add_h += 0.3;
        bonus_y += 0.3;
    }
    if p.uncertainty.last_hard_colliding_entity.has_occurred_since(2) {
        add_h += 0.1;
        bonus_y += 0.1;
    }
    if p.point_three.near_horizontal_flow && d.is_zero_point_zero_three() {
        add_h += 0.014 * 2.0;
    }
    let u = &p.uncertainty;
    let uncertainty = Vec3::new(avg_colliding * 0.08, add_v, avg_colliding * 0.08);
    let min = Vec3::new(u.x_neg - add_h, -bonus_y + u.y_neg, u.z_neg - add_h);
    let max = Vec3::new(u.x_pos + add_h, bonus_y + u.y_pos, u.z_pos + add_h);
    let mut min_v = d.vector.add(min.sub(uncertainty));
    let mut max_v = d.vector.add(max.add(uncertainty));

    if p.uncertainty.on_ground_uncertain && d.vector.y < 0.0 && !p.uncertainty.influenced_by_bouncy_block() {
        let movement_y = min_v.y.max(-MOVEMENT_THRESHOLD);
        let bounce_y = collisions::velocity_after_vertical_collision(p, min_v.y, movement_y, f64::from(collisions::entity_bounciness(p)));
        if bounce_y < 0.0 {
            min_v.y = bounce_y;
        } else {
            max_v.y = bounce_y;
        }
    }

    let gravity_offset = additional_vertical_uncertainty(p, d);
    if gravity_offset > 0.0 {
        max_v.y += gravity_offset;
    } else {
        min_v.y += gravity_offset;
    }
    if (p.point_three.near_bubble_column || p.point_three.near_vertical_flow) && d.is_zero_point_zero_three() {
        min_v.y -= 0.014 * 2.0;
    }
    if p.point_three.near_bubble_column && d.is_zero_point_zero_three() {
        max_v.y += 0.35;
        min_v.y -= 0.35;
    }
    if !p.point_three.can_predict_next_vertical_movement() {
        min_v.y -= p.attr(Attr::Gravity);
    }
    if p.uncertainty.influenced_by_bouncy_block() && p.uncertainty.this_tick_slime_uncertainty != 0.0 && !d.is_jump() {
        if p.uncertainty.this_tick_slime_uncertainty > max_v.y {
            max_v.y = p.uncertainty.this_tick_slime_uncertainty;
        }
        if min_v.y > 0.0 {
            min_v.y = 0.0;
        }
    }
    if d.is_zero_point_zero_three() && d.is_swim_hop() {
        min_v.y -= 0.06;
    }

    let mut b = Aabb::from_points(min_v, max_v);
    for level in [
        p.point_three.positive_levitation(max_v.y),
        p.point_three.positive_levitation(min_v.y),
        p.point_three.negative_levitation(max_v.y),
        p.point_three.negative_levitation(min_v.y),
    ] {
        b = b.include_point(b.min_x, level, b.min_z);
    }
    b.min_x += p.sneaking_hidden.min_x;
    b.min_z += p.sneaking_hidden.min_z;
    b.max_x += p.sneaking_hidden.max_x;
    b.max_z += p.sneaking_hidden.max_z;
    if p.uncertainty.stuck_on_edge.has_occurred_since(0) || p.uncertainty.influenced_by_slime() {
        b = b.include_point(0.0, b.max_y, 0.0);
    }
    if d.vector.y > 0.0
        && d.is_zero_point_zero_three()
        && !collisions::is_empty(cx.world, p.box_at(p.last_x, d.vector.y + p.last_y + 0.6, p.last_z, 0.6, 1.26))
    {
        b = b.include_point(0.0, 0.0, 0.0);
    }
    cut_box_to_vector(target, b.min(), b.max())
}

/// `PredictionEngineNormal.staticVectorEndOfTick`.
fn static_vector_end_of_tick(p: &mut Movement, v: &mut Vec3) {
    let mut adjusted_y = v.y;
    if let Some(level) = p.effects.level(Effect::Levitation) {
        adjusted_y += (0.05 * f64::from(level + 1) - v.y) * 0.2;
        p.fall_distance = 0.0;
    } else if p.has_gravity {
        let mut gravity = p.attr(Attr::Gravity);
        if v.y <= 0.0 && p.slow_falling() {
            gravity = gravity.min(0.01);
        }
        adjusted_y -= gravity;
    }
    let f = f64::from(p.friction);
    v.x *= f;
    v.y = adjusted_y * f64::from(collisions::modified_air_drag(0.98, p));
    v.z *= f;
}

fn static_vector_end_of_tick_normal(p: &mut Movement) {
    let mut v = p.client_velocity;
    static_vector_end_of_tick(p, &mut v);
    p.client_velocity = v;
}

/// `endOfTick` of the engine that ran.
fn end_of_tick(p: &mut Movement, world: &dyn BlockView, kind: EngineKind) {
    p.can_swim_hop = can_swim_hop(p, world);
    p.last_was_climbing = 0.0;
    match kind {
        EngineKind::Normal => {
            p.is_climbing = collisions::on_climbable(world, p.x, p.y, p.z);
            if p.last_was_climbing == 0.0 && (p.point_three.is_near_climbable || p.is_climbing) {
                let mut ladder = p.client_velocity.with_y(0.2);
                static_vector_end_of_tick(p, &mut ladder);
                p.last_was_climbing = ladder.y;
            }
            static_vector_end_of_tick_normal(p);
        }
        EngineKind::Water { gravity, falling, friction } => {
            let f = f64::from(friction);
            let v = p.client_velocity.mul(f, f64::from(0.8_f32), f);
            p.client_velocity = fluid_falling_adjusted(p, gravity, falling, v);
        }
        EngineKind::Lava => {}
    }
}

/// `PredictionEngine.canSwimHop`.
fn can_swim_hop(p: &mut Movement, world: &dyn BlockView) -> bool {
    let old_box = p.box_at(p.last_x, p.last_y, p.last_z, 0.6, 1.8);
    if !fluid::contains_liquid(world, old_box.inflate(0.1)) {
        return false;
    }
    let old_bb = p.bounding_box;
    let t = MOVEMENT_THRESHOLD;
    p.bounding_box = p.bounding_box.expand(-t, 0.0, -t);
    let to_ground = collisions::collide_simple(p, world, 0.0, -t, 0.0).y + COLLISION_EPSILON;
    p.bounding_box = old_bb;
    let new_box = p.box_at(p.x, p.y, p.z, 0.6, 1.8);
    p.uncertainty.last_hard_colliding_entity.has_occurred_since(3)
        || !collisions::is_empty(
            world,
            new_box.expand_towards(p.client_velocity.x, -to_ground, p.client_velocity.z).expand(0.5, 0.03, 0.5),
        )
}

/// `PointThreeEstimator.endOfTickTick`.
fn end_of_tick_point_three(p: &mut Movement, world: &dyn BlockView) {
    let t = MOVEMENT_THRESHOLD;
    let point_three_box = p.box_at(p.x, p.y - t, p.z, 0.6 + (t * 2.0) as f32, 1.8 + (t * 2.0) as f32);
    let old_bb = p.bounding_box;
    let mut head_hitter = false;
    let sizes: &[f32] = if p.skipped_tick_in_actual_movement { &[0.6, 1.5, 1.8] } else { &[0.0] };
    for &s in sizes {
        let size = if s == 0.0 { p.pose.height() } else { s };
        p.bounding_box = p.box_at(p.x, p.y + f64::from(size - 0.01), p.z, 0.6, 0.01);
        head_hitter = head_hitter || collisions::collide_simple(p, world, 0.0, t, 0.0).y != t;
    }
    p.bounding_box = old_bb;
    p.point_three.head_hitter = head_hitter;

    let (mut climbable, mut bubble, mut fluid_near, mut h_flow, mut v_flow) = (false, false, false, false, false);
    collisions::for_each_block(world, point_three_box, |x, y, z, b| {
        if b.climbable {
            climbable = true;
        }
        if b.kind == BlockKind::BubbleColumn {
            bubble = true;
        }
        let f = world.fluid(x, y, z);
        if f.kind != crate::world::FluidKind::None {
            fluid_near = true;
            let flow = fluid::flow(world, x, y, z);
            if flow.x != 0.0 || flow.z != 0.0 {
                h_flow = true;
            }
            if flow.y != 0.0 {
                v_flow = true;
            }
        }
    });
    let pt = &mut p.point_three;
    pt.is_near_climbable = climbable;
    pt.near_bubble_column = bubble;
    pt.is_near_fluid = fluid_near;
    pt.near_horizontal_flow = h_flow;
    pt.near_vertical_flow = v_flow;
    pt.max_positive_levitation = i32::MIN;
    pt.min_negative_levitation = i32::MAX;
    pt.is_gliding = p.is_gliding;
    pt.gravity_changed = false;
    pt.was_always_certain = true;
    pt.is_pushing = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{BlockFacts, TestWorld};

    fn ctx<'a>(w: &'a TestWorld, kb: &'a mut KnockbackHandler, ex: &'a mut ExplosionHandler) -> Context<'a> {
        Context { world: w, kb, explosion: ex, kb_cfg: VelocityConfig::default(), explosion_threshold: 1e-5, disallow_stepping: false }
    }

    #[test]
    fn standing_still_on_ground_predicts_zero() {
        let mut w = TestWorld::new();
        w.floor(-4, 4, 63, -4, 4, BlockFacts::SOLID);
        let mut p = Movement::default();
        (p.x, p.y, p.z, p.last_x, p.last_y, p.last_z) = (0.5, 64.0, 0.5, 0.5, 64.0, 0.5);
        p.bounding_box = p.pose_box_at(0.5, 64.0, 0.5);
        p.on_ground = true;
        p.last_on_ground = true;
        let (mut kb, mut ex) = (KnockbackHandler::default(), ExplosionHandler::default());
        let mut cx = ctx(&w, &mut kb, &mut ex);
        let r = check(&mut p, &mut cx, PositionUpdate { to: Vec3::new(0.5, 64.0, 0.5), on_ground: true, is_teleport: false, teleport_velocity: None });
        assert!(r.checked);
        assert!(r.offset < 1e-9, "offset {}", r.offset);
    }
}
