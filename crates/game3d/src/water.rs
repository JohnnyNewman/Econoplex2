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
//! - **flow**: in the rivers the ripples and foam drift downstream, along a
//!   flow map baked from the rivers' courses (`flow_map`);
//! - **foam** along the shore, moving a little;
//! - **ice** in deep winter: pale, matt and still;
//! - lamp light, the shared fog and the toon lighting with shadows like
//!   everything else.
//!
//! Everything that changes over time comes from the shared buffer
//! (`fog::FOG_BUFFER`: sun, wind, wetness, season), so the material itself
//! never changes after it is made.

use crate::fog::FOG_BUFFER;
use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use sim_core::config::RiverSpec;

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
    /// Which way the water flows, per heightmap texel (`flow_map`).
    #[texture(103, sample_type = "float", filterable = false)]
    pub flow: Handle<Image>,
}

impl MaterialExtension for WaterExt {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
}

/// The flow map: per heightmap texel (corner `i`, `j` at map position
/// `origin + (i, j) * cell`), which way a river flows there. rg: the 3D
/// direction (x, z) mapped to 0..1; b: 1 in a river (fading over its banks),
/// 0 elsewhere (the sea and lakes keep the wind). Downstream is the order of
/// each river's points.
pub fn flow_map(rivers: &[RiverSpec], origin: (f32, f32), cell: f32, w: usize, h: usize) -> Image {
    let mut data = Vec::with_capacity(w * h * 4);
    for j in 0..h {
        for i in 0..w {
            let p = Vec2::new(origin.0 + i as f32 * cell, origin.1 + j as f32 * cell);
            // The nearest river segment within reach of this texel.
            let mut best: Option<(f32, Vec2, f32)> = None;
            for r in rivers {
                let reach = r.width / 2.0 + cell;
                for seg in r.points.windows(2) {
                    let (a, b) = (Vec2::new(seg[0].0, seg[0].1), Vec2::new(seg[1].0, seg[1].1));
                    let ab = b - a;
                    let u = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                    let d = (a + ab * u).distance(p);
                    if d <= reach && best.is_none_or(|(bd, _, _)| d < bd) {
                        best = Some((d, ab.normalize_or_zero(), reach));
                    }
                }
            }
            let (dir, strength) = match best {
                // Map (x, y) is 3D (x, -z).
                Some((d, dir, reach)) => (Vec2::new(dir.x, -dir.y), 1.0 - (d / reach).powi(4)),
                None => (Vec2::ZERO, 0.0),
            };
            let enc = |v: f32| ((v * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
            data.extend([
                enc(dir.x),
                enc(dir.y),
                (strength.clamp(0.0, 1.0) * 255.0).round() as u8,
                255,
            ]);
        }
    }
    Image::new(
        Extent3d {
            width: w as u32,
            height: h as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Spawn the water plane at `level` over the map, reading the terrain's
/// heightmap (`grid` and `dims` as in `crowd::CrowdTerrain`) and the flow map.
#[allow(clippy::too_many_arguments)]
pub fn spawn_water(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<WaterMaterial>,
    heights: Handle<Image>,
    flow: Handle<Image>,
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
            flow,
        },
    });
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(400.0, 400.0))),
        MeshMaterial3d(material),
        Transform::from_xyz(0.0, level, 0.0),
        NotShadowCaster,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flow_map_follows_the_river_downstream_and_is_still_elsewhere() {
        // A river running along +x in map units, 20 wide, at y = 0.
        let river = RiverSpec {
            points: vec![(0.0, 0.0), (100.0, 0.0)],
            width: 20.0,
            fords: vec![],
            ford_width: 0.0,
            ford_cost: 1.0,
        };
        let img = flow_map(&[river], (0.0, -50.0), 10.0, 11, 11);
        let data = img.data.as_ref().unwrap();
        let at = |i: usize, j: usize| &data[(j * 11 + i) * 4..(j * 11 + i) * 4 + 4];
        // On the river (j = 5 is y = 0): flowing +x, full strength.
        let mid = at(5, 5);
        assert!(mid[0] > 250 && (126..=129).contains(&mid[1]) && mid[2] > 250);
        // Far from it: no flow.
        assert_eq!(at(5, 0)[2], 0);
    }
}
