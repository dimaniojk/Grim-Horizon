// SPDX-License-Identifier: GPL-3.0-only
fn send(sender: &PlayerSender, body: &Vec<u8>) {
    crate::lan::hub::grim_horizon::observe_send(&sender.pending, body);
}
