//! Econoplex 3D view: a first prototype for the graphics/UI subproject (see
//! docs/design/graphics-ui.md). It builds a world from the seed with
//! `sim_core::worldgen` and shows it as a stylised 3D landscape: procedural
//! terrain coloured by biome, rivers, scattered trees, and each town as a cluster
//! of low-poly houses, under a procedural sky (shader step 01) with distance fog
//! and bloom. It only reads the scenario; the simulation itself does not run yet.
//!
//! econoplex3d [--seed S] [--towns N] [--hour H] [--pbr] [--terrain-view N] [--grid-step K]
//!     [--fog-view N] [--no-fog] [--days N] [--speed 0-4] [--paused] [--trade] [--bloom-view] [--focus-town I] [--focus X,Z] [--view-from YAW,PITCH,DIST]
//!     [--no-ui] [--shot FILE]
//! econoplex3d --playground [--hour H] [--view N] [--toon-view N] [--pbr] [--face-sun] [--shot FILE]
//!     (shader playground)

use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap};
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use sim_core::config::Scenario;
use sim_core::terrain::Terrain;
use std::path::PathBuf;

mod bloom;
mod buildings;
mod camera;
mod crowd;
mod crowd_compute;
mod fog;
mod hud;
mod live;
mod nature;
mod outline;
mod overlay;
mod playground;
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
    };
    let (mut shot, mut hour, mut view, mut playground, mut face_sun) =
        (None, None, None, false, false);
    let mut toon_settings = toon::ToonSettings::default();
    let mut terrain_settings = terrain::TerrainSettings::default();
    let mut fog_settings = fog::FogSettings::default();
    // The live simulation: fast-forward days, start speed (0-4), paused.
    let (mut days, mut speed, mut paused) = (0u64, 0usize, false);
    let mut no_outlines = false;
    let mut trade = false;
    let mut no_ui = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--seed" => args.seed = it.next().and_then(|v| v.parse().ok()).unwrap_or(7),
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
            "--trade" => trade = true,
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
            "--speed" => speed = it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
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
        clock.hour = h.rem_euclid(24.0);
    }
    let paths = sim_core::AssetPaths::discover();
    let (db, params, mut scenario) = sim_core::load(&paths).expect("cannot load assets");
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
    let live = live::LiveSim::start(sim, speed, paused);

    App::new()
        .add_plugins(DefaultPlugins.set(asset_plugin()).set(WindowPlugin {
            primary_window: Some(Window {
                title: "Econoplex 3D (prototype)".into(),
                resolution: (1600, 900).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.62, 0.74, 0.86)))
        .insert_resource(DirectionalLightShadowMap { size: 4096 })
        .insert_resource(World3d { scenario, terrain })
        .insert_resource(OutlineStart(no_outlines))
        .insert_resource(ui::UiHidden(no_ui))
        .insert_resource(live)
        .insert_resource(args)
        .insert_resource(ShotPath(shot))
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
        ))
        .insert_resource(clock)
        .insert_resource(toon_settings)
        .insert_resource(terrain_settings)
        .insert_resource(fog_settings)
        .add_systems(PreStartup, apply_outline_start)
        .insert_resource(if trade {
            overlay::Overlay::Trade
        } else {
            overlay::Overlay::Off
        })
        .add_systems(Startup, setup)
        .add_systems(Update, (shoot, hud::update_help))
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
    // --- trees: soft cones scattered on meadows and hills
    let trunk = meshes.add(Cylinder::new(0.12, 0.8));
    let crown = meshes.add(Cone::new(0.7, 2.0));
    let wood = toons.add(toon::toon(Color::srgb(0.40, 0.28, 0.18)));
    // Crowns sway in the wind (shader step 06, sway.wesl); trunks stand still.
    let leaves = [
        trees.add(nature::tree_material(Color::srgb(0.22, 0.42, 0.22))),
        trees.add(nature::tree_material(Color::srgb(0.30, 0.50, 0.24))),
        trees.add(nature::tree_material(Color::srgb(0.18, 0.36, 0.24))),
    ];
    for c in 0..t.width * t.height {
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
        let k = (c * 7) % 3;
        commands.spawn((
            Mesh3d(trunk.clone()),
            MeshMaterial3d(wood.clone()),
            Transform::from_translation(at + Vec3::Y * 0.4),
        ));
        commands.spawn((
            Mesh3d(crown.clone()),
            MeshMaterial3d(leaves[k].clone()),
            Transform::from_translation(at + Vec3::Y * 1.6)
                .with_scale(Vec3::splat(0.8 + jitter * 0.6)),
        ));
    }
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
        CascadeShadowConfigBuilder {
            maximum_distance: 260.0,
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
        "WASD pan, Q/E or right-drag rotate, wheel zoom, Home reset, V trade overlay, B bloom, N glow only",
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
