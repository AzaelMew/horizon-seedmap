fn seedmap_release(game: &crate::game::Game<'_>, button: i32, canceled: &mut bool) {
    crate::game::seedmap::released(game, button, canceled);
}
