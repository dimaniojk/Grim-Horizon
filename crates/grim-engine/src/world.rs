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
    /// What a block state is, wherever it stands: for judging a client that
    /// may still see a state the server has since changed. `None` when the
    /// view cannot tell (its moves near such changes are then not judged).
    fn state_shape(&self, _state: u32) -> Option<StateShape> {
        None
    }
    /// The state at a position, as `state_shape` knows it.
    fn state_at(&self, _x: i32, _y: i32, _z: i32) -> Option<u32> {
        None
    }
}

/// A block state's facts, fluid and collision boxes (relative to its cell).
#[derive(Clone, Debug, PartialEq)]
pub struct StateShape {
    pub facts: BlockFacts,
    pub fluid: FluidFacts,
    pub boxes: Vec<Aabb>,
}

/// A world as one client may see it: `base`, with some cells in other states.
pub struct Overlay<'a> {
    pub base: &'a dyn BlockView,
    pub cells: Vec<([i32; 3], &'a StateShape)>,
}

impl Overlay<'_> {
    fn cell(&self, x: i32, y: i32, z: i32) -> Option<&StateShape> {
        self.cells
            .iter()
            .find(|(p, _)| *p == [x, y, z])
            .map(|(_, s)| *s)
    }
}

impl BlockView for Overlay<'_> {
    fn min_y(&self) -> i32 {
        self.base.min_y()
    }
    fn max_y(&self) -> i32 {
        self.base.max_y()
    }
    fn is_chunk_loaded(&self, cx: i32, cz: i32) -> bool {
        self.base.is_chunk_loaded(cx, cz)
    }
    fn block(&self, x: i32, y: i32, z: i32) -> BlockFacts {
        self.cell(x, y, z)
            .map_or_else(|| self.base.block(x, y, z), |s| s.facts)
    }
    fn fluid(&self, x: i32, y: i32, z: i32) -> FluidFacts {
        self.cell(x, y, z)
            .map_or_else(|| self.base.fluid(x, y, z), |s| s.fluid)
    }
    fn block_collision_boxes(&self, x: i32, y: i32, z: i32, out: &mut Vec<Aabb>) {
        match self.cell(x, y, z) {
            Some(s) => out.extend(
                s.boxes
                    .iter()
                    .map(|b| b.offset(f64::from(x), f64::from(y), f64::from(z))),
            ),
            None => self.base.block_collision_boxes(x, y, z, out),
        }
    }
    fn collision_boxes(&self, query: Aabb, out: &mut Vec<Aabb>) {
        let start = out.len();
        self.base.collision_boxes(query, out);
        // A block's boxes start inside its own cell (fences reach above it).
        let mut i = start;
        while i < out.len() {
            let b = out[i];
            if self
                .cell(floor(b.min_x), floor(b.min_y), floor(b.min_z))
                .is_some()
            {
                out.swap_remove(i);
            } else {
                i += 1;
            }
        }
        let grown = query.inflate(1.0e-7);
        for (p, s) in &self.cells {
            for b in &s.boxes {
                let b = b.offset(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
                if b.intersects(&grown) {
                    out.push(b);
                }
            }
        }
    }
    fn state_shape(&self, state: u32) -> Option<StateShape> {
        self.base.state_shape(state)
    }
    fn state_at(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        self.base.state_at(x, y, z)
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
    /// Nearby positions whose states are known -> what the client may show.
    entries: std::collections::HashMap<[i32; 3], Pending>,
    /// Positions whose state is not kept (far from the player, or not sent)
    /// -> the transaction that confirms them; unknown until then.
    unknown: std::collections::HashMap<[i32; 3], u32>,
    /// Changes past `LIMIT`, or not reported at all, by 16x16x16 section:
    /// every block of the section is unknown until its transaction.
    sections: std::collections::HashMap<[i32; 3], u32>,
    /// Set only when the sections are full too: every block is unknown.
    overflow: Option<u32>,
    /// The state each recently changed position settled in, once confirmed:
    /// what the client shows there when it changes again.
    confirmed: std::collections::HashMap<[i32; 3], u32>,
    /// The next change's place in the send order.
    seq: u64,
    /// Changes that fell back to section and to world granularity.
    pub spilled: u64,
    pub overflowed: u64,
}

/// A state (`None`: not known), the transaction that confirms it and its
/// place in the order the server sent the changes.
type Change = (Option<u32>, u32, u64);
/// The client's own guess at its dig or place: unknown until the server's
/// answer for the cell replaces it.
const GUESS: u32 = u32::MAX;

#[derive(Debug)]
struct Pending {
    /// What the client showed before the first unconfirmed change, if known.
    base: Option<u32>,
    /// Unconfirmed changes before the newest, in order (usually none, so no
    /// allocation): the state (if known) and the transaction that confirms it.
    // Boxed: 8 bytes per entry instead of 24, at thousands of entries.
    #[allow(clippy::box_collection)]
    older: Option<Box<Vec<Change>>>,
    newest: Change,
}

/// What a client may see around a movement.
#[derive(Debug, PartialEq)]
pub enum Around {
    /// Exactly what the server has.
    Known,
    /// The server's world, except these cells, in one of these sets of
    /// states. A client handles block updates in the order they were sent,
    /// so what it shows is the changes up to some transaction: one world per
    /// unanswered transaction, not one per combination of cells.
    Worlds {
        cells: Vec<[i32; 3]>,
        worlds: Vec<Vec<u32>>,
    },
    /// Something the engine cannot know.
    Unknown,
}

impl PendingBlocks {
    /// Hard caps so a flood of block updates cannot grow memory without bound
    /// (about 300 KB per player at most).
    pub const LIMIT: usize = 4096;
    pub const SECTION_LIMIT: usize = 1024;
    /// Cells one movement may depend on, and worlds it is judged in at most.
    pub const CELLS: usize = 64;
    pub const WORLDS: usize = 16;

    /// A change whose new state is not known.
    pub fn add(&mut self, pos: [i32; 3], confirm_by: u32) {
        self.add_change(pos, None, confirm_by);
    }
    /// The player's own dig or place at `pos`, where the server had `before`:
    /// the client shows that or its own guess until the server answers with
    /// the block's state (vanilla sends it right after handling the action);
    /// the answer then stands for the guess (they agree unless the server
    /// refused, and then the answer is `before` again).
    pub fn add_own(&mut self, pos: [i32; 3], before: Option<u32>, confirm_by: u32) {
        if !self.entries.contains_key(&pos)
            && let Some(b) = before
        {
            self.confirmed.insert(pos, b);
        }
        self.add_change(pos, Some(GUESS), confirm_by);
    }
    /// The server set `pos` to `state`. Far from `player` (more than 16
    /// blocks) the state is not kept: no move can reach the cell before the
    /// client confirms it, and after a teleport it counts as unknown.
    pub fn add_state(&mut self, pos: [i32; 3], state: u32, confirm_by: u32, player: [i32; 3]) {
        let near = (0..3).all(|i| (pos[i] - player[i]).abs() <= 16);
        self.add_change(pos, near.then_some(state), confirm_by);
    }
    fn add_change(&mut self, pos: [i32; 3], state: Option<u32>, confirm_by: u32) {
        self.seq += 1;
        let seq = self.seq;
        if let Some(c) = self.unknown.get_mut(&pos) {
            if state.is_none() {
                *c = (*c).max(confirm_by);
                return;
            }
            // A known state again (the player came near): unknown until the
            // unknown change is confirmed, then this one tells.
            let before = (None, *c, 0);
            self.unknown.remove(&pos);
            self.entries.insert(
                pos,
                Pending {
                    base: None,
                    older: Some(Box::new(vec![before])),
                    newest: (state, confirm_by, seq),
                },
            );
            return;
        }
        if state.is_none() {
            // No history to keep: 8 bytes, not a Pending.
            self.entries.remove(&pos);
            if self.entries.len() + self.unknown.len() < Self::LIMIT {
                self.unknown.insert(pos, confirm_by);
            } else {
                self.spilled += 1;
                self.add_unknown_section(pos.map(|c| c >> 4), confirm_by);
            }
            return;
        }
        if let Some(e) = self.entries.get_mut(&pos) {
            if let Some(s) = state
                && s != GUESS
                && e.newest.0 == Some(GUESS)
            {
                // The server's answer to the player's own action.
                e.newest = (Some(s), confirm_by.max(e.newest.1), e.newest.2);
                return;
            }
            // shortcut: a cell changing more than 64 times in a round trip
            // forgets its oldest changes and counts as unknown.
            let older = e.older.get_or_insert_with(Default::default);
            if older.len() >= 64 {
                older.remove(0);
                e.base = None;
            }
            older.push(e.newest);
            e.newest = (state, confirm_by, seq);
        } else if self.entries.len() + self.unknown.len() < Self::LIMIT {
            // What the client shows until it gets this change: the state it
            // settled in last time, when that is known.
            let base = self.confirmed.get(&pos).copied();
            self.entries.insert(
                pos,
                Pending {
                    base,
                    older: None,
                    newest: (state, confirm_by, seq),
                },
            );
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
    /// now known to have reached it. Settled states are remembered near
    /// `player` only (where a block changing again can matter).
    pub fn confirm(&mut self, acked: u32, player: [i32; 3]) {
        let confirmed = &mut self.confirmed;
        self.unknown.retain(|pos, c| {
            if *c > acked {
                return true;
            }
            // Usually empty: no hashing for a crater's thousands of cells.
            if !confirmed.is_empty() {
                confirmed.remove(pos);
            }
            false
        });
        self.entries.retain(|pos, e| {
            // Changes the client has handled: their state is what it shows.
            if let Some(older) = &mut e.older {
                let done = older.iter().take_while(|(_, c, _)| *c <= acked).count();
                if done > 0 {
                    e.base = older[done - 1].0;
                    older.drain(..done);
                }
                if older.is_empty() {
                    e.older = None;
                }
            }
            if e.older.is_some() || e.newest.1 > acked {
                return true;
            }
            let near = (0..3).all(|i| (pos[i] - player[i]).abs() <= 16);
            match e.newest.0 {
                Some(s) if near => {
                    // shortcut: forgets every settled state at 4,096; a clock
                    // then goes unjudged for one round trip.
                    if confirmed.len() >= PendingBlocks::LIMIT {
                        confirmed.clear();
                    }
                    confirmed.insert(*pos, s);
                }
                _ => {
                    confirmed.remove(pos);
                }
            }
            false
        });
        self.sections.retain(|_, c| *c > acked);
        if self.overflow.is_some_and(|o| o <= acked) {
            self.overflow = None;
        }
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
            && self.unknown.is_empty()
            && self.sections.is_empty()
            && self.overflow.is_none()
    }
    /// Positions and sections waiting.
    pub fn len(&self) -> usize {
        self.entries.len() + self.unknown.len() + self.sections.len()
    }
    /// A new level: nothing carries over.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.unknown.clear();
        self.sections.clear();
        self.confirmed.clear();
        self.overflow = None;
    }
    /// Whether any unconfirmed block lies within `area`.
    pub fn touches(&self, area: Aabb) -> bool {
        self.around(area) != Around::Known
    }
    /// What the client may see within `area` (grown by one block so a
    /// changed neighbour's shape counts too).
    pub fn around(&self, area: Aabb) -> Around {
        if self.overflow.is_some() {
            return Around::Unknown;
        }
        let lo = [
            floor(area.min_x) - 1,
            floor(area.min_y) - 1,
            floor(area.min_z) - 1,
        ];
        let hi = [
            floor(area.max_x) + 1,
            floor(area.max_y) + 1,
            floor(area.max_z) + 1,
        ];
        if lookup_or_scan(
            &self.sections,
            lo.map(|c| c >> 4),
            hi.map(|c| c >> 4),
            |_| Some(()),
        )
        .is_some()
        {
            return Around::Unknown;
        }
        if lookup_or_scan(&self.unknown, lo, hi, |_| Some(())).is_some() {
            return Around::Unknown;
        }
        let mut cells: Vec<[i32; 3]> = Vec::new();
        let mut bases: Vec<u32> = Vec::new();
        let mut events: Vec<(u64, u32, usize, u32)> = Vec::new();
        let unknown = lookup_or_scan(&self.entries, lo, hi, |(pos, e): (&[i32; 3], &Pending)| {
            let (Some(base), true) = (e.base, cells.len() < Self::CELLS) else {
                return Some(());
            };
            let i = cells.len();
            cells.push(*pos);
            bases.push(base);
            for &(s, c, seq) in e
                .older
                .iter()
                .flat_map(|o| o.iter())
                .chain(std::iter::once(&e.newest))
            {
                match s {
                    Some(s) if s != GUESS => events.push((seq, c, i, s)),
                    _ => return Some(()),
                }
            }
            None
        });
        if unknown.is_some() {
            return Around::Unknown;
        }
        if cells.is_empty() {
            return Around::Known;
        }
        // In send order; the client answers a transaction only after
        // applying everything sent before it, so worlds change only between
        // transactions (Grim's compensated world has the same granularity).
        events.sort_unstable_by_key(|e| e.0);
        let mut now = bases;
        let mut worlds = vec![now.clone()];
        for (k, &(_, c, i, s)) in events.iter().enumerate() {
            now[i] = s;
            let group_ends = events.get(k + 1).is_none_or(|next| next.1 != c);
            if group_ends && worlds.last() != Some(&now) {
                worlds.push(now.clone());
                if worlds.len() > Self::WORLDS {
                    return Around::Unknown;
                }
            }
        }
        Around::Worlds { cells, worlds }
    }
}

/// Visits the entries of `map` in the box `lo..=hi`, stopping at the first
/// `Some`: looks up the box's cells, or scans the map when it is the smaller
/// (a huge claimed move must not cost more than the map).
fn lookup_or_scan<V, R>(
    map: &std::collections::HashMap<[i32; 3], V>,
    lo: [i32; 3],
    hi: [i32; 3],
    mut visit: impl FnMut((&[i32; 3], &V)) -> Option<R>,
) -> Option<R> {
    if map.is_empty() {
        return None;
    }
    let cells = (0..3)
        .map(|i| i64::from(hi[i]) - i64::from(lo[i]) + 1)
        .product::<i64>();
    if cells <= map.len() as i64 {
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    if let Some(e) = map.get_key_value(&[x, y, z])
                        && let Some(r) = visit(e)
                    {
                        return Some(r);
                    }
                }
            }
        }
        None
    } else {
        map.iter()
            .filter(|(p, _)| (0..3).all(|i| (lo[i]..=hi[i]).contains(&p[i])))
            .find_map(visit)
    }
}

/// A simple in-memory world for tests, benchmarks and replays: full or
/// custom boxes per block, fluids and block facts.
#[derive(Clone, Default)]
pub struct TestWorld {
    pub blocks: std::collections::HashMap<[i32; 3], (BlockFacts, Vec<Aabb>)>,
    pub fluids: std::collections::HashMap<[i32; 3], FluidFacts>,
    pub min_y: i32,
    pub max_y: i32,
    /// Chunks reported as unloaded.
    pub unloaded: std::collections::HashSet<(i32, i32)>,
    /// Block states by id, for `state_shape`, and where `set_state` put them.
    pub states: Vec<StateShape>,
    pub state_ids: std::collections::HashMap<[i32; 3], u32>,
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
    /// Registers a block state; its id for `Clientbound::BlockState`.
    pub fn add_state(&mut self, facts: BlockFacts, boxes: &[Aabb]) -> u32 {
        self.states.push(StateShape {
            facts,
            fluid: FluidFacts::default(),
            boxes: boxes.to_vec(),
        });
        self.states.len() as u32 - 1
    }
    /// Puts a registered state at a position.
    pub fn set_state(&mut self, x: i32, y: i32, z: i32, state: u32) {
        let s = self.states[state as usize].clone();
        self.state_ids.insert([x, y, z], state);
        if s.facts.air {
            self.remove(x, y, z);
        } else {
            self.set_shape(x, y, z, s.facts, &s.boxes);
        }
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
    fn state_shape(&self, state: u32) -> Option<StateShape> {
        self.states.get(state as usize).cloned()
    }
    fn state_at(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        self.state_ids.get(&[x, y, z]).copied()
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
        p.confirm(9, [0, 0, 0]);
        assert_eq!(p.len(), 1);
        p.confirm(10, [0, 0, 0]);
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
        p.confirm(6, [0, 0, 0]);
        assert!(
            p.touches(same_section),
            "the spilled change is still unconfirmed"
        );
        p.confirm(7, [0, 0, 0]);
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
        p.confirm(4, [0, 0, 0]);
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
    fn a_cell_changing_faster_than_it_is_confirmed_shows_every_state() {
        let mut p = PendingBlocks::default();
        let area = Aabb::new(0.2, 64.0, 0.2, 0.8, 65.8, 0.8);
        // First change: what the client showed before is unknown.
        p.add_state([1, 64, 0], 7, 1, [0, 64, 0]);
        assert_eq!(p.around(area), Around::Unknown);
        // A clock: it changes again before each change is confirmed.
        p.add_state([1, 64, 0], 8, 2, [0, 64, 0]);
        p.confirm(1, [0, 64, 0]);
        assert_eq!(
            p.around(area),
            Around::Worlds {
                cells: vec![[1, 64, 0]],
                worlds: vec![vec![7], vec![8]]
            }
        );
        p.add_state([1, 64, 0], 7, 3, [0, 64, 0]);
        p.confirm(2, [0, 64, 0]);
        assert_eq!(
            p.around(area),
            Around::Worlds {
                cells: vec![[1, 64, 0]],
                worlds: vec![vec![8], vec![7]]
            }
        );
        // Settled: remembered, so its next change is known at once.
        p.confirm(3, [0, 64, 0]);
        assert_eq!(p.around(area), Around::Known);
        p.add_state([1, 64, 0], 8, 4, [0, 64, 0]);
        assert_eq!(
            p.around(area),
            Around::Worlds {
                cells: vec![[1, 64, 0]],
                worlds: vec![vec![7], vec![8]]
            }
        );
    }

    #[test]
    fn worlds_follow_the_send_order_not_every_combination() {
        let mut p = PendingBlocks::default();
        let area = Aabb::new(0.2, 64.0, 0.2, 0.8, 65.8, 0.8);
        let row = [[1, 64, -1], [1, 64, 0], [1, 64, 1], [-1, 64, 0]];
        for c in row {
            p.add_state(c, 7, 1, [0, 64, 0]);
        }
        p.confirm(1, [0, 64, 0]);
        // Four cells switch together, twice, before the client answers:
        // 2^4 combinations each time, but only 3 worlds a client can show.
        for (state, tx) in [(8, 2), (7, 3)] {
            for c in row {
                p.add_state(c, state, tx, [0, 64, 0]);
            }
        }
        match p.around(area) {
            Around::Worlds { cells, worlds } => {
                assert_eq!(cells.len(), 4);
                assert_eq!(
                    worlds,
                    vec![vec![7, 7, 7, 7], vec![8, 8, 8, 8], vec![7, 7, 7, 7]]
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn settled_states_are_remembered_only_near_the_player() {
        let mut p = PendingBlocks::default();
        p.add_state([100, 64, 0], 7, 1, [0, 64, 0]);
        p.confirm(1, [0, 64, 0]);
        p.add_state([100, 64, 0], 8, 2, [0, 64, 0]);
        let there = Aabb::new(99.5, 64.0, 0.2, 100.1, 65.8, 0.8);
        assert_eq!(p.around(there), Around::Unknown, "not kept 100 blocks away");
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
        p.confirm(4, [0, 0, 0]);
        assert_eq!(p.len(), 1);
        p.confirm(5, [0, 0, 0]);
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
