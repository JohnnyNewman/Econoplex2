//! Shader step 03: the terrain's vertex shader (docs/shaders/03-terrain.md).
//!
//! The CPU builds two small textures from the generated world, once:
//! - a **heightmap** (`R32Float`, one texel per terrain cell corner), and
//! - a **biome mask** (`Rgba8Unorm`: r meadow, g fields, b water, a unused).
//!
//! The terrain mesh itself is a **flat grid** (y = 0). Its vertex shader
//! (`assets/shaders/terrain.wesl`) lifts every vertex to its height, computes
//! the normal from neighbouring heights, and the fragment shader paints the
//! biomes with procedural noise and lights them with the toon lighting of
//! step 02.
//!
//! Because the shape comes from the texture, the same grid can be coarser or
//! finer: `L` cycles the grid step (1, 2, 4 texels per quad), a first taste of
//! the "LOD from the generator" idea.
//!
//! Keys: `L` grid step; with `Tab` on "terrain", `0`–`4` pick a debug view
//! (0 final, 1 height, 2 normals, 3 biome weights, 4 slope).

use crate::fog::FOG_BUFFER;
use crate::sky::{digit_pressed, DebugTarget};
use crate::toon::{ToonSettings, ToonUniform};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat,
};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

const SHADER: &str = "shaders/terrain.wesl";

pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, TerrainExt>;

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<TerrainMaterial>::default())
            .init_resource::<TerrainSettings>()
            .add_systems(Update, (terrain_keys, update_terrain).chain());
    }
}

/// The terrain's uniform block (binding 100), read as `terrain` in the shader.
#[derive(ShaderType, Reflect, Clone, Copy, Debug, Default)]
pub struct TerrainUniform {
    /// x, y: world x and z of texel (0, 0); z: world distance between texels;
    /// w: height scale.
    pub grid: Vec4,
    /// x, y: texels across (i, +x) and deep (j, −z); z: towns tinted by
    /// culture (the culture overlay; 0 = off); w: debug view.
    pub dims: Vec4,
    /// The toon parameters of step 02 (shadow tint; wrap and band edges);
    /// `debug.y` of the toon uniform turns toon on (1) or PBR (0).
    pub toon: ToonUniform,
    /// The culture overlay: per town, x, z of its centre and the radius of its
    /// tint (`dims.z` of them are used)...
    pub towns: [Vec4; MAX_TINTED],
    /// ...and its culture colour (linear rgb).
    pub cultures: [Vec4; MAX_TINTED],
}

/// How many towns the culture overlay can tint (the uniform's array size).
pub const MAX_TINTED: usize = 16;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct TerrainExt {
    #[uniform(100)]
    pub terrain: TerrainUniform,
    /// Heights in world units. 32-bit float textures can't be filtered on all
    /// GPUs, so the shader reads texels directly with `textureLoad`.
    #[texture(101, sample_type = "float", filterable = false)]
    pub heights: Handle<Image>,
    #[texture(102)]
    pub biomes: Handle<Image>,
    /// The fog, shared by all materials (`fog::FOG_BUFFER`).
    #[storage(103, read_only)]
    pub fog: Handle<ShaderBuffer>,
}

impl MaterialExtension for TerrainExt {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
    /// The prepass, which also renders the shadow maps, must displace the
    /// grid too, or the terrain would cast the shadow of a flat plane.
    fn prepass_vertex_shader() -> ShaderRef {
        "shaders/terrain_prepass.wesl".into()
    }
}

/// The generated world as textures, plus where they sit in the 3D scene.
pub struct TerrainData {
    /// Texels across (+x) and deep (−z).
    pub width: usize,
    pub depth: usize,
    /// 3D position of texel (0, 0), and the distance between texels.
    pub origin: Vec2,
    pub cell: f32,
    /// Height of each texel, row by row (j outer, i inner).
    pub heights: Vec<f32>,
    /// Biome weights of each texel: meadow, fields, water, unused (0..1).
    pub biomes: Vec<[f32; 4]>,
}

/// Grid step and debug view for the terrain.
#[derive(Resource, Debug)]
pub struct TerrainSettings {
    /// Texels per grid quad: 1, 2 or 4.
    pub step: usize,
    pub debug: u32,
}

impl Default for TerrainSettings {
    fn default() -> Self {
        Self { step: 1, debug: 0 }
    }
}

impl TerrainSettings {
    pub const VIEWS: [&'static str; 5] = ["final", "height", "normals", "biomes", "slope"];
}

/// Marks the terrain entity; keeps what's needed to rebuild its grid.
#[derive(Component)]
pub struct TerrainGrid {
    width: usize,
    depth: usize,
    origin: Vec2,
    cell: f32,
    step: usize,
}

/// A flat grid: one vertex every `step` texels, all at y = 0, normals up.
/// The UV is the texel position, so the shader could also find its texel
/// from it; we use the world position instead.
fn flat_grid(width: usize, depth: usize, origin: Vec2, cell: f32, step: usize) -> Mesh {
    let cols: Vec<usize> = (0..width).step_by(step).chain([width - 1]).collect();
    let rows: Vec<usize> = (0..depth).step_by(step).chain([depth - 1]).collect();
    let dedup = |v: Vec<usize>| {
        let mut v = v;
        v.dedup();
        v
    };
    let (cols, rows) = (dedup(cols), dedup(rows));
    let (nc, nr) = (cols.len(), rows.len());
    let mut pos = Vec::with_capacity(nc * nr);
    let mut uv = Vec::with_capacity(nc * nr);
    for &j in &rows {
        for &i in &cols {
            pos.push([origin.x + i as f32 * cell, 0.0, origin.y - j as f32 * cell]);
            uv.push([i as f32 / (width - 1) as f32, j as f32 / (depth - 1) as f32]);
        }
    }
    let mut idx = Vec::with_capacity((nc - 1) * (nr - 1) * 6);
    for r in 0..nr - 1 {
        for c in 0..nc - 1 {
            let a = (r * nc + c) as u32;
            let (b, cc, d) = (a + 1, a + nc as u32, a + nc as u32 + 1);
            // Counter-clockwise seen from above (rows run toward −z).
            idx.extend_from_slice(&[a, b, cc, b, d, cc]);
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; nc * nr]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// Spawn the terrain: upload the textures, build the grid, add the material.
pub fn spawn_terrain(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<TerrainMaterial>,
    data: &TerrainData,
) -> Handle<Image> {
    let size = Extent3d {
        width: data.width as u32,
        height: data.depth as u32,
        depth_or_array_layers: 1,
    };
    let height_bytes: Vec<u8> = data.heights.iter().flat_map(|h| h.to_le_bytes()).collect();
    let heights = Image::new(
        size,
        TextureDimension::D2,
        height_bytes,
        TextureFormat::R32Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    let biome_bytes: Vec<u8> = data
        .biomes
        .iter()
        .flat_map(|b| b.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8))
        .collect();
    let biomes = Image::new(
        size,
        TextureDimension::D2,
        biome_bytes,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    let heights = images.add(heights);
    let material = ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            ..default()
        },
        extension: TerrainExt {
            terrain: TerrainUniform {
                grid: Vec4::new(data.origin.x, data.origin.y, data.cell, 1.0),
                dims: Vec4::new(data.width as f32, data.depth as f32, 0.0, 0.0),
                // No rim light on the ground: hills would glow along every ridge.
                toon: ToonUniform {
                    rim: Vec4::ZERO,
                    ..default()
                },
                towns: [Vec4::ZERO; MAX_TINTED],
                cultures: [Vec4::ZERO; MAX_TINTED],
            },
            heights: heights.clone(),
            biomes: images.add(biomes),
            fog: FOG_BUFFER,
        },
    };
    let step = 1;
    commands.spawn((
        TerrainGrid {
            width: data.width,
            depth: data.depth,
            origin: data.origin,
            cell: data.cell,
            step,
        },
        Mesh3d(meshes.add(flat_grid(
            data.width,
            data.depth,
            data.origin,
            data.cell,
            step,
        ))),
        MeshMaterial3d(materials.add(material)),
        // The mesh is flat, so its bounding box says nothing about the hills
        // the vertex shader makes: don't let Bevy cull it by that box.
        NoFrustumCulling,
    ));
    heights
}

fn terrain_keys(
    keys: Res<ButtonInput<KeyCode>>,
    target: Res<DebugTarget>,
    mut settings: ResMut<TerrainSettings>,
) {
    if keys.just_pressed(KeyCode::KeyL) {
        settings.step = match settings.step {
            1 => 2,
            2 => 4,
            _ => 1,
        };
    }
    if *target == DebugTarget::Terrain {
        if let Some(v) = digit_pressed(&keys) {
            settings.debug = v.min(4);
        }
    }
}

/// Settings into the material; rebuild the grid when its step changes.
fn update_terrain(
    settings: Res<TerrainSettings>,
    toon: Res<ToonSettings>,
    mut grids: Query<(
        &mut TerrainGrid,
        &mut Mesh3d,
        &MeshMaterial3d<TerrainMaterial>,
    )>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
) {
    if !settings.is_changed() && !toon.is_changed() {
        return;
    }
    for (mut grid, mut mesh, handle) in &mut grids {
        if grid.step != settings.step {
            grid.step = settings.step;
            mesh.0 = meshes.add(flat_grid(
                grid.width,
                grid.depth,
                grid.origin,
                grid.cell,
                grid.step,
            ));
        }
        if let Some(mut m) = materials.get_mut(handle) {
            m.extension.terrain.dims.w = settings.debug as f32;
            m.extension.terrain.toon.debug.y = toon.on as u32 as f32;
        }
    }
}
