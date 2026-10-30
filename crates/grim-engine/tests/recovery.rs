// SPDX-License-Identifier: GPL-3.0-only
// A panic inside the engine must cost only the player whose packets caused
// it, and leave nothing half-updated behind.

mod common;

use common::{Client, Keys, flat_world, stone};
use grim_horizon_engine::engine::{Clientbound, Engine, Serverbound};
use grim_horizon_engine::math::{Aabb, Vec3};
use grim_horizon_engine::world::{BlockFacts, BlockView, FluidFacts, TestWorld};
use grim_horizon_engine::{CheckId, Config};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// A world whose collision query panics: stands in for any engine bug.
struct Broken<'a>(&'a TestWorld);
impl BlockView for Broken<'_> {
    fn min_y(&self) -> i32 {
        self.0.min_y()
    }
    fn max_y(&self) -> i32 {
        self.0.max_y()
    }
    fn is_chunk_loaded(&self, x: i32, z: i32) -> bool {
        self.0.is_chunk_loaded(x, z)
    }
    fn block(&self, x: i32, y: i32, z: i32) -> BlockFacts {
        self.0.block(x, y, z)
    }
    fn fluid(&self, x: i32, y: i32, z: i32) -> FluidFacts {
        self.0.fluid(x, y, z)
    }
    fn collision_boxes(&self, _: Aabb, _: &mut Vec<Aabb>) {
        panic!("injected");
    }
    fn block_collision_boxes(&self, x: i32, y: i32, z: i32, out: &mut Vec<Aabb>) {
        self.0.block_collision_boxes(x, y, z, out);
    }
}

const SPAWN: Vec3 = Vec3::new(0.5, 64.0, 0.5);

fn join(e: &mut Engine, w: &TestWorld, key: u32) -> Client {
    e.add_player(key, u128::from(key), &format!("P{key}"), 0);
    e.on_outgoing(
        key,
        Clientbound::Teleport {
            id: 1,
            pos: SPAWN,
            delta: Vec3::ZERO,
            relatives: 0,
        },
    );
    e.on_incoming(
        key,
        &[Serverbound::AcceptTeleport {
            id: 1,
            pos: SPAWN,
            rot: (0.0, 0.0),
        }],
        w,
        0,
    );
    Client::new(SPAWN)
}

fn flagged(e: &mut Engine) -> usize {
    e.drain_actions()
        .iter()
        .filter(|a| matches!(a, grim_horizon_engine::Action::Log(l) if l.contains(&format!("failed {}", CheckId::Simulation.info().name))))
        .count()
}

#[test]
fn a_panic_costs_only_the_player_whose_packets_caused_it() {
    let world = flat_world(stone());
    let mut e = Engine::new(Config::default());
    let mut a = join(&mut e, &world, 1);
    let mut b = join(&mut e, &world, 2);
    let packets = |c: &mut Client, cheat: bool| {
        c.packets(&world, Keys::forward(), |c| {
            if cheat {
                c.pos.z += 0.3;
            }
        })
    };
    // Player 1's packets hit the bug.
    let p = packets(&mut a, false);
    let caught = catch_unwind(AssertUnwindSafe(|| {
        e.on_incoming(1, &p, &Broken(&world), 0)
    }));
    assert!(caught.is_err());
    // Its state is gone, not half-updated: nothing about it is judged on.
    assert!(e.player(1).is_none());
    let p = packets(&mut a, true);
    assert!(
        e.on_incoming(1, &p, &world, 0)
            .iter()
            .all(|v| *v == grim_horizon_engine::Verdict::Keep)
    );
    // Everyone else is still checked: a cheat beside the failure flags.
    e.drain_actions();
    for t in 0..5 {
        let p = packets(&mut b, true);
        e.on_incoming(2, &p, &world, t);
    }
    assert!(flagged(&mut e) > 0, "player 2 stopped being checked");
    // Player 1 rejoins (the adapter kicked it): fresh state, checked again.
    let mut a = join(&mut e, &world, 1);
    e.drain_actions();
    for t in 0..5 {
        let p = packets(&mut a, true);
        e.on_incoming(1, &p, &world, t);
    }
    assert!(flagged(&mut e) > 0, "the rejoined player is not checked");
}

#[test]
fn repeated_panics_from_one_player_leave_the_engine_usable() {
    let world = flat_world(stone());
    let mut e = Engine::new(Config::default());
    let mut b = join(&mut e, &world, 2);
    for _ in 0..50 {
        let mut a = join(&mut e, &world, 1);
        let p = a.packets(&world, Keys::forward(), |_| {});
        assert!(
            catch_unwind(AssertUnwindSafe(|| e.on_incoming(
                1,
                &p,
                &Broken(&world),
                0
            )))
            .is_err()
        );
        e.remove_player(1);
    }
    assert_eq!(e.players().count(), 1);
    e.drain_actions();
    for t in 0..5 {
        let p = b.packets(&world, Keys::forward(), |c| c.pos.z += 0.3);
        e.on_incoming(2, &p, &world, t);
    }
    assert!(flagged(&mut e) > 0);
}
