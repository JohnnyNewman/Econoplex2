//! Econoplex 3D view: a first prototype for the graphics/UI subproject (see
//! docs/design/graphics-ui.md). It builds a world from the seed with
//! `sim_core::worldgen` and shows it as a stylised 3D landscape: procedural
//! terrain coloured by biome, rivers, scattered trees, and each town as a cluster
//! of low-poly houses, under a low sun with distance fog and bloom. It only reads
//! the scenario; the simulation itself does not run yet.
//!
//! econoplex3d [--seed S] [--towns N] [--shot FILE]

use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use sim_core::config::Scenario;
use sim_core::terrain::Terrain;
use std::path::PathBuf;

#[derive(Resource, Clone)]
struct Args {
    seed: u64,
    towns: u32,
    shot: Option<PathBuf>,
}

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
        shot: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--seed" => args.seed = it.next().and_then(|v| v.parse().ok()).unwrap_or(7),
            "--towns" => args.towns = it.next().and_then(|v| v.parse().ok()).unwrap_or(5),
            "--shot" => args.shot = it.next().map(PathBuf::from),
            _ => {}
        }
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
        .add_plugins(DefaultPlugins.set(WindowPlugin {
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
        .add_systems(Startup, setup)
        .add_systems(Update, shoot)
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

fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    world: Res<World3d>,
) {
    let t = &world.terrain;
    let s = &world.scenario;
    // --- terrain: one vertex per cell corner, coloured by biome
    let (w, h) = (t.width + 1, t.height + 1);
    let mut pos = Vec::with_capacity(w * h);
    let mut col = Vec::with_capacity(w * h);
    for j in 0..h {
        for i in 0..w {
            let p = (
                t.origin.0 + i as f32 * t.cell,
                t.origin.1 + j as f32 * t.cell,
            );
            let y = height(t, p);
            pos.push([p.0 / SCALE, y, -p.1 / SCALE]);
            let n = noise(3, p.0 / 120.0, p.1 / 120.0);
            let meadow = lerp([0.45, 0.62, 0.30], [0.62, 0.70, 0.36], n);
            let rgb = if y < -0.5 {
                [0.25, 0.48, 0.66]
            } else if y > 6.0 {
                lerp(meadow, [0.62, 0.58, 0.48], ((y - 6.0) / 6.0).min(1.0))
            } else {
                // Fields near towns: a patchwork of crops.
                let near = s.towns.iter().any(|tw| {
                    let d = ((tw.position.0 - p.0).powi(2) + (tw.position.1 - p.1).powi(2)).sqrt();
                    d > 90.0 && d < 260.0
                });
                if near {
                    let k = ((p.0 / 60.0).floor() as i64 * 7 + (p.1 / 60.0).floor() as i64 * 13)
                        .rem_euclid(3);
                    [[0.86, 0.76, 0.36], [0.55, 0.68, 0.28], [0.74, 0.62, 0.34]][k as usize]
                } else {
                    meadow
                }
            };
            col.push([rgb[0], rgb[1], rgb[2], 1.0]);
        }
    }
    let mut idx = Vec::new();
    for j in 0..h - 1 {
        for i in 0..w - 1 {
            let a = (j * w + i) as u32;
            let (b, c, d) = (a + 1, a + w as u32, a + w as u32 + 1);
            // Counter-clockwise seen from above (z is flipped: world y grows north, 3D -z).
            idx.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, Default::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    mesh.insert_indices(Indices::U32(idx));
    mesh.compute_smooth_normals();
    commands.spawn((
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.95,
            ..default()
        })),
    ));
    // --- water plane
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(400.0, 400.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.30, 0.55, 0.75, 0.85),
            perceptual_roughness: 0.2,
            alpha_mode: AlphaMode::Blend,
            ..default()
        })),
        Transform::from_xyz(0.0, -0.6, 0.0),
    ));
    // --- trees: soft cones scattered on meadows and hills
    let trunk = meshes.add(Cylinder::new(0.12, 0.8));
    let crown = meshes.add(Cone::new(0.7, 2.0));
    let wood = materials.add(Color::srgb(0.40, 0.28, 0.18));
    let leaves = [
        materials.add(Color::srgb(0.22, 0.42, 0.22)),
        materials.add(Color::srgb(0.30, 0.50, 0.24)),
        materials.add(Color::srgb(0.18, 0.36, 0.24)),
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
    let walls = materials.add(Color::srgb(0.88, 0.82, 0.70));
    let roofs = [
        materials.add(Color::srgb(0.66, 0.26, 0.20)),
        materials.add(Color::srgb(0.50, 0.32, 0.22)),
        materials.add(Color::srgb(0.36, 0.38, 0.46)),
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
    // --- light: low warm sun with soft shadows, cool sky light
    commands.spawn((
        DirectionalLight {
            illuminance: 12_000.0,
            color: Color::srgb(1.0, 0.93, 0.80),
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(-60.0, 50.0, 40.0).looking_at(Vec3::ZERO, Vec3::Y),
        CascadeShadowConfigBuilder {
            maximum_distance: 260.0,
            ..default()
        }
        .build(),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.70, 0.80, 1.0),
        brightness: 350.0,
        ..default()
    });
    // --- camera: tilted over the first town, with distance fog and bloom
    let focus = s.towns.first().map_or(Vec3::ZERO, |tw| {
        Vec3::new(tw.position.0 / SCALE, 0.0, -tw.position.1 / SCALE)
    });
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(focus + Vec3::new(-55.0, 48.0, 70.0))
            .looking_at(focus, Vec3::Y),
        DistanceFog {
            color: Color::srgba(0.66, 0.76, 0.86, 1.0),
            directional_light_color: Color::srgba(1.0, 0.92, 0.75, 0.5),
            directional_light_exponent: 20.0,
            falloff: FogFalloff::from_visibility_colors(
                700.0,
                Color::srgb(0.55, 0.62, 0.70),
                Color::srgb(0.85, 0.88, 0.92),
            ),
        },
        Bloom::NATURAL,
    ));
}

#[derive(Default)]
struct Shot {
    frames: u32,
}

fn shoot(
    mut commands: Commands,
    args: Res<Args>,
    mut state: Local<Shot>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = &args.shot else { return };
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
