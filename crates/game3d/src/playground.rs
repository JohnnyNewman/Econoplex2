//! The shader playground: one small scene for learning and trying shaders.
//!
//! `econoplex3d --playground [--hour H] [--view N] [--toon-view N] [--pbr] [--face-sun]
//! [--shot FILE]`
//!
//! - The sky (step 01) and a few test shapes on a ground plane. Later steps put
//!   their materials on the shapes (toon lighting, rim light, ...).
//! - Shaders hot-reload: edit `crates/game3d/assets/shaders/*.wesl`, save, and
//!   the scene updates without a restart.
//! - Debug views (`0`–`4`) show intermediate values of the shader as colours.
//! - Camera: drag with the left mouse button or use the arrow keys to orbit;
//!   the mouse wheel or `+` / `-` zooms.
//!
//! See docs/shaders/00-playground.md.

use crate::sky::{DebugTarget, SkyClock, SkyPlugin};
use crate::toon::{toon, ToonMaterial, ToonPlugin, ToonSettings};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;

/// Where the orbit camera looks from, around the origin.
#[derive(Resource)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
}

/// Marks the help text.
#[derive(Component)]
struct Help;

pub fn run(
    hour: Option<f32>,
    view: Option<u32>,
    face_sun: bool,
    toon_settings: ToonSettings,
    shot: Option<std::path::PathBuf>,
) {
    let mut clock = SkyClock::default();
    if let Some(h) = hour {
        clock.hour = h.rem_euclid(24.0);
    }
    if let Some(v) = view {
        clock.debug = v.min(4);
    }
    let clock_hour = clock.hour;
    App::new()
        .add_plugins(DefaultPlugins.set(crate::asset_plugin()).set(WindowPlugin {
            primary_window: Some(Window {
                title: "Econoplex shader playground".into(),
                resolution: (1280, 720).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((SkyPlugin, ToonPlugin))
        .insert_resource(clock)
        .insert_resource(toon_settings)
        .insert_resource(crate::ShotPath(shot))
        .insert_resource(Orbit {
            // --face-sun: start looking toward the sun (the camera sits on the
            // opposite side of the shapes).
            yaw: if face_sun {
                let s = crate::sky::sun_direction(clock_hour);
                (-s.x).atan2(-s.z)
            } else {
                -0.6
            },
            pitch: if face_sun { 0.02 } else { 0.12 },
            distance: 14.0,
        })
        .add_systems(Startup, setup)
        .add_systems(Update, (orbit_camera, update_help, crate::shoot))
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ToonMaterial>>,
) {
    // Ground: a large disc of grass colour.
    commands.spawn((
        Mesh3d(meshes.add(Circle::new(60.0))),
        MeshMaterial3d(materials.add(toon(Color::srgb(0.42, 0.58, 0.30)))),
        Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
    ));
    // Test shapes: smooth curvature (sphere, torus) shows lighting ramps best;
    // flat faces (cube) show hard light changes.
    let shapes: [(Mesh, Vec3, Color); 4] = [
        (
            Sphere::new(1.0).mesh().uv(64, 32),
            Vec3::new(-3.0, 1.0, 0.0),
            Color::srgb(0.85, 0.82, 0.76),
        ),
        (
            Torus::new(0.55, 1.1).into(),
            Vec3::new(0.0, 1.1, 0.0),
            Color::srgb(0.80, 0.42, 0.30),
        ),
        (
            Cuboid::new(1.6, 1.6, 1.6).into(),
            Vec3::new(3.0, 0.8, 0.0),
            Color::srgb(0.36, 0.46, 0.70),
        ),
        (
            Cone::new(0.9, 2.2).into(),
            Vec3::new(0.0, 1.1, -3.2),
            Color::srgb(0.30, 0.50, 0.26),
        ),
    ];
    for (mesh, at, color) in shapes {
        let mut tf = Transform::from_translation(at);
        if at.x == 0.0 && at.z == 0.0 {
            tf.rotate_x(1.1);
        }
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(toon(color))),
            tf,
        ));
    }
    // The sun; the sky plugin points it and sets its colour from the time of day.
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::default(),
    ));
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 3.0, 14.0).looking_at(Vec3::Y, Vec3::Y),
        Bloom::NATURAL,
    ));
    commands.spawn((
        Help,
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(12.0),
            top: Val::Px(10.0),
            ..default()
        },
    ));
}

fn orbit_camera(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    mut orbit: ResMut<Orbit>,
    mut cam: Query<&mut Transform, With<Camera3d>>,
) {
    let dt = time.delta_secs();
    if mouse.pressed(MouseButton::Left) {
        orbit.yaw -= motion.delta.x * 0.005;
        orbit.pitch += motion.delta.y * 0.005;
    }
    let axis = |neg: KeyCode, pos: KeyCode| {
        keys.pressed(pos) as i32 as f32 - keys.pressed(neg) as i32 as f32
    };
    orbit.yaw += axis(KeyCode::ArrowRight, KeyCode::ArrowLeft) * 1.5 * dt;
    orbit.pitch += axis(KeyCode::ArrowDown, KeyCode::ArrowUp) * 1.0 * dt;
    orbit.pitch = orbit.pitch.clamp(-0.05, 1.45);
    let zoom = -scroll.delta.y * 0.1 + axis(KeyCode::Equal, KeyCode::Minus) * dt;
    orbit.distance = (orbit.distance * (1.0 + zoom)).clamp(4.0, 120.0);
    let target = Vec3::Y;
    let offset = Vec3::new(
        orbit.yaw.sin() * orbit.pitch.cos(),
        orbit.pitch.sin(),
        orbit.yaw.cos() * orbit.pitch.cos(),
    ) * orbit.distance;
    for mut tf in &mut cam {
        *tf = Transform::from_translation(target + offset).looking_at(target, Vec3::Y);
    }
}

fn update_help(
    clock: Res<SkyClock>,
    toon: Res<ToonSettings>,
    target: Res<DebugTarget>,
    mut text: Query<&mut Text, With<Help>>,
) {
    if !clock.is_changed() && !toon.is_changed() && !target.is_changed() {
        return;
    }
    let h = clock.hour;
    let mark = |t: DebugTarget| if *target == t { ">" } else { " " };
    let line = format!(
        "Shader playground - steps 01 sky, 02 toon\n\
         time {:02}:{:02}  [ ] change  T run ({})\n\
         {} sky view {}: {}\n\
         {} toon view {}: {}   M toon/PBR ({})\n\
         Tab: which shader keys 0-4 control\n\
         drag / arrows orbit, wheel zoom\n\
         edit assets/shaders/*.wesl and save to reload",
        h.floor() as u32,
        ((h.fract()) * 60.0) as u32,
        if clock.running { "on" } else { "off" },
        mark(DebugTarget::Sky),
        clock.debug,
        SkyClock::VIEWS[clock.debug as usize],
        mark(DebugTarget::Toon),
        toon.debug,
        ToonSettings::VIEWS[toon.debug as usize],
        if toon.on { "toon" } else { "PBR" },
    );
    for mut t in &mut text {
        t.0 = line.clone();
    }
}
