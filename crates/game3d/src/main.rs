//! Econoplex 3D view: a first prototype for the graphics/UI subproject (see
//! docs/design/graphics-ui.md). It builds a world from the seed with
//! `sim_core::worldgen` and shows it as a stylised 3D landscape: procedural
//! terrain coloured by biome, rivers, scattered trees, and each town as a cluster
//! of low-poly houses, under a procedural sky (shader step 01) with distance fog
//! and bloom. It only reads the scenario; the simulation itself does not run yet.
//!
//! econoplex3d [--seed S] [--towns N] [--hour H] [--pbr] [--terrain-view N] [--grid-step K]
//!     [--fog-view N] [--no-fog] [--days N] [--speed 0-8] [--paused] [--trade] [--culture] [--product-space] [--bloom-view] [--focus-town I] [--focus X,Z] [--view-from YAW,PITCH,DIST]
//!     [--no-ui] [--no-lod] [--lod-view] [--fps] [--render-scale F] [--shot FILE] [--quit-after S]
//!     [--weather clear|cloudy|rain|storm|snow|fog] [--season spring|summer|autumn|winter]
//!     [--preset FILE]
//! econoplex3d --playground [--hour H] [--view N] [--toon-view N] [--pbr] [--face-sun] [--shot FILE]
//!     (shader playground)

use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap};
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use sim_core::config::Scenario;
use sim_core::terrain::Terrain;
use std::path::PathBuf;

mod bake;
mod bloom;
mod buildings;
mod camera;
mod crowd;
mod crowd_compute;
mod env;
mod fog;
mod hud;
mod lights;
mod live;
mod nature;
mod outline;
mod overlay;
mod playground;
mod precip;
mod render_scale;
mod roads;
mod sky;
mod terrain;
mod toon;
mod ui;

#[derive(Resource, Clone)]
struct Args {
    /// Start view: focus town index, and optional yaw, pitch, distance.
    focus_town: usize,
    view_from: Option<(f32, f32, f32)>,
    /// Start focus on the ground (3D x, z), overrides the town.
    focus: Option<(f32, f32)>,
    /// Start with the bloom debug view (the glow alone).
    bloom_view: bool,
    seed: u64,
    towns: u32,
    /// Overrides of models.ron (`--preset FILE`).
    sets: Vec<(String, String)>,
}

/// Bevy's asset plugin, pointed at `crates/game3d/assets` (shaders), so the
/// shaders load and hot-reload whether the binary runs via `cargo run` or
/// directly from `target/`.
fn asset_plugin() -> AssetPlugin {
    let dev = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let file_path = if dev.exists() {
        dev.to_string_lossy().into_owned()
    } else {
        "assets".to_string()
    };
    AssetPlugin {
        file_path,
        ..default()
    }
}

/// Whether outlines start off (`--no-outlines`).
#[derive(Resource)]
struct OutlineStart(bool);

fn apply_outline_start(mut commands: Commands, start: Res<OutlineStart>) {
    if start.0 {
        commands.insert_resource(outline::OutlinesOff);
    }
}

/// Where to save a screenshot (then quit), if anywhere.
#[derive(Resource)]
struct ShotPath(Option<PathBuf>);

#[derive(Resource)]
struct World3d {
    scenario: Scenario,
    terrain: Terrain,
}

/// World units per 3D unit (the sim's map is about 2000 units across).
const SCALE: f32 = 10.0;

fn main() {
    let mut args = Args {
        seed: 7,
        towns: 5,
        focus_town: 0,
        view_from: None,
        focus: None,
        bloom_view: false,
        sets: Vec::new(),
    };
    let (mut shot, mut hour, mut view, mut playground, mut face_sun) =
        (None, None, None, false, false);
    let mut toon_settings = toon::ToonSettings::default();
    let mut terrain_settings = terrain::TerrainSettings::default();
    let mut fog_settings = fog::FogSettings::default();
    // The live simulation: fast-forward days, start speed (0-4), paused.
    let (mut days, mut speed, mut paused) = (0u64, live::DEFAULT_SPEED, false);
    let mut no_outlines = false;
    let mut overlay = overlay::Overlay::Off;
    let mut no_ui = false;
    let mut no_lod = false;
    let mut lod_view = false;
    let mut fps = false;
    let mut quit_after: Option<f32> = None;
    let mut render_scale = 1.0f32;
    let mut env_override = env::EnvOverride::default();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--seed" => args.seed = it.next().and_then(|v| v.parse().ok()).unwrap_or(7),
            "--preset" => {
                if let Some(path) = it.next() {
                    let sets = sim_core::read_preset(std::path::Path::new(&path))
                        .expect("cannot read the preset");
                    args.sets.extend(sets);
                }
            }
            "--towns" => args.towns = it.next().and_then(|v| v.parse().ok()).unwrap_or(5),
            "--shot" => shot = it.next().map(PathBuf::from),
            "--hour" => hour = it.next().and_then(|v| v.parse().ok()),
            "--view" => view = it.next().and_then(|v| v.parse().ok()),
            "--playground" => playground = true,
            "--face-sun" => face_sun = true,
            "--toon-view" => {
                toon_settings.debug = it.next().and_then(|v| v.parse().ok()).unwrap_or(0).min(6)
            }
            "--pbr" => toon_settings.on = false,
            "--no-rim" => toon_settings.rim = false,
            "--no-outlines" => no_outlines = true,
            "--no-ui" => no_ui = true,
            "--no-lod" => no_lod = true,
            "--lod-view" => lod_view = true,
            "--fps" => fps = true,
            "--quit-after" => quit_after = it.next().and_then(|v| v.parse().ok()),
            "--render-scale" => {
                render_scale = it.next().and_then(|v| v.parse().ok()).unwrap_or(1.0)
            }
            "--weather" => {
                let name = it.next().unwrap_or_default();
                env_override.weather = env::Preset::parse(&name);
                if env_override.weather.is_none() {
                    eprintln!("unknown weather `{name}`");
                }
            }
            "--season" => {
                let name = it.next().unwrap_or_default();
                env_override.season = env::SEASON_NAMES
                    .iter()
                    .position(|s| *s == name)
                    .map(|i| i as u32);
            }
            "--trade" => overlay = overlay::Overlay::Trade,
            "--culture" => overlay = overlay::Overlay::Culture,
            "--product-space" => overlay = overlay::Overlay::ProductSpace,
            "--bloom-view" => args.bloom_view = true,
            "--terrain-view" => {
                terrain_settings.debug = it.next().and_then(|v| v.parse().ok()).unwrap_or(0).min(4)
            }
            "--fog-view" => {
                fog_settings.debug = it.next().and_then(|v| v.parse().ok()).unwrap_or(0).min(4)
            }
            "--no-fog" => fog_settings.on = false,
            "--focus-town" => args.focus_town = it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            "--focus" => {
                let v: Vec<f32> = it
                    .next()
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|x| x.parse().ok())
                    .collect();
                if let [x, z] = v[..] {
                    args.focus = Some((x, z));
                }
            }
            "--view-from" => {
                // yaw,pitch,distance (degrees, degrees, world units)
                let v: Vec<f32> = it
                    .next()
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|x| x.parse().ok())
                    .collect();
                if let [y, p, d] = v[..] {
                    args.view_from = Some((y.to_radians(), p.to_radians(), d));
                }
            }
            "--days" => days = it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            "--speed" => {
                speed = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(live::DEFAULT_SPEED)
            }
            "--paused" => paused = true,
            "--grid-step" => {
                terrain_settings.step = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1)
                    .clamp(1, 4)
            }
            other => eprintln!("ignoring unknown argument `{other}`"),
        }
    }
    if playground {
        playground::run(
            hour,
            view,
            face_sun,
            toon_settings,
            fog_settings,
            no_outlines,
            shot,
        );
        return;
    }
    let mut clock = sky::SkyClock::default();
    if let Some(h) = hour {
        // A fixed hour: set by hand, not the simulation's (`C` follows it).
        clock.hour = h.rem_euclid(24.0);
        clock.free = true;
    }
    let paths = sim_core::AssetPaths::discover();
    let (db, params, mut scenario) =
        sim_core::load_with(&paths, &args.sets).expect("cannot load assets");
    scenario.seed = args.seed;
    scenario.generate = Some(sim_core::worldgen::GenSpec {
        towns: args.towns,
        ..Default::default()
    });
    let scenario = sim_core::worldgen::resolve(&scenario, &db);
    let terrain = Terrain::build(&scenario);
    // The simulation of this world, run ahead by `--days`, then on its own thread.
    let mut sim = sim_core::Simulation::new(db, params, &scenario);
    for _ in 0..days {
        sim.step();
    }
    let space = sim_core::view::ProductSpace::new(sim.world.resource::<sim_core::Db>());
    let live = live::LiveSim::start(sim, speed, paused);

    let mut app = App::new();
    if fps {
        // Frame times in the log, without waiting for the screen's refresh.
        app.add_plugins((
            bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
            bevy::diagnostic::LogDiagnosticsPlugin::default(),
        ));
    }
    app.insert_resource(crowd::LodSettings {
        on: !no_lod,
        show: lod_view,
        ..default()
    })
    .add_plugins(DefaultPlugins.set(asset_plugin()).set(WindowPlugin {
        primary_window: Some(Window {
            title: "Econoplex 3D (prototype)".into(),
            resolution: (1600, 900).into(),
            present_mode: if fps {
                bevy::window::PresentMode::AutoNoVsync
            } else {
                bevy::window::PresentMode::AutoVsync
            },
            ..default()
        }),
        ..default()
    }))
    .add_plugins(PassTimings(fps))
    .insert_resource(ClearColor(Color::srgb(0.62, 0.74, 0.86)))
    .insert_resource(DirectionalLightShadowMap { size: 2048 })
    .insert_resource(World3d { scenario, terrain })
    .insert_resource(OutlineStart(no_outlines))
    .insert_resource(ui::UiHidden(no_ui))
    .insert_resource(live)
    .insert_resource(ui::Space(space))
    .insert_resource(render_scale::RenderScale(render_scale))
    .insert_resource(args)
    .insert_resource(ShotPath(shot))
    .insert_resource(QuitAfter(quit_after))
    .add_plugins((
        sky::SkyPlugin,
        toon::ToonPlugin,
        terrain::TerrainPlugin,
        fog::FogPlugin,
        camera::WorldCameraPlugin,
        live::LivePlugin,
        outline::OutlinePlugin,
        nature::NaturePlugin,
        crowd::CrowdPlugin,
        crowd_compute::CrowdComputePlugin,
        ui::PanelsPlugin,
        overlay::OverlayPlugin,
        bloom::BloomPlugin,
        render_scale::RenderScalePlugin,
        roads::RoadsPlugin,
    ))
    .add_plugins((precip::PrecipPlugin, lights::LightsPlugin))
    .insert_resource(env_override)
    .insert_resource(clock)
    .insert_resource(toon_settings)
    .insert_resource(terrain_settings)
    .insert_resource(fog_settings)
    .add_systems(PreStartup, apply_outline_start)
    .insert_resource(overlay)
    .add_systems(Startup, setup)
    .add_systems(Update, (shoot, quit_after_secs, hud::update_help))
    .run();
}

/// Cheap deterministic value noise in [0, 1].
fn noise(seed: u64, x: f32, y: f32) -> f32 {
    let lattice = |i: i64, j: i64| {
        let mut h = seed
            ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        h ^= h >> 31;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 29;
        (h >> 40) as f32 / (1u64 << 24) as f32
    };
    let (i, j) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy) = (s(fx), s(fy));
    let top = lattice(i, j) * (1.0 - sx) + lattice(i + 1, j) * sx;
    let bottom = lattice(i, j + 1) * (1.0 - sx) + lattice(i + 1, j + 1) * sx;
    top * (1.0 - sy) + bottom * sy
}

/// Height of the ground at a world position: hills from the sim's travel cost,
/// gentle rolling noise, river beds sunk.
/// The terrain surface's height at `p`, as the terrain mesh draws it: the
/// heights at the four cell corners around `p`, interpolated. (`height` itself
/// is one value per cell; the mesh is built from it at the corners.)
fn ground(t: &Terrain, p: (f32, f32)) -> f32 {
    let fx = (p.0 - t.origin.0) / t.cell;
    let fy = (p.1 - t.origin.1) / t.cell;
    let (i, j) = (fx.floor(), fy.floor());
    let (u, v) = (fx - i, fy - j);
    let corner = |a: f32, b: f32| height(t, (t.origin.0 + a * t.cell, t.origin.1 + b * t.cell));
    let bottom = corner(i, j) * (1.0 - u) + corner(i + 1.0, j) * u;
    let top = corner(i, j + 1.0) * (1.0 - u) + corner(i + 1.0, j + 1.0) * u;
    bottom * (1.0 - v) + top * v
}

fn height(t: &Terrain, p: (f32, f32)) -> f32 {
    let Some(c) = t.cell_at(p) else { return 0.0 };
    if t.water[c] {
        return -1.2;
    }
    let hill = if t.cost[c].is_finite() {
        (t.cost[c] - 1.0).max(0.0)
    } else {
        0.0
    };
    let roll = noise(11, p.0 / 300.0, p.1 / 300.0) * 1.5;
    hill * 9.0 + roll
}

#[allow(clippy::too_many_arguments)]
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut toons: ResMut<Assets<toon::ToonMaterial>>,
    mut terrains: ResMut<Assets<terrain::TerrainMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut trees: ResMut<Assets<nature::TreeMaterial>>,
    mut grass: ResMut<Assets<nature::GrassMaterial>>,
    mut buffers: ResMut<Assets<bevy::render::storage::ShaderBuffer>>,
    world: Res<World3d>,
    args: Res<Args>,
) {
    let t = &world.terrain;
    let s = &world.scenario;
    // --- terrain (shader step 03): heights and biome weights per cell corner
    // go into two textures; the GPU shapes and paints a flat grid from them.
    let (w, h) = (t.width + 1, t.height + 1);
    let mut heights = Vec::with_capacity(w * h);
    let mut biomes = Vec::with_capacity(w * h);
    for j in 0..h {
        for i in 0..w {
            let p = (
                t.origin.0 + i as f32 * t.cell,
                t.origin.1 + j as f32 * t.cell,
            );
            let y = height(t, p);
            heights.push(y);
            // Fields in a ring around each town, water in river beds, meadow
            // elsewhere. The shader adds rock on steep and high ground.
            let near = s.towns.iter().any(|tw| {
                let d = ((tw.position.0 - p.0).powi(2) + (tw.position.1 - p.1).powi(2)).sqrt();
                d > 90.0 && d < 260.0
            });
            biomes.push(if y < -0.5 {
                [0.0, 0.0, 1.0, 0.0]
            } else if near {
                [0.0, 1.0, 0.0, 0.0]
            } else {
                [1.0, 0.0, 0.0, 0.0]
            });
        }
    }
    let heights = terrain::spawn_terrain(
        &mut commands,
        &mut meshes,
        &mut images,
        &mut terrains,
        &terrain::TerrainData {
            width: w,
            depth: h,
            origin: Vec2::new(t.origin.0 / SCALE, -t.origin.1 / SCALE),
            cell: t.cell / SCALE,
            heights,
            biomes,
        },
    );
    // The walking crowd (step 07) reads the same heightmap on the GPU.
    commands.insert_resource(crowd::CrowdTerrain {
        heights,
        grid: Vec4::new(t.origin.0 / SCALE, -t.origin.1 / SCALE, t.cell / SCALE, 1.0),
        dims: Vec4::new(w as f32, h as f32, 0.0, 0.0),
    });
    // --- water plane
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(400.0, 400.0))),
        // Opaque toon water, so it gets the same fog as everything else.
        MeshMaterial3d(toons.add(toon::toon(Color::srgb(0.30, 0.52, 0.70)))),
        Transform::from_xyz(0.0, -0.6, 0.0),
    ));
    // --- roads between the towns (roads.rs); grass and trees keep off them
    let town_pos: Vec<(f32, f32)> = s.towns.iter().map(|tw| tw.position).collect();
    let road_cells = roads::spawn_roads(&mut commands, &mut meshes, &mut toons, t, &town_pos);
    // --- trees: soft cones scattered on meadows and hills
    // Crowns sway in the wind (shader step 06, sway.wesl); trunks stand still.
    // All trees are baked into one mesh per chunk (nature.rs).
    let lin = |c: Color| c.to_linear().to_f32_array();
    let crowns = [
        lin(Color::srgb(0.22, 0.42, 0.22)),
        lin(Color::srgb(0.30, 0.50, 0.24)),
        lin(Color::srgb(0.18, 0.36, 0.24)),
    ];
    let mut forest = Vec::new();
    for c in 0..t.width * t.height {
        if road_cells.contains(&c) {
            continue;
        }
        let p = t.center(c);
        if t.water[c] || noise(5, p.0 / 160.0, p.1 / 160.0) < 0.62 {
            continue;
        }
        if s.towns.iter().any(|tw| {
            ((tw.position.0 - p.0).powi(2) + (tw.position.1 - p.1).powi(2)).sqrt() < 280.0
        }) {
            continue;
        }
        let jitter = noise(9, p.0, p.1);
        if jitter < 0.45 {
            continue;
        }
        let y = height(t, p);
        let at = Vec3::new(p.0 / SCALE + jitter - 0.5, y, -p.1 / SCALE);
        forest.push(nature::TreeSpec {
            root: at,
            scale: 0.8 + jitter * 0.6,
            crown: crowns[(c * 7) % 3],
        });
    }
    let chunks = nature::spawn_forest(
        &mut commands,
        &mut meshes,
        &mut trees,
        &forest,
        lin(Color::srgb(0.40, 0.28, 0.18)),
        25.0,
    );
    info!("trees: {} in {} chunks", forest.len(), chunks);
    // --- grass (shader step 06): blades over the meadows, away from towns,
    // rivers and steep slopes; a few per square unit, jittered by a hash.
    let mut blades = Vec::new();
    let mut rng = 0x9E37_79B9_7F4A_7C15u64 ^ world.scenario.seed;
    let mut rand = move || {
        rng = rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    };
    let cell3 = t.cell / SCALE;
    let per_cell = (cell3 * cell3 * 14.0) as usize;
    for c in 0..t.width * t.height {
        if road_cells.contains(&c) {
            continue;
        }
        let p = t.center(c);
        if t.water[c] {
            continue;
        }
        let near_town = s.towns.iter().any(|tw| {
            ((tw.position.0 - p.0).powi(2) + (tw.position.1 - p.1).powi(2)).sqrt() < 260.0
        });
        if near_town {
            continue;
        }
        // Not on steep ground: compare heights across the cell.
        let d = t.cell * 0.5;
        let slope = ((height(t, (p.0 + d, p.1)) - height(t, (p.0 - d, p.1))).abs()
            + (height(t, (p.0, p.1 + d)) - height(t, (p.0, p.1 - d))).abs())
            / cell3;
        if slope > 0.9 {
            continue;
        }
        for _ in 0..per_cell {
            let q = (p.0 + (rand() - 0.5) * t.cell, p.1 + (rand() - 0.5) * t.cell);
            blades.push(nature::Blade {
                pos: Vec3::new(q.0 / SCALE, height(t, q), -q.1 / SCALE),
                height: 0.35 + rand() * 0.35,
                lean: rand() * std::f32::consts::TAU,
                tint: rand(),
            });
        }
    }
    let chunks = nature::spawn_grass(
        &mut commands,
        &mut meshes,
        &mut buffers,
        &mut grass,
        &blades,
        25.0,
    );
    info!("grass: {} blades in {} chunks", blades.len(), chunks);
    // --- towns: buildings and houses come from the live simulation (live.rs)
    // --- light: the sun with soft shadows; the sky plugin points it and sets its
    // colour and the ambient light from the time of day.
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::default(),
        // Three cascades to 200 units: beyond that the fog hides shadows anyway,
        // and every cascade draws every shadow caster again.
        CascadeShadowConfigBuilder {
            num_cascades: 3,
            maximum_distance: 200.0,
            ..default()
        }
        .build(),
    ));
    // --- camera: a strategy camera over a town (camera.rs), with bloom
    let focus = s
        .towns
        .get(args.focus_town)
        .or(s.towns.first())
        .map_or(Vec3::ZERO, |tw| {
            Vec3::new(tw.position.0 / SCALE, 0.0, -tw.position.1 / SCALE)
        });
    let focus = args.focus.map_or(focus, |(x, z)| {
        Vec3::new(x, height(t, (x * SCALE, -z * SCALE)), z)
    });
    let (yaw, pitch, distance) = args.view_from.unwrap_or((-0.67, 0.49, 102.0));
    let min = Vec2::new(
        t.origin.0 / SCALE,
        -(t.origin.1 + t.height as f32 * t.cell) / SCALE,
    );
    let max = Vec2::new(
        (t.origin.0 + t.width as f32 * t.cell) / SCALE,
        -t.origin.1 / SCALE,
    );
    info!(
        "map from {min} to {max} (3D units); towns at {:?}",
        s.towns
            .iter()
            .map(|tw| (tw.position.0 / SCALE, -tw.position.1 / SCALE))
            .collect::<Vec<_>>()
    );
    let cam = camera::WorldCamera::new(
        focus,
        yaw,
        pitch,
        distance,
        Vec4::new(min.x, min.y, max.x, max.y),
    );
    // HDR, with our hand-written bloom (shader step 08, bloom.rs).
    commands.spawn((
        Camera3d::default(),
        cam.transform(),
        cam,
        bevy::camera::Hdr,
        bloom::BloomSettings {
            debug: if args.bloom_view { 1.0 } else { 0.0 },
            ..default()
        },
    ));
    hud::spawn_help(
        &mut commands,
        "Econoplex 3D (prototype)",
        "WASD pan, Q/E or right-drag rotate, wheel zoom, Home reset, V overlays (trade, culture, product space), B bloom, N glow only",
    );
}

#[derive(Default)]
struct Shot {
    frames: u32,
}

/// With `--shot FILE`: save a screenshot after 60 frames, then quit.
fn shoot(
    mut commands: Commands,
    args: Res<ShotPath>,
    mut state: Local<Shot>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = &args.0 else { return };
    state.frames += 1;
    if state.frames == 60 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
    }
    if state.frames == 120 {
        exit.write(AppExit::Success);
    }
}

/// With `--quit-after S`: quit cleanly after S seconds of running, so a
/// profiling run (`--features profile`, docs/profiling.md) has a fixed length
/// and its trace file is written out.
#[derive(Resource)]
struct QuitAfter(Option<f32>);

fn quit_after_secs(time: Res<Time<Real>>, quit: Res<QuitAfter>, mut exit: MessageWriter<AppExit>) {
    if quit.0.is_some_and(|s| time.elapsed_secs() >= s) {
        exit.write(AppExit::Success);
    }
}

/// With `--fps`: GPU and CPU time per render pass in the log (timestamp
/// queries). Added after the render plugin, which it needs.
struct PassTimings(bool);

impl Plugin for PassTimings {
    fn build(&self, app: &mut App) {
        if self.0 {
            app.add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin)
                .add_systems(Update, count_draws);
        }
    }
}

/// With `--fps`: once, after a few seconds, how many things of each kind are
/// drawn (each mesh entity costs CPU time every frame, in the shadow passes too).
#[allow(clippy::type_complexity)]
fn count_draws(
    time: Res<Time>,
    mut done: Local<bool>,
    all: Query<(), With<Mesh3d>>,
    toon: Query<(), With<MeshMaterial3d<toon::ToonMaterial>>>,
    tree: Query<(), With<MeshMaterial3d<nature::TreeMaterial>>>,
    outline: Query<(), With<MeshMaterial3d<outline::OutlineMaterial>>>,
    shadowless: Query<(), (With<Mesh3d>, With<bevy::light::NotShadowCaster>)>,
) {
    if *done || time.elapsed_secs() < 8.0 {
        return;
    }
    *done = true;
    info!(
        "mesh entities: {} (toon {}, tree crowns {}, outline hulls {}, not casting shadows {})",
        all.iter().count(),
        toon.iter().count(),
        tree.iter().count(),
        outline.iter().count(),
        shadowless.iter().count()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first snapshot of the simulation shows exactly the buildings the
    /// scenario gives each town (workshop, mill, bakery everywhere; furnace and
    /// smithy where the land has ore).
    #[test]
    fn every_town_gets_its_buildings() {
        let paths = sim_core::AssetPaths::discover();
        let (db, params, mut scenario) = sim_core::load(&paths).unwrap();
        scenario.seed = 23;
        scenario.generate = Some(sim_core::worldgen::GenSpec {
            towns: 5,
            ..Default::default()
        });
        let scenario = sim_core::worldgen::resolve(&scenario, &db);
        let sim = sim_core::Simulation::new(db, params, &scenario);
        let snap = sim_core::view::ViewSnapshot::capture(&sim.world, &mut Default::default());
        for (town, spec) in snap.towns.iter().zip(&scenario.towns) {
            let mut kinds: Vec<&str> = town.buildings.iter().map(|b| b.kind.as_str()).collect();
            kinds.sort();
            let mut want: Vec<&str> = spec.buildings.iter().map(String::as_str).collect();
            want.sort();
            assert_eq!(kinds, want, "town {}", spec.name);
            for starter in ["workshop", "mill", "bakery"] {
                assert!(
                    kinds.contains(&starter),
                    "town {} lacks a {starter}",
                    spec.name
                );
            }
        }
    }

    #[test]
    fn houses_follow_population() {
        assert_eq!(live::houses_for(0), 12);
        assert_eq!(live::houses_for(600), 100);
    }
}
