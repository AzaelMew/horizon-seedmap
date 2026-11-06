// Seed Map: the generator thread. It owns the vanilla overworld generator for the typed seed
// (`horizon_world`, the engine's own worldgen) and fills the cache; the game thread only reads.
// No lock is held while generating: a job is picked under the lock, run outside it, and its
// result is handed over under the lock again.
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard};

use horizon_core::position::ChunkPos;
use horizon_world::registry::BlockRegistry;
use horizon_world::worldgen::generator::{GeneratorData, GeneratorWorker, OverworldGenerator};

use crate::game::seedmap_model::{
    self as m, Cache, Job, Marker, Request, TILE, UNKNOWN,
};

pub struct Init {
    pub dir: PathBuf,
    pub registry: &'static BlockRegistry,
}

pub struct Shared {
    pub init: Option<Init>,
    pub req: Option<Request>,
    pub cache: Cache,
    pub started: bool,
}

static SHARED: LazyLock<Mutex<Shared>> = LazyLock::new(|| {
    Mutex::new(Shared { init: None, req: None, cache: Cache::default(), started: false })
});
static WAKE: Condvar = Condvar::new();

fn lock() -> MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Runs `f` on the shared state. Keep it short: the generator thread waits on the same lock.
pub fn with<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    f(&mut lock())
}

/// Hands the thread the game's resources and starts it (once).
pub fn ensure_started(dir: PathBuf, registry: &'static BlockRegistry) {
    let spawn = {
        let mut s = lock();
        if s.init.is_none() {
            s.init = Some(Init { dir, registry });
        }
        !std::mem::replace(&mut s.started, true)
    };
    if spawn {
        let spawned = std::thread::Builder::new()
            .name("seedmap-gen".into())
            .spawn(worker_main);
        if let Err(e) = spawned {
            let mut s = lock();
            s.started = false;
            s.cache.failed = Some(format!("cannot start the generator thread: {e}"));
        }
    }
    WAKE.notify_all();
}

/// Replaces what the map wants; the thread wakes when it changed.
pub fn set_request(req: Option<Request>) {
    let changed = {
        let mut s = lock();
        if s.req == req {
            false
        } else {
            s.req = req;
            true
        }
    };
    if changed {
        WAKE.notify_all();
    }
}

struct Gen {
    seed: i64,
    dim: u8,
    g: OverworldGenerator,
    w: GeneratorWorker,
}

fn make(data: &Arc<GeneratorData>, seed: i64, dim: u8) -> Result<Gen, String> {
    let g = OverworldGenerator::for_world(
        data.clone(),
        seed,
        "minecraft:normal",
        m::DIMS[usize::from(dim).min(2)].0,
        true,
    )
    .map_err(|e| e.to_string())?;
    let w = g.worker();
    Ok(Gen { seed, dim, g, w })
}

fn worker_main() {
    let mut data: Option<Arc<GeneratorData>> = None;
    let mut gen_state: Option<Gen> = None;
    loop {
        // Wait for something the request still needs.
        let (job, dir_registry) = {
            let mut s = lock();
            loop {
                if s.init.is_some()
                    && let Some(req) = s.req.clone()
                    && let Some(job) = s.cache.next_job(&req)
                {
                    let init = s.init.as_ref().expect("checked");
                    break (job, (init.dir.clone(), init.registry));
                }
                // Idle: give the generator's scratch memory back.
                if let Some(g) = gen_state.as_mut() {
                    g.w.release();
                }
                s = WAKE.wait(s).unwrap_or_else(|e| e.into_inner());
            }
        };
        if data.is_none() {
            match horizon_client::vanillagen::shared_data(
                &dir_registry.0.join("worldgen"),
                dir_registry.1,
            ) {
                Ok(d) => data = Some(d),
                Err(e) => {
                    fail(job, format!("worldgen data: {e}"));
                    continue;
                }
            }
        }
        let data_ref = data.as_ref().expect("loaded above");
        let (seed, dim) = job_world(job);
        if gen_state.as_ref().is_none_or(|g| g.seed != seed || g.dim != dim) {
            gen_state = None;
            match make(data_ref, seed, dim) {
                Ok(g) => gen_state = Some(g),
                Err(e) => {
                    fail(job, format!("generator: {e}"));
                    continue;
                }
            }
        }
        let g = gen_state.as_mut().expect("made above");
        let done = catch_unwind(AssertUnwindSafe(|| run(g, job)));
        match done {
            Ok(result) => finish(job, result),
            Err(_) => {
                gen_state = None;
                fail(job, "the generator panicked".into());
            }
        }
    }
}

fn job_world(job: Job) -> (i64, u8) {
    match job {
        Job::Tile(k) => (k.seed, k.dim),
        Job::Region(k) => (k.seed, k.dim),
        Job::Strongholds(seed) | Job::Spawn(seed) => (seed, 0),
    }
}

/// Whether a structure piece is the End city's ship. The piece's `Debug` starts with its template
/// name; reading only that head avoids formatting its whole block palette.
fn is_ship_piece(piece: &impl std::fmt::Debug) -> bool {
    use std::fmt::Write;
    struct Head(String);
    impl Write for Head {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            self.0.push_str(s);
            if self.0.len() >= 240 { Err(std::fmt::Error) } else { Ok(()) }
        }
    }
    let mut head = Head(String::new());
    let _ = write!(head, "{piece:?}");
    head.0.contains("end_city/ship")
}

enum Output {
    Tile(Vec<Option<String>>),
    Region(Vec<Marker>),
    Strongholds(Vec<[i32; 2]>),
    Spawn([i32; 2]),
}

fn run(g: &mut Gen, job: Job) -> Output {
    match job {
        Job::Tile(key) => {
            let bpc = m::blocks_per_cell(i32::from(key.level)) as i64;
            let mut quarts = Vec::with_capacity((TILE * TILE) as usize);
            for j in 0..TILE {
                for i in 0..TILE {
                    let cx = i64::from(key.tx) * i64::from(TILE) + i64::from(i);
                    let cz = i64::from(key.tz) * i64::from(TILE) + i64::from(j);
                    let bx = cx * bpc + bpc / 2;
                    let bz = cz * bpc + bpc / 2;
                    // Quart y 16 is block 64: the surface of the overworld, mid-height elsewhere.
                    quarts.push([bx.div_euclid(4) as i32, 16, bz.div_euclid(4) as i32]);
                }
            }
            let mut names = g.w.noise_biomes(&quarts);
            // Block 64 can be inside the ground, where the cave biomes live: a map shows what
            // is on the surface, so those columns are asked again from higher up.
            for y in [40, 80] {
                let caves: Vec<usize> = (0..names.len())
                    .filter(|&i| names[i].as_deref().is_some_and(m::is_cave_biome))
                    .collect();
                if caves.is_empty() {
                    break;
                }
                let up: Vec<[i32; 3]> = caves.iter().map(|&i| [quarts[i][0], y, quarts[i][2]]).collect();
                for (&i, name) in caves.iter().zip(g.w.noise_biomes(&up)) {
                    names[i] = name;
                }
            }
            Output::Tile(names)
        }
        Job::Region(key) => {
            let mut out = Vec::new();
            for cz in key.rz * m::REGION..(key.rz + 1) * m::REGION {
                for cx in key.rx * m::REGION..(key.rx + 1) * m::REGION {
                    for s in g.w.structure_starts(ChunkPos::new(cx, cz)) {
                        // Where `/locate` points: the start's chunk (its centre, not the
                        // centre of the pieces, which can lie chunks away).
                        let mut cat = m::category(&s.name);
                        let mut name = s.name.clone();
                        if cat == m::END_CITY {
                            // A city with a ship has the elytra in the ship's item frame.
                            let ship = s.pieces.iter().any(is_ship_piece);
                            if ship {
                                cat = m::END_SHIP;
                                name = "minecraft:end_ship_with_elytra".into();
                            } else {
                                name = "minecraft:end_city_without_ship".into();
                            }
                        }
                        out.push(Marker {
                            cat: cat as u8,
                            x: s.chunk.x * 16 + 8,
                            z: s.chunk.z * 16 + 8,
                            name,
                        });
                    }
                }
            }
            Output::Region(out)
        }
        Job::Strongholds(_) => {
            let rings = g.w.ring_positions("minecraft:strongholds").unwrap_or_default();
            Output::Strongholds(rings.iter().map(|c| [c.x * 16 + 8, c.z * 16 + 8]).collect())
        }
        Job::Spawn(_) => Output::Spawn(g.g.spawn_position().unwrap_or([0, 0])),
    }
}

fn finish(job: Job, out: Output) {
    let mut s = lock();
    let c = &mut s.cache;
    c.trim();
    match (job, out) {
        (Job::Tile(key), Output::Tile(names)) => {
            let ids = names
                .iter()
                .map(|n| n.as_deref().map_or(UNKNOWN, |n| c.intern(n)))
                .collect();
            c.tiles.insert(key, ids);
        }
        (Job::Region(key), Output::Region(markers)) => {
            c.regions.insert(key, markers);
        }
        (Job::Strongholds(seed), Output::Strongholds(list)) => {
            c.strongholds.insert(seed, list);
        }
        (Job::Spawn(seed), Output::Spawn(at)) => {
            c.spawn.insert(seed, at);
        }
        _ => {}
    }
}

/// A job that cannot run still leaves an (empty) result, so it is not tried again every wake.
fn fail(job: Job, why: String) {
    let mut s = lock();
    let c = &mut s.cache;
    c.failed = Some(why);
    match job {
        Job::Tile(key) => {
            c.tiles.insert(key, vec![UNKNOWN; (TILE * TILE) as usize]);
        }
        Job::Region(key) => {
            c.regions.insert(key, Vec::new());
        }
        Job::Strongholds(seed) => {
            c.strongholds.insert(seed, Vec::new());
        }
        Job::Spawn(seed) => {
            c.spawn.insert(seed, [0, 0]);
        }
    }
}
