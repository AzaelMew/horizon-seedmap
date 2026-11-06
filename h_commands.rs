fn seedmap_commands(game: &crate::game::Game<'_>, commands: &mut Vec<&'static str>) {
    crate::game::seedmap::commands(game, commands);
}
