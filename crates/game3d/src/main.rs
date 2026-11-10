//! Econoplex 3D view: a first prototype for the graphics/UI subproject (see
//! docs/design/graphics-ui.md). It builds a world from the seed with
//! `sim_core::worldgen` and shows it as a stylised 3D landscape: procedural
//! terrain coloured by biome, rivers, scattered trees, and each town as a cluster
//! of low-poly houses, under a procedural sky (shader step 01) with distance fog
//! and bloom. It only reads the scenario; the simulation itself does not run yet.
//!
//! econoplex3d [--seed S] [--towns N] [--hour H] [--pbr] [--terrain-view N] [--grid-step K]
//!     [--fog-view N] [--no-fog] [--focus-town I] [--view-from YAW,PITCH,DIST]
//!     [--shot FILE]
//! econoplex3d --playground [--hour H] [--view N] [--toon-view N] [--pbr] [--face-sun] [--shot FILE]
//!     (shader playground)

use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use sim_core::config::Scenario;
use sim_core::terrain::Terrain;
use std::path::PathBuf;

mod camera;
mod fog;
mod hud;
mod playground;
mod sky;
mod terrain;
mod toon;

#[derive(Resource, Clone)]
struct Args {
    /// Start view: focus town index, and optional yaw, pitch, distance.
    focus_town: usize,
    view_from: Option<(f32, f32, f32)>,
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
    };
    let (mut shot, mut hour, mut view, mut playground, mut face_sun) =
        (None, None, None, false, false);
    let mut toon_settings = toon::ToonSettings::default();
    let mut terrain_settings = terrain::TerrainSettings::default();
    let mut fog_settings = fog::FogSettings::default();
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
                toon_settings.debug = it.next().and_then(|v| v.parse().ok()).unwrap_or(0).min(4)
            }
            "--pbr" => toon_settings.on = false,
            "--terrain-view" => {
                terrain_settings.debug = it.next().and_then(|v| v.parse().ok()).unwrap_or(0).min(4)
            }
            "--fog-view" => {
                fog_settings.debug = it.next().and_then(|v| v.parse().ok()).unwrap_or(0).min(4)
            }
            "--no-fog" => fog_settings.on = false,
            "--focus-town" => args.focus_town = it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
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
        playground::run(hour, view, face_sun, toon_settings, fog_settings, shot);
        return;
    }
    let mut clock = sky::SkyClock::default();
    if let Some(h) = hour {
        clock.hour = h.rem_euclid(24.0);
    }
    let paths = sim_core::AssetPaths::discover();
    let (db, _params, mut scenario) = sim_core::load(&paths).expect("cannot load assets");
    scenario.seed = args.seed;
    scenario.generate = Some(sim_core::worldgen::GenSpec {
        towns: args.towns,
        ..Default::default()
    });
    let scenario = sim_core::worldgen::resolve(&scenario, &db);
    let terrain = Terrain::build(&scenario);

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
        .insert_resource(args)
        .insert_resource(ShotPath(shot))
        .add_plugins((
            sky::SkyPlugin,
            toon::ToonPlugin,
            terrain::TerrainPlugin,
            fog::FogPlugin,
            camera::WorldCameraPlugin,
        ))
        .insert_resource(clock)
        .insert_resource(toon_settings)
        .insert_resource(terrain_settings)
        .insert_resource(fog_settings)
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

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut toons: ResMut<Assets<toon::ToonMaterial>>,
    mut terrains: ResMut<Assets<terrain::TerrainMaterial>>,
    mut images: ResMut<Assets<Image>>,
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
    terrain::spawn_terrain(
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
    let leaves = [
        toons.add(toon::toon(Color::srgb(0.22, 0.42, 0.22))),
        toons.add(toon::toon(Color::srgb(0.30, 0.50, 0.24))),
        toons.add(toon::toon(Color::srgb(0.18, 0.36, 0.24))),
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
    // --- towns: houses around the centre, a hall in the middle
    let house = meshes.add(Cuboid::new(1.0, 0.8, 1.2));
    let roof = meshes.add(Cone::new(0.95, 0.7));
    let walls = toons.add(toon::toon(Color::srgb(0.88, 0.82, 0.70)));
    let roofs = [
        toons.add(toon::toon(Color::srgb(0.66, 0.26, 0.20))),
        toons.add(toon::toon(Color::srgb(0.50, 0.32, 0.22))),
        toons.add(toon::toon(Color::srgb(0.36, 0.38, 0.46))),
    ];
    let hall = meshes.add(Cuboid::new(2.4, 1.6, 2.4));
    for (ti, tw) in s.towns.iter().enumerate() {
        let n = (tw.population / 6).max(12);
        for k in 0..n {
            let a = k as f32 * 2.399_963 + ti as f32;
            let r = 1.8 + (k as f32).sqrt() * 1.05;
            let p = (
                tw.position.0 + r * SCALE * a.cos(),
                tw.position.1 + r * SCALE * a.sin(),
            );
            if t.cell_at(p).is_some_and(|c| t.water[c]) {
                continue;
            }
            let y = height(t, p);
            let at = Vec3::new(p.0 / SCALE, y, -p.1 / SCALE);
            let rot = Quat::from_rotation_y(a);
            commands.spawn((
                Mesh3d(house.clone()),
                MeshMaterial3d(walls.clone()),
                Transform::from_translation(at + Vec3::Y * 0.4).with_rotation(rot),
            ));
            commands.spawn((
                Mesh3d(roof.clone()),
                MeshMaterial3d(roofs[k as usize % 3].clone()),
                Transform::from_translation(at + Vec3::Y * 1.15).with_rotation(rot),
            ));
        }
        let y = height(t, tw.position);
        commands.spawn((
            Mesh3d(hall.clone()),
            MeshMaterial3d(roofs[0].clone()),
            Transform::from_xyz(tw.position.0 / SCALE, y + 0.8, -tw.position.1 / SCALE),
        ));
    }
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
    let (yaw, pitch, distance) = args.view_from.unwrap_or((-0.67, 0.49, 102.0));
    let min = Vec2::new(
        t.origin.0 / SCALE,
        -(t.origin.1 + t.height as f32 * t.cell) / SCALE,
    );
    let max = Vec2::new(
        (t.origin.0 + t.width as f32 * t.cell) / SCALE,
        -t.origin.1 / SCALE,
    );
    let cam = camera::WorldCamera::new(
        focus,
        yaw,
        pitch,
        distance,
        Vec4::new(min.x, min.y, max.x, max.y),
    );
    commands.spawn((Camera3d::default(), cam.transform(), cam, Bloom::NATURAL));
    hud::spawn_help(
        &mut commands,
        "Econoplex 3D (prototype)",
        "WASD pan, Q/E or right-drag rotate, wheel zoom, Home reset",
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
