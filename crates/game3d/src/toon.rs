//! Shader step 02: soft toon lighting (docs/shaders/02-toon.md).
//!
//! `ToonMaterial` is Bevy's `StandardMaterial` with an extension: the standard
//! part still provides albedo, vertex colours, normals and the shadow maps; our
//! fragment shader (`assets/shaders/toon.wesl`) replaces only the lighting:
//! wrap lighting, a three-band ramp, and tinted shadows.
//!
//! Keys: `M` switches every toon object between toon and Bevy's standard PBR
//! lighting (to compare); with `Tab` on "toon", `0`–`4` pick a debug view
//! (0 final, 1 normals, 2 n·l, 3 ramp, 4 shadow map).

use crate::fog::FOG_BUFFER;
use crate::sky::{digit_pressed, DebugTarget};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

const SHADER: &str = "shaders/toon.wesl";

/// StandardMaterial + our toon extension.
pub type ToonMaterial = ExtendedMaterial<StandardMaterial, ToonExt>;

pub struct ToonPlugin;

impl Plugin for ToonPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<ToonMaterial>::default())
            .init_resource::<ToonSettings>()
            .add_systems(Update, (toon_keys, update_toon).chain());
    }
}

/// The toon uniform block, read in the shader as `toon` (binding 100; the
/// standard material uses the low binding numbers).
#[derive(ShaderType, Reflect, Clone, Copy, Debug)]
pub struct ToonUniform {
    /// rgb: colour of the shadow side (multiplied by the ambient light).
    pub shadow_tint: Vec4,
    /// x: wrap (how far light reaches past 90°, 0..1), y: first band edge,
    /// z: second band edge, w: softness of a band edge.
    pub ramp: Vec4,
    /// x: debug view; y: 1 = toon on, 0 = standard PBR lighting; zw unused.
    pub debug: Vec4,
    /// Step 05, rim light: x strength (0 = off), y fresnel power; zw unused.
    pub rim: Vec4,
}

impl Default for ToonUniform {
    fn default() -> Self {
        Self {
            shadow_tint: Vec4::new(0.42, 0.48, 0.85, 1.0),
            ramp: Vec4::new(0.4, 0.28, 0.62, 0.05),
            debug: Vec4::new(0.0, 1.0, 0.0, 0.0),
            rim: Vec4::new(1.0, 3.0, 0.0, 0.0),
        }
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct ToonExt {
    #[uniform(100)]
    pub toon: ToonUniform,
    /// The fog, shared by all materials (`fog::FOG_BUFFER`).
    #[storage(101, read_only)]
    pub fog: Handle<ShaderBuffer>,
}

impl Default for ToonExt {
    fn default() -> Self {
        Self {
            toon: default(),
            fog: FOG_BUFFER,
        }
    }
}

impl MaterialExtension for ToonExt {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
}

/// A toon material with a flat colour (or white, to show vertex colours).
pub fn toon(color: Color) -> ToonMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            base_color: color,
            perceptual_roughness: 1.0,
            ..default()
        },
        extension: ToonExt::default(),
    }
}

/// Debug view and on/off for all toon materials.
#[derive(Resource, Debug)]
pub struct ToonSettings {
    pub debug: u32,
    pub on: bool,
    /// Step 05: rim light on the toon materials (`R`).
    pub rim: bool,
}

impl Default for ToonSettings {
    fn default() -> Self {
        Self {
            debug: 0,
            on: true,
            rim: true,
        }
    }
}

impl ToonSettings {
    pub const VIEWS: [&'static str; 7] = [
        "final",
        "normals",
        "n.l",
        "ramp",
        "shadow map",
        "fresnel (05)",
        "n.v (05)",
    ];
}

fn toon_keys(
    keys: Res<ButtonInput<KeyCode>>,
    target: Res<DebugTarget>,
    mut settings: ResMut<ToonSettings>,
) {
    if keys.just_pressed(KeyCode::KeyM) {
        settings.on = !settings.on;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        settings.rim = !settings.rim;
    }
    if *target == DebugTarget::Toon {
        if let Some(v) = digit_pressed(&keys) {
            settings.debug = v.min(6);
        }
    }
}

/// Copy the settings into every toon material (the fog has its own buffer).
fn update_toon(settings: Res<ToonSettings>, mut materials: ResMut<Assets<ToonMaterial>>) {
    if !settings.is_changed() {
        return;
    }
    let debug = Vec4::new(settings.debug as f32, settings.on as u32 as f32, 0.0, 0.0);
    for (_, m) in materials.iter_mut() {
        m.extension.toon.debug = debug;
        m.extension.toon.rim.x = if settings.rim { 1.0 } else { 0.0 };
    }
}
