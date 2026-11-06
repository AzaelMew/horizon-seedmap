// Seed Map: the pure model (no game types): seed parsing, view maths, biome colours,
// structure categories and the tile/region cache the generator thread fills.
// Tested standalone (see devtests/), then compiled into the game as `seedmap_model`.
use std::collections::HashMap;

/// GUI pixels per map cell.
pub const CELL: f32 = 2.0;
/// Cells along one side of a cached tile.
pub const TILE: i32 = 64;
/// Zoom levels: a cell is `4 << level` blocks wide (level 0 = one biome quart).
pub const MAX_LEVEL: i32 = 10;
/// Structures are only looked up at this level and closer (the work grows with the area).
pub const STRUCT_MAX_LEVEL: i32 = 3;
/// Chunks along one side of a structure region.
pub const REGION: i32 = 32;
/// Beyond this a view is clamped (the world border).
pub const LIMIT: f64 = 30_000_000.0;
/// Biome id of a cell that is not generated (yet).
pub const UNKNOWN: u8 = 255;

pub fn blocks_per_cell(level: i32) -> f64 {
    (4i64 << level.clamp(0, MAX_LEVEL)) as f64
}

pub fn blocks_per_px(level: i32) -> f64 {
    blocks_per_cell(level) / f64::from(CELL)
}

/// What vanilla's seed box does: a number is the seed, any other text is its Java
/// `String.hashCode`. Nothing typed is no seed.
pub fn parse_seed(text: &str) -> Option<i64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(n) = t.parse::<i64>() {
        return Some(n);
    }
    let mut h: i32 = 0;
    for unit in t.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(i32::from(unit));
    }
    Some(i64::from(h))
}

pub const DIMS: [(&str, &str); 3] = [
    ("minecraft:overworld", "Overworld"),
    ("minecraft:the_nether", "Nether"),
    ("minecraft:the_end", "End"),
];

pub fn dim_index(id: &str) -> Option<usize> {
    DIMS.iter().position(|d| d.0 == id)
}

pub fn floor_div(a: i32, b: i32) -> i32 {
    a.div_euclid(b)
}

// ---- structure categories ---------------------------------------------------------------------

pub struct Cat {
    pub key: &'static str,
    pub label: &'static str,
    pub letter: &'static str,
    /// The game's own item whose icon stands for the category ("" for none).
    pub item: &'static str,
    pub color: u32,
    /// Bit 0 overworld, bit 1 nether, bit 2 end.
    pub dims: u8,
    pub default_on: bool,
}

const fn cat(
    key: &'static str,
    label: &'static str,
    letter: &'static str,
    item: &'static str,
    color: u32,
    dims: u8,
    default_on: bool,
) -> Cat {
    Cat { key, label, letter, item, color, dims, default_on }
}

pub const SPAWN: usize = 0;
pub const STRONGHOLD: usize = 15;
pub const END_CITY: usize = 22;
pub const END_SHIP: usize = 23;
pub const OTHER: usize = 24;

pub const CATS: [Cat; 25] = [
    cat("spawn", "World spawn", "S", "minecraft:compass", 0xFFFFFF, 1, true),
    cat("village", "Village", "V", "minecraft:bell", 0x4CAF50, 1, true),
    cat("pillager_outpost", "Pillager outpost", "O", "minecraft:crossbow", 0xC62828, 1, true),
    cat("mansion", "Woodland mansion", "M", "minecraft:totem_of_undying", 0x8D6E63, 1, true),
    cat("monument", "Ocean monument", "N", "minecraft:prismarine_bricks", 0x29B6F6, 1, true),
    cat("desert_pyramid", "Desert temple", "D", "minecraft:chiseled_sandstone", 0xFBC02D, 1, true),
    cat("jungle_pyramid", "Jungle temple", "J", "minecraft:mossy_cobblestone", 0x2E7D32, 1, true),
    cat("swamp_hut", "Witch hut", "W", "minecraft:cauldron", 0x8E24AA, 1, true),
    cat("igloo", "Igloo", "I", "minecraft:snow_block", 0xB2EBF2, 1, true),
    cat("ruined_portal", "Ruined portal", "R", "minecraft:crying_obsidian", 0xAB47BC, 3, true),
    cat("shipwreck", "Shipwreck", "K", "minecraft:oak_boat", 0x6D4C41, 1, true),
    cat("ocean_ruin", "Ocean ruin", "U", "minecraft:sea_lantern", 0x26A69A, 1, true),
    cat("buried_treasure", "Buried treasure", "X", "minecraft:heart_of_the_sea", 0xFFD54F, 1, true),
    cat("ancient_city", "Ancient city", "A", "minecraft:sculk_shrieker", 0x37474F, 1, true),
    cat("trial_chambers", "Trial chambers", "T", "minecraft:trial_key", 0xFF8A65, 1, true),
    cat("stronghold", "Stronghold", "+", "minecraft:ender_eye", 0xEEEEEE, 1, true),
    cat("mineshaft", "Mineshaft", "m", "minecraft:rail", 0x9E9E9E, 1, false),
    cat("abandoned_camp", "Abandoned camp", "C", "minecraft:campfire", 0xA1887F, 1, true),
    cat("trail_ruins", "Trail ruins", "Y", "minecraft:brush", 0xBCAAA4, 1, true),
    cat("fortress", "Nether fortress", "F", "minecraft:nether_bricks", 0xD84315, 2, true),
    cat("bastion", "Bastion remnant", "B", "minecraft:gilded_blackstone", 0x5D4037, 2, true),
    cat("nether_fossil", "Nether fossil", "L", "minecraft:bone_block", 0xE0E0E0, 2, false),
    cat("end_city", "End city (no ship)", "E", "minecraft:purpur_block", 0xCE93D8, 4, true),
    cat("end_ship", "End ship (elytra)", "e", "minecraft:elytra", 0xFFC107, 4, true),
    cat("other", "Other", "?", "", 0x90A4AE, 7, false),
];

/// The category of a structure by its registry name (`minecraft:village_plains`).
pub fn category(name: &str) -> usize {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    const PREFIXES: [(&str, usize); 22] = [
        ("village", 1),
        ("pillager_outpost", 2),
        ("mansion", 3),
        ("monument", 4),
        ("desert_pyramid", 5),
        ("jungle_pyramid", 6),
        ("swamp_hut", 7),
        ("igloo", 8),
        ("ruined_portal", 9),
        ("shipwreck", 10),
        ("ocean_ruin", 11),
        ("buried_treasure", 12),
        ("ancient_city", 13),
        ("trial_chambers", 14),
        ("stronghold", STRONGHOLD),
        ("mineshaft", 16),
        ("abandoned_camp", 17),
        ("trail_ruins", 18),
        ("fortress", 19),
        ("bastion", 20),
        ("nether_fossil", 21),
        ("end_city", END_CITY),
    ];
    PREFIXES
        .iter()
        .find(|(p, _)| n.starts_with(p))
        .map_or(OTHER, |&(_, i)| i)
}

/// Biomes that only exist underground; the map asks for the surface above them.
pub fn is_cave_biome(name: &str) -> bool {
    matches!(
        name.strip_prefix("minecraft:").unwrap_or(name),
        "lush_caves" | "dripstone_caves" | "deep_dark"
    )
}

pub fn pretty(name: &str) -> String {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    let mut out = String::with_capacity(n.len());
    let mut up = true;
    for c in n.chars() {
        if c == '_' {
            out.push(' ');
            up = false;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

// ---- biome colours ----------------------------------------------------------------------------

pub fn biome_color(name: &str) -> u32 {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    match n {
        "ocean" => 0x000070,
        "deep_ocean" => 0x000030,
        "frozen_ocean" => 0x7070D6,
        "deep_frozen_ocean" => 0x404090,
        "cold_ocean" => 0x202070,
        "deep_cold_ocean" => 0x202038,
        "lukewarm_ocean" => 0x0000AC,
        "deep_lukewarm_ocean" => 0x000064,
        "warm_ocean" => 0x0000FF,
        "river" => 0x0000FF,
        "frozen_river" => 0xA0A0FF,
        "beach" => 0xFADE55,
        "snowy_beach" => 0xFAF0C0,
        "stony_shore" => 0xA2A284,
        "plains" => 0x8DB360,
        "sunflower_plains" => 0xB5DB88,
        "snowy_plains" => 0xFFFFFF,
        "ice_spikes" => 0xB4DCDC,
        "desert" => 0xFA9418,
        "swamp" => 0x07F9B2,
        "mangrove_swamp" => 0x67C28B,
        "forest" => 0x056621,
        "flower_forest" => 0x2D8E49,
        "birch_forest" => 0x307444,
        "old_growth_birch_forest" => 0x589C6C,
        "dark_forest" => 0x40511A,
        "pale_garden" => 0x9AA79A,
        "taiga" => 0x0B6659,
        "old_growth_pine_taiga" => 0x596651,
        "old_growth_spruce_taiga" => 0x818E79,
        "snowy_taiga" => 0x31554A,
        "savanna" => 0xBDB25F,
        "savanna_plateau" => 0xA79D64,
        "windswept_savanna" => 0xE5DA87,
        "jungle" => 0x537B09,
        "sparse_jungle" => 0x628B17,
        "bamboo_jungle" => 0x768E14,
        "badlands" => 0xD94515,
        "eroded_badlands" => 0xFF6D3D,
        "wooded_badlands" => 0xB09765,
        "meadow" => 0x83BB6D,
        "cherry_grove" => 0xF4B4D0,
        "grove" => 0xA0C8B4,
        "snowy_slopes" => 0xE8F4F4,
        "frozen_peaks" => 0xB4C8D8,
        "jagged_peaks" => 0xDCDCEB,
        "stony_peaks" => 0x8A8A8A,
        "windswept_hills" => 0x606060,
        "windswept_gravelly_hills" => 0x888888,
        "windswept_forest" => 0x507050,
        "mushroom_fields" => 0xFF00FF,
        "dripstone_caves" => 0x7B5E44,
        "lush_caves" => 0x7ACF3E,
        "deep_dark" => 0x0A2A2A,
        "the_void" => 0x000000,
        "nether_wastes" => 0xBF3B3B,
        "soul_sand_valley" => 0x5E3830,
        "crimson_forest" => 0xDD0808,
        "warped_forest" => 0x49907B,
        "basalt_deltas" => 0x403636,
        "the_end" => 0x8080FF,
        "end_highlands" => 0xCBA0F0,
        "end_midlands" => 0xB080D0,
        "small_end_islands" => 0x8070C0,
        "end_barrens" => 0x7060B0,
        other => {
            // A stable colour for a biome this table doesn't know (a data pack's).
            let mut h: u32 = 2166136261;
            for b in other.bytes() {
                h = (h ^ u32::from(b)).wrapping_mul(16777619);
            }
            0x404040 | (h & 0xBFBFBF)
        }
    }
}

// ---- the view -----------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub cx: f64,
    pub cz: f64,
    pub level: i32,
}

impl View {
    pub const fn new() -> View {
        View { cx: 0.0, cz: 0.0, level: 1 }
    }
    pub fn bpc(&self) -> f64 {
        blocks_per_cell(self.level)
    }
    pub fn bpp(&self) -> f64 {
        blocks_per_px(self.level)
    }
    pub fn to_screen(&self, map: [f32; 4], wx: f64, wz: f64) -> [f32; 2] {
        let bpp = self.bpp();
        [
            map[0] + map[2] / 2.0 + ((wx - self.cx) / bpp) as f32,
            map[1] + map[3] / 2.0 + ((wz - self.cz) / bpp) as f32,
        ]
    }
    pub fn to_world(&self, map: [f32; 4], sx: f32, sy: f32) -> [f64; 2] {
        let bpp = self.bpp();
        [
            self.cx + f64::from(sx - map[0] - map[2] / 2.0) * bpp,
            self.cz + f64::from(sy - map[1] - map[3] / 2.0) * bpp,
        ]
    }
    pub fn clamp(&mut self) {
        self.cx = self.cx.clamp(-LIMIT, LIMIT);
        self.cz = self.cz.clamp(-LIMIT, LIMIT);
        self.level = self.level.clamp(0, MAX_LEVEL);
    }
    /// Drag by `dx`, `dy` GUI pixels: the map follows the cursor.
    pub fn pan_px(&mut self, dx: f32, dy: f32) {
        let bpp = self.bpp();
        self.cx -= f64::from(dx) * bpp;
        self.cz -= f64::from(dy) * bpp;
        self.clamp();
    }
    pub fn center_on(&mut self, x: f64, z: f64) {
        self.cx = x;
        self.cz = z;
        self.clamp();
    }
    /// `dir` > 0 zooms in, < 0 out, keeping the block under `at` where it is.
    pub fn zoom_at(&mut self, map: [f32; 4], at: [f32; 2], dir: i32) {
        let level = (self.level - dir.signum()).clamp(0, MAX_LEVEL);
        if level == self.level {
            return;
        }
        let under = self.to_world(map, at[0], at[1]);
        self.level = level;
        let bpp = self.bpp();
        self.cx = under[0] - f64::from(at[0] - map[0] - map[2] / 2.0) * bpp;
        self.cz = under[1] - f64::from(at[1] - map[1] - map[3] / 2.0) * bpp;
        self.clamp();
    }
    /// The cell range the map rectangle shows (inclusive).
    pub fn cells(&self, map: [f32; 4]) -> ([i32; 2], [i32; 2]) {
        let bpc = self.bpc();
        let bpp = self.bpp();
        let (hw, hh) = (f64::from(map[2]) / 2.0 * bpp, f64::from(map[3]) / 2.0 * bpp);
        let f = |v: f64| (v / bpc).floor() as i32;
        ([f(self.cx - hw), f(self.cz - hh)], [f(self.cx + hw), f(self.cz + hh)])
    }
    /// The tiles the map shows, nearest the centre first.
    pub fn tiles(&self, map: [f32; 4]) -> Vec<(i32, i32)> {
        let (a, b) = self.cells(map);
        let (tx0, tz0) = (floor_div(a[0], TILE), floor_div(a[1], TILE));
        let (tx1, tz1) = (floor_div(b[0], TILE), floor_div(b[1], TILE));
        let mid = |lo: i32, hi: i32| f64::from(lo + hi) / 2.0;
        let (mx, mz) = (mid(tx0, tx1), mid(tz0, tz1));
        let mut out = Vec::new();
        for tz in tz0..=tz1 {
            for tx in tx0..=tx1 {
                out.push((tx, tz));
            }
        }
        out.sort_by(|p, q| {
            let d = |t: &(i32, i32)| (f64::from(t.0) - mx).powi(2) + (f64::from(t.1) - mz).powi(2);
            d(p).total_cmp(&d(q))
        });
        out
    }
    /// The structure regions the map shows, nearest first; none when zoomed too far out.
    pub fn regions(&self, map: [f32; 4]) -> Vec<(i32, i32)> {
        if self.level > STRUCT_MAX_LEVEL {
            return Vec::new();
        }
        let bpp = self.bpp();
        let (hw, hh) = (f64::from(map[2]) / 2.0 * bpp, f64::from(map[3]) / 2.0 * bpp);
        let span = f64::from(REGION * 16);
        let f = |v: f64| (v / span).floor() as i32;
        let (rx0, rx1) = (f(self.cx - hw), f(self.cx + hw));
        let (rz0, rz1) = (f(self.cz - hh), f(self.cz + hh));
        let (mx, mz) = (f64::from(rx0 + rx1) / 2.0, f64::from(rz0 + rz1) / 2.0);
        let mut out = Vec::new();
        for rz in rz0..=rz1 {
            for rx in rx0..=rx1 {
                out.push((rx, rz));
            }
        }
        out.sort_by(|p, q| {
            let d = |t: &(i32, i32)| (f64::from(t.0) - mx).powi(2) + (f64::from(t.1) - mz).powi(2);
            d(p).total_cmp(&d(q))
        });
        out.truncate(160);
        out
    }
}

// ---- the cache ------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub seed: i64,
    pub dim: u8,
    pub level: u8,
    pub tx: i32,
    pub tz: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RegionKey {
    pub seed: i64,
    pub dim: u8,
    pub rx: i32,
    pub rz: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub cat: u8,
    pub x: i32,
    pub z: i32,
    pub name: String,
}

/// What the map wants generated right now.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Request {
    pub seed: i64,
    pub dim: u8,
    pub level: u8,
    pub tiles: Vec<(i32, i32)>,
    pub regions: Vec<(i32, i32)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Job {
    Tile(TileKey),
    Region(RegionKey),
    Strongholds(i64),
    Spawn(i64),
}

pub const MAX_TILES: usize = 4000;

#[derive(Default)]
pub struct Cache {
    pub tiles: HashMap<TileKey, Vec<u8>>,
    pub regions: HashMap<RegionKey, Vec<Marker>>,
    pub names: Vec<String>,
    pub colors: Vec<u32>,
    pub strongholds: HashMap<i64, Vec<[i32; 2]>>,
    pub spawn: HashMap<i64, [i32; 2]>,
    pub failed: Option<String>,
}

impl Cache {
    /// The id of a biome name, adding it. Ids stay below `UNKNOWN`.
    pub fn intern(&mut self, name: &str) -> u8 {
        if let Some(i) = self.names.iter().position(|n| n == name) {
            return i as u8;
        }
        if self.names.len() >= usize::from(UNKNOWN) {
            return UNKNOWN;
        }
        self.names.push(name.to_owned());
        self.colors.push(biome_color(name));
        (self.names.len() - 1) as u8
    }

    pub fn tile_key(req: &Request, tx: i32, tz: i32) -> TileKey {
        TileKey { seed: req.seed, dim: req.dim, level: req.level, tx, tz }
    }

    /// The first thing the request needs that isn't generated, tiles before structures.
    pub fn next_job(&self, req: &Request) -> Option<Job> {
        for &(tx, tz) in &req.tiles {
            let key = Self::tile_key(req, tx, tz);
            if !self.tiles.contains_key(&key) {
                return Some(Job::Tile(key));
            }
        }
        if req.dim == 0 && !self.spawn.contains_key(&req.seed) {
            return Some(Job::Spawn(req.seed));
        }
        if req.dim == 0 && !self.strongholds.contains_key(&req.seed) {
            return Some(Job::Strongholds(req.seed));
        }
        for &(rx, rz) in &req.regions {
            let key = RegionKey { seed: req.seed, dim: req.dim, rx, rz };
            if !self.regions.contains_key(&key) {
                return Some(Job::Region(key));
            }
        }
        None
    }

    pub fn missing(&self, req: &Request) -> usize {
        req.tiles
            .iter()
            .filter(|&&(tx, tz)| !self.tiles.contains_key(&Self::tile_key(req, tx, tz)))
            .count()
            + req
                .regions
                .iter()
                .filter(|&&(rx, rz)| {
                    !self.regions.contains_key(&RegionKey { seed: req.seed, dim: req.dim, rx, rz })
                })
                .count()
    }

    /// Bounds memory: past the limit the old tiles go (the visible ones are made again).
    pub fn trim(&mut self) {
        if self.tiles.len() > MAX_TILES {
            self.tiles.clear();
        }
        if self.regions.len() > MAX_TILES / 4 {
            self.regions.clear();
        }
    }

    pub fn biome_at(&self, req: &Request, cell: [i32; 2]) -> u8 {
        let key = Self::tile_key(req, floor_div(cell[0], TILE), floor_div(cell[1], TILE));
        match self.tiles.get(&key) {
            Some(t) => t[(cell[1].rem_euclid(TILE) * TILE + cell[0].rem_euclid(TILE)) as usize],
            None => UNKNOWN,
        }
    }
}

// ---- the map field's text ------------------------------------------------------------------------

/// The tail of `text` that fits `max` pixels, for a field that is shorter than its text.
pub fn tail_fit(text: &str, max: f32, width: &dyn Fn(&str) -> f32) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0;
    while start < chars.len() {
        let s: String = chars[start..].iter().collect();
        if width(&s) <= max {
            return s;
        }
        start += 1;
    }
    String::new()
}

// ---- scale ----------------------------------------------------------------------------------------

/// A round length in blocks (1, 2 or 5 times a power of ten) that is about `target_px` wide at
/// `blocks_per_px`, for a scale bar; returns the length and its width in pixels.
pub fn scale_bar(blocks_per_px: f64, target_px: f32) -> (u64, f32) {
    let want = (blocks_per_px * f64::from(target_px)).max(1.0);
    let mut best = (1u64, f64::MAX);
    let mut pow = 1u64;
    while pow <= 100_000_000 {
        for k in [1u64, 2, 5] {
            let n = pow * k;
            let off = (n as f64 / want).ln().abs();
            if off < best.1 {
                best = (n, off);
            }
        }
        pow *= 10;
    }
    (best.0, (best.0 as f64 / blocks_per_px) as f32)
}

/// `1500` -> "1500 blocks", `20000` -> "20k blocks".
pub fn blocks_label(n: u64) -> String {
    if n >= 10_000 && n % 1000 == 0 {
        format!("{}k blocks", n / 1000)
    } else {
        format!("{n} blocks")
    }
}

/// How the minimap's size and zoom are chosen: cells across, and the zoom levels on offer.
pub const MINI_CELLS: [i32; 3] = [36, 44, 60];
pub const MINI_MAX_LEVEL: i32 = 5;
