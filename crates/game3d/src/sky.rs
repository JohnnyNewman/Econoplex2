//! Shader step 01: the gradient sky (docs/shaders/01-sky.md).
//!
//! The sky is a big sphere around the camera with a custom material. Its
//! fragment shader (`assets/shaders/sky.wesl`) colours every pixel from the view
//! direction alone: a horizon-to-zenith gradient, a ground haze, and a sun disc
//! with a glow.
//!
//! The split between CPU and GPU is part of the lesson:
//! - The CPU (this file) works out values that are the same for every pixel,
//!   once per frame: the sun direction from the time of day and the sky colours
//!   for that sun height. They go to the GPU as one small uniform block.
//! - The GPU (the shader) does the per-pixel work.
//!
//! Keys: `[` / `]` move the time of day, `T` lets time run, `Tab` picks which
//! shader the number keys control, and `0`–`4` pick its debug view (for the sky: (0 final, 1 view direction, 2 gradient parameter, 3 sun term, 4 free for
//! the exercises).

use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

/// The shader file, relative to `crates/game3d/assets`. Edit and save it while
/// the app runs: Bevy's file watcher recompiles it and the sky updates.
const SHADER: &str = "shaders/sky.wesl";

/// Radius of the sky sphere (it stays centred on the camera, inside the far plane).
const RADIUS: f32 = 800.0;

pub struct SkyPlugin;

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<SkyMaterial>::default())
            .init_resource::<SkyClock>()
            .init_resource::<DebugTarget>()
            .add_systems(Startup, spawn_sky)
            .add_systems(Update, (sky_keys, update_sky, follow_camera).chain());
    }
}

/// The uniform block the shader reads as `sky` (one copy for all pixels).
/// `vec4`s keep the layout simple: WGSL aligns `vec3` to 16 bytes anyway.
#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct SkyUniform {
    /// Colour at the horizon (linear RGB, can exceed 1 for HDR).
    pub horizon: Vec4,
    /// Colour straight up.
    pub zenith: Vec4,
    /// Haze below the horizon.
    pub ground: Vec4,
    /// xyz: unit vector toward the sun.
    pub sun_dir: Vec4,
    /// rgb: sun colour; a: angular radius of the disc in radians.
    pub sun_color: Vec4,
    /// x: debug view (0 final, 1 direction, 2 gradient, 3 sun term, 4 free); yzw unused.
    pub debug: Vec4,
}

/// The material: Bevy turns the `#[uniform(0)]` field into binding 0 of the
/// material's bind group, which the shader declares as `@binding(0)`.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
pub struct SkyMaterial {
    #[uniform(0)]
    pub sky: SkyUniform,
}

impl Material for SkyMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    /// We look at the sphere from inside, so its triangles face away from us.
    /// Turning culling off draws them anyway.
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Time of day and the chosen debug view.
#[derive(Resource)]
pub struct SkyClock {
    /// Hour of the day, 0..24.
    pub hour: f32,
    /// Hours per second while time runs (`T`).
    pub speed: f32,
    pub running: bool,
    pub debug: u32,
    /// Fraction of the year, 0..1 (0.5 = midsummer). The live simulation sets
    /// it from its calendar; it decides how high the sun climbs.
    pub season: f32,
}

impl Default for SkyClock {
    fn default() -> Self {
        Self {
            hour: 16.5,
            speed: 1.0,
            running: false,
            debug: 0,
            season: 0.5,
        }
    }
}

impl SkyClock {
    pub const VIEWS: [&'static str; 5] = [
        "final",
        "view direction",
        "gradient t",
        "sun term",
        "your own (exercise 3)",
    ];
}

/// Which shader the number keys (debug views) control; `Tab` cycles.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DebugTarget {
    #[default]
    Sky,
    Toon,
    Terrain,
    Fog,
}

/// The number key pressed this frame (0–6), if any. Each shader clamps it to
/// the views it has.
pub fn digit_pressed(keys: &ButtonInput<KeyCode>) -> Option<u32> {
    [
        KeyCode::Digit0,
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
    ]
    .iter()
    .position(|k| keys.just_pressed(*k))
    .map(|i| i as u32)
}

/// Marks the sky sphere.
#[derive(Component)]
pub struct Sky;

/// Unit vector toward the sun. The sun rises in the east (+x) at 6, stands
/// highest at 12 (tilted toward +z), sets in the west at 18, and is below the
/// horizon at night.
pub fn sun_direction(hour: f32) -> Vec3 {
    sun_direction_in(hour, 0.5)
}

/// The sun's direction at a time of day and a time of year: in winter (season
/// 0) it climbs about half as high as at midsummer (season 0.5).
pub fn sun_direction_in(hour: f32, season: f32) -> Vec3 {
    let a = (hour - 6.0) / 24.0 * std::f32::consts::TAU;
    let summer = 0.5 - 0.5 * (season * std::f32::consts::TAU).cos(); // 0 winter, 1 summer
    let lift = 0.85 * (0.55 + 0.45 * summer);
    Vec3::new(a.cos(), a.sin() * lift, a.sin() * 0.45).normalize()
}

impl SkyClock {
    /// Toward the sun, now.
    pub fn sun(&self) -> Vec3 {
        sun_direction_in(self.hour, self.season)
    }
}

/// Colour (and HDR brightness) of the sun disc: warm when low, white when high.
pub fn sun_color(dir: Vec3) -> Vec3 {
    let warm = (1.0 - dir.y.clamp(0.0, 1.0)).powi(3);
    Vec3::new(1.0, 0.95, 0.85).lerp(Vec3::new(1.0, 0.55, 0.25), warm) * 6.0
}

/// Sky colours for a sun height `e` (= sun_dir.y, -1..1): day, sunset and
/// night palettes, blended by how high the sun is.
pub fn palette(e: f32) -> (Vec3, Vec3, Vec3) {
    let s = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let day = (
        Vec3::new(0.62, 0.78, 0.95),
        Vec3::new(0.08, 0.26, 0.80),
        Vec3::new(0.50, 0.55, 0.58),
    );
    let dusk = (
        Vec3::new(1.10, 0.55, 0.28),
        Vec3::new(0.22, 0.26, 0.52),
        Vec3::new(0.35, 0.26, 0.24),
    );
    let night = (
        Vec3::new(0.035, 0.045, 0.09),
        Vec3::new(0.005, 0.008, 0.025),
        Vec3::new(0.01, 0.01, 0.015),
    );
    let k = s(0.02, 0.35, e);
    let n = s(0.0, -0.25, e);
    let mix = |a: Vec3, b: Vec3, t: f32| a.lerp(b, t);
    (
        mix(mix(dusk.0, day.0, k), night.0, n),
        mix(mix(dusk.1, day.1, k), night.1, n),
        mix(mix(dusk.2, day.2, k), night.2, n),
    )
}

fn spawn_sky(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SkyMaterial>>,
) {
    commands.spawn((
        Sky,
        Mesh3d(meshes.add(Sphere::new(RADIUS).mesh().uv(48, 24))),
        MeshMaterial3d(materials.add(SkyMaterial::default())),
        NotShadowCaster,
        NotShadowReceiver,
    ));
}

fn sky_keys(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut clock: ResMut<SkyClock>,
    mut target: ResMut<DebugTarget>,
) {
    if keys.just_pressed(KeyCode::Tab) {
        *target = match *target {
            DebugTarget::Sky => DebugTarget::Toon,
            DebugTarget::Toon => DebugTarget::Terrain,
            DebugTarget::Terrain => DebugTarget::Fog,
            DebugTarget::Fog => DebugTarget::Sky,
        };
    }
    if keys.just_pressed(KeyCode::KeyT) {
        clock.running = !clock.running;
    }
    let dt = time.delta_secs();
    if clock.running {
        clock.hour += clock.speed * dt;
    }
    if keys.pressed(KeyCode::BracketRight) {
        clock.hour += 3.0 * dt;
    }
    if keys.pressed(KeyCode::BracketLeft) {
        clock.hour -= 3.0 * dt;
    }
    // Only write when it changes: a write marks the clock changed, and the
    // sky, fog and every material follow a changed clock.
    let wrapped = clock.hour.rem_euclid(24.0);
    if wrapped != clock.hour {
        clock.hour = wrapped;
    }
    if *target == DebugTarget::Sky {
        if let Some(v) = digit_pressed(&keys) {
            clock.debug = v.min(4);
        }
    }
}

/// Once per frame on the CPU: sun direction and colours into the uniform; the
/// sun light and the ambient light follow.
fn update_sky(
    clock: Res<SkyClock>,
    sky: Query<&MeshMaterial3d<SkyMaterial>, With<Sky>>,
    mut materials: ResMut<Assets<SkyMaterial>>,
    mut sun: Query<(&mut Transform, &mut DirectionalLight)>,
    mut ambient: ResMut<GlobalAmbientLight>,
) {
    if !clock.is_changed() {
        return;
    }
    let dir = clock.sun();
    let (horizon, zenith, ground) = palette(dir.y);
    let sun_rgb = sun_color(dir);
    for handle in &sky {
        if let Some(mut m) = materials.get_mut(handle) {
            m.sky = SkyUniform {
                horizon: horizon.extend(1.0),
                zenith: zenith.extend(1.0),
                ground: ground.extend(1.0),
                sun_dir: dir.extend(0.0),
                sun_color: sun_rgb.extend(0.03),
                debug: Vec4::new(clock.debug as f32, 0.0, 0.0, 0.0),
            };
        }
    }
    // Light fades in and out around sunrise and sunset.
    let day = ((dir.y + 0.05) / 0.3).clamp(0.0, 1.0);
    for (mut tf, mut light) in &mut sun {
        *tf = Transform::from_translation(dir * 100.0).looking_at(Vec3::ZERO, Vec3::Y);
        light.illuminance = 9_000.0 * day;
        light.color = Color::linear_rgb(sun_rgb.x / 6.0, sun_rgb.y / 6.0, sun_rgb.z / 6.0);
    }
    ambient.color = Color::linear_rgb(zenith.x, zenith.y, zenith.z).mix(&Color::WHITE, 0.5);
    ambient.brightness = 40.0 + 220.0 * day;
}

/// Keep the sphere centred on the camera, so the sky is always "infinitely" far.
fn follow_camera(
    cam: Query<&GlobalTransform, With<Camera3d>>,
    mut sky: Query<&mut Transform, With<Sky>>,
) {
    let Some(cam) = cam.iter().next() else { return };
    for mut tf in &mut sky {
        tf.translation = cam.translation();
    }
}
