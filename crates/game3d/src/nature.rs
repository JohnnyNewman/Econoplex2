//! Shader step 06: grass and trees in the wind (docs/shaders/06-grass.md).
//!
//! - **Grass:** blades over the meadows, placed once on the CPU and uploaded as
//!   storage buffers (one per chunk). Each chunk is a single mesh holding many
//!   copies of a 5-vertex blade; `grass.wesl` reads each blade's data from the
//!   buffer by vertex index and bends it in the wind.
//! - **Trees:** the crowns share one mesh and one material per colour, so Bevy
//!   draws them instanced; `sway.wesl` sways each instance with its own phase.
//!
//! Keys: `G` grass on/off; with `Tab` on "toon", view 1 also colours blades by
//! their height (debug).

use crate::fog::FogState;
use crate::toon::{ToonSettings, ToonUniform};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

pub type GrassMaterial = ExtendedMaterial<StandardMaterial, GrassExt>;
pub type TreeMaterial = ExtendedMaterial<StandardMaterial, TreeExt>;

pub struct NaturePlugin;

impl Plugin for NaturePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            MaterialPlugin::<GrassMaterial>::default(),
            MaterialPlugin::<TreeMaterial>::default(),
        ))
        .add_systems(Update, update_nature);
    }
}

/// The grass uniform block (binding 100).
#[derive(ShaderType, Reflect, Clone, Copy, Debug)]
pub struct GrassUniform {
    pub toon: ToonUniform,
    /// xy: wind direction (unit), z: amplitude (radians of bend), w: wave
    /// frequency across the field.
    pub wind: Vec4,
    /// x: fade start, y: fade end (distance to the camera), z: debug view.
    pub params: Vec4,
}

impl Default for GrassUniform {
    fn default() -> Self {
        Self {
            toon: ToonUniform {
                rim: Vec4::ZERO,
                ..default()
            },
            wind: Vec4::new(0.8, 0.6, 0.6, 0.35),
            params: Vec4::new(70.0, 110.0, 0.0, 0.0),
        }
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct GrassExt {
    #[uniform(100)]
    pub grass: GrassUniform,
    /// 8 floats per blade: position xyz, height, lean, tint, 2 padding.
    #[storage(101, read_only)]
    pub blades: Handle<ShaderBuffer>,
}

impl MaterialExtension for GrassExt {
    fn vertex_shader() -> ShaderRef {
        "shaders/grass.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/grass.wesl".into()
    }
}

/// Tree crowns: the toon material of step 02 with a swaying vertex shader.
/// The uniform block is the same as `ToonExt`'s, so `toon.wesl` reads it.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct TreeExt {
    #[uniform(100)]
    pub toon: ToonUniform,
}

impl MaterialExtension for TreeExt {
    fn vertex_shader() -> ShaderRef {
        "shaders/sway.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/toon.wesl".into()
    }
}

pub fn tree_material(color: Color) -> TreeMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            base_color: color,
            perceptual_roughness: 1.0,
            ..default()
        },
        extension: TreeExt::default(),
    }
}

/// One grass blade: 5 vertices (two pairs and a tip), 3 triangles. x is across
/// the blade (it is 0.14 wide at the root), y runs from 0 (root) to 1 (tip); the
/// shader scales y by the blade's height.
const BLADE: [[f32; 2]; 5] = [
    [-0.07, 0.0],
    [0.07, 0.0],
    [-0.045, 0.5],
    [0.045, 0.5],
    [0.0, 1.0],
];
const BLADE_TRIS: [u32; 9] = [0, 1, 2, 1, 3, 2, 2, 3, 4];

/// A chunk mesh: `n` copies of the blade shape. All copies sit at the origin;
/// the shader moves each to its blade's position.
fn chunk_mesh(n: usize) -> Mesh {
    let mut pos = Vec::with_capacity(n * 5);
    let mut uv = Vec::with_capacity(n * 5);
    let mut idx = Vec::with_capacity(n * 9);
    for b in 0..n {
        for v in BLADE {
            pos.push([v[0], v[1], 0.0]);
            uv.push([v[1], 0.0]);
        }
        idx.extend(BLADE_TRIS.iter().map(|i| i + 5 * b as u32));
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 0.0, 1.0]; n * 5]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// Where the grass grows: blades as (position, height, lean, tint).
pub struct Blade {
    pub pos: Vec3,
    pub height: f32,
    pub lean: f32,
    pub tint: f32,
}

/// Marks grass chunks.
#[derive(Component)]
pub struct GrassChunk;

/// Spawn the grass, split into square chunks of `chunk` world units.
pub fn spawn_grass(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    buffers: &mut Assets<ShaderBuffer>,
    materials: &mut Assets<GrassMaterial>,
    blades: &[Blade],
    chunk: f32,
) -> usize {
    use std::collections::BTreeMap;
    let mut chunks: BTreeMap<(i32, i32), Vec<[f32; 8]>> = BTreeMap::new();
    for b in blades {
        let key = (
            (b.pos.x / chunk).floor() as i32,
            (b.pos.z / chunk).floor() as i32,
        );
        chunks.entry(key).or_default().push([
            b.pos.x, b.pos.y, b.pos.z, b.height, b.lean, b.tint, 0.0, 0.0,
        ]);
    }
    let n_chunks = chunks.len();
    for data in chunks.into_values() {
        let mesh = meshes.add(chunk_mesh(data.len()));
        let material = materials.add(ExtendedMaterial {
            base: StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 1.0,
                // Blades are seen from both sides.
                cull_mode: None,
                double_sided: true,
                ..default()
            },
            extension: GrassExt {
                grass: GrassUniform::default(),
                blades: buffers.add(ShaderBuffer::from(data)),
            },
        });
        commands.spawn((
            GrassChunk,
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::default(),
            // The mesh's own bounds are a blade at the origin: don't cull by them.
            NoFrustumCulling,
            NotShadowCaster,
        ));
    }
    n_chunks
}

/// Settings and fog into the grass and tree materials; `G` hides the grass.
fn update_nature(
    keys: Res<ButtonInput<KeyCode>>,
    toon: Res<ToonSettings>,
    fog: Res<FogState>,
    mut grass_mats: ResMut<Assets<GrassMaterial>>,
    mut tree_mats: ResMut<Assets<TreeMaterial>>,
    mut chunks: Query<&mut Visibility, With<GrassChunk>>,
) {
    if keys.just_pressed(KeyCode::KeyG) {
        for mut v in &mut chunks {
            v.toggle_visible_hidden();
        }
    }
    if !toon.is_changed() && !fog.is_changed() {
        return;
    }
    let debug = Vec4::new(toon.debug as f32, toon.on as u32 as f32, 0.0, 0.0);
    for (_, m) in grass_mats.iter_mut() {
        m.extension.grass.toon.debug = debug;
        m.extension.grass.toon.fog = fog.0;
        m.extension.grass.params.z = if toon.debug == 1 { 1.0 } else { 0.0 };
    }
    for (_, m) in tree_mats.iter_mut() {
        m.extension.toon.debug = debug;
        m.extension.toon.fog = fog.0;
        m.extension.toon.rim.x = if toon.rim { 1.0 } else { 0.0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shader finds a blade's data at `(vertex_index - first) / 5`, so every
    /// blade must own exactly 5 consecutive vertices and only index those.
    #[test]
    fn each_blade_owns_five_consecutive_vertices() {
        let mesh = chunk_mesh(7);
        assert_eq!(mesh.count_vertices(), 35);
        let Some(Indices::U32(idx)) = mesh.indices() else {
            panic!("u32 indices expected")
        };
        assert_eq!(idx.len(), 7 * 9);
        for (tri_set, chunk) in idx.chunks(9).enumerate() {
            assert!(chunk.iter().all(|&i| i as usize / 5 == tri_set));
        }
    }
}
