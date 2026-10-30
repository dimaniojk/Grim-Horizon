// Grim-Horizon: a Rust port of Grim Anticheat (https://github.com/GrimAnticheat/Grim).
// Copyright (C) GrimAC contributors (original Java), Grim-Horizon contributors (port).
// Licensed under the GNU General Public License, version 3 or (at your option) any later version; see LICENSE.

//! The engine's view of the world. Grim keeps its own client-side copy of
//! every chunk (`CompensatedWorld`); this port reads the server's
//! authoritative world through [`BlockView`] instead and compensates for the
//! difference with [`PendingBlocks`]: every block the server changed and the
//! client may not have received yet makes predictions near it uncertain.

use crate::math::{Aabb, Vec3, floor};

/// Facts about one block state that the movement code needs. The adapter
/// fills these from the server's block registry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockFacts {
    pub air: bool,
    /// `BlockProperties.getMaterialFriction` (`Block.getFriction`).
    pub friction: f32,
    /// `Block.getSpeedFactor` (honey and soul sand 0.4).
    pub speed_factor: f32,
    /// `Block.getJumpFactor` (honey 0.5).
    pub jump_factor: f32,
    /// `BlockProperties.getBlockBounceRestitution` (slime 1, beds 0.75...).
    pub bounce: f32,
    /// `#minecraft:climbable` (ladders, vines, scaffolding...).
    pub climbable: bool,
    pub kind: BlockKind,
    /// `Entity.makeStuckInBlock` multiplier while inside it (cobweb, berry
    /// bush, powder snow).
    pub stuck: Option<Vec3>,
    /// `BlockState.isSuffocating`.
    pub suffocates: bool,
    /// `BlockBehaviour.blocksMotion`: solid for fluid flow (`isSolidBlockingBlacklist`).
    pub blocks_motion: bool,
    /// A bubble column's `drag` property (magma below: pulls down).
    pub drag: bool,
}

impl BlockFacts {
    pub const AIR: BlockFacts = BlockFacts {
        air: true,
        friction: 0.6,
        speed_factor: 1.0,
        jump_factor: 1.0,
        bounce: 0.0,
        climbable: false,
        kind: BlockKind::Air,
        stuck: None,
        suffocates: false,
        blocks_motion: false,
        drag: false,
    };
    pub const SOLID: BlockFacts = BlockFacts {
        air: false,
        kind: BlockKind::Other,
        suffocates: true,
        blocks_motion: true,
        ..BlockFacts::AIR
    };
}

/// Block identities the ported code compares against by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Air,
    Other,
    SlimeBlock,
    HoneyBlock,
    Ice,
    Bed,
    BubbleColumn,
    Scaffolding,
    PowderSnow,
    Cobweb,
    SweetBerryBush,
    SoulSand,
    /// Fences, walls and fence gates (`getOnPos`'s special case).
    FenceLike,
    Water,
    Lava,
    /// Blocks with `#minecraft:suppresses_bounce` (26.2+).
    SuppressesBounce,
}

/// One fluid cell, from `FluidState`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FluidFacts {
    pub kind: FluidKind,
    /// `FluidState.getAmount`: 1 to 8 (8 for sources).
    pub amount: u8,
    pub source: bool,
    /// `FluidState.isFalling` (the `falling` property).
    pub falling: bool,
    /// `FluidState.getHeight(level, pos)`: 1.0 when the same fluid is above,
    /// else `amount / 9`.
    pub height: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FluidKind {
    #[default]
    None,
    Water,
    Lava,
}

/// Read access to the world a player is in. Implementations must be cheap
/// to call repeatedly: the prediction runs collision queries many times per
/// movement packet.
pub trait BlockView {
    fn min_y(&self) -> i32;
    /// Exclusive.
    fn max_y(&self) -> i32;
    fn is_chunk_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool;
    fn block(&self, x: i32, y: i32, z: i32) -> BlockFacts;
    fn fluid(&self, x: i32, y: i32, z: i32) -> FluidFacts;
    /// Every collision box (in world coordinates) of the blocks whose shapes
    /// may reach into `query`, as `Collisions.getCollisionBoxes` gathers
    /// them, plus the world border's walls when the query is near it.
    fn collision_boxes(&self, query: Aabb, out: &mut Vec<Aabb>);
    /// The collision boxes (world coordinates) of the block at one position
    /// only, including parts that reach outside its cell (fences, walls).
    fn block_collision_boxes(&self, x: i32, y: i32, z: i32, out: &mut Vec<Aabb>);
    /// True when `blocks motion` / the block has a full top face; used by
    /// `FluidTypeFlowing` to decide whether a neighbouring solid stops flow.
    fn is_solid_face(&self, x: i32, y: i32, z: i32) -> bool {
        let b = self.block(x, y, z);
        b.blocks_motion && b.suffocates
    }
}

impl dyn BlockView + '_ {
    pub fn block_at(&self, x: f64, y: f64, z: f64) -> BlockFacts {
        self.block(floor(x), floor(y), floor(z))
    }
    pub fn are_chunks_loaded(&self, min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> bool {
        for cx in (min_x >> 4)..=(max_x >> 4) {
            for cz in (min_z >> 4)..=(max_z >> 4) {
                if !self.is_chunk_loaded(cx, cz) {
                    return false;
                }
            }
        }
        true
    }
    pub fn water_height(&self, x: i32, y: i32, z: i32) -> f32 {
        let f = self.fluid(x, y, z);
        if f.kind == FluidKind::Water {
            f.height
        } else {
            0.0
        }
    }
    pub fn lava_height(&self, x: i32, y: i32, z: i32) -> f32 {
        let f = self.fluid(x, y, z);
        if f.kind == FluidKind::Lava {
            f.height
        } else {
            0.0
        }
    }
    pub fn fluid_height(&self, x: i32, y: i32, z: i32) -> f32 {
        self.fluid(x, y, z).height
    }
}

/// Blocks whose client-side state is unknown: the server changed them and
/// the transaction that follows the change has not come back yet, or the
/// player itself predicted a break or place there. Grim keeps exact
/// per-transaction block history in `CompensatedWorld`; without a client
/// world copy we keep the positions and refuse to judge movement near them.
#[derive(Debug, Default)]
pub struct PendingBlocks {
    /// Position -> the transaction that confirms it.
    entries: std::collections::HashMap<[i32; 3], u32>,
    /// Changes past `LIMIT`, or not reported at all, by 16x16x16 section:
    /// every block of the section is unknown until its transaction.
    sections: std::collections::HashMap<[i32; 3], u32>,
    /// Set only when the sections are full too: every block is unknown.
    overflow: Option<u32>,
    /// Changes that fell back to section and to world granularity.
    pub spilled: u64,
    pub overflowed: u64,
}

impl PendingBlocks {
    /// Hard caps so a flood of block updates cannot grow memory without bound
    /// (about 100 KB per player at most).
    pub const LIMIT: usize = 4096;
    pub const SECTION_LIMIT: usize = 1024;

    pub fn add(&mut self, pos: [i32; 3], confirm_by: u32) {
        if let Some(e) = self.entries.get_mut(&pos) {
            *e = (*e).max(confirm_by);
        } else if self.entries.len() < Self::LIMIT {
            self.entries.insert(pos, confirm_by);
        } else {
            // Dropping a change would treat it as confirmed; the section it is
            // in counts as unknown instead.
            self.spilled += 1;
            self.add_unknown_section(pos.map(|c| c >> 4), confirm_by);
        }
    }
    /// Changes in `section` were sent that are not known: all of its blocks
    /// are unconfirmed until `confirm_by`.
    pub fn add_unknown_section(&mut self, section: [i32; 3], confirm_by: u32) {
        if let Some(e) = self.sections.get_mut(&section) {
            *e = (*e).max(confirm_by);
        } else if self.sections.len() < Self::SECTION_LIMIT {
            self.sections.insert(section, confirm_by);
        } else {
            self.overflowed += 1;
            self.overflow = Some(self.overflow.map_or(confirm_by, |o| o.max(confirm_by)));
        }
    }
    /// The client answered transaction `acked`: changes sent before it are
    /// now known to have reached it.
    pub fn confirm(&mut self, acked: u32) {
        self.entries.retain(|_, c| *c > acked);
        self.sections.retain(|_, c| *c > acked);
        if self.overflow.is_some_and(|o| o <= acked) {
            self.overflow = None;
        }
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.sections.is_empty() && self.overflow.is_none()
    }
    /// Positions and sections waiting.
    pub fn len(&self) -> usize {
        self.entries.len() + self.sections.len()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.sections.clear();
        self.overflow = None;
    }
    /// Whether any unconfirmed block lies within `area` (grown by one block
    /// so a changed neighbour's shape counts too).
    pub fn touches(&self, area: Aabb) -> bool {
        if self.overflow.is_some() {
            return true;
        }
        let (x0, y0, z0) = (
            floor(area.min_x) - 1,
            floor(area.min_y) - 1,
            floor(area.min_z) - 1,
        );
        let (x1, y1, z1) = (
            floor(area.max_x) + 1,
            floor(area.max_y) + 1,
            floor(area.max_z) + 1,
        );
        let s0 = [x0 >> 4, y0 >> 4, z0 >> 4];
        let s1 = [x1 >> 4, y1 >> 4, z1 >> 4];
        if lookup_or_scan(&self.sections, s0, s1) {
            return true;
        }
        lookup_or_scan(&self.entries, [x0, y0, z0], [x1, y1, z1])
    }
}

/// Whether `map` has a key in the box `lo..=hi`: looks up the box's cells, or
/// scans the map when it is the smaller (a huge claimed move must not cost
/// more than the map).
fn lookup_or_scan(
    map: &std::collections::HashMap<[i32; 3], u32>,
    lo: [i32; 3],
    hi: [i32; 3],
) -> bool {
    if map.is_empty() {
        return false;
    }
    let cells = (0..3)
        .map(|i| i64::from(hi[i]) - i64::from(lo[i]) + 1)
        .product::<i64>();
    if cells <= map.len() as i64 {
        (lo[0]..=hi[0])
            .any(|x| (lo[1]..=hi[1]).any(|y| (lo[2]..=hi[2]).any(|z| map.contains_key(&[x, y, z]))))
    } else {
        map.keys()
            .any(|p| (0..3).all(|i| (lo[i]..=hi[i]).contains(&p[i])))
    }
}

/// A simple in-memory world for tests, benchmarks and replays: full or
/// custom boxes per block, fluids and block facts.
#[derive(Default)]
pub struct TestWorld {
    pub blocks: std::collections::HashMap<[i32; 3], (BlockFacts, Vec<Aabb>)>,
    pub fluids: std::collections::HashMap<[i32; 3], FluidFacts>,
    pub min_y: i32,
    pub max_y: i32,
    /// Chunks reported as unloaded.
    pub unloaded: std::collections::HashSet<(i32, i32)>,
}

impl TestWorld {
    pub fn new() -> Self {
        Self {
            min_y: -64,
            max_y: 320,
            ..Default::default()
        }
    }
    /// A full cube of `facts` at a position.
    pub fn set_full(&mut self, x: i32, y: i32, z: i32, facts: BlockFacts) {
        let b = Aabb::new(
            f64::from(x),
            f64::from(y),
            f64::from(z),
            f64::from(x + 1),
            f64::from(y + 1),
            f64::from(z + 1),
        );
        self.blocks.insert([x, y, z], (facts, vec![b]));
    }
    pub fn set_shape(&mut self, x: i32, y: i32, z: i32, facts: BlockFacts, local: &[Aabb]) {
        let boxes = local
            .iter()
            .map(|b| b.offset(f64::from(x), f64::from(y), f64::from(z)))
            .collect();
        self.blocks.insert([x, y, z], (facts, boxes));
    }
    /// A block with facts but no collision (water, vines...).
    pub fn set_passable(&mut self, x: i32, y: i32, z: i32, facts: BlockFacts) {
        self.blocks.insert([x, y, z], (facts, Vec::new()));
    }
    pub fn set_fluid(&mut self, x: i32, y: i32, z: i32, fluid: FluidFacts) {
        self.fluids.insert([x, y, z], fluid);
    }
    pub fn remove(&mut self, x: i32, y: i32, z: i32) {
        self.blocks.remove(&[x, y, z]);
        self.fluids.remove(&[x, y, z]);
    }
    /// A flat floor of `facts` with its top at `y + 1`, spanning [x0, x1] x [z0, z1].
    pub fn floor(&mut self, x0: i32, x1: i32, y: i32, z0: i32, z1: i32, facts: BlockFacts) {
        for x in x0..=x1 {
            for z in z0..=z1 {
                self.set_full(x, y, z, facts);
            }
        }
    }
    /// A pool of still water source blocks filling [x0, x1] x [y0, y1] x [z0, z1].
    pub fn water_pool(&mut self, x0: i32, x1: i32, y0: i32, y1: i32, z0: i32, z1: i32) {
        for x in x0..=x1 {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    let facts = BlockFacts {
                        kind: BlockKind::Water,
                        air: false,
                        suffocates: false,
                        ..BlockFacts::AIR
                    };
                    self.set_passable(x, y, z, facts);
                    let height = if y < y1 { 1.0 } else { 8.0 / 9.0 };
                    self.set_fluid(
                        x,
                        y,
                        z,
                        FluidFacts {
                            kind: FluidKind::Water,
                            amount: 8,
                            source: true,
                            falling: false,
                            height,
                        },
                    );
                }
            }
        }
    }
}

impl BlockView for TestWorld {
    fn block_collision_boxes(&self, x: i32, y: i32, z: i32, out: &mut Vec<Aabb>) {
        if let Some((_, boxes)) = self.blocks.get(&[x, y, z]) {
            out.extend(boxes.iter().copied());
        }
    }
    fn min_y(&self) -> i32 {
        self.min_y
    }
    fn max_y(&self) -> i32 {
        self.max_y
    }
    fn is_chunk_loaded(&self, cx: i32, cz: i32) -> bool {
        !self.unloaded.contains(&(cx, cz))
    }
    fn block(&self, x: i32, y: i32, z: i32) -> BlockFacts {
        self.blocks.get(&[x, y, z]).map_or(BlockFacts::AIR, |b| b.0)
    }
    fn fluid(&self, x: i32, y: i32, z: i32) -> FluidFacts {
        self.fluids.get(&[x, y, z]).copied().unwrap_or_default()
    }
    fn collision_boxes(&self, query: Aabb, out: &mut Vec<Aabb>) {
        let (x0, y0, z0) = (
            floor(query.min_x) - 1,
            floor(query.min_y) - 1,
            floor(query.min_z) - 1,
        );
        let (x1, y1, z1) = (
            floor(query.max_x) + 1,
            floor(query.max_y) + 1,
            floor(query.max_z) + 1,
        );
        let volume = i64::from(x1 - x0 + 1) * i64::from(y1 - y0 + 1) * i64::from(z1 - z0 + 1);
        if volume as usize > self.blocks.len() {
            for (p, (_, boxes)) in &self.blocks {
                if (x0..=x1).contains(&p[0])
                    && (y0..=y1).contains(&p[1])
                    && (z0..=z1).contains(&p[2])
                {
                    out.extend(boxes.iter().copied());
                }
            }
        } else {
            for x in x0..=x1 {
                for y in y0..=y1 {
                    for z in z0..=z1 {
                        if let Some((_, boxes)) = self.blocks.get(&[x, y, z]) {
                            out.extend(boxes.iter().copied());
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_blocks_confirm_and_touch() {
        let mut p = PendingBlocks::default();
        p.add([5, 64, 5], 10);
        assert!(p.touches(Aabb::new(4.0, 64.0, 4.0, 5.0, 66.0, 5.0)));
        assert!(!p.touches(Aabb::new(40.0, 64.0, 40.0, 41.0, 66.0, 41.0)));
        p.confirm(9);
        assert_eq!(p.len(), 1);
        p.confirm(10);
        assert!(p.is_empty());
    }

    #[test]
    fn pending_blocks_overflow_is_unknown_until_confirmed() {
        let mut p = PendingBlocks::default();
        for i in 0..PendingBlocks::LIMIT as i32 {
            p.add([i, 0, 0], 3);
        }
        // One change too many: its section is unknown, nothing else.
        p.add([1_000_000, 70, 5], 7);
        assert_eq!(p.spilled, 1);
        let same_section = Aabb::new(1_000_008.5, 66.0, 1.5, 1_000_009.1, 67.8, 2.1);
        let far = Aabb::new(-500.0, 100.0, -500.0, -499.0, 101.0, -499.0);
        assert!(p.touches(same_section));
        assert!(!p.touches(far));
        p.confirm(6);
        assert!(
            p.touches(same_section),
            "the spilled change is still unconfirmed"
        );
        p.confirm(7);
        assert!(!p.touches(same_section) && p.is_empty());
    }

    #[test]
    fn pending_blocks_full_sections_make_everything_unknown_until_confirmed() {
        let mut p = PendingBlocks::default();
        for i in 0..PendingBlocks::SECTION_LIMIT as i32 {
            p.add_unknown_section([i, 0, 0], 2);
        }
        p.add_unknown_section([-50, 0, 0], 4);
        assert_eq!(p.overflowed, 1);
        let far = Aabb::new(-500.0, 100.0, -500.0, -499.0, 101.0, -499.0);
        assert!(p.touches(far));
        p.confirm(4);
        assert!(!p.touches(far) && p.is_empty());
    }

    #[test]
    fn pending_blocks_huge_claimed_moves_cost_no_more_than_the_map() {
        let mut p = PendingBlocks::default();
        p.add_unknown_section([5, 4, 5], 1);
        p.add([3, 64, 3], 1);
        // A claimed move across the world: a scan, not 10^13 lookups.
        let huge = Aabb::new(-3.0e7, -64.0, -3.0e7, 3.0e7, 320.0, 3.0e7);
        let t = std::time::Instant::now();
        assert!(p.touches(huge));
        assert!(t.elapsed().as_millis() < 50);
    }

    #[test]
    fn pending_blocks_lookup_small_and_large_areas() {
        let mut p = PendingBlocks::default();
        p.add([10, 64, 10], 1);
        p.add([10, 64, 10], 5); // the later confirmation wins
        assert!(p.touches(Aabb::new(9.5, 64.0, 9.5, 10.5, 65.8, 10.5)));
        // A huge area (teleport-sized move) takes the map-scan path.
        assert!(p.touches(Aabb::new(-1000.0, -64.0, -1000.0, 1000.0, 320.0, 1000.0)));
        assert!(!p.touches(Aabb::new(30.0, 64.0, 30.0, 31.0, 65.0, 31.0)));
        p.confirm(4);
        assert_eq!(p.len(), 1);
        p.confirm(5);
        assert!(p.is_empty());
    }

    #[test]
    fn test_world_collects_floor() {
        let mut w = TestWorld::new();
        w.floor(-2, 2, 63, -2, 2, BlockFacts::SOLID);
        let mut out = Vec::new();
        w.collision_boxes(Aabb::new(-0.3, 63.5, -0.3, 0.3, 65.3, 0.3), &mut out);
        assert!(!out.is_empty());
    }
}
