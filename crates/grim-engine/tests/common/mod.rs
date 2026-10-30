// Grim-Horizon test support: an independent model of the vanilla 26.3
// client's movement (LocalPlayer -> LivingEntity.travel), written from the
// game's behaviour rather than from the engine's code, plus a harness that
// drives the engine with the packets such a client sends.
#![allow(dead_code)]

use grim_horizon_engine::engine::{Action, Clientbound, Engine, PlayerKey, Serverbound, Verdict};
use grim_horizon_engine::math::{Aabb, Vec3, mc_cos, mc_sin};
use grim_horizon_engine::world::{BlockFacts, BlockKind, FluidKind, TestWorld};
use grim_horizon_engine::{CheckId, Config};

pub const KEY: PlayerKey = 7;

/// A vanilla client, simplified to what the scenarios use.
#[derive(Clone, Debug)]
pub struct Client {
    pub pos: Vec3,
    pub vel: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    pub sprinting: bool,
    pub sneaking: bool,
    pub no_jump_delay: i32,
    pub movement_speed: f64,
    pub jump_strength: f32,
    pub position_reminder: i32,
    pub last_sent_pos: Vec3,
    pub last_sent_rot: (f32, f32),
    pub last_sent_ground: bool,
    pub last_sent_input: u8,
    pub last_sent_sprint: bool,
    pub in_water: bool,
    pub jump_boost: Option<i32>,
    /// The last tick's movement, for the supporting-block fallback.
    pub last_move: Vec3,
    pub support: Option<[i32; 3]>,
    pub pose_height: f64,
    pub eye_height: f64,
    pub crouching_pose: bool,
    pub swimming: bool,
    pub swimming_pose_last_was_swim: bool,
    pub eye_in_water_last: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Keys {
    pub forward: bool,
    pub backward: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub sneak: bool,
    pub sprint: bool,
}

impl Keys {
    pub fn flags(&self) -> u8 {
        u8::from(self.forward)
            | u8::from(self.backward) << 1
            | u8::from(self.left) << 2
            | u8::from(self.right) << 3
            | u8::from(self.jump) << 4
            | u8::from(self.sneak) << 5
            | u8::from(self.sprint) << 6
    }
    pub fn forward() -> Self {
        Keys {
            forward: true,
            ..Default::default()
        }
    }
}

/// `EntityDimensions.makeBoundingBox`: the half width is `0.6F / 2.0F`
/// widened to a double, and heights are floats too.
fn player_box(p: Vec3, height: f64) -> Aabb {
    let half = f64::from(0.6_f32 / 2.0);
    let height = f64::from(height as f32);
    Aabb::new(
        p.x - half,
        p.y,
        p.z - half,
        p.x + half,
        p.y + height,
        p.z + half,
    )
}

impl Client {
    pub fn new(pos: Vec3) -> Self {
        Self {
            pos,
            vel: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
            sprinting: false,
            sneaking: false,
            no_jump_delay: 0,
            movement_speed: 0.10000000149011612,
            jump_strength: 0.42,
            position_reminder: 0,
            last_sent_pos: pos,
            last_sent_rot: (0.0, 0.0),
            last_sent_ground: false,
            last_sent_input: 0,
            last_sent_sprint: false,
            in_water: false,
            jump_boost: None,
            last_move: Vec3::ZERO,
            support: None,
            pose_height: 1.8,
            eye_height: 1.62,
            crouching_pose: false,
            swimming: false,
            swimming_pose_last_was_swim: false,
            eye_in_water_last: false,
        }
    }

    fn height(&self) -> f64 {
        self.pose_height
    }

    /// `Entity.collide` with vanilla's step-up, against the world's boxes.
    fn collide(&self, world: &TestWorld, m: Vec3) -> Vec3 {
        use grim_horizon_engine::world::BlockView;
        let bb = player_box(self.pos, self.height());
        let mut boxes = Vec::new();
        world.collision_boxes(
            bb.expand_towards(m.x, m.y, m.z)
                .expand(0.0, 0.6, 0.0)
                .inflate(1.0),
            &mut boxes,
        );
        let along = |mut v: Vec3, start: Aabb| {
            let mut b = start;
            v.y = shapes_collide(v.y, &boxes, |c, d| c.collide_y(&b, d));
            b = b.offset(0.0, v.y, 0.0);
            let x_first = v.x.abs() >= v.z.abs();
            if x_first {
                v.x = shapes_collide(v.x, &boxes, |c, d| c.collide_x(&b, d));
                b = b.offset(v.x, 0.0, 0.0);
                v.z = shapes_collide(v.z, &boxes, |c, d| c.collide_z(&b, d));
            } else {
                v.z = shapes_collide(v.z, &boxes, |c, d| c.collide_z(&b, d));
                b = b.offset(0.0, 0.0, v.z);
                v.x = shapes_collide(v.x, &boxes, |c, d| c.collide_x(&b, d));
            }
            v
        };
        let mut r = along(m, bb);
        let landed = r.y != m.y && m.y < 0.0;
        if (self.on_ground || landed) && (r.x != m.x || r.z != m.z) {
            // Step up by at most 0.6: try the stair heights.
            let start = if landed { bb.offset(0.0, r.y, 0.0) } else { bb };
            let mut heights: Vec<f64> = boxes
                .iter()
                .flat_map(|c| [c.min_y, c.max_y])
                .map(|y| f64::from((y - start.min_y) as f32))
                .filter(|h| *h >= 0.0 && *h <= 0.6 && *h != f64::from(r.y as f32))
                .collect();
            heights.sort_by(|a, b| a.total_cmp(b));
            heights.dedup();
            for h in heights {
                let s = along(Vec3::new(m.x, h, m.z), start);
                if s.x * s.x + s.z * s.z > r.x * r.x + r.z * r.z {
                    r = Vec3::new(s.x, s.y + (start.min_y - bb.min_y), s.z);
                    break;
                }
            }
        }
        r
    }

    /// `Entity.checkSupportingBlock`: the block under the feet whose shape
    /// touches the box, closest to the player (vanilla's tie-break by lower
    /// y, then lower x and z); else under the box moved back by the last
    /// movement while still on the ground.
    fn update_supporting_block(&mut self, world: &TestWorld) {
        use grim_horizon_engine::world::BlockView;
        if !self.on_ground {
            self.support = None;
            return;
        }
        let find = |bb: Aabb| -> Option<[i32; 3]> {
            let below = Aabb::new(
                bb.min_x,
                bb.min_y - 1.0e-6,
                bb.min_z,
                bb.max_x,
                bb.min_y,
                bb.max_z,
            );
            let mut best: Option<([i32; 3], f64)> = None;
            for x in (below.min_x.floor() as i32)..=(below.max_x.floor() as i32) {
                for y in (below.min_y.floor() as i32)..=(below.max_y.floor() as i32) {
                    for z in (below.min_z.floor() as i32)..=(below.max_z.floor() as i32) {
                        let mut boxes = Vec::new();
                        world.block_collision_boxes(x, y, z, &mut boxes);
                        if !boxes.iter().any(|c| c.intersects(&below)) {
                            continue;
                        }
                        let c =
                            Vec3::new(f64::from(x) + 0.5, f64::from(y) + 0.5, f64::from(z) + 0.5);
                        let d = self.pos.distance_squared(c);
                        let better = match best {
                            None => true,
                            Some((b, bd)) => {
                                d < bd
                                    || (d == bd
                                        && (y < b[1]
                                            || (y == b[1]
                                                && (x < b[0] || (x == b[0] && z < b[2])))))
                            }
                        };
                        if better {
                            best = Some(([x, y, z], d));
                        }
                    }
                }
            }
            best.map(|b| b.0)
        };
        let bb = player_box(self.pos, self.height());
        self.support =
            find(bb).or_else(|| find(bb.offset(-self.last_move.x, 0.0, -self.last_move.z)));
    }

    /// `getBlockPosBelowThatAffectsMyMovement` (`getOnPos(0.500001)`).
    fn block_friction(&self, world: &TestWorld) -> f32 {
        use grim_horizon_engine::world::BlockView;
        let below_y = (self.pos.y - 0.500001).floor() as i32;
        match self.support {
            Some(s) => world.block(s[0], below_y, s[2]).friction,
            None => {
                world
                    .block(
                        self.pos.x.floor() as i32,
                        below_y,
                        self.pos.z.floor() as i32,
                    )
                    .friction
            }
        }
    }

    fn water_top_over_box(&self, world: &TestWorld) -> f64 {
        use grim_horizon_engine::world::BlockView;
        let bb = player_box(self.pos, self.height()).inflate(-0.001);
        let mut h = 0.0_f64;
        for x in (bb.min_x.floor() as i32)..(bb.max_x.ceil() as i32) {
            for y in (bb.min_y.floor() as i32)..(bb.max_y.ceil() as i32) {
                for z in (bb.min_z.floor() as i32)..(bb.max_z.ceil() as i32) {
                    let f = world.fluid(x, y, z);
                    if f.kind == FluidKind::Water {
                        let top = f64::from(y) + f64::from(f.height);
                        if top >= bb.min_y {
                            h = h.max(top - bb.min_y);
                        }
                    }
                }
            }
        }
        h
    }

    fn eye_in_water(&self, world: &TestWorld) -> bool {
        use grim_horizon_engine::world::BlockView;
        let eye = self.pos.y + self.eye_height - 0.1111111119389534;
        let (x, z) = (self.pos.x.floor() as i32, self.pos.z.floor() as i32);
        let y = eye.floor() as i32;
        let f = world.fluid(x, y, z);
        f.kind == FluidKind::Water && eye <= f64::from(y) + f64::from(f.height)
    }

    /// One client tick in vanilla's order: `Entity.baseTick` (fluid state,
    /// swimming), `LocalPlayer.aiStep` (crouching from last tick's key, the
    /// keys, sprinting, sinking), `LivingEntity.aiStep` (jumping),
    /// `Player.travel` / `LivingEntity.travel`, then `updatePlayerPose`.
    /// One client tick with these keys, and the packets it sends
    /// (LocalPlayer.tick: input, sprint command, sendPosition, tick end).
    pub fn packets(
        &mut self,
        world: &TestWorld,
        keys: Keys,
        tamper: impl FnOnce(&mut Client),
    ) -> Vec<Serverbound> {
        let mut packets = Vec::new();
        let flags = keys.flags();
        if flags != self.last_sent_input {
            packets.push(Serverbound::Input { flags });
            self.last_sent_input = flags;
        }
        self.tick(world, keys);
        tamper(self);
        if self.sprinting != self.last_sent_sprint {
            packets.push(Serverbound::PlayerCommand {
                action: if self.sprinting { 1 } else { 2 },
            });
            self.last_sent_sprint = self.sprinting;
        }
        // LocalPlayer.sendPosition.
        let c = self;
        c.position_reminder += 1;
        let d = c.pos.sub(c.last_sent_pos);
        let moved = d.length_squared() > 2.0e-4 * 2.0e-4 || c.position_reminder >= 20;
        let turned = (c.yaw, c.pitch) != c.last_sent_rot;
        let packet = if moved && turned {
            Some(Serverbound::Move {
                pos: Some(c.pos),
                rot: Some((c.yaw, c.pitch)),
                on_ground: c.on_ground,
                horizontal_collision: false,
            })
        } else if moved {
            Some(Serverbound::Move {
                pos: Some(c.pos),
                rot: None,
                on_ground: c.on_ground,
                horizontal_collision: false,
            })
        } else if turned {
            Some(Serverbound::Move {
                pos: None,
                rot: Some((c.yaw, c.pitch)),
                on_ground: c.on_ground,
                horizontal_collision: false,
            })
        } else if c.on_ground != c.last_sent_ground {
            Some(Serverbound::Move {
                pos: None,
                rot: None,
                on_ground: c.on_ground,
                horizontal_collision: false,
            })
        } else {
            None
        };
        if moved {
            c.last_sent_pos = c.pos;
            c.position_reminder = 0;
        }
        if turned {
            c.last_sent_rot = (c.yaw, c.pitch);
        }
        if packet.is_some() {
            c.last_sent_ground = c.on_ground;
        }
        packets.extend(packet);
        packets.push(Serverbound::TickEnd);
        packets
    }

    pub fn tick(&mut self, world: &TestWorld, keys: Keys) {
        use grim_horizon_engine::world::BlockView;
        // --- Entity.baseTick
        let was_eye_in_water = self.eye_in_water_last;
        let fluid_height = self.water_top_over_box(world);
        let water = fluid_height > 0.0;
        self.in_water = water;
        let feet_in_water = world
            .fluid(
                self.pos.x.floor() as i32,
                self.pos.y.floor() as i32,
                self.pos.z.floor() as i32,
            )
            .kind
            == FluidKind::Water;
        // Player.updateSwimming, with last tick's sprinting.
        self.swimming = if self.swimming {
            self.sprinting && water
        } else {
            self.sprinting && was_eye_in_water && water && feet_in_water
        };
        // updateFluidOnEyes for the next tick.
        self.eye_in_water_last = self.eye_in_water(world);
        let under_water = was_eye_in_water && water;

        // --- LocalPlayer.aiStep
        // `crouching` is decided before `input.tick()` reads this tick's keys:
        // not swimming, room to crouch, and last tick's shift key.
        let fits = |h: f64| {
            let b = player_box(self.pos, h).inflate(-1.0e-7);
            let mut boxes = Vec::new();
            world.collision_boxes(b, &mut boxes);
            !boxes.iter().any(|c| c.touches(&b))
        };
        let crouching = !self.swimming && fits(1.5) && (self.sneaking || !fits(1.8));
        self.sneaking = keys.sneak;
        // Sprinting: needs forward impulse, stops when it is lost, while
        // sneaking, or in water when not under it.
        let impulse = keys.forward && !keys.backward;
        if self.sprinting && (!impulse || keys.sneak || (water && !under_water)) {
            self.sprinting = false;
        } else if !self.sprinting
            && keys.sprint
            && impulse
            && !keys.sneak
            && (!water || under_water)
        {
            self.sprinting = true;
        }
        if water && keys.sneak {
            self.vel.y += f64::from(-0.04_f32); // goDownInWater
        }

        // --- LivingEntity.aiStep
        if self.no_jump_delay > 0 {
            self.no_jump_delay -= 1;
        }
        // 1.21.5+: tiny horizontal velocity is dropped as a whole.
        if self.vel.x * self.vel.x + self.vel.z * self.vel.z < 9.0e-6 {
            self.vel.x = 0.0;
            self.vel.z = 0.0;
        }
        if self.vel.y.abs() < 0.003 {
            self.vel.y = 0.0;
        }
        if keys.jump {
            let threshold = 0.4; // getFluidJumpThreshold: eye height >= 0.4
            if water && !(self.on_ground && fluid_height <= threshold) {
                self.vel.y += f64::from(0.04_f32); // jumpInLiquid
            } else if (self.on_ground || (water && fluid_height <= threshold))
                && self.no_jump_delay == 0
            {
                let mut power = self.jump_strength;
                if let Some(level) = self.jump_boost {
                    power += 0.1 * (level + 1) as f32;
                }
                self.vel.y = f64::from(power).max(self.vel.y);
                if self.sprinting {
                    let r = self.yaw * (std::f64::consts::PI / 180.0) as f32;
                    self.vel.x += -f64::from(mc_sin(f64::from(r))) * 0.2;
                    self.vel.z += f64::from(mc_cos(f64::from(r))) * 0.2;
                }
                self.no_jump_delay = 10;
            }
        } else {
            self.no_jump_delay = 0;
        }
        // Input: normalised, times 0.98, crouch slowdown, square movement.
        let fwd = f32::from(i8::from(keys.forward) - i8::from(keys.backward));
        let strafe = f32::from(i8::from(keys.left) - i8::from(keys.right));
        let len = (fwd * fwd + strafe * strafe).sqrt();
        let (mut ix, mut iz) = if len < 1.0e-4 {
            (0.0, 0.0)
        } else {
            (strafe / len, fwd / len)
        };
        if ix != 0.0 || iz != 0.0 {
            ix *= 0.98;
            iz *= 0.98;
            // isMovingSlowly: crouching, or crawling (swimming pose out of water).
            if crouching || (self.pose_height < 1.0 && !water) {
                ix *= 0.3;
                iz *= 0.3;
            }
            let l = (ix * ix + iz * iz).sqrt();
            let (mx, mz) = (ix / l, iz / l);
            let (ax, az) = (mx.abs(), mz.abs());
            let add = if az > ax { ax / az } else { az / ax };
            let m = (l * (1.0 + add * add).sqrt()).min(1.0);
            ix = mx * m;
            iz = mz * m;
        }
        let speed_attr = if self.sprinting {
            f64::from((self.movement_speed + self.movement_speed * f64::from(0.3_f32)) as f32)
        } else {
            self.movement_speed
        };

        if water {
            // Player.travel: a swimmer follows its look up and down.
            if self.swimming {
                let look_y = grim_horizon_engine::math::look_vector(self.yaw, self.pitch).y;
                let k = if look_y < -0.2 { 0.085 } else { 0.06 };
                let above = world
                    .fluid(
                        self.pos.x.floor() as i32,
                        (self.pos.y + 1.0 - 0.1).floor() as i32,
                        self.pos.z.floor() as i32,
                    )
                    .kind;
                if look_y <= 0.0 || keys.jump || above != FluidKind::None {
                    self.vel.y += (look_y - self.vel.y) * k;
                }
            }
            let falling = self.vel.y <= 0.0;
            let friction: f32 = if self.sprinting { 0.9 } else { 0.8 };
            self.move_relative(0.02, ix, iz);
            let m = self.vel;
            let r = self.collide(world, m);
            self.apply_move(m, r, world);
            self.vel = self
                .vel
                .mul(f64::from(friction), f64::from(0.8_f32), f64::from(friction));
            // Fluid-falling adjustment (not sprinting).
            if !self.sprinting {
                let g = 0.08;
                let y = self.vel.y;
                self.vel.y =
                    if falling && (y - 0.005).abs() >= 0.003 && (y - g / 16.0).abs() < 0.003 {
                        -0.003
                    } else {
                        y - g / 16.0
                    };
            }
        } else {
            let block_friction = self.block_friction(world);
            let friction = if self.on_ground {
                f64::from(block_friction * 0.91)
            } else {
                0.91
            };
            let speed = if self.on_ground {
                if block_friction <= 0.6 {
                    speed_attr as f32
                } else {
                    speed_attr as f32
                        * (0.21600002 / (block_friction * block_friction * block_friction))
                }
            } else if self.sprinting {
                0.025999999
            } else {
                0.02
            };
            self.move_relative(speed, ix, iz);
            let mut m = self.vel;
            // Sneaking at an edge (Player.maybeBackOffFromEdge).
            if self.sneaking && self.on_ground {
                m = self.back_off(world, m);
            }
            let r = self.collide(world, m);
            self.apply_move(m, r, world);
            // Gravity and drag.
            self.vel.y = (self.vel.y - 0.08) * f64::from(0.98_f32);
            self.vel.x *= friction;
            self.vel.z *= friction;
        }
        self.update_pose(world);
    }

    /// `Player.updatePlayerPose`: swimming, crouching or standing, if the
    /// box fits.
    fn update_pose(&mut self, world: &TestWorld) {
        use grim_horizon_engine::world::BlockView;
        let fits = |h: f64| {
            let b = player_box(self.pos, h).inflate(-1.0e-7);
            let mut boxes = Vec::new();
            world.collision_boxes(b, &mut boxes);
            !boxes.iter().any(|c| c.touches(&b))
        };
        if !fits(0.6) {
            return;
        }
        let (mut height, mut eye) = if self.swimming {
            (0.6, 0.4)
        } else if self.sneaking {
            (1.5, 1.27)
        } else {
            (1.8, 1.62)
        };
        if !fits(height) {
            (height, eye) = if fits(1.5) { (1.5, 1.27) } else { (0.6, 0.4) };
        }
        self.swimming_pose_last_was_swim = self.swimming;
        self.crouching_pose = (height - 1.5).abs() < 1e-9;
        self.pose_height = height;
        self.eye_height = eye;
    }

    fn back_off(&self, world: &TestWorld, mut m: Vec3) -> Vec3 {
        use grim_horizon_engine::world::BlockView;
        let empty = |dx: f64, dz: f64| {
            let b = player_box(self.pos, self.height()).offset(dx, -0.6, dz);
            let mut boxes = Vec::new();
            world.collision_boxes(b, &mut boxes);
            !boxes.iter().any(|c| c.touches(&b))
        };
        let step = |a: &mut f64| {
            if *a < 0.05 && *a >= -0.05 {
                *a = 0.0
            } else if *a > 0.0 {
                *a -= 0.05
            } else {
                *a += 0.05
            }
        };
        while m.x != 0.0 && empty(m.x, 0.0) {
            step(&mut m.x);
        }
        while m.z != 0.0 && empty(0.0, m.z) {
            step(&mut m.z);
        }
        while m.x != 0.0 && m.z != 0.0 && empty(m.x, m.z) {
            step(&mut m.x);
            step(&mut m.z);
        }
        m
    }

    fn move_relative(&mut self, speed: f32, ix: f32, iz: f32) {
        let r = self.yaw * (std::f64::consts::PI / 180.0) as f32;
        let (sin, cos) = (
            f64::from(mc_sin(f64::from(r))),
            f64::from(mc_cos(f64::from(r))),
        );
        let (ix, iz) = (f64::from(ix), f64::from(iz));
        let s = f64::from(speed);
        self.vel.x += (ix * cos - iz * sin) * s;
        self.vel.z += (iz * cos + ix * sin) * s;
    }

    fn apply_move(&mut self, wanted: Vec3, done: Vec3, world: &TestWorld) {
        use grim_horizon_engine::world::BlockView;
        let mut done = done;
        if done.length_squared() <= 1e-7 && wanted.length_squared() - done.length_squared() >= 1e-7
        {
            done = Vec3::ZERO;
        }
        self.pos = self.pos.add(done);
        self.last_move = done;
        let x_hit = (wanted.x - done.x).abs() >= f64::from(1.0e-5_f32);
        let z_hit = (wanted.z - done.z).abs() >= f64::from(1.0e-5_f32);
        if x_hit {
            self.vel.x = 0.0;
        }
        if z_hit {
            self.vel.z = 0.0;
        }
        let vertical = wanted.y != done.y;
        self.on_ground = vertical && wanted.y < 0.0;
        self.update_supporting_block(world);
        if vertical {
            // Slime bounces (not while sneaking).
            let below = world.block(
                self.pos.x.floor() as i32,
                (self.pos.y - 0.2).floor() as i32,
                self.pos.z.floor() as i32,
            );
            if below.kind == BlockKind::SlimeBlock && !self.sneaking && self.vel.y < 0.0 {
                // getVelocityAfterVerticalCollision with restitution 1.
                let g = 0.08;
                if -self.vel.y > g {
                    let portion = done.y / self.vel.y;
                    let drag = 1.0 + portion * (f64::from(0.98_f32) - 1.0);
                    self.vel.y = (portion * g - self.vel.y) * drag;
                } else {
                    self.vel.y = 0.0;
                }
            } else {
                self.vel.y = 0.0;
            }
        }
        // Block speed factor (soul sand / honey under the player).
        let under = world.block(
            self.pos.x.floor() as i32,
            (self.pos.y - 0.500001).floor() as i32,
            self.pos.z.floor() as i32,
        );
        let inside = world.block(
            self.pos.x.floor() as i32,
            self.pos.y.floor() as i32,
            self.pos.z.floor() as i32,
        );
        let f = if inside.speed_factor != 1.0 {
            inside.speed_factor
        } else {
            under.speed_factor
        };
        self.vel.x *= f64::from(f);
        self.vel.z *= f64::from(f);
    }
}

/// `Shapes.collide`: below 1e-7 is no movement once there is a shape.
fn shapes_collide(mut v: f64, boxes: &[Aabb], collide: impl Fn(&Aabb, f64) -> f64) -> f64 {
    for c in boxes {
        if v.abs() < 1.0e-7 {
            return 0.0;
        }
        v = collide(c, v);
    }
    v
}

/// Drives the engine with a client's packets and answers transactions.
pub struct Harness {
    pub engine: Engine,
    /// The server's world (and the client's, unless `client_world` is set).
    pub world: TestWorld,
    /// What a lagging client still sees, when it differs from the server.
    pub client_world: Option<TestWorld>,
    pub client: Client,
    pub now: u64,
    /// Ticks between a ping and its pong (latency).
    pub latency_ticks: usize,
    pings: std::collections::VecDeque<(usize, i32)>,
    tick: usize,
    pub flags: Vec<(CheckId, String)>,
    pub actions: Vec<Action>,
    pub verdicts: Vec<Verdict>,
    pub teleport_id: i32,
    /// Time spent inside the engine (on_incoming, on_outgoing, end_tick).
    pub engine_nanos: u64,
}

impl Harness {
    pub fn new(world: TestWorld, spawn: Vec3) -> Self {
        let mut config = Config::default();
        // Tests want every flag as an alert line.
        config.alerts.format = "%check%|%verbose%".into();
        let mut engine = Engine::new(config);
        engine.add_player(KEY, 1, "Tester", 0);
        // A staff member to receive alerts.
        engine.add_player(99, 2, "Staff", 4);
        let mut h = Self {
            engine,
            world,
            client_world: None,
            client: Client::new(spawn),
            now: 1_000_000_000_000,
            latency_ticks: 1,
            pings: Default::default(),
            tick: 0,
            flags: Vec::new(),
            actions: Vec::new(),
            verdicts: Vec::new(),
            teleport_id: 0,
            engine_nanos: 0,
        };
        h.engine.on_outgoing(KEY, Clientbound::EntityId(42));
        h.teleport(spawn);
        h
    }

    pub fn with_config(mut self, f: impl FnOnce(&mut Config)) -> Self {
        f(&mut self.engine.config);
        self
    }

    /// A server teleport and the client's acceptance.
    pub fn teleport(&mut self, pos: Vec3) {
        self.teleport_id += 1;
        let id = self.teleport_id;
        self.engine.on_outgoing(
            KEY,
            Clientbound::Teleport {
                id,
                pos,
                delta: Vec3::ZERO,
                relatives: 0,
            },
        );
        self.client.pos = pos;
        self.client.vel = Vec3::ZERO;
        // No rotation relatives: the client takes the look in the packet,
        // which is the server's, the last one the client reported (Horizon's
        // teleport_relative sends the guest's yaw and pitch).
        (self.client.yaw, self.client.pitch) = self.client.last_sent_rot;
        // handleMovePlayer leaves LocalPlayer.sendPosition's baseline alone:
        // the next tick reports the teleported position.
        self.send(vec![Serverbound::AcceptTeleport {
            id,
            pos,
            rot: (self.client.yaw, self.client.pitch),
        }]);
    }

    pub fn send(&mut self, packets: Vec<Serverbound>) {
        let start = std::time::Instant::now();
        let v = self
            .engine
            .on_incoming(KEY, &packets, &self.world, self.now);
        self.engine_nanos += start.elapsed().as_nanos() as u64;
        self.verdicts.extend(v);
        self.collect();
    }

    fn collect(&mut self) {
        for a in self.engine.drain_actions() {
            if let Action::Message { key: 99, text } = &a {
                let mut parts = text.splitn(2, '|');
                let name = parts.next().unwrap_or_default();
                if let Some(c) = CheckId::from_name(name) {
                    self.flags
                        .push((c, parts.next().unwrap_or_default().to_owned()));
                }
            }
            if let Action::SendPing { key: KEY, id } = a {
                self.engine.on_outgoing(KEY, Clientbound::Ping { id });
                self.pings.push_back((self.tick + self.latency_ticks, id));
            }
            self.actions.push(a);
        }
    }

    /// Server-side velocity (knockback) for the player.
    pub fn velocity(&mut self, v: Vec3) {
        self.engine.on_outgoing(KEY, Clientbound::Velocity(v));
    }

    /// One client tick with these keys, then one server tick.
    pub fn tick(&mut self, keys: Keys) {
        self.client_tick(keys, |_| {});
    }

    /// `n` client ticks whose packets reach the server within one server
    /// tick (a client catching up after a stall, or network jitter).
    pub fn burst(&mut self, n: usize, keys: Keys) {
        let mut all = Vec::new();
        for _ in 0..n {
            all.extend(self.client_packets(keys, |_| {}));
        }
        self.send(all);
        self.server_tick();
    }

    /// Like `tick`, but lets a test tamper with the client before it sends.
    pub fn client_tick(&mut self, keys: Keys, tamper: impl FnOnce(&mut Client)) {
        let packets = self.client_packets(keys, tamper);
        self.send(packets);
        self.server_tick();
    }

    fn client_packets(&mut self, keys: Keys, tamper: impl FnOnce(&mut Client)) -> Vec<Serverbound> {
        let mut packets = Vec::new();
        // Pongs that arrived since the last tick.
        while let Some(&(due, id)) = self.pings.front() {
            if due > self.tick {
                break;
            }
            self.pings.pop_front();
            packets.push(Serverbound::Pong { id });
        }
        let world = self.client_world.as_ref().unwrap_or(&self.world);
        packets.extend(self.client.packets(world, keys, tamper));
        packets
    }

    pub fn server_tick(&mut self) {
        self.now += 50_000_000;
        let start = std::time::Instant::now();
        self.engine.end_tick(self.now);
        self.engine_nanos += start.elapsed().as_nanos() as u64;
        self.collect();
        self.tick += 1;
    }

    pub fn flagged(&self, c: CheckId) -> usize {
        self.flags.iter().filter(|(k, _)| *k == c).count()
    }

    pub fn last_offset(&self) -> f64 {
        self.engine.last_offset(KEY).unwrap()
    }

    pub fn checked_predictions(&self) -> (u64, u64) {
        let p = self.engine.player(KEY).unwrap();
        (p.predictions, p.skipped)
    }
}

pub fn flat_world(facts: BlockFacts) -> TestWorld {
    let mut w = TestWorld::new();
    w.floor(-64, 64, 63, -64, 64, facts);
    w
}

pub fn stone() -> BlockFacts {
    BlockFacts::SOLID
}
