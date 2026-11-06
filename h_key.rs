fn seedmap_key(game: &mut crate::game::Game<'_>, key: &str, pressed: bool) {
    crate::game::seedmap::on_key(game, key, pressed);
}
