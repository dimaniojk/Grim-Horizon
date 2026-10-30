// Grim-Horizon: a Rust port of Grim Anticheat (https://github.com/GrimAnticheat/Grim).
// Copyright (C) GrimAC contributors (original Java), Grim-Horizon contributors (port).
// Licensed under the GNU General Public License, version 3 or (at your option) any later version; see LICENSE.

//! Per-player movement state: the parts of Grim's `GrimPlayer`,
//! `UncertaintyHandler`, `VectorData`, `PacketEntitySelf` (attributes and
//! effects) and `PointThreeEstimator` that the 26.3 movement path reads.

use crate::math::{Aabb, Vec3};

/// `Pose`: width, height, eye height.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Pose {
    #[default]
    Standing,
    FallFlying,
    Sleeping,
    Swimming,
    SpinAttack,
    Crouching,
    Dying,
}

impl Pose {
    pub fn width(self) -> f32 {
        match self {
            Pose::Sleeping => 0.2,
            _ => 0.6,
        }
    }
    pub fn height(self) -> f32 {
        match self {
            Pose::Standing | Pose::Dying => 1.8,
            Pose::FallFlying | Pose::Swimming | Pose::SpinAttack => 0.6,
            Pose::Sleeping => 0.2,
            Pose::Crouching => 1.5,
        }
    }
    pub fn eye_height(self) -> f32 {
        match self {
            Pose::Standing | Pose::Dying => 1.62,
            Pose::FallFlying | Pose::Swimming | Pose::SpinAttack => 0.4,
            Pose::Sleeping => 0.2,
            Pose::Crouching => 1.27,
        }
    }
}

/// Attribute identities the movement code reads, by their vanilla names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Attr {
    MovementSpeed,
    Gravity,
    JumpStrength,
    StepHeight,
    Scale,
    SneakingSpeed,
    WaterMovementEfficiency,
    MovementEfficiency,
    EntityInteractionRange,
    BlockInteractionRange,
    SafeFallDistance,
    FallDamageMultiplier,
    FrictionModifier,
    AirDragModifier,
    Bounciness,
}

impl Attr {
    pub const ALL: [Attr; 15] = [
        Attr::MovementSpeed,
        Attr::Gravity,
        Attr::JumpStrength,
        Attr::StepHeight,
        Attr::Scale,
        Attr::SneakingSpeed,
        Attr::WaterMovementEfficiency,
        Attr::MovementEfficiency,
        Attr::EntityInteractionRange,
        Attr::BlockInteractionRange,
        Attr::SafeFallDistance,
        Attr::FallDamageMultiplier,
        Attr::FrictionModifier,
        Attr::AirDragModifier,
        Attr::Bounciness,
    ];
    /// The registry name, without `minecraft:`.
    pub fn name(self) -> &'static str {
        match self {
            Attr::MovementSpeed => "movement_speed",
            Attr::Gravity => "gravity",
            Attr::JumpStrength => "jump_strength",
            Attr::StepHeight => "step_height",
            Attr::Scale => "scale",
            Attr::SneakingSpeed => "sneaking_speed",
            Attr::WaterMovementEfficiency => "water_movement_efficiency",
            Attr::MovementEfficiency => "movement_efficiency",
            Attr::EntityInteractionRange => "entity_interaction_range",
            Attr::BlockInteractionRange => "block_interaction_range",
            Attr::SafeFallDistance => "safe_fall_distance",
            Attr::FallDamageMultiplier => "fall_damage_multiplier",
            Attr::FrictionModifier => "friction_modifier",
            Attr::AirDragModifier => "air_drag_modifier",
            Attr::Bounciness => "bounciness",
        }
    }
    pub fn from_name(name: &str) -> Option<Attr> {
        let name = name.strip_prefix("minecraft:").unwrap_or(name);
        Attr::ALL.into_iter().find(|a| a.name() == name)
    }
    /// The player's default base value (`Player.createAttributes`).
    pub fn default_base(self) -> f64 {
        match self {
            Attr::MovementSpeed => 0.10000000149011612,
            Attr::Gravity => 0.08,
            Attr::JumpStrength => 0.41999998688697815,
            Attr::StepHeight => 0.6,
            Attr::Scale => 1.0,
            Attr::SneakingSpeed => 0.3,
            Attr::WaterMovementEfficiency => 0.0,
            Attr::MovementEfficiency => 0.0,
            Attr::EntityInteractionRange => 3.0,
            Attr::BlockInteractionRange => 4.5,
            Attr::SafeFallDistance => 3.0,
            Attr::FallDamageMultiplier => 1.0,
            Attr::FrictionModifier => 1.0,
            Attr::AirDragModifier => 1.0,
            Attr::Bounciness => 0.0,
        }
    }
    fn range(self) -> (f64, f64) {
        match self {
            Attr::MovementSpeed => (0.0, 1024.0),
            Attr::Gravity => (-1.0, 1.0),
            Attr::JumpStrength => (0.0, 32.0),
            Attr::StepHeight => (0.0, 10.0),
            Attr::Scale => (0.0625, 16.0),
            Attr::SneakingSpeed => (0.0, 1.0),
            Attr::WaterMovementEfficiency | Attr::MovementEfficiency => (0.0, 1.0),
            Attr::EntityInteractionRange | Attr::BlockInteractionRange => (0.0, 64.0),
            Attr::SafeFallDistance => (-1024.0, 1024.0),
            Attr::FallDamageMultiplier => (0.0, 100.0),
            Attr::FrictionModifier | Attr::AirDragModifier => (0.0, 1024.0),
            Attr::Bounciness => (0.0, 1.0),
        }
    }
}

/// `AttributeModifier.Operation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModifierOp {
    AddValue,
    AddMultipliedBase,
    AddMultipliedTotal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Modifier {
    pub id: String,
    pub amount: f64,
    pub op: ModifierOp,
}

/// The vanilla sprint speed modifier, which Grim strips from server
/// snapshots and applies from the sprinting state instead.
pub const SPRINT_MODIFIER: &str = "minecraft:sprinting";

#[derive(Clone, Debug)]
pub struct AttributeInstance {
    pub base: f64,
    pub modifiers: Vec<Modifier>,
    value: f64,
}

impl AttributeInstance {
    fn new(attr: Attr) -> Self {
        let base = attr.default_base();
        Self {
            base,
            modifiers: Vec::new(),
            value: base,
        }
    }
    /// `AttributeInstance.calculateValue`.
    fn recalculate(&mut self, attr: Attr) {
        let mut v = self.base;
        for m in self
            .modifiers
            .iter()
            .filter(|m| m.op == ModifierOp::AddValue)
        {
            v += m.amount;
        }
        let mut total = v;
        for m in self
            .modifiers
            .iter()
            .filter(|m| m.op == ModifierOp::AddMultipliedBase)
        {
            total += v * m.amount;
        }
        for m in self
            .modifiers
            .iter()
            .filter(|m| m.op == ModifierOp::AddMultipliedTotal)
        {
            total *= 1.0 + m.amount;
        }
        let (lo, hi) = attr.range();
        self.value = total.clamp(lo, hi);
    }
    pub fn value(&self) -> f64 {
        self.value
    }
}

#[derive(Clone, Debug)]
pub struct Attributes {
    instances: Vec<AttributeInstance>,
}

impl Default for Attributes {
    fn default() -> Self {
        Self {
            instances: Attr::ALL
                .iter()
                .map(|a| AttributeInstance::new(*a))
                .collect(),
        }
    }
}

impl Attributes {
    fn index(attr: Attr) -> usize {
        Attr::ALL.iter().position(|a| *a == attr).unwrap_or(0)
    }
    pub fn get(&self, attr: Attr) -> f64 {
        self.instances[Self::index(attr)].value()
    }
    /// Replaces an attribute from a server snapshot (`UpdateAttributes`).
    /// The sprint modifier is dropped: the engine applies sprinting itself,
    /// as Grim's `PacketEntitySelf` does.
    pub fn set(&mut self, attr: Attr, base: f64, modifiers: Vec<Modifier>) {
        let inst = &mut self.instances[Self::index(attr)];
        inst.base = base;
        inst.modifiers = modifiers
            .into_iter()
            .filter(|m| m.id != SPRINT_MODIFIER)
            .collect();
        inst.recalculate(attr);
    }
    pub fn instance(&self, attr: Attr) -> &AttributeInstance {
        &self.instances[Self::index(attr)]
    }
}

/// Status effects the movement code reads, by vanilla name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Effect {
    JumpBoost,
    Levitation,
    SlowFalling,
    DolphinsGrace,
    Speed,
    Slowness,
}

impl Effect {
    pub fn from_name(name: &str) -> Option<Effect> {
        Some(match name.strip_prefix("minecraft:").unwrap_or(name) {
            "jump_boost" => Effect::JumpBoost,
            "levitation" => Effect::Levitation,
            "slow_falling" => Effect::SlowFalling,
            "dolphins_grace" => Effect::DolphinsGrace,
            "speed" => Effect::Speed,
            "slowness" => Effect::Slowness,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Effects {
    active: Vec<(Effect, i32)>,
}

impl Effects {
    pub fn level(&self, e: Effect) -> Option<i32> {
        self.active.iter().find(|(k, _)| *k == e).map(|(_, a)| *a)
    }
    pub fn set(&mut self, e: Effect, amplifier: Option<i32>) {
        self.active.retain(|(k, _)| *k != e);
        if let Some(a) = amplifier {
            self.active.push((e, a));
        }
    }
    pub fn clear(&mut self) {
        self.active.clear();
    }
}

/// `LastInstance`: movements since an event, starting "long ago".
#[derive(Clone, Copy, Debug)]
pub struct LastInstance(i32);
impl Default for LastInstance {
    fn default() -> Self {
        LastInstance(100)
    }
}
impl LastInstance {
    pub fn has_occurred_since(&self, ticks: i32) -> bool {
        self.0 <= ticks
    }
    pub fn reset(&mut self) {
        self.0 = 0;
    }
    pub fn tick(&mut self) {
        if self.0 == i32::MAX {
            self.0 = 100;
        }
        self.0 += 1;
    }
}

/// `VectorData.VectorType` flags carried along a vector's history.
pub mod vt {
    pub const KNOCKBACK: u32 = 1;
    pub const FIRST_BREAD_KB: u32 = 1 << 1;
    pub const EXPLOSION: u32 = 1 << 2;
    pub const FIRST_BREAD_EXPLOSION: u32 = 1 << 3;
    pub const TRIDENT: u32 = 1 << 4;
    pub const ZERO_POINT_ZERO_THREE: u32 = 1 << 5;
    pub const SWIM_HOP: u32 = 1 << 6;
    pub const FLIP_ITEM: u32 = 1 << 7;
    pub const JUMP: u32 = 1 << 8;
    pub const ATTACK_SLOW: u32 = 1 << 9;
    pub const CLIMBABLE: u32 = 1 << 10;
    pub const TELEPORT: u32 = 1 << 11;
    pub const SPECTATOR: u32 = 1 << 12;
    pub const DEAD: u32 = 1 << 13;
    pub const STUCK: u32 = 1 << 14;
    pub const SURFACE_SWIMMING: u32 = 1 << 15;
    pub const SWIMMING_SPACE: u32 = 1 << 16;
    pub const BEST_VEL_PICKED: u32 = 1 << 17;
    pub const NORMAL: u32 = 0;
    /// Types that stay with every vector derived from this one
    /// (`VectorData`'s copied booleans).
    pub const STICKY: u32 = KNOCKBACK
        | FIRST_BREAD_KB
        | EXPLOSION
        | FIRST_BREAD_EXPLOSION
        | TRIDENT
        | ZERO_POINT_ZERO_THREE
        | SWIM_HOP
        | FLIP_ITEM
        | JUMP
        | ATTACK_SLOW;
}

/// `VectorData`.
#[derive(Clone, Copy, Debug)]
pub struct VectorData {
    pub vector: Vec3,
    /// Sticky flags of the whole chain plus this step's type.
    pub types: u32,
    /// The movement input that produced it (`VectorData.input`).
    pub input: Option<Vec3>,
    /// `stuckSpeedMultiplier` applied to it.
    pub stuck: Option<Vec3>,
    /// The vector at the start of the chain, before inputs.
    pub origin: Vec3,
}

impl VectorData {
    pub fn new(vector: Vec3, ty: u32) -> Self {
        Self {
            vector,
            types: ty,
            input: None,
            stuck: None,
            origin: vector,
        }
    }
    /// `returnNewModified`.
    pub fn modified(&self, vector: Vec3, ty: u32) -> Self {
        Self {
            vector,
            types: (self.types & vt::STICKY) | ty,
            input: self.input,
            stuck: self.stuck,
            origin: self.origin,
        }
    }
    #[inline]
    pub fn has(&self, flag: u32) -> bool {
        self.types & flag != 0
    }
    pub fn is_knockback(&self) -> bool {
        self.has(vt::KNOCKBACK)
    }
    pub fn is_first_bread_kb(&self) -> bool {
        self.has(vt::FIRST_BREAD_KB)
    }
    pub fn is_explosion(&self) -> bool {
        self.has(vt::EXPLOSION)
    }
    pub fn is_first_bread_explosion(&self) -> bool {
        self.has(vt::FIRST_BREAD_EXPLOSION)
    }
    pub fn is_zero_point_zero_three(&self) -> bool {
        self.has(vt::ZERO_POINT_ZERO_THREE)
    }
    pub fn is_jump(&self) -> bool {
        self.has(vt::JUMP)
    }
    pub fn is_swim_hop(&self) -> bool {
        self.has(vt::SWIM_HOP)
    }
    pub fn is_trident(&self) -> bool {
        self.has(vt::TRIDENT)
    }
    pub fn is_flip_item(&self) -> bool {
        self.has(vt::FLIP_ITEM)
    }
}

/// `VelocityData`: a knockback or explosion the server sent.
#[derive(Clone, Copy, Debug)]
pub struct VelocityData {
    pub vector: Vec3,
    /// The last transaction sent before the velocity went out.
    pub transaction: u32,
    /// Smallest prediction offset seen with this velocity applied.
    pub offset: f64,
    /// Sent as part of a setback (Grim's `isSetback`): never flagged.
    pub is_setback: bool,
}

impl VelocityData {
    pub fn new(vector: Vec3, transaction: u32) -> Self {
        Self {
            vector,
            transaction,
            offset: f64::from(i32::MAX),
            is_setback: false,
        }
    }
}

/// `UncertaintyHandler`, without the vehicle, piston, firework, fishing-rod
/// and boat parts this port does not model.
#[derive(Clone, Debug, Default)]
pub struct Uncertainty {
    pub is_step_movement: bool,
    pub x_neg: f64,
    pub x_pos: f64,
    pub y_neg: f64,
    pub y_pos: f64,
    pub z_neg: f64,
    pub z_pos: f64,
    pub this_tick_slime_uncertainty: f64,
    pub next_tick_slime_uncertainty: f64,
    pub on_ground_uncertain: bool,
    pub last_packet_was_ground_packet: bool,
    pub was_stepping_on_slime: bool,
    pub is_stepping_on_slime: bool,
    pub is_stepping_on_ice: bool,
    pub is_stepping_on_honey: bool,
    pub was_stepping_on_bouncy: bool,
    pub is_stepping_on_bouncy: bool,
    pub is_stepping_near_bubble_column: bool,
    pub is_stepping_near_scaffolding: bool,
    pub last_movement_was_003: bool,
    pub last_movement_was_unknown_003_reset: bool,
    pub was_003_vertically: bool,
    pub should_simulate_stuck_speed: bool,
    pub last_flying_ticks: LastInstance,
    pub last_flying_status_change: LastInstance,
    pub last_stuck_speed_multiplier: LastInstance,
    pub last_teleport_ticks: LastInstance,
    pub last_point_three: LastInstance,
    pub stuck_on_edge: LastInstance,
    pub last_stuck_north: LastInstance,
    pub last_stuck_south: LastInstance,
    pub last_stuck_west: LastInstance,
    pub last_stuck_east: LastInstance,
    pub last_hard_colliding_entity: LastInstance,
    pub last_horizontal_offset: f64,
    pub last_vertical_offset: f64,
    /// Entities that may push the player (`collidingEntities`, last 3 ticks).
    pub colliding_entities: [u32; 3],
}

impl Uncertainty {
    /// `UncertaintyHandler.tick`, run before each prediction.
    pub fn tick(&mut self) {
        self.is_step_movement = false;
        self.was_stepping_on_slime = self.is_stepping_on_slime;
        self.was_stepping_on_bouncy = self.is_stepping_on_bouncy;
        self.is_stepping_on_slime = false;
        self.is_stepping_on_bouncy = false;
        self.is_stepping_on_ice = false;
        self.is_stepping_on_honey = false;
        self.is_stepping_near_bubble_column = false;
        self.is_stepping_near_scaffolding = false;
    }
    pub fn tick_instances(&mut self) {
        for li in [
            &mut self.last_flying_ticks,
            &mut self.last_flying_status_change,
            &mut self.last_stuck_speed_multiplier,
            &mut self.last_teleport_ticks,
            &mut self.last_point_three,
            &mut self.stuck_on_edge,
            &mut self.last_stuck_north,
            &mut self.last_stuck_south,
            &mut self.last_stuck_west,
            &mut self.last_stuck_east,
            &mut self.last_hard_colliding_entity,
        ] {
            li.tick();
        }
    }
    pub fn influenced_by_bouncy_block(&self) -> bool {
        self.is_stepping_on_bouncy || self.was_stepping_on_bouncy
    }
    pub fn influenced_by_slime(&self) -> bool {
        self.is_stepping_on_slime || self.was_stepping_on_slime
    }
    pub fn was_affected_by_stuck_speed(&self) -> bool {
        self.last_stuck_speed_multiplier.has_occurred_since(5)
    }
    pub fn max_colliding_entities(&self) -> u32 {
        self.colliding_entities.iter().copied().max().unwrap_or(0)
    }
    pub fn push_colliding_entities(&mut self, n: u32) {
        self.colliding_entities = [self.colliding_entities[1], self.colliding_entities[2], n];
    }
}

/// `PointThreeEstimator` state (with 26.3's 2e-4 movement threshold).
#[derive(Clone, Debug)]
pub struct PointThree {
    pub is_near_fluid: bool,
    pub head_hitter: bool,
    pub is_near_climbable: bool,
    pub is_gliding: bool,
    pub gravity_changed: bool,
    pub near_horizontal_flow: bool,
    pub near_vertical_flow: bool,
    pub near_bubble_column: bool,
    pub max_positive_levitation: i32,
    pub min_negative_levitation: i32,
    pub is_pushing: bool,
    pub was_always_certain: bool,
}

impl Default for PointThree {
    fn default() -> Self {
        Self {
            is_near_fluid: false,
            head_hitter: false,
            is_near_climbable: false,
            is_gliding: false,
            gravity_changed: false,
            near_horizontal_flow: false,
            near_vertical_flow: false,
            near_bubble_column: false,
            max_positive_levitation: i32::MIN,
            min_negative_levitation: i32::MAX,
            is_pushing: false,
            was_always_certain: true,
        }
    }
}

impl PointThree {
    pub fn can_predict_next_vertical_movement(&self) -> bool {
        !self.gravity_changed
            && self.max_positive_levitation == i32::MIN
            && self.min_negative_levitation == i32::MAX
    }
    pub fn positive_levitation(&self, y: f64) -> f64 {
        if self.max_positive_levitation == i32::MIN {
            return y;
        }
        0.05 * f64::from(self.max_positive_levitation + 1) - y * 0.2
    }
    pub fn negative_levitation(&self, y: f64) -> f64 {
        if self.min_negative_levitation == i32::MAX {
            return y;
        }
        0.05 * f64::from(self.min_negative_levitation + 1) - y * 0.2
    }
    pub fn update_levitation(&mut self, level: Option<i32>) {
        self.max_positive_levitation = self.max_positive_levitation.max(level.unwrap_or(i32::MIN));
        self.min_negative_levitation = self.min_negative_levitation.min(level.unwrap_or(i32::MAX));
    }
}

/// `MainSupportingBlockData`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SupportingBlock {
    pub pos: Option<[i32; 3]>,
    pub on_ground: bool,
}

impl SupportingBlock {
    pub fn last_on_ground_and_no_block(&self) -> bool {
        self.pos.is_none() && self.on_ground
    }
}

/// The 26.3 player input (`ServerboundPlayerInputPacket` flags).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KnownInput {
    pub forward: bool,
    pub backward: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub shift: bool,
    pub sprint: bool,
}

impl KnownInput {
    pub fn from_flags(f: u8) -> Self {
        Self {
            forward: f & 1 != 0,
            backward: f & 2 != 0,
            left: f & 4 != 0,
            right: f & 8 != 0,
            jump: f & 16 != 0,
            shift: f & 32 != 0,
            sprint: f & 64 != 0,
        }
    }
    pub fn moving(&self) -> bool {
        self.forward || self.backward || self.left || self.right
    }
}

/// Game modes by network id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GameMode {
    #[default]
    Survival,
    Creative,
    Adventure,
    Spectator,
}

impl GameMode {
    pub fn from_id(id: i32) -> Self {
        match id {
            1 => GameMode::Creative,
            2 => GameMode::Adventure,
            3 => GameMode::Spectator,
            _ => GameMode::Survival,
        }
    }
}

/// The movement simulation's view of one player (`GrimPlayer`'s fields).
#[derive(Clone, Debug)]
pub struct Movement {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub last_x: f64,
    pub last_y: f64,
    pub last_z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub last_yaw: f32,
    pub last_pitch: f32,
    pub on_ground: bool,
    pub last_on_ground: bool,
    pub client_claims_last_on_ground: bool,
    /// Whether the last flying packet claimed ground (`packetPlayerOnGround`).
    pub packet_on_ground: bool,
    pub client_velocity: Vec3,
    pub start_tick_client_vel: Vec3,
    pub predicted: VectorData,
    pub actual_movement: Vec3,
    pub bounding_box: Aabb,
    pub pose: Pose,
    pub last_pose: Pose,
    pub is_sprinting: bool,
    pub last_sprinting: bool,
    pub is_sneaking: bool,
    pub was_sneaking: bool,
    pub is_flying: bool,
    pub was_flying: bool,
    pub is_gliding: bool,
    pub was_gliding: bool,
    pub is_swimming: bool,
    pub was_swimming: bool,
    pub is_climbing: bool,
    pub is_slow_movement: bool,
    pub was_touching_water: bool,
    pub was_was_touching_water: bool,
    pub was_touching_lava: bool,
    pub was_eye_in_water: bool,
    pub water_height: f64,
    pub lava_height: f64,
    pub eye_in_water: bool,
    pub eye_in_lava: bool,
    pub friction: f32,
    pub speed: f64,
    pub gravity: f64,
    pub has_gravity: bool,
    pub fall_distance: f64,
    pub depth_strider_level: f32,
    pub sneaking_speed_multiplier: f32,
    pub horizontal_collision: bool,
    pub vertical_collision: bool,
    pub soft_horizontal_collision: bool,
    pub last_was_climbing: f64,
    pub can_swim_hop: bool,
    pub could_skip_tick: bool,
    pub skipped_tick_in_actual_movement: bool,
    pub stuck_speed: Option<Vec3>,
    pub last_stuck_speed: Option<Vec3>,
    pub supporting: SupportingBlock,
    pub base_tick_addition: Vec3,
    pub base_tick_water_pushing: Vec3,
    pub known_input: KnownInput,
    pub is_jumping: bool,
    pub slowed_by_using_item: bool,
    pub fly_speed: f32,
    pub may_fly: bool,
    pub game_mode: GameMode,
    pub is_dead: bool,
    pub in_vehicle: bool,
    pub in_bed: bool,
    pub attributes: Attributes,
    pub effects: Effects,
    pub uncertainty: Uncertainty,
    pub point_three: PointThree,
    pub likely_kb: Option<VelocityData>,
    pub first_bread_kb: Option<VelocityData>,
    pub likely_explosion: Option<VelocityData>,
    pub first_bread_explosion: Option<VelocityData>,
    /// Whether the last flying packet carried a position.
    pub did_last_movement_include_position: bool,
    pub did_last_last_movement_include_position: bool,
    /// A movement packet arrived since the last `ClientTickEnd`.
    pub did_send_movement_before_tick_end: bool,
    /// Attacks this tick that may slow the player (`minAttackSlow`/`maxAttackSlow`).
    pub min_attack_slow: i32,
    pub max_attack_slow: i32,
    /// `SneakingEstimator.sneakingPotentialHiddenVelocity`.
    pub sneaking_hidden: Aabb,
    /// The chosen input's velocity before the hidden-sneak lenience cut it
    /// to the movement seen, when it did.
    pub hidden_cut_from: Option<Vec3>,
    /// The inputs tried in the last prediction (`storePossibleVelocities`).
    pub last_possible: Vec<VectorData>,
}

impl Default for Movement {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            last_x: 0.0,
            last_y: 0.0,
            last_z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            last_yaw: 0.0,
            last_pitch: 0.0,
            on_ground: false,
            last_on_ground: false,
            client_claims_last_on_ground: false,
            packet_on_ground: false,
            client_velocity: Vec3::ZERO,
            start_tick_client_vel: Vec3::ZERO,
            predicted: VectorData::new(Vec3::ZERO, vt::NORMAL),
            actual_movement: Vec3::ZERO,
            bounding_box: Aabb::from_pos_size(0.0, 0.0, 0.0, 0.6, 1.8),
            pose: Pose::Standing,
            last_pose: Pose::Standing,
            is_sprinting: false,
            last_sprinting: false,
            is_sneaking: false,
            was_sneaking: false,
            is_flying: false,
            was_flying: false,
            is_gliding: false,
            was_gliding: false,
            is_swimming: false,
            was_swimming: false,
            is_climbing: false,
            is_slow_movement: false,
            was_touching_water: false,
            was_was_touching_water: false,
            was_touching_lava: false,
            was_eye_in_water: false,
            water_height: 0.0,
            lava_height: 0.0,
            eye_in_water: false,
            eye_in_lava: false,
            friction: 0.91,
            speed: 0.1,
            gravity: 0.08,
            has_gravity: true,
            fall_distance: 0.0,
            depth_strider_level: 0.0,
            sneaking_speed_multiplier: 0.3,
            horizontal_collision: false,
            vertical_collision: false,
            soft_horizontal_collision: false,
            last_was_climbing: 0.0,
            can_swim_hop: false,
            could_skip_tick: false,
            skipped_tick_in_actual_movement: false,
            stuck_speed: None,
            last_stuck_speed: None,
            supporting: SupportingBlock::default(),
            base_tick_addition: Vec3::ZERO,
            base_tick_water_pushing: Vec3::ZERO,
            known_input: KnownInput::default(),
            is_jumping: false,
            slowed_by_using_item: false,
            fly_speed: 0.05,
            may_fly: false,
            game_mode: GameMode::Survival,
            is_dead: false,
            in_vehicle: false,
            in_bed: false,
            attributes: Attributes::default(),
            effects: Effects::default(),
            uncertainty: Uncertainty::default(),
            point_three: PointThree::default(),
            likely_kb: None,
            first_bread_kb: None,
            likely_explosion: None,
            first_bread_explosion: None,
            did_last_movement_include_position: true,
            did_last_last_movement_include_position: true,
            did_send_movement_before_tick_end: false,
            min_attack_slow: 0,
            max_attack_slow: 0,
            sneaking_hidden: Aabb::default(),
            hidden_cut_from: None,
            last_possible: Vec::new(),
        }
    }
}

/// `getMovementThreshold` for 1.18.2+ clients.
pub const MOVEMENT_THRESHOLD: f64 = 0.0002;
/// How far above a surface a client can stop and stand: Entity.move does
/// not apply a collided movement shorter than sqrt(1e-7).
pub const HOVER_GAP: f64 = 3.1623e-4;

impl Movement {
    pub fn position(&self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }
    pub fn last_position(&self) -> Vec3 {
        Vec3::new(self.last_x, self.last_y, self.last_z)
    }
    pub fn attr(&self, a: Attr) -> f64 {
        self.attributes.get(a)
    }
    pub fn scale(&self) -> f32 {
        self.attr(Attr::Scale) as f32
    }
    /// `getMaxUpStep`.
    pub fn max_up_step(&self) -> f64 {
        f64::from(self.attr(Attr::StepHeight) as f32)
    }
    /// `getEyeHeight`.
    pub fn eye_height(&self) -> f64 {
        self.attr(Attr::Scale) * f64::from(self.pose.eye_height())
    }
    /// `GetBoundingBox.getBoundingBoxFromPosAndSize` with the scale applied.
    pub fn box_at(&self, x: f64, y: f64, z: f64, width: f32, height: f32) -> Aabb {
        let s = self.scale();
        Aabb::from_pos_size(x, y, z, width * s, height * s)
    }
    /// `getPlayerBoundingBox` for the current pose.
    pub fn pose_box_at(&self, x: f64, y: f64, z: f64) -> Aabb {
        self.box_at(x, y, z, self.pose.width(), self.pose.height())
    }
    pub fn is_pushed_by_fluid(&self) -> bool {
        !self.is_flying
    }
    /// `exemptOnGround`.
    pub fn exempt_on_ground(&self) -> bool {
        self.in_vehicle
            || self.uncertainty.is_step_movement
            || self.is_flying
            || self.is_dead
            || self.in_bed
            || self
                .uncertainty
                .last_flying_status_change
                .has_occurred_since(30)
            || self
                .uncertainty
                .last_hard_colliding_entity
                .has_occurred_since(3)
    }
    /// `wouldCollisionResultFlagGroundSpoof`.
    pub fn would_collision_result_flag_ground_spoof(&self, input_y: f64, collision_y: f64) -> bool {
        let vertical_collision = input_y != collision_y;
        let calculated_on_ground = vertical_collision && input_y < 0.0;
        if self.exempt_on_ground() {
            return false;
        }
        if input_y == -crate::math::COLLISION_EPSILON
            && collision_y > -crate::math::COLLISION_EPSILON
            && collision_y <= 0.0
        {
            return false;
        }
        calculated_on_ground != self.on_ground
    }
    /// `getPossibleEyeHeights`: for this pose, the eye heights the client may use.
    pub fn possible_eye_heights(&self) -> [f64; 3] {
        let s = self.attr(Attr::Scale);
        let (stand, sneak, swim) = (1.62 * s, 1.27 * s, 0.4 * s);
        match self.pose {
            Pose::FallFlying | Pose::SpinAttack | Pose::Swimming => [swim, stand, sneak],
            Pose::Crouching => [sneak, stand, swim],
            _ => [stand, sneak, swim],
        }
    }
    /// `getPossibleVelocitiesMinusKnockback`, without swim hop and climbing
    /// extras that need the world (added by the prediction engine).
    pub fn slow_falling(&self) -> bool {
        self.effects.level(Effect::SlowFalling).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribute_modifiers_follow_vanilla_order() {
        let mut a = Attributes::default();
        a.set(
            Attr::MovementSpeed,
            0.1,
            vec![
                Modifier {
                    id: "minecraft:effect.speed".into(),
                    amount: 0.2,
                    op: ModifierOp::AddMultipliedTotal,
                },
                Modifier {
                    id: SPRINT_MODIFIER.into(),
                    amount: 0.3,
                    op: ModifierOp::AddMultipliedTotal,
                },
            ],
        );
        // The sprint modifier is dropped, Speed I multiplies the total by 1.2.
        assert!((a.get(Attr::MovementSpeed) - 0.12).abs() < 1e-12);
    }

    #[test]
    fn last_instance_counts_ticks() {
        let mut l = LastInstance::default();
        assert!(!l.has_occurred_since(5));
        l.reset();
        assert!(l.has_occurred_since(0));
        l.tick();
        assert!(!l.has_occurred_since(0) && l.has_occurred_since(1));
    }
}
