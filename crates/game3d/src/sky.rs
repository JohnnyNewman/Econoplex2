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

use crate::env::Environment;
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
            .init_resource::<Environment>()
            .add_systems(Startup, spawn_sky)
            .init_resource::<crate::env::EnvOverride>()
            .add_systems(
                Update,
                (
                    sky_keys,
                    crate::env::env_keys,
                    crate::env::update_environment,
                    update_sky,
                    follow_camera,
                )
                    .chain(),
            );
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
    /// xyz: unit vector toward the moon; w: moon phase (0 new, 0.5 full).
    pub moon_dir: Vec4,
    /// x: cloud cover 0..1, y: how bright the stars are 0..1, z: haze 0..1,
    /// w: precipitation 0..1 (darkens the clouds).
    pub weather: Vec4,
    /// xy: where the wind blows (m/s), moves the clouds; zw unused.
    pub wind: Vec4,
    /// rgb: colour of the lit side of the clouds; a: their shadow side (0..1 of it).
    pub cloud_color: Vec4,
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

/// Hours of daylight at a time of year: 16 on average (so the night is about
/// a third of the day), 14 in midwinter, 18 at midsummer.
pub fn day_length(season: f32) -> f32 {
    16.0 - 2.0 * (season * std::f32::consts::TAU).cos()
}

/// Sunrise and sunset (hours), symmetric around noon.
pub fn sunrise_sunset(season: f32) -> (f32, f32) {
    let half = day_length(season) * 0.5;
    (12.0 - half, 12.0 + half)
}

/// The sun's direction at a time of day and a time of year. The sun runs a
/// circle once a day: east (+x) in the morning, highest at noon (tilted toward
/// +z), west in the evening. The circle sits above the horizon for
/// `day_length(season)` hours; in winter it also climbs about half as high.
pub fn sun_direction_in(hour: f32, season: f32) -> Vec3 {
    let a = (hour - 12.0) / 24.0 * std::f32::consts::TAU; // 0 at noon
    let half = day_length(season) / 24.0 * std::f32::consts::PI; // half a day's arc
    let summer = 0.5 - 0.5 * (season * std::f32::consts::TAU).cos(); // 0 winter, 1 summer
    let noon = 0.85 * (0.55 + 0.45 * summer); // height at noon
                                              // y = A (cos a - cos half): zero at sunrise and sunset, `noon` at noon.
    let y = noon * (a.cos() - half.cos()) / (1.0 - half.cos());
    Vec3::new(-a.sin(), y, 0.45 * a.cos().max(0.0) + 0.1).normalize()
}

/// The moon's direction: roughly opposite the sun, a little higher, so it
/// lights the night from the other side of the sky.
pub fn moon_direction_in(hour: f32, season: f32) -> Vec3 {
    let s = sun_direction_in(hour, season);
    Vec3::new(-s.x, -s.y * 0.8 + 0.12, -s.z * 0.6 + 0.2).normalize()
}

impl SkyClock {
    /// Toward the sun, now.
    pub fn sun(&self) -> Vec3 {
        sun_direction_in(self.hour, self.season)
    }
    /// Toward the moon, now.
    pub fn moon(&self) -> Vec3 {
        moon_direction_in(self.hour, self.season)
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
    env: Res<Environment>,
    sky: Query<&MeshMaterial3d<SkyMaterial>, With<Sky>>,
    mut materials: ResMut<Assets<SkyMaterial>>,
    mut sun: Query<(&mut Transform, &mut DirectionalLight)>,
    mut ambient: ResMut<GlobalAmbientLight>,
) {
    if !clock.is_changed() && !env.is_changed() {
        return;
    }
    let dir = clock.sun();
    let moon = clock.moon();
    let w = &env.weather;
    let (horizon, zenith, ground) = weathered_palette(dir.y, w.cloud, w.precip);
    let sun_rgb = sun_color(dir);
    let light = sky_light(dir, moon, env.moon_phase(), w.cloud, w.precip);
    // Clouds: lit by the sun by day (warm at dawn and dusk), by the moon at
    // night; rain clouds are darker.
    let lit = (sun_rgb / 6.0 * light.day * 1.1 + Vec3::new(0.06, 0.07, 0.11) * light.moon)
        .lerp(zenith.max(horizon) * 0.9, 0.3)
        * (1.0 - 0.55 * w.precip);
    for handle in &sky {
        if let Some(mut m) = materials.get_mut(handle) {
            m.sky = SkyUniform {
                horizon: horizon.extend(1.0),
                zenith: zenith.extend(1.0),
                ground: ground.extend(1.0),
                sun_dir: dir.extend(0.0),
                sun_color: (sun_rgb * (1.0 - 0.9 * w.cloud)).extend(0.03),
                debug: Vec4::new(clock.debug as f32, 0.0, 0.0, 0.0),
                moon_dir: moon.extend(env.moon_phase()),
                weather: Vec4::new(w.cloud, light.stars, w.haze, w.precip),
                wind: w.wind.extend(0.0).extend(0.0),
                cloud_color: lit.extend(0.55),
            };
        }
    }
    for (mut tf, mut l) in &mut sun {
        *tf = Transform::from_translation(light.dir * 100.0).looking_at(Vec3::ZERO, Vec3::Y);
        l.illuminance = light.illuminance;
        l.color = light.color;
    }
    // At night the sky light is the deep blue of the zenith.
    ambient.color =
        Color::linear_rgb(zenith.x, zenith.y, zenith.z).mix(&Color::WHITE, 0.15 + 0.35 * light.day);
    ambient.brightness = light.ambient;
}

/// The one directional light of the scene, and the ambient light, for a sun
/// and moon position and the weather.
#[derive(Clone, Copy, Debug)]
pub struct SkyLight {
    /// Toward the light (the sun by day, the moon at night).
    pub dir: Vec3,
    pub illuminance: f32,
    pub color: Color,
    pub ambient: f32,
    /// How much it is day (0 night .. 1 full day) and how much moonlight there is.
    pub day: f32,
    pub moon: f32,
    /// Star brightness, 0 .. 1.
    pub stars: f32,
}

/// Sun light fades out around sunset; the moon takes over once the sun is
/// well below the horizon (the light switches over in the darkest moment, so
/// the jump of the shadows is not seen). Clouds dim both and soften the light
/// toward ambient.
pub fn sky_light(sun: Vec3, moon: Vec3, phase: f32, cloud: f32, precip: f32) -> SkyLight {
    let day = ((sun.y + 0.05) / 0.3).clamp(0.0, 1.0);
    let full = 1.0 - (phase - 0.5).abs() * 2.0; // 1 full, 0 new
    let night = crate::env::smooth(-0.04, -0.16, sun.y);
    let moon_light = night * (0.25 + 0.75 * full) * crate::env::smooth(-0.05, 0.15, moon.y);
    let dim = 1.0 - 0.75 * cloud - 0.15 * precip;
    let sun_rgb = sun_color(sun) / 6.0;
    let (dir, illuminance, color) = if day > 0.0 || moon_light <= 0.0 {
        (
            sun,
            9_000.0 * day * dim,
            Color::linear_rgb(sun_rgb.x, sun_rgb.y, sun_rgb.z),
        )
    } else {
        (
            moon,
            320.0 * moon_light * dim,
            Color::linear_rgb(0.55, 0.65, 1.0),
        )
    };
    SkyLight {
        dir,
        illuminance,
        color,
        // Overcast days are flat: less sun, a bit more sky light.
        ambient: (15.0 + 20.0 * moon_light)
            + 220.0 * day * (1.0 + 0.25 * cloud) * (1.0 - 0.3 * precip),
        day,
        moon: moon_light,
        stars: night * (1.0 - cloud).powi(2) * (1.0 - 0.6 * full),
    }
}

/// The sky palette under clouds: grey and flat, darker in rain.
pub fn weathered_palette(e: f32, cloud: f32, precip: f32) -> (Vec3, Vec3, Vec3) {
    let (h, z, g) = palette(e);
    let grey = |c: Vec3, k: f32| {
        let l = c.dot(Vec3::new(0.3, 0.55, 0.15));
        c.lerp(Vec3::splat(l) * 0.95, cloud * k) * (1.0 - 0.45 * precip)
    };
    let zg = grey(z, 0.9);
    let hg = grey(h, 0.7);
    // Overcast: the zenith is almost as bright as the horizon.
    (hg, zg.lerp(hg * 0.9, cloud * 0.6), grey(g, 0.5))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The sun is up for `day_length` hours: about two thirds of the day.
    #[test]
    fn the_night_is_about_a_third_of_the_day() {
        for season in [0.0, 0.25, 0.5, 0.75] {
            let up = (0..2400)
                .filter(|i| sun_direction_in(*i as f32 / 100.0, season).y > 0.0)
                .count() as f32
                / 100.0;
            assert!((up - day_length(season)).abs() < 0.05, "{season}: {up}");
        }
        assert!((day_length(0.25) - 16.0).abs() < 1e-3);
        let (rise, set) = sunrise_sunset(0.25);
        assert!(
            sun_direction_in(rise + 0.1, 0.25).x > 0.5,
            "rises in the east"
        );
        assert!(
            sun_direction_in(set - 0.1, 0.25).x < -0.5,
            "sets in the west"
        );
    }

    #[test]
    fn the_moon_lights_the_night_and_clouds_hide_the_stars() {
        let night = sky_light(
            sun_direction_in(1.0, 0.5),
            moon_direction_in(1.0, 0.5),
            0.5,
            0.0,
            0.0,
        );
        assert_eq!(night.day, 0.0);
        assert!(night.illuminance > 50.0 && night.dir.y > 0.0, "{night:?}");
        assert!(night.stars > 0.0);
        let overcast = sky_light(
            sun_direction_in(1.0, 0.5),
            moon_direction_in(1.0, 0.5),
            0.5,
            1.0,
            0.0,
        );
        assert_eq!(overcast.stars, 0.0);
        let noon = sky_light(
            sun_direction_in(12.0, 0.5),
            moon_direction_in(12.0, 0.5),
            0.5,
            0.0,
            0.0,
        );
        assert!(noon.illuminance > 8_000.0 && noon.dir.y > 0.5);
    }
}
