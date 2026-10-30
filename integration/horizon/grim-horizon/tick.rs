// SPDX-License-Identifier: GPL-3.0-only
fn tick(game: &mut Game<'_>) {
    if !horizon_client::lan::hub::grim_horizon::sizes_registered() {
        let sizes: Vec<(String, f32, f32)> = game
            .res
            .mob_types()
            .iter()
            .map(|(id, t)| (id.clone(), t.width as f32, t.height as f32))
            .collect();
        horizon_client::lan::hub::grim_horizon::register_sizes(&sizes);
    }
    let dedicated = game.options.dedicated.is_some();
    for line in horizon_client::lan::hub::grim_horizon::take_log() {
        if dedicated {
            super::dedicated::log("INFO", &line);
        } else {
            log::info!("{line}");
        }
    }
}
