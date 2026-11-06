fn seedmap_char(game: &crate::game::Game<'_>, c: char, modifiers: u32, canceled: &mut bool) {
    crate::game::seedmap::typed(game, c, modifiers, canceled);
}
