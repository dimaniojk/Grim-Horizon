// Grim-Horizon: a Rust port of Grim Anticheat (https://github.com/GrimAnticheat/Grim).
// Copyright (C) GrimAC contributors (original Java), Grim-Horizon contributors (port).
// SPDX-License-Identifier: GPL-3.0-only (GNU General Public License, version 3; see LICENSE)

//! Routes packets seen at a network sender to the player they belong to.
//!
//! A server's send path often knows only the sender object, not the player.
//! Its address identifies it while it lives, but a freed address can be
//! reused by the next connection, which would route the new player's join
//! packets (entity id, spawn teleport) to the old player. [`SenderRoutes`]
//! therefore keeps a strong reference to every sender it maps or buffers
//! for: an address cannot be reused while it is known here, and buffers
//! whose sender is gone everywhere else are dropped by [`SenderRoutes::sweep`].

use std::collections::HashMap;
use std::sync::Arc;

use crate::engine::PlayerKey;

/// Packets buffered for one sender before its player is known.
pub const BUFFER_LIMIT: usize = 512;

pub struct SenderRoutes<I: ?Sized, T> {
    bound: HashMap<usize, (PlayerKey, Arc<I>)>,
    unbound: HashMap<usize, (Arc<I>, Vec<T>)>,
}

impl<I: ?Sized, T> Default for SenderRoutes<I, T> {
    fn default() -> Self {
        Self {
            bound: HashMap::new(),
            unbound: HashMap::new(),
        }
    }
}

fn id<I: ?Sized>(sender: &Arc<I>) -> usize {
    Arc::as_ptr(sender) as *const () as usize
}

impl<I: ?Sized, T> SenderRoutes<I, T> {
    /// The player a packet from `sender` belongs to; `None` buffers it until
    /// [`Self::bind`].
    pub fn route(&mut self, sender: &Arc<I>, item: T) -> Option<(PlayerKey, T)> {
        let k = id(sender);
        if let Some((key, _)) = self.bound.get(&k) {
            return Some((*key, item));
        }
        let (_, list) = self
            .unbound
            .entry(k)
            .or_insert_with(|| (sender.clone(), Vec::new()));
        if list.len() < BUFFER_LIMIT {
            list.push(item);
        }
        None
    }

    /// `sender` belongs to `key` from now on; returns what was buffered for it.
    /// A previous sender of `key` is forgotten.
    pub fn bind(&mut self, sender: &Arc<I>, key: PlayerKey) -> Vec<T> {
        self.unbind(key);
        let k = id(sender);
        self.bound.insert(k, (key, sender.clone()));
        self.unbound
            .remove(&k)
            .map(|(_, list)| list)
            .unwrap_or_default()
    }

    /// The player left: its sender may be freed and its address reused.
    pub fn unbind(&mut self, key: PlayerKey) {
        self.bound.retain(|_, (k, _)| *k != key);
    }

    /// Drops buffers of senders nobody else holds any more (a connection
    /// that closed before its player was bound).
    pub fn sweep(&mut self) {
        self.unbound
            .retain(|_, (sender, _)| Arc::strong_count(sender) > 1);
    }

    pub fn bound_len(&self) -> usize {
        self.bound.len()
    }
    pub fn unbound_len(&self) -> usize {
        self.unbound.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    type Routes = SenderRoutes<AtomicUsize, &'static str>;

    #[test]
    fn join_burst_is_buffered_then_delivered() {
        let mut r = Routes::default();
        let a = Arc::new(AtomicUsize::new(0));
        assert!(r.route(&a, "login").is_none());
        assert!(r.route(&a, "spawn teleport").is_none());
        assert_eq!(r.bind(&a, 7), vec!["login", "spawn teleport"]);
        assert_eq!(r.route(&a, "velocity"), Some((7, "velocity")));
    }

    #[test]
    fn a_leaving_players_address_is_never_reused_while_mapped() {
        let mut r = Routes::default();
        let old = Arc::new(AtomicUsize::new(0));
        r.bind(&old, 1);
        let old_addr = id(&old);
        // The old connection closes in the same tick a new one opens: the
        // server drops its sender, but the route still holds it, so the
        // allocator cannot hand its address to the new sender.
        drop(old);
        let mut fresh = Vec::new();
        for _ in 0..64 {
            let s = Arc::new(AtomicUsize::new(0));
            assert_ne!(id(&s), old_addr, "a mapped sender's address was reused");
            fresh.push(s);
        }
        let new = &fresh[0];
        assert!(
            r.route(new, "spawn teleport").is_none(),
            "must not reach player 1"
        );
        assert_eq!(r.bind(new, 2), vec!["spawn teleport"]);
        // Player 1 leaves; its route and the last reference go away.
        r.unbind(1);
        assert_eq!(r.bound_len(), 1);
    }

    #[test]
    fn closed_connections_buffers_are_swept() {
        let mut r = Routes::default();
        let gone = Arc::new(AtomicUsize::new(0));
        let alive = Arc::new(AtomicUsize::new(0));
        r.route(&gone, "login");
        r.route(&alive, "login");
        drop(gone);
        r.sweep();
        assert_eq!(r.unbound_len(), 1);
        assert_eq!(r.bind(&alive, 3), vec!["login"]);
    }

    #[test]
    fn rebinding_a_player_forgets_its_old_sender() {
        let mut r = Routes::default();
        let a = Arc::new(AtomicUsize::new(0));
        let b = Arc::new(AtomicUsize::new(0));
        r.bind(&a, 5);
        r.bind(&b, 5);
        assert_eq!(r.bound_len(), 1);
        assert!(r.route(&a, "stale").is_none());
    }

    #[test]
    fn buffers_are_bounded() {
        let mut r = Routes::default();
        let a = Arc::new(AtomicUsize::new(0));
        for _ in 0..BUFFER_LIMIT + 10 {
            r.route(&a, "x");
        }
        assert_eq!(r.bind(&a, 1).len(), BUFFER_LIMIT);
    }
}
