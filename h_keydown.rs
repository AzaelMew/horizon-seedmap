fn seedmap_keydown(game: &crate::game::Game<'_>, key: &str, modifiers: u32, canceled: &mut bool) {
    crate::game::seedmap::key(game, key, modifiers, canceled);
}
