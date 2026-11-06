fn seedmap_keyup(game: &crate::game::Game<'_>, key: &str) {
    crate::game::seedmap::key_released(game, key);
}
