// The model compiled standalone (the same source the game gets as `seedmap_model`).
#![allow(dead_code)]
mod model {
    include!("../../model.rs");
}
#[cfg(test)]
mod tests {
    use super::model::*;

    #[test]
    fn seeds_parse_like_vanilla() {
        assert_eq!(parse_seed("12345"), Some(12345));
        assert_eq!(parse_seed(" -77 "), Some(-77));
        assert_eq!(parse_seed(""), None);
        // Java: "hello".hashCode() == 99162322, "Minecraft".hashCode() == -1170105035
        assert_eq!(parse_seed("hello"), Some(99162322));
        assert_eq!(parse_seed("Minecraft"), Some(-1595926131));
        assert_eq!(parse_seed("9223372036854775807"), Some(i64::MAX));
        // Too big for a long: hashed, as Long.parseLong throws.
        assert!(parse_seed("9223372036854775808").is_some_and(|v| v.abs() < 1 << 32));
    }

    #[test]
    fn view_round_trips_and_zoom_keeps_the_cursor_block() {
        let map = [0.0, 24.0, 800.0, 400.0];
        let mut v = View { cx: 1234.5, cz: -987.0, level: 3 };
        let p = v.to_screen(map, 100.0, 200.0);
        let w = v.to_world(map, p[0], p[1]);
        assert!((w[0] - 100.0).abs() < 0.01 && (w[1] - 200.0).abs() < 0.01);
        let at = [300.0, 150.0];
        let before = v.to_world(map, at[0], at[1]);
        v.zoom_at(map, at, 1);
        assert_eq!(v.level, 2);
        let after = v.to_world(map, at[0], at[1]);
        assert!((before[0] - after[0]).abs() < 0.01 && (before[1] - after[1]).abs() < 0.01);
        v.level = 0;
        v.zoom_at(map, at, 1);
        assert_eq!(v.level, 0);
        v.level = MAX_LEVEL;
        v.zoom_at(map, at, -1);
        assert_eq!(v.level, MAX_LEVEL);
    }

    #[test]
    fn tiles_cover_the_map_nearest_first() {
        let map = [0.0, 24.0, 800.0, 400.0];
        let v = View { cx: -5000.0, cz: 3000.0, level: 2 };
        let tiles = v.tiles(map);
        let (a, b) = v.cells(map);
        for cz in a[1]..=b[1] {
            for cx in a[0]..=b[0] {
                assert!(tiles.contains(&(floor_div(cx, TILE), floor_div(cz, TILE))));
            }
        }
        let centre = v.cells(map);
        let mid = [(centre.0[0] + centre.1[0]) / 2, (centre.0[1] + centre.1[1]) / 2];
        let want = (floor_div(mid[0], TILE), floor_div(mid[1], TILE));
        assert!((tiles[0].0 - want.0).abs() <= 1 && (tiles[0].1 - want.1).abs() <= 1);
    }

    #[test]
    fn regions_only_when_zoomed_in() {
        let map = [0.0, 24.0, 800.0, 400.0];
        assert!(!View { cx: 0.0, cz: 0.0, level: 2 }.regions(map).is_empty());
        assert!(View { cx: 0.0, cz: 0.0, level: STRUCT_MAX_LEVEL + 1 }.regions(map).is_empty());
    }

    #[test]
    fn categories_match_registry_names() {
        assert_eq!(CATS[category("minecraft:village_plains")].key, "village");
        assert_eq!(CATS[category("minecraft:abandoned_camp_taiga")].key, "abandoned_camp");
        assert_eq!(CATS[category("minecraft:ruined_portal_nether")].key, "ruined_portal");
        assert_eq!(CATS[category("minecraft:mineshaft_mesa")].key, "mineshaft");
        assert_eq!(CATS[category("minecraft:fortress")].key, "fortress");
        assert_eq!(CATS[category("minecraft:ocean_ruin_cold")].key, "ocean_ruin");
        assert_eq!(CATS[category("minecraft:stronghold")].key, "stronghold");
        assert_eq!(category("somepack:thing"), OTHER);
        assert_eq!(CATS[SPAWN].key, "spawn");
        assert_eq!(CATS[STRONGHOLD].key, "stronghold");
        assert_eq!(CATS[OTHER].key, "other");
        assert_eq!(CATS[END_CITY].key, "end_city");
        assert_eq!(CATS[END_SHIP].key, "end_ship");
        assert!(CATS.iter().all(|c| c.key == "other" || !c.item.is_empty()));
    }

    #[test]
    fn cache_asks_for_missing_work_tiles_first() {
        let mut c = Cache::default();
        let req = Request { seed: 7, dim: 0, level: 2, tiles: vec![(0, 0), (1, 0)], regions: vec![(0, 0)] };
        assert_eq!(c.next_job(&req), Some(Job::Tile(Cache::tile_key(&req, 0, 0))));
        c.tiles.insert(Cache::tile_key(&req, 0, 0), vec![0; 4096]);
        assert_eq!(c.next_job(&req), Some(Job::Tile(Cache::tile_key(&req, 1, 0))));
        c.tiles.insert(Cache::tile_key(&req, 1, 0), vec![0; 4096]);
        assert_eq!(c.next_job(&req), Some(Job::Spawn(7)));
        c.spawn.insert(7, [0, 0]);
        assert_eq!(c.next_job(&req), Some(Job::Strongholds(7)));
        c.strongholds.insert(7, vec![]);
        assert!(matches!(c.next_job(&req), Some(Job::Region(_))));
        c.regions.insert(RegionKey { seed: 7, dim: 0, rx: 0, rz: 0 }, vec![]);
        assert_eq!(c.next_job(&req), None);
        assert_eq!(c.missing(&req), 0);
        // The nether has no spawn or strongholds to look up.
        let nether = Request { dim: 1, ..req.clone() };
        assert!(matches!(c.next_job(&nether), Some(Job::Tile(_))));
    }

    #[test]
    fn biome_lookup_wraps_negative_cells() {
        let mut c = Cache::default();
        let req = Request { seed: 1, dim: 0, level: 1, tiles: vec![], regions: vec![] };
        let mut t = vec![UNKNOWN; 4096];
        t[(63 * TILE + 63) as usize] = 9; // cell (-1, -1) is the last of tile (-1, -1)
        c.tiles.insert(Cache::tile_key(&req, -1, -1), t);
        assert_eq!(c.biome_at(&req, [-1, -1]), 9);
        assert_eq!(c.biome_at(&req, [0, 0]), UNKNOWN);
    }

    #[test]
    fn names_are_interned_with_colours() {
        let mut c = Cache::default();
        let a = c.intern("minecraft:plains");
        let b = c.intern("minecraft:desert");
        assert_eq!((a, b), (0, 1));
        assert_eq!(c.intern("minecraft:plains"), 0);
        assert_eq!(c.colors[0], 0x8DB360);
        assert_eq!(pretty("minecraft:old_growth_pine_taiga"), "Old growth pine taiga");
    }

    #[test]
    fn the_field_shows_the_tail_that_fits() {
        let w = |s: &str| s.chars().count() as f32 * 6.0;
        assert_eq!(tail_fit("1234567890", 30.0, &w), "67890");
        assert_eq!(tail_fit("12", 30.0, &w), "12");
    }

    #[test]
    fn scale_bars_are_round_and_about_the_target_width() {
        for bpp in [0.5, 2.0, 4.0, 16.0, 96.0, 2048.0] {
            let (n, px) = scale_bar(bpp, 70.0);
            assert!((35.0..=140.0).contains(&px), "{bpp}: {n} blocks is {px}px");
            let lead = n.to_string();
            assert!(lead.starts_with('1') || lead.starts_with('2') || lead.starts_with('5'));
            assert!(lead[1..].chars().all(|c| c == '0'));
        }
        assert_eq!(scale_bar(4.0, 70.0).0, 200);
        assert_eq!(blocks_label(500), "500 blocks");
        assert_eq!(blocks_label(20000), "20k blocks");
    }
}
