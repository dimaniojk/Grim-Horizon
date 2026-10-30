fn console(game: &mut Game<'_>, line: &str) -> std::ops::ControlFlow<()> {
    let line = line.strip_prefix('/').unwrap_or(line);
    let Some(lines) = horizon_client::lan::hub::grim_horizon::console(line) else {
        return std::ops::ControlFlow::Continue(());
    };
    for l in lines {
        game.console_reply(Text::literal(l));
    }
    std::ops::ControlFlow::Break(())
}
