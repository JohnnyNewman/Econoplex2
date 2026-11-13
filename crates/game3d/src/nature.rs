//! Shader step 06: grass and trees in the wind (docs/shaders/06-grass.md).
//!
//! - **Grass:** blades over the meadows, placed once on the CPU and uploaded as
//!   storage buffers (one per chunk). Each chunk is a single mesh holding many
//!   copies of a 5-vertex blade; `grass.wesl` reads each blade's data from the
//!   buffer by vertex index and bends it in the wind.
//! - **Trees:** baked together per chunk: every tree's trunk and crown, moved
//!   to its place, in one mesh with vertex colours, so a forest costs a few
//!   entities instead of two per tree (each entity costs CPU time every frame,
//!   in every shadow cascade too). `sway.wesl` sways the crowns; how far up the
//!   crown a vertex is comes in its UV, its tree's root in the second UV set.
//! - **Culling:** grass chunks carry their own bounds, so chunks outside the
//!   view are skipped, and chunks beyond the grass's fade distance are hidden.
//!
//! Keys: `G` grass on/off; with `Tab` on "toon", view 1 also colours blades by
//! their height (debug).

use crate::bake::Baked;
use crate::fog::FogState;
use crate::toon::{ToonSettings, ToonUniform};
use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
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
        // rim.z = 1: foliage, it turns with the seasons (step 11).
        extension: TreeExt {
            toon: ToonUniform {
                rim: Vec4::new(1.0, 3.0, 1.0, 0.0),
                ..default()
            },
        },
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

/// A grass chunk and its bounds (for hiding it when far).
#[derive(Component)]
pub struct GrassChunk {
    pub min: Vec3,
    pub max: Vec3,
}

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
        // The chunk's bounds: its blades' roots, raised by the tallest blade
        // and widened by the wind's bend.
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        for b in &data {
            let p = Vec3::new(b[0], b[1], b[2]);
            min = min.min(p);
            max = max.max(p + Vec3::Y * b[3]);
        }
        min -= Vec3::splat(0.5);
        max += Vec3::splat(0.5);
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
            GrassChunk { min, max },
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::default(),
            // The mesh's own bounds are a blade at the origin: give the real
            // ones, so frustum culling skips chunks outside the view.
            Aabb::from_min_max(min, max),
            NotShadowCaster,
        ));
    }
    n_chunks
}

/// One tree: where its trunk stands, its size, and its crown colour (linear).
pub struct TreeSpec {
    pub root: Vec3,
    pub scale: f32,
    pub crown: [f32; 4],
}

/// Spawn the trees, baked into one mesh per square chunk of `chunk` units.
pub fn spawn_forest(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<TreeMaterial>,
    trees: &[TreeSpec],
    trunk_colour: [f32; 4],
    chunk: f32,
) -> usize {
    use std::collections::BTreeMap;
    let trunk = Mesh::from(Cylinder::new(0.12, 0.8));
    let crown = Mesh::from(Cone::new(0.7, 2.0));
    let mut chunks: BTreeMap<(i32, i32), Vec<&TreeSpec>> = BTreeMap::new();
    for t in trees {
        let key = (
            (t.root.x / chunk).floor() as i32,
            (t.root.z / chunk).floor() as i32,
        );
        chunks.entry(key).or_default().push(t);
    }
    // One material for all chunks: white, the vertex colours give the colour.
    let material = materials.add(tree_material(Color::WHITE));
    let n = chunks.len();
    for list in chunks.into_values() {
        let mut baked = Baked::default();
        for t in list {
            // The trunk stands on the root; the crown sits on the trunk and
            // sways (`up` 0 at its base, 1 at its tip). Trunks don't sway.
            let root = Vec2::new(t.root.x, t.root.z);
            baked.add(
                &trunk,
                Transform::from_translation(t.root + Vec3::Y * 0.4),
                trunk_colour,
                None,
            );
            baked.add(
                &crown,
                Transform::from_translation(t.root + Vec3::Y * 1.6)
                    .with_scale(Vec3::splat(t.scale)),
                t.crown,
                Some(root),
            );
        }
        commands.spawn((
            Mesh3d(meshes.add(baked.mesh())),
            MeshMaterial3d(material.clone()),
        ));
    }
    n
}

/// Settings and fog into the grass and tree materials; `G` hides the grass;
/// chunks beyond the fade distance are hidden.
#[allow(clippy::too_many_arguments)]
fn update_nature(
    keys: Res<ButtonInput<KeyCode>>,
    toon: Res<ToonSettings>,
    fog: Res<FogState>,
    mut grass_on: Local<Option<bool>>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    mut grass_mats: ResMut<Assets<GrassMaterial>>,
    mut tree_mats: ResMut<Assets<TreeMaterial>>,
    mut chunks: Query<(&mut Visibility, &GrassChunk)>,
) {
    let on = grass_on.get_or_insert(true);
    if keys.just_pressed(KeyCode::KeyG) {
        *on = !*on;
    }
    // A chunk whose nearest point is past the fade end draws nothing but still
    // costs all its vertices: hide it.
    let fade_end = GrassUniform::default().params.y;
    if let Some(cam) = camera.iter().next() {
        let eye = cam.translation();
        for (mut v, c) in &mut chunks {
            let near = eye.clamp(c.min, c.max).distance(eye) < fade_end;
            v.set_if_neq(if *on && near {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
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
