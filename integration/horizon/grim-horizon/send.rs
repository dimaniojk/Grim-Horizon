fn send(sender: &PlayerSender, body: &Vec<u8>) {
    crate::lan::hub::grim_horizon::observe_send(&sender.pending, body);
}
