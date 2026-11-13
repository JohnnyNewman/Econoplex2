//! Shader step 04: distance and height fog (docs/shaders/04-fog.md).
//!
//! The fog is computed in the fragment shader of every toon and terrain
//! material (`assets/shaders/fog.wesl`), from the fragment's world position and
//! the camera position:
//! - **distance fog** grows exponentially with distance;
//! - **height fog** is denser low down, integrated along the view ray in closed
//!   form, so valleys fill with haze and hill tops poke out;
//! - its colour is the sky's horizon colour (so far hills melt into the sky),
//!   warmed toward the sun (in-scattering).
//!
//! The CPU fills one `FogUniform` per frame from the time of day; the toon and
//! terrain materials copy it into their uniform blocks.
//!
//! Keys: `F` fog on/off; with `Tab` on "fog", `0`–`4` pick a debug view
//! (0 final, 1 fog amount, 2 distance part, 3 height part, 4 fog colour).

use crate::env::Environment;
use crate::sky::{digit_pressed, sun_color, weathered_palette, DebugTarget, SkyClock};
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;

pub struct FogPlugin;

impl Plugin for FogPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FogSettings>()
            .init_resource::<FogState>()
            .add_systems(Update, (fog_keys, update_fog_state).chain());
    }
}

/// The fog part of the toon and terrain uniform blocks.
#[derive(ShaderType, Reflect, Clone, Copy, Debug, Default, PartialEq)]
pub struct FogUniform {
    /// rgb: fog colour (the sky's horizon colour); a: 1 = fog on, 0 = off.
    pub color: Vec4,
    /// xyz: unit vector toward the sun; w: exponent of the sun glow in the fog.
    pub sun_dir: Vec4,
    /// rgb: colour the fog takes toward the sun; a: strength of that glow.
    pub sun_color: Vec4,
    /// x: distance density (per world unit), y: height-fog density at the base
    /// height, z: height falloff (world units per e-fold), w: base height.
    pub params: Vec4,
    /// x: debug view; yzw unused.
    pub debug: Vec4,
    /// Step 11, the season and weather on the ground, for every material that
    /// carries the fog: x snow cover, y wetness, z autumn colour, w winter
    /// (bare, brown vegetation); all 0..1.
    pub ground: Vec4,
    /// Step 11: xy wind direction (unit, 3D x and z), z strength (0 calm .. 1
    /// gale), w spring green 0..1.
    pub wind: Vec4,
}

/// Fog parameters, adjustable at runtime.
#[derive(Resource, Debug, Clone)]
pub struct FogSettings {
    pub on: bool,
    pub debug: u32,
    /// Distance fog density per world unit.
    pub density: f32,
    /// Height fog density at `base`.
    pub height_density: f32,
    /// Height over which the height fog thins by a factor e.
    pub falloff: f32,
    /// Height of the densest fog (the valley floors).
    pub base: f32,
}

impl Default for FogSettings {
    fn default() -> Self {
        Self {
            on: true,
            debug: 0,
            density: 0.0008,
            height_density: 0.006,
            falloff: 3.0,
            base: 0.0,
        }
    }
}

impl FogSettings {
    pub const VIEWS: [&'static str; 5] = [
        "final",
        "fog amount",
        "distance part",
        "height part",
        "fog colour",
    ];
}

/// This frame's fog uniform, shared by all materials.
#[derive(Resource, Default)]
pub struct FogState(pub FogUniform);

fn fog_keys(
    keys: Res<ButtonInput<KeyCode>>,
    target: Res<DebugTarget>,
    mut settings: ResMut<FogSettings>,
) {
    if keys.just_pressed(KeyCode::KeyF) {
        settings.on = !settings.on;
    }
    if *target == DebugTarget::Fog {
        if let Some(v) = digit_pressed(&keys) {
            settings.debug = v.min(4);
        }
    }
}

/// Once per frame on the CPU: the fog colour follows the sky. Written only
/// when it differs: every material copies it when it changes, and a changed
/// material is prepared for the GPU again.
fn update_fog_state(
    clock: Res<SkyClock>,
    env: Res<Environment>,
    settings: Res<FogSettings>,
    mut state: ResMut<FogState>,
) {
    let dir = clock.sun();
    let w = &env.weather;
    let (horizon, _, _) = weathered_palette(dir.y, w.cloud, w.precip);
    // Below the horizon there is no sun to glow through the fog; clouds hide it too.
    let glow = (dir.y * 8.0 + 0.5).clamp(0.0, 1.0) * 0.6 * (1.0 - 0.8 * w.cloud);
    // Step 11: rain and mist thicken the air.
    let thick = 1.0 + 3.0 * w.precip + 5.0 * w.haze;
    let wind = w.wind.normalize_or(Vec2::X);
    let fog = FogUniform {
        color: horizon.extend(if settings.on { 1.0 } else { 0.0 }),
        sun_dir: dir.extend(8.0),
        sun_color: (sun_color(dir) / 6.0).extend(glow),
        params: Vec4::new(
            settings.density * thick,
            settings.height_density * (1.0 + 4.0 * w.haze),
            settings.falloff,
            settings.base,
        ),
        debug: Vec4::new(settings.debug as f32, 0.0, 0.0, 0.0),
        ground: Vec4::new(env.ground.snow, env.ground.wet, env.autumn(), env.winter()),
        wind: Vec4::new(
            wind.x,
            wind.y,
            (w.wind.length() / 15.0).min(1.0),
            env.spring(),
        ),
    };
    if state.0 != fog {
        state.0 = fog;
    }
}
