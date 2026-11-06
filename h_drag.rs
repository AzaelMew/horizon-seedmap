fn seedmap_drag(game: &crate::game::Game<'_>, button: i32, delta: [f32; 2], canceled: &mut bool) {
    crate::game::seedmap::dragged(game, button, delta, canceled);
}
