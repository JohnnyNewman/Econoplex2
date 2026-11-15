//! Water: rivers and lakes (docs/shaders/13-water.md).
//!
//! One flat plane at the water level, under which the terrain sinks its river
//! beds. Its fragment shader (`assets/shaders/water.wesl`) does all the work:
//! - **depth** from the terrain's own heightmap (the same texture the terrain
//!   and the crowd read): light and green in the shallows, dark and blue in
//!   the middle of a river;
//! - **ripples**: a normal from a few travelling waves that run with the wind
//!   and grow with its strength, plus rain rings when it rains;
//! - **sky**: at grazing angles the water shows the colour of the horizon
//!   (fresnel), and the sun leaves a bright glint that the bloom catches;
//! - **foam** along the shore, moving a little;
//! - **ice** in deep winter: pale, matt and still;
//! - lamp light, the shared fog and the toon lighting with shadows like
//!   everything else.
//!
//! Everything that changes over time comes from the shared buffer
//! (`fog::FOG_BUFFER`: sun, wind, wetness, season), so the material itself
//! never changes after it is made.

use crate::fog::FOG_BUFFER;
use bevy::light::NotShadowCaster;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

const SHADER: &str = "shaders/water.wesl";

/// StandardMaterial (for the mesh input and shadows) + our water shading.
pub type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterExt>;

pub struct WaterPlugin;

impl Plugin for WaterPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<WaterMaterial>::default());
    }
}

/// The water's uniform block (binding 100), read as `water` in the shader.
#[derive(ShaderType, Reflect, Clone, Copy, Debug)]
pub struct WaterUniform {
    /// Where the heightmap lies: x, z of texel (0, 0), distance between
    /// texels, height scale (as the terrain's `grid`).
    pub grid: Vec4,
    /// x, y: texels across and deep; z: the water level (3D y); w unused.
    pub dims: Vec4,
    /// rgb: colour of the shallows; a: depth at which the water looks deep.
    pub shallow: Vec4,
    /// rgb: colour of deep water; a: how strongly the sky shows (0..1).
    pub deep: Vec4,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct WaterExt {
    #[uniform(100)]
    pub water: WaterUniform,
    /// The terrain's heights (3D units before the height scale), read texel
    /// by texel like the terrain does.
    #[texture(101, sample_type = "float", filterable = false)]
    pub heights: Handle<Image>,
    /// The shared fog, sun, wind, season and lamps (`fog::FOG_BUFFER`).
    #[storage(102, read_only)]
    pub fog: Handle<ShaderBuffer>,
}

impl MaterialExtension for WaterExt {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
}

/// Spawn the water plane at `level` over the map, reading the terrain's
/// heightmap (`grid` and `dims` as in `crowd::CrowdTerrain`).
pub fn spawn_water(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<WaterMaterial>,
    heights: Handle<Image>,
    grid: Vec4,
    dims: Vec4,
    level: f32,
) {
    let material = materials.add(ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.2,
            ..default()
        },
        extension: WaterExt {
            water: WaterUniform {
                grid,
                dims: Vec4::new(dims.x, dims.y, level, 0.0),
                shallow: Vec4::new(0.30, 0.60, 0.58, 0.45),
                deep: Vec4::new(0.06, 0.22, 0.38, 0.6),
            },
            heights,
            fog: FOG_BUFFER,
        },
    });
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(400.0, 400.0))),
        MeshMaterial3d(material),
        Transform::from_xyz(0.0, level, 0.0),
        NotShadowCaster,
    ));
}
