// Seed Map's runtime, a module of the game (`crate::game::seedmap`).
//
// A native mod screen drawn and driven through the screen hooks: a seed field, the three
// dimensions, a pannable and zoomable biome map from the engine's own worldgen (the generator
// thread, `seedmap_gen`), structure icons, the world spawn and the player. Everything the
// screen hooks need to change in the game (close the screen, the clipboard) is queued here
// and done by the frame hook, which has the game mutably.
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::game::seedmap_worker as worker;
use crate::game::seedmap_model::{self as m, CATS, Cache, DIMS, RegionKey, Request, UNKNOWN, View};
use crate::game::{Game, Screen};
use horizon_client::hud::HudBatch;
use horizon_client::text::Text;

const ID: &str = "seedmap";
const VERSION: &str = "0.2.0";
const KEY: &str = "open";
const DEFAULT_KEY: &str = "key.keyboard.m";
const COMMAND: &str = "seedmap";
const MINI_KEY: &str = "minimap";
const MINI_DEFAULT_KEY: &str = "key.keyboard.n";
const MINI_IN_KEY: &str = "mini_in";
const MINI_OUT_KEY: &str = "mini_out";
const MINI_SIZE_KEY: &str = "mini_size";

const TOP_H: f32 = 24.0;
const STATUS_H: f32 = 14.0;
const LEGEND_W: f32 = 128.0;
const COMPACT_W: f32 = 62.0;
const CTRL: u32 = 2;
const MAX_SEED_CHARS: usize = 48;
const PAN_SPEED: f64 = 420.0;

const WHITE: u32 = 0xFFFFFF;
const GRAY: u32 = 0xAAAAAA;
const DIM_GRAY: u32 = 0x777777;
const RED: u32 = 0xFF5555;

struct Ui {
    seed_text: String,
    seed: Option<i64>,
    focused: bool,
    dim: usize,
    view: View,
    hidden: Vec<bool>,
    dragging: bool,
    moved: f32,
    held: [bool; 4],
    closing: bool,
    pending_open: Option<Instant>,
    opened: Option<Instant>,
    want_center: bool,
    paste: bool,
    mini: bool,
    mini_level: i32,
    mini_size: usize,
    /// When Ctrl+V was pressed: its `v` character still arrives and must not be typed.
    pasted_at: Option<Instant>,
    copy: Option<String>,
    player: Option<(f64, f64, f32, usize)>,
    layout: Option<Layout>,
    hover: Option<Hover>,
    loaded: bool,
    read_only: Option<String>,
    note: Option<String>,
}

#[derive(Clone)]
struct Hover {
    x: i32,
    z: i32,
}

impl Ui {
    const fn new() -> Ui {
        Ui {
            seed_text: String::new(),
            seed: None,
            focused: false,
            dim: 0,
            view: View::new(),
            hidden: Vec::new(),
            dragging: false,
            moved: 0.0,
            held: [false; 4],
            closing: false,
            pending_open: None,
            opened: None,
            want_center: false,
            paste: false,
            mini: true,
            mini_level: 1,
            mini_size: 1,
            pasted_at: None,
            copy: None,
            player: None,
            layout: None,
            hover: None,
            loaded: false,
            read_only: None,
            note: None,
        }
    }
}

static UI: Mutex<Ui> = Mutex::new(Ui::new());

fn ui() -> MutexGuard<'static, Ui> {
    UI.lock().unwrap_or_else(|e| e.into_inner())
}

fn is_hidden(ui: &Ui, cat: usize) -> bool {
    ui.hidden.get(cat).copied().unwrap_or(false)
}

fn own(game: &Game<'_>) -> bool {
    game.native_on(ID) && game.native_mod_screen().is_some_and(|id| id == ID)
}

fn say(game: &mut Game<'_>, message: impl Into<String>) {
    game.native_feedback(Text::literal(&message.into()));
}

// ---- config: the last seed and the structure filter ---------------------------------------------

fn config_file() -> Result<PathBuf, String> {
    Game::native_config_dir().map(|dir| dir.join("seedmap.json"))
}

fn ensure_hidden(ui: &mut Ui) {
    if ui.hidden.len() != CATS.len() {
        ui.hidden = CATS.iter().map(|c| !c.default_on).collect();
    }
}

fn ensure_loaded(ui: &mut Ui) {
    if ui.loaded {
        return;
    }
    ui.loaded = true;
    ensure_hidden(ui);
    let path = match config_file() {
        Ok(p) => p,
        Err(e) => {
            ui.read_only = Some(format!("no config folder ({e}): settings are not saved"));
            return;
        }
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(v) => {
                if let Some(seed) = v["seed"].as_str() {
                    ui.seed_text = seed.chars().take(MAX_SEED_CHARS).collect();
                    ui.seed = m::parse_seed(&ui.seed_text);
                }
                if let Some(list) = v["hidden"].as_array() {
                    for (i, c) in CATS.iter().enumerate() {
                        ui.hidden[i] = list.iter().any(|k| k.as_str() == Some(c.key));
                    }
                }
                if let Some(on) = v["mini"].as_bool() {
                    ui.mini = on;
                }
                if let Some(l) = v["mini_level"].as_i64() {
                    ui.mini_level = (l as i32).clamp(0, m::MINI_MAX_LEVEL);
                }
                if let Some(s) = v["mini_size"].as_u64() {
                    ui.mini_size = (s as usize).min(m::MINI_CELLS.len() - 1);
                }
                if let Some(level) = v["level"].as_i64() {
                    ui.view.level = (level as i32).clamp(0, m::MAX_LEVEL);
                }
            }
            Err(e) => {
                ui.read_only = Some(format!("{} is unreadable ({e}); it is left alone", path.display()));
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => ui.read_only = Some(format!("cannot read {} ({e})", path.display())),
    }
}

fn save(ui: &mut Ui) {
    if ui.read_only.is_some() {
        return;
    }
    let Ok(path) = config_file() else { return };
    let hidden: Vec<&str> = CATS
        .iter()
        .enumerate()
        .filter(|(i, _)| is_hidden(ui, *i))
        .map(|(_, c)| c.key)
        .collect();
    let json = serde_json::json!({
        "seed": ui.seed_text,
        "hidden": hidden,
        "level": ui.view.level,
        "mini": ui.mini,
        "mini_level": ui.mini_level,
        "mini_size": ui.mini_size,
    });
    let tmp = path.with_extension("json.tmp");
    let written = serde_json::to_string_pretty(&json)
        .map_err(|e| e.to_string())
        .and_then(|text| std::fs::write(&tmp, text).map_err(|e| e.to_string()))
        .and_then(|()| std::fs::rename(&tmp, &path).map_err(|e| e.to_string()));
    if let Err(e) = written {
        ui.note = Some(format!("could not save the settings: {e}"));
    }
}

fn apply_seed(ui: &mut Ui) {
    ui.seed = m::parse_seed(&ui.seed_text);
    save(ui);
}

// ---- Options > Mods, the command, the key -------------------------------------------------------

pub(super) fn listed(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    mods.push(crate::game::native_hooks::NativeMod {
        id: ID.into(),
        title: format!("Seed Map {VERSION}"),
        description: "A map of any world seed from the game's own world generation: biomes, \
            structures, spawn and where you are. Type a seed, drag to pan, scroll to zoom. \
            Opens with the key below or /seedmap [seed]."
            .into(),
        keys: vec![
            [KEY.into(), "Open the seed map".into(), DEFAULT_KEY.into()],
            [MINI_KEY.into(), "Show or hide the minimap".into(), MINI_DEFAULT_KEY.into()],
            [MINI_IN_KEY.into(), "Minimap: zoom in".into(), "key.keyboard.right.bracket".into()],
            [MINI_OUT_KEY.into(), "Minimap: zoom out".into(), "key.keyboard.left.bracket".into()],
            [MINI_SIZE_KEY.into(), "Minimap: change its size".into(), "key.keyboard.backslash".into()],
        ],
    });
}

pub(super) fn commands(game: &Game<'_>, commands: &mut Vec<&'static str>) {
    if game.native_on(ID) {
        commands.push(COMMAND);
    }
}

/// `/seedmap [seed]`: opens the map once the chat has closed (the frame step).
pub(super) fn command(game: &Game<'_>, command: &str, ran: &mut bool) {
    if !game.native_on(ID) {
        return;
    }
    let mut parts = command.trim_start().splitn(2, char::is_whitespace);
    if parts.next() != Some(COMMAND) {
        return;
    }
    *ran = true;
    let rest = parts.next().unwrap_or("").trim();
    let mut ui = ui();
    ensure_loaded(&mut ui);
    if !rest.is_empty() {
        ui.seed_text = rest.chars().take(MAX_SEED_CHARS).collect();
        apply_seed(&mut ui);
    }
    ui.pending_open = Some(Instant::now());
}

/// Every key, any screen: the open key while playing asks for the map.
pub(super) fn on_key(game: &mut Game<'_>, key: &str, pressed: bool) {
    if !pressed || !game.native_on(ID) || game.screen != Screen::Playing {
        return;
    }
    if key == game.native_mod_key(ID, KEY, DEFAULT_KEY) {
        ui().pending_open = Some(Instant::now());
    } else if key == game.native_mod_key(ID, MINI_KEY, MINI_DEFAULT_KEY) {
        let mut ui = ui();
        ensure_loaded(&mut ui);
        ui.mini = !ui.mini;
        save(&mut ui);
    } else if key == game.native_mod_key(ID, MINI_IN_KEY, "key.keyboard.right.bracket") {
        let mut ui = ui();
        ensure_loaded(&mut ui);
        ui.mini_level = (ui.mini_level - 1).max(0);
        save(&mut ui);
    } else if key == game.native_mod_key(ID, MINI_OUT_KEY, "key.keyboard.left.bracket") {
        let mut ui = ui();
        ensure_loaded(&mut ui);
        ui.mini_level = (ui.mini_level + 1).min(m::MINI_MAX_LEVEL);
        save(&mut ui);
    } else if key == game.native_mod_key(ID, MINI_SIZE_KEY, "key.keyboard.backslash") {
        let mut ui = ui();
        ensure_loaded(&mut ui);
        ui.mini_size = (ui.mini_size + 1) % m::MINI_CELLS.len();
        save(&mut ui);
    }
}

fn open_map(game: &mut Game<'_>) {
    worker::ensure_started(game.options.resources.clone(), game.pack.assets.registry);
    if !game.native_mod_screen_open(ID) {
        say(game, "Seed Map: it cannot open right now.");
        return;
    }
    let mut ui = ui();
    ensure_loaded(&mut ui);
    ui.closing = false;
    ui.dragging = false;
    ui.held = [false; 4];
    ui.hover = None;
    ui.opened = Some(Instant::now());
    ui.focused = ui.seed.is_none();
    if let Some((x, z, _, dim)) = ui.player {
        ui.dim = dim;
        ui.view.center_on(x, z);
    }
}

// ---- once per rendered frame ---------------------------------------------------------------------

pub(super) fn frame(game: &mut Game<'_>, elapsed: Duration) {
    if game.renderer.is_none() {
        return;
    }
    let on = game.native_on(ID);
    let is_open = game.native_mod_screen().is_some_and(|id| id == ID);
    let player = game.native_player().map(|p| {
        (p.x, p.z, p.yaw, m::dim_index(p.dimension).unwrap_or(0))
    });
    let mut open_now = false;
    let mut close = false;
    let mut paste = false;
    let mut copy = None;
    {
        let mut ui = ui();
        ui.player = player;
        if !on {
            ui.pending_open = None;
            close = is_open;
        } else {
            if is_open {
                close = std::mem::take(&mut ui.closing);
                paste = std::mem::take(&mut ui.paste);
                copy = ui.copy.take();
                if std::mem::take(&mut ui.want_center)
                    && let Some((x, z, _, dim)) = ui.player
                {
                    ui.dim = dim;
                    ui.view.center_on(x, z);
                }
                let dt = elapsed.as_secs_f64().min(0.1);
                let [up, left, down, right] = ui.held;
                if !ui.focused && (up || left || down || right) {
                    let step = PAN_SPEED * dt * ui.view.bpp();
                    ui.view.cx += step * (f64::from(right) - f64::from(left));
                    ui.view.cz += step * (f64::from(down) - f64::from(up));
                    ui.view.clamp();
                }
            } else {
                ui.dragging = false;
                ui.held = [false; 4];
                ui.closing = false;
                if ui.opened.take().is_some() {
                    worker::set_request(None);
                    save(&mut ui);
                }
            }
            match ui.pending_open {
                Some(since) if since.elapsed() > Duration::from_secs(2) => ui.pending_open = None,
                Some(_) if !is_open && game.screen == Screen::Playing => {
                    ui.pending_open = None;
                    open_now = true;
                }
                Some(_) if is_open => ui.pending_open = None,
                _ => {}
            }
            if let Some(note) = ui.note.take() {
                drop(ui);
                say(game, format!("Seed Map: {note}"));
            }
        }
    }
    if close {
        game.native_mod_screen_close(ID);
    }
    if paste {
        use horizon_client::clipboard::Clipboard;
        let text = game.clipboard.get();
        let mut ui = ui();
        for c in text.chars().filter(|c| !c.is_control()) {
            if ui.seed_text.chars().count() < MAX_SEED_CHARS {
                ui.seed_text.push(c);
            }
        }
    }
    if let Some(text) = copy {
        use horizon_client::clipboard::Clipboard;
        game.clipboard.set(&text);
        say(game, format!("Seed Map: copied {text}"));
    }
    if open_now {
        open_map(game);
    }
}

// ---- the minimap (the top left corner of the HUD) -------------------------------------------------

pub(super) fn hud(game: &Game<'_>, batch: &mut HudBatch, corners: &mut horizon_client::extras_hud::Corners) {
    use horizon_client::extras::Corner;
    if !game.native_on(ID) || game.native_mod_screen().is_some() || !game.native_corner_shown(Corner::TopLeft) {
        return;
    }
    let Some(p) = game.native_player() else { return };
    let mut guard = ui();
    let ui = &mut *guard;
    ensure_loaded(ui);
    let Some(seed) = ui.seed.filter(|_| ui.mini) else { return };
    if !worker::with(|s| s.init.is_some()) {
        worker::ensure_started(game.options.resources.clone(), game.pack.assets.registry);
    }
    let dim = m::dim_index(p.dimension).unwrap_or(0);
    let level = ui.mini_level;
    let size = m::MINI_CELLS[ui.mini_size.min(m::MINI_CELLS.len() - 1)] as f32 * m::CELL;
    let (x, y) = corners.place(Corner::TopLeft, size + 4.0, size + 15.0);
    let map = [x + 2.0, y + 2.0, size, size];
    let view = View { cx: p.x, cz: p.z, level };
    let req = Request {
        seed,
        dim: dim as u8,
        level: level as u8,
        tiles: view.tiles(map),
        regions: view.regions(map),
    };
    worker::set_request(Some(req.clone()));
    let (rects, marks) = worker::with(|s| {
        (biome_rects(&s.cache, &req, &view, map), markers(&ui.hidden, dim, &s.cache, &req, &view, map))
    });
    game.native_fill(batch, [x, y, size + 4.0, size + 4.0], 0xFF000000);
    game.native_fill(batch, map, 0xFF161B22);
    for (r, argb) in &rects {
        game.native_fill(batch, *r, *argb);
    }
    // How far the minimap sees, so a structure's distance can be judged at a glance.
    let span = (f64::from(size) * view.bpp()).round() as u64;
    let label = format!("{} across", m::blocks_label(span));
    let lw = game.native_text_width(&label);
    game.native_fill(batch, [x, y + size + 4.0, lw + 6.0, 11.0], 0x90000000);
    game.native_text(batch, &label, [x + 3.0, y + size + 6.0], GRAY, false);
    for mk in &marks {
        let (mx, my) = (mk.at[0].round(), mk.at[1].round());
        let dot = [mx - 2.0, my - 2.0, 5.0, 5.0];
        if clip(dot, map) == Some(dot) {
            game.native_fill(batch, dot, 0xFF000000);
            game.native_fill(batch, [mx - 1.0, my - 1.0, 3.0, 3.0], 0xFF00_0000 | CATS[mk.cat].color);
        }
    }
    let (cx, cy) = (map[0] + map[2] / 2.0, map[1] + map[3] / 2.0);
    let a = f64::from(p.yaw).to_radians();
    for k in [4.0f64, 6.0] {
        let (dx, dy) = ((-a.sin() * k) as f32, (a.cos() * k) as f32);
        game.native_fill(batch, [cx.round() + dx - 1.0, cy.round() + dy - 1.0, 2.0, 2.0], 0xFFFF3030);
    }
    game.native_fill(batch, [cx.round() - 3.0, cy.round() - 3.0, 6.0, 6.0], 0xFF000000);
    game.native_fill(batch, [cx.round() - 2.0, cy.round() - 2.0, 4.0, 4.0], 0xFFFFFFFF);
    game.native_fill(batch, [cx.round() - 1.0, cy.round() - 1.0, 2.0, 2.0], 0xFFE02020);
}

// ---- layout ----------------------------------------------------------------------------------

#[derive(Clone)]
struct Layout {
    map: [f32; 4],
    legend: [f32; 4],
    field: [f32; 4],
    go: [f32; 4],
    dims: [[f32; 4]; 3],
    me: [f32; 4],
    compact: bool,
    legend_w: f32,
    rows: Vec<(usize, [f32; 4])>,
}

fn inside(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[0] < r[0] + r[2] && p[1] >= r[1] && p[1] < r[1] + r[3]
}

fn clip(r: [f32; 4], to: [f32; 4]) -> Option<[f32; 4]> {
    let (x0, y0) = (r[0].max(to[0]), r[1].max(to[1]));
    let (x1, y1) = ((r[0] + r[2]).min(to[0] + to[2]), (r[1] + r[3]).min(to[1] + to[3]));
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

fn layout(game: &Game<'_>, w: f32, h: f32, dim: usize) -> Layout {
    let tw = |s: &str| game.native_text_width(s);
    let (by, bh) = (4.0, 16.0);
    // The buttons keep their width; the seed field takes what is left (the GUI can be small).
    let labels = ["Go", DIMS[0].1, DIMS[1].1, DIMS[2].1, "Me"];
    let widths: Vec<f32> = labels.iter().map(|l| tw(l) + 12.0).collect();
    let fixed: f32 = widths.iter().sum::<f32>() + 3.0 * 4.0 + 6.0 + 4.0 + 8.0;
    let field_w = (w - fixed).clamp(48.0, 150.0);
    let mut x = 4.0;
    let field = [x, by, field_w, bh];
    x += field_w + 4.0;
    let go = [x, by, widths[0], bh];
    x += widths[0] + 8.0;
    let mut dims = [[0.0; 4]; 3];
    for (i, d) in dims.iter_mut().enumerate() {
        *d = [x, by, widths[1 + i], bh];
        x += widths[1 + i] + 3.0;
    }
    x += 3.0;
    let me = [x, by, widths[4], bh];
    let shown: Vec<usize> = CATS
        .iter()
        .enumerate()
        .filter(|(i, c)| c.dims & (1 << dim) != 0 && *i != m::OTHER)
        .map(|(i, _)| i)
        .collect();
    let body = h - TOP_H - STATUS_H;
    let compact = (shown.len() as f32) * 12.0 + 16.0 > body;
    let legend_w = if compact { COMPACT_W } else { LEGEND_W };
    let legend = [w - legend_w, TOP_H, legend_w, h - TOP_H];
    let map = [0.0, TOP_H, w - legend_w, body];
    let mut rows = Vec::new();
    if compact {
        let cols = ((legend_w - 4.0) / 14.0).floor().max(1.0) as usize;
        for (n, i) in shown.iter().enumerate() {
            let (cx, cy) = ((n % cols) as f32, (n / cols) as f32);
            rows.push((*i, [legend[0] + 4.0 + cx * 14.0, legend[1] + 16.0 + cy * 14.0, 12.0, 12.0]));
        }
    } else {
        let mut y = legend[1] + 16.0;
        for i in shown {
            rows.push((i, [legend[0] + 2.0, y, legend[2] - 4.0, 12.0]));
            y += 12.0;
        }
    }
    Layout { map, legend, field, go, dims, me, compact, legend_w, rows }
}

// ---- drawing --------------------------------------------------------------------------------

struct Mark {
    at: [f32; 2],
    cat: usize,
    x: i32,
    z: i32,
    label: String,
}

struct Frame {
    rects: Vec<([f32; 4], u32)>,
    marks: Vec<Mark>,
    biome_under_mouse: Option<String>,
    missing: usize,
    failed: Option<String>,
}

fn biome_color(cache: &Cache, id: u8) -> u32 {
    if id == UNKNOWN {
        return 0x161B22;
    }
    cache.colors.get(usize::from(id)).copied().unwrap_or(0x161B22)
}

/// The biome colours of what `view` shows in `map` as ARGB rectangles: a run of equal cells is one
/// rectangle, and a faint line is drawn where one biome meets another.
fn biome_rects(cache: &Cache, req: &Request, view: &View, map: [f32; 4]) -> Vec<([f32; 4], u32)> {
    let (a, b) = view.cells(map);
    let (bpc, bpp) = (view.bpc(), view.bpp());
    let sx = |cx: i32| (map[0] + map[2] / 2.0 + ((f64::from(cx) * bpc - view.cx) / bpp) as f32).round();
    let sy = |cz: i32| (map[1] + map[3] / 2.0 + ((f64::from(cz) * bpc - view.cz) / bpp) as f32).round();
    const EDGE: u32 = 0x40000000;
    let row = |cz: i32| -> Vec<u8> { (a[0]..=b[0]).map(|cx| cache.biome_at(req, [cx, cz])).collect() };
    let mut rects = Vec::new();
    let mut prev = row(a[1] - 1);
    for cz in a[1]..=b[1] {
        let ids = row(cz);
        let (y0, y1) = (sy(cz), sy(cz + 1));
        let mut push = |r: [f32; 4], argb: u32| {
            if let Some(r) = clip(r, map) {
                rects.push((r, argb));
            }
        };
        // Runs of one biome.
        let mut start = 0usize;
        for i in 1..=ids.len() {
            if i == ids.len() || ids[i] != ids[start] {
                let (x0, x1) = (sx(a[0] + start as i32), sx(a[0] + i as i32));
                push([x0, y0, x1 - x0, y1 - y0], 0xFF00_0000 | biome_color(cache, ids[start]));
                // A line down the boundary with the next run (not against ungenerated ground).
                if i < ids.len() && ids[start] != UNKNOWN && ids[i] != UNKNOWN {
                    push([x1, y0, 1.0, y1 - y0], EDGE);
                }
                start = i;
            }
        }
        // A line along the top where the row above is another biome.
        let mut s = None;
        for i in 0..=ids.len() {
            let edge = i < ids.len() && ids[i] != prev[i] && ids[i] != UNKNOWN && prev[i] != UNKNOWN;
            match (edge, s) {
                (true, None) => s = Some(i),
                (false, Some(st)) => {
                    let (x0, x1) = (sx(a[0] + st as i32), sx(a[0] + i as i32));
                    push([x0, y0, x1 - x0, 1.0], EDGE);
                    s = None;
                }
                _ => {}
            }
        }
        prev = ids;
    }
    rects
}

/// The structure icons inside `map` (a margin beyond it): regions, strongholds and spawn.
fn markers(hidden: &[bool], dim: usize, cache: &Cache, req: &Request, view: &View, map: [f32; 4]) -> Vec<Mark> {
    let pad = [map[0] - 8.0, map[1] - 8.0, map[2] + 16.0, map[3] + 16.0];
    let mut marks = Vec::new();
    let mut push = |cat: usize, x: i32, z: i32, label: &dyn Fn() -> String| {
        if hidden.get(cat).copied().unwrap_or(false) || CATS[cat].dims & (1 << dim) == 0 {
            return;
        }
        let at = view.to_screen(map, f64::from(x), f64::from(z));
        if inside(pad, at) {
            marks.push(Mark { at, cat, x, z, label: label() });
        }
    };
    for &(rx, rz) in &req.regions {
        let key = RegionKey { seed: req.seed, dim: req.dim, rx, rz };
        if let Some(list) = cache.regions.get(&key) {
            for mk in list {
                push(usize::from(mk.cat), mk.x, mk.z, &|| m::pretty(&mk.name));
            }
        }
    }
    if dim == 0 {
        if let Some(list) = cache.strongholds.get(&req.seed) {
            for p in list {
                push(m::STRONGHOLD, p[0], p[1], &|| "Stronghold".into());
            }
        }
        if let Some(p) = cache.spawn.get(&req.seed) {
            push(m::SPAWN, p[0], p[1], &|| "World spawn".into());
        }
    }
    marks
}

fn collect(ui: &Ui, lay: &Layout, req: &Request, cache: &Cache, mouse: [f32; 2]) -> Frame {
    let view = &ui.view;
    let map = lay.map;
    let bpc = view.bpc();
    let rects = biome_rects(cache, req, view, map);
    let marks = markers(&ui.hidden, ui.dim, cache, req, view, map);
    let biome_under_mouse = inside(map, mouse).then(|| {
        let w = view.to_world(map, mouse[0], mouse[1]);
        let cell = [(w[0] / bpc).floor() as i32, (w[1] / bpc).floor() as i32];
        let id = cache.biome_at(req, cell);
        if id == UNKNOWN {
            "generating…".to_owned()
        } else {
            cache.names.get(usize::from(id)).map_or(String::new(), |n| m::pretty(n))
        }
    });
    Frame {
        rects,
        marks,
        biome_under_mouse,
        missing: cache.missing(req),
        failed: cache.failed.clone(),
    }
}

/// A structure's icon centred on `at`: the game's own item for it on the category colour, else
/// the colour with its letter. `plain` is a coloured dot.
fn icon(game: &Game<'_>, batch: &mut HudBatch, cat: &m::Cat, at: [f32; 2], size: f32, plain: bool) {
    let (x, y) = (at[0].round(), at[1].round());
    let half = (size / 2.0).floor();
    game.native_fill(batch, [x - half, y - half, half * 2.0, half * 2.0], 0xFF000000);
    game.native_fill(
        batch,
        [x - half + 1.0, y - half + 1.0, half * 2.0 - 2.0, half * 2.0 - 2.0],
        0xFF00_0000 | cat.color,
    );
    if plain {
        return;
    }
    let inner = half * 2.0 - 2.0;
    if !cat.item.is_empty() && game.native_item(batch, cat.item, [x - half + 1.0, y - half + 1.0], inner) {
        return;
    }
    let lw = game.native_text_width(cat.letter);
    let light = matches!(cat.color, 0xFFFFFF | 0xEEEEEE | 0xB2EBF2);
    game.native_text(batch, cat.letter, [x - lw / 2.0, y - 4.0], if light { 0x000000 } else { WHITE }, false);
}

fn box_border(game: &Game<'_>, batch: &mut HudBatch, r: [f32; 4], border: u32, fill: u32) {
    game.native_fill(batch, r, border);
    game.native_fill(batch, [r[0] + 1.0, r[1] + 1.0, r[2] - 2.0, r[3] - 2.0], fill);
}

fn button(game: &Game<'_>, batch: &mut HudBatch, r: [f32; 4], label: &str, active: bool, mouse: [f32; 2]) {
    let fill = if active {
        0xFF3B6EA8
    } else if inside(r, mouse) {
        0xFF3A4048
    } else {
        0xFF2A2F36
    };
    box_border(game, batch, r, 0xFF000000, fill);
    let tw = game.native_text_width(label);
    game.native_text(batch, label, [r[0] + (r[2] - tw) / 2.0, r[1] + 4.0], WHITE, true);
}

pub(super) fn render(game: &Game<'_>, batch: &mut HudBatch, mouse: [f32; 2]) {
    if !own(game) {
        return;
    }
    let mut guard = ui();
    let ui = &mut *guard;
    ensure_loaded(ui);
    let (w, h) = (batch.width, batch.height);
    let lay = layout(game, w, h, ui.dim);
    ui.view.clamp();
    let map = lay.map;

    // What the generator should have: only while there is a seed.
    let req = ui.seed.map(|seed| Request {
        seed,
        dim: ui.dim as u8,
        level: ui.view.level as u8,
        tiles: ui.view.tiles(map),
        regions: ui.view.regions(map),
    });
    worker::set_request(req.clone());

    game.native_fill(batch, [0.0, 0.0, w, h], 0xFF0B0E11);
    let frame = req.as_ref().map(|req| worker::with(|s| collect(ui, &lay, req, &s.cache, mouse)));
    if let Some(frame) = &frame {
        for (r, argb) in &frame.rects {
            game.native_fill(batch, *r, *argb);
        }
        // The axes.
        let o = ui.view.to_screen(map, 0.0, 0.0);
        if let Some(r) = clip([o[0], map[1], 1.0, map[3]], map) {
            game.native_fill(batch, r, 0x70FFFFFF);
        }
        if let Some(r) = clip([map[0], o[1], map[2], 1.0], map) {
            game.native_fill(batch, r, 0x70FFFFFF);
        }
    } else {
        let msg = "Type a world seed above (click the field) and press Enter";
        let tw = game.native_text_width(msg);
        game.native_text(batch, msg, [map[0] + (map[2] - tw) / 2.0, map[1] + map[3] / 2.0 - 4.0], GRAY, true);
    }

    // The scale bar, bottom left of the map.
    {
        let (blocks, px) = m::scale_bar(ui.view.bpp(), 70.0);
        let label = m::blocks_label(blocks);
        let (bx, by) = (map[0] + 8.0, map[1] + map[3] - 10.0);
        let lw = game.native_text_width(&label);
        game.native_fill(batch, [bx - 3.0, by - 14.0, px.max(lw) + 6.0, 22.0], 0x90000000);
        game.native_text(batch, &label, [bx, by - 12.0], WHITE, false);
        game.native_fill(batch, [bx, by, px, 2.0], 0xFFFFFFFF);
        game.native_fill(batch, [bx, by - 3.0, 1.0, 5.0], 0xFFFFFFFF);
        game.native_fill(batch, [bx + px - 1.0, by - 3.0, 1.0, 5.0], 0xFFFFFFFF);
    }

    // The nearest icon to the cursor is the one the tooltip names.
    let mut hover: Option<usize> = None;
    if let Some(frame) = &frame {
        let mut best = 7.0f32 * 7.0;
        for (i, mk) in frame.marks.iter().enumerate() {
            let d = (mk.at[0] - mouse[0]).powi(2) + (mk.at[1] - mouse[1]).powi(2);
            if inside(map, mouse) && d <= best {
                best = d;
                hover = Some(i);
            }
        }
        // Zoomed far out the icons shrink to coloured dots, or the map is all icons.
        let big = ui.view.level <= 1;
        let half = if big { 7.0 } else { 4.0 };
        for mk in &frame.marks {
            let (x, y) = (mk.at[0].round(), mk.at[1].round());
            let outer = [x - 7.0, y - 7.0, 14.0, 14.0];
            // An icon cut by the map's edge is skipped rather than drawn half over the panels.
            if clip(outer, map) != Some(outer) {
                continue;
            }
            // The End ship (elytra) is the thing worth finding there: it never shrinks to a dot.
            let ship = mk.cat == m::END_SHIP;
            let size = if ship { 14.0 } else { half * 2.0 };
            icon(game, batch, &CATS[mk.cat], [x, y], size, !big && !ship);
        }
    }

    // The player.
    if let Some((px, pz, yaw, pdim)) = ui.player
        && pdim == ui.dim
    {
        let at = ui.view.to_screen(map, px, pz);
        if inside(map, at) {
            let (x, y) = (at[0].round(), at[1].round());
            let a = f64::from(yaw).to_radians();
            for k in [5.0f64, 8.0, 11.0] {
                let (dx, dy) = ((-a.sin() * k) as f32, (a.cos() * k) as f32);
                if let Some(r) = clip([x + dx - 1.0, y + dy - 1.0, 3.0, 3.0], map) {
                    game.native_fill(batch, r, 0xFFFF3030);
                }
            }
            game.native_fill(batch, [x - 4.0, y - 4.0, 9.0, 9.0], 0xFF000000);
            game.native_fill(batch, [x - 3.0, y - 3.0, 7.0, 7.0], 0xFFFFFFFF);
            game.native_fill(batch, [x - 1.0, y - 1.0, 3.0, 3.0], 0xFFE02020);
        }
    }

    // The top bar.
    game.native_fill(batch, [0.0, 0.0, w, TOP_H], 0xFF1B1F24);
    let border = if ui.focused { 0xFF8AB4F8 } else { 0xFF555C66 };
    box_border(game, batch, lay.field, border, 0xFF000000);
    let inner = lay.field[2] - 8.0;
    if ui.seed_text.is_empty() && !ui.focused {
        game.native_text(batch, "seed (click, or Ctrl+V)", [lay.field[0] + 4.0, lay.field[1] + 4.0], DIM_GRAY, false);
    } else {
        let mut shown = ui.seed_text.clone();
        if ui.focused {
            shown.push('_');
        }
        let text = m::tail_fit(&shown, inner, &|s| game.native_text_width(s));
        game.native_text(batch, &text, [lay.field[0] + 4.0, lay.field[1] + 4.0], WHITE, false);
    }
    button(game, batch, lay.go, "Go", false, mouse);
    for (i, r) in lay.dims.iter().enumerate() {
        button(game, batch, *r, DIMS[i].1, i == ui.dim, mouse);
    }
    button(game, batch, lay.me, "Me", false, mouse);

    // The legend.
    game.native_fill(batch, lay.legend, 0xFF1B1F24);
    game.native_text(batch, if lay.compact { "Show" } else { "Structures" }, [lay.legend[0] + 6.0, lay.legend[1] + 4.0], GRAY, true);
    let mut legend_hint = None;
    for (i, r) in &lay.rows {
        let cat = &CATS[*i];
        let off = is_hidden(ui, *i);
        if inside(*r, mouse) {
            legend_hint = Some(cat.label);
        }
        let center = [r[0] + 6.0, r[1] + 6.0];
        if lay.compact {
            icon(game, batch, cat, center, 12.0, false);
            if off {
                game.native_fill(batch, *r, 0xC00B0E11);
            }
            if inside(*r, mouse) {
                game.native_fill(batch, [r[0], r[1], r[2], 1.0], 0xFFFFFFFF);
                game.native_fill(batch, [r[0], r[1] + r[3] - 1.0, r[2], 1.0], 0xFFFFFFFF);
            }
        } else {
            if inside(*r, mouse) {
                game.native_fill(batch, *r, 0xFF2E343B);
            }
            icon(game, batch, cat, [r[0] + 8.0, r[1] + 6.0], 12.0, false);
            if off {
                game.native_fill(batch, [r[0] + 2.0, r[1], 12.0, 12.0], 0xC01B1F24);
            }
            game.native_text(batch, cat.label, [r[0] + 18.0, r[1] + 2.0], if off { DIM_GRAY } else { WHITE }, false);
        }
    }

    // The status bar.
    let sb = [0.0, h - STATUS_H, w, STATUS_H];
    game.native_fill(batch, sb, 0xFF1B1F24);
    let mut left = String::new();
    if inside(map, mouse) {
        let wp = ui.view.to_world(map, mouse[0], mouse[1]);
        left = format!("X {}  Z {}", wp[0].floor() as i64, wp[1].floor() as i64);
        if let Some(Some(b)) = frame.as_ref().map(|f| f.biome_under_mouse.clone()) {
            left.push_str(&format!("   {b}"));
        }
    }
    if let Some(label) = legend_hint.filter(|_| lay.compact) {
        left = format!("{label} (click to show or hide)");
    }
    // The left text gives way to the right one when they would meet.
    let right_w = game.native_text_width(&format!("seed {}   {} blocks/px   ", ui.seed.unwrap_or(0), ui.view.bpp()));
    while !left.is_empty() && game.native_text_width(&left) > w - lay.legend_w - right_w - 12.0 {
        left.pop();
    }
    game.native_text(batch, &left, [6.0, sb[1] + 3.0], WHITE, false);
    let mut right = String::new();
    if let Some(frame) = &frame {
        if frame.missing > 0 {
            right.push_str("generating…   ");
        }
    }
    if let Some(seed) = ui.seed {
        right.push_str(&format!("seed {seed}   "));
    }
    right.push_str(&format!("{} blocks/px", ui.view.bpp()));
    let rw = game.native_text_width(&right);
    game.native_text(batch, &right, [w - lay.legend_w - rw - 6.0, sb[1] + 3.0], GRAY, false);
    if let Some(why) = frame.as_ref().and_then(|f| f.failed.as_ref()) {
        let shown = m::tail_fit(why, 260.0, &|s| game.native_text_width(s));
        game.native_text(batch, &shown, [(w - lay.legend_w) / 2.0 - 60.0, sb[1] + 3.0], RED, false);
    }

    // The tooltip.
    ui.hover = None;
    if let (Some(i), Some(frame)) = (hover, &frame) {
        let mk = &frame.marks[i];
        let line1 = mk.label.clone();
        let line2 = format!("X {}  Z {}   click to copy", mk.x, mk.z);
        let tw = game.native_text_width(&line1).max(game.native_text_width(&line2)) + 8.0;
        let (th, mut tx, mut ty) = (22.0, mouse[0] + 12.0, mouse[1] + 12.0);
        if tx + tw > w {
            tx = mouse[0] - tw - 6.0;
        }
        if ty + th > h {
            ty = mouse[1] - th - 6.0;
        }
        game.native_fill(batch, [tx, ty, tw, th], 0xF0101216);
        game.native_text(batch, &line1, [tx + 4.0, ty + 3.0], WHITE, true);
        game.native_text(batch, &line2, [tx + 4.0, ty + 12.0], GRAY, false);
        ui.hover = Some(Hover { x: mk.x, z: mk.z });
    }
    ui.layout = Some(lay);
}

// ---- input --------------------------------------------------------------------------------------

pub(super) fn mouse(game: &Game<'_>, at: [f32; 2], button: i32, canceled: &mut bool) {
    if !own(game) {
        return;
    }
    *canceled = true;
    let mut guard = ui();
    let ui = &mut *guard;
    let Some(lay) = ui.layout.clone() else { return };
    if button == 1 {
        if inside(lay.map, at) {
            let wp = ui.view.to_world(lay.map, at[0], at[1]);
            ui.copy = Some(format!("{} {}", wp[0].floor() as i64, wp[1].floor() as i64));
        }
        return;
    }
    if button != 0 {
        return;
    }
    ui.focused = inside(lay.field, at);
    if ui.focused {
        return;
    }
    if inside(lay.go, at) {
        apply_seed(ui);
    } else if let Some(i) = lay.dims.iter().position(|r| inside(*r, at)) {
        switch_dim(ui, i);
    } else if inside(lay.me, at) {
        ui.want_center = true;
    } else if let Some((i, _)) = lay.rows.iter().find(|(_, r)| inside(*r, at)) {
        ensure_hidden(ui);
        ui.hidden[*i] = !ui.hidden[*i];
        save(ui);
    } else if inside(lay.map, at) {
        ui.dragging = true;
        ui.moved = 0.0;
    }
}

/// Switching dimension keeps the place, scaled 8:1 between the overworld and the nether.
fn switch_dim(ui: &mut Ui, to: usize) {
    if to == ui.dim {
        return;
    }
    let (cx, cz) = match (ui.dim, to) {
        (0, 1) => (ui.view.cx / 8.0, ui.view.cz / 8.0),
        (1, 0) => (ui.view.cx * 8.0, ui.view.cz * 8.0),
        _ => (ui.view.cx, ui.view.cz),
    };
    ui.dim = to;
    ui.view.center_on(cx, cz);
    if let Some((x, z, _, pdim)) = ui.player
        && pdim == to
    {
        ui.view.center_on(x, z);
    }
}

pub(super) fn released(game: &Game<'_>, button: i32, canceled: &mut bool) {
    if !own(game) {
        return;
    }
    *canceled = true;
    let mut ui = ui();
    if button == 0 && std::mem::take(&mut ui.dragging) && ui.moved < 3.0 {
        if let Some(h) = ui.hover.clone() {
            ui.copy = Some(format!("{} {}", h.x, h.z));
        }
    }
}

pub(super) fn dragged(game: &Game<'_>, button: i32, delta: [f32; 2], canceled: &mut bool) {
    if !own(game) {
        return;
    }
    *canceled = true;
    let mut ui = ui();
    if button == 0 && ui.dragging {
        ui.moved += delta[0].abs() + delta[1].abs();
        ui.view.pan_px(delta[0], delta[1]);
    }
}

pub(super) fn scrolled(game: &Game<'_>, at: [f32; 2], scroll: [f64; 2], canceled: &mut bool) {
    if !own(game) {
        return;
    }
    *canceled = true;
    let mut guard = ui();
    let ui = &mut *guard;
    let Some(map) = ui.layout.as_ref().map(|l| l.map) else { return };
    if scroll[1] != 0.0 {
        ui.view.zoom_at(map, if inside(map, at) { at } else { [map[0] + map[2] / 2.0, map[1] + map[3] / 2.0] }, if scroll[1] > 0.0 { 1 } else { -1 });
    }
}

pub(super) fn key(game: &Game<'_>, key: &str, modifiers: u32, canceled: &mut bool) {
    if !own(game) {
        return;
    }
    let mut guard = ui();
    let ui = &mut *guard;
    let name = key.strip_prefix("key.keyboard.").unwrap_or(key);
    if ui.focused {
        match name {
            "backspace" => {
                ui.seed_text.pop();
            }
            "delete" => ui.seed_text.clear(),
            "enter" | "keypad.enter" | "keypad_enter" => {
                ui.focused = false;
                apply_seed(ui);
            }
            "escape" => ui.focused = false,
            "v" if modifiers & CTRL != 0 => {
                ui.paste = true;
                ui.pasted_at = Some(Instant::now());
            }
            _ => {}
        }
        // Everything typed in the field stays in it, Escape included (it leaves the field first).
        *canceled = true;
        return;
    }
    if key == game.native_mod_key(ID, KEY, DEFAULT_KEY) {
        ui.closing = true;
        *canceled = true;
        return;
    }
    let center = ui.layout.as_ref().map(|l| {
        [l.map[0] + l.map[2] / 2.0, l.map[1] + l.map[3] / 2.0]
    });
    *canceled = true;
    match name {
        "w" | "up" => ui.held[0] = true,
        "a" | "left" => ui.held[1] = true,
        "s" | "down" => ui.held[2] = true,
        "d" | "right" => ui.held[3] = true,
        "equal" | "keypad.add" | "keypad_add" => {
            if let (Some(c), Some(l)) = (center, ui.layout.as_ref().map(|l| l.map)) {
                ui.view.zoom_at(l, c, 1);
            }
        }
        "minus" | "keypad.subtract" | "keypad_subtract" => {
            if let (Some(c), Some(l)) = (center, ui.layout.as_ref().map(|l| l.map)) {
                ui.view.zoom_at(l, c, -1);
            }
        }
        "c" | "home" => ui.want_center = true,
        "tab" | "enter" | "keypad.enter" | "keypad_enter" => ui.focused = true,
        // Escape closes the screen itself; anything else is not ours.
        _ => *canceled = false,
    }
}

pub(super) fn key_released(game: &Game<'_>, key: &str) {
    if !own(game) {
        return;
    }
    let mut ui = ui();
    match key.strip_prefix("key.keyboard.").unwrap_or(key) {
        "w" | "up" => ui.held[0] = false,
        "a" | "left" => ui.held[1] = false,
        "s" | "down" => ui.held[2] = false,
        "d" | "right" => ui.held[3] = false,
        _ => {}
    }
}

pub(super) fn typed(game: &Game<'_>, c: char, modifiers: u32, canceled: &mut bool) {
    if !own(game) {
        return;
    }
    let mut ui = ui();
    if !ui.focused {
        return;
    }
    *canceled = true;
    // Ctrl+V's own `v` (and any Ctrl shortcut's letter) is a key, not text.
    if modifiers & CTRL != 0 || ui.pasted_at.is_some_and(|t| t.elapsed() < Duration::from_millis(150)) {
        return;
    }
    // The key that opened the map types its own character a moment later.
    if ui.opened.is_some_and(|t| t.elapsed() < Duration::from_millis(250)) {
        return;
    }
    if !c.is_control() && ui.seed_text.chars().count() < MAX_SEED_CHARS {
        ui.seed_text.push(c);
    }
}
