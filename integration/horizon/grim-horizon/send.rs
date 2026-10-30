fn send(sender: &PlayerSender, body: &Vec<u8>) {
    crate::lan::hub::grim_horizon::observe_send(std::sync::Arc::as_ptr(&sender.pending) as usize, body);
}
