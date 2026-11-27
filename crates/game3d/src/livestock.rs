//! Livestock: flocks of sheep grazing on the towns' pastures (step 19,
//! docs/shaders/19-livestock.md).
//!
//! The sim keeps pastures (nature sites the wool comes from) and how much
//! grass stands on each (`TownView::pastures`). Each pasture carries a flock
//! of four to nine sheep, more the more grass it has. A town's sheep are
//! baked into one mesh, built again only when its pastures change.
//!
//! They move on the GPU (`assets/shaders/graze.wesl`), like the trees sway:
//! each vertex carries its sheep's home spot (UV 1) and whether it belongs
//! to the head (UV 0 x). The vertex shader lets every sheep wander slowly
//! around its spot and dip its head to graze. The fragment shader is the toon
//! shader (toon.wesl), unchanged.

use crate::fog::FOG_BUFFER;
use crate::live::LiveSim;
use crate::toon::ToonUniform;
use crate::{height, World3d, SCALE};
use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

pub type GrazeMaterial = ExtendedMaterial<StandardMaterial, GrazeExt>;

pub struct LivestockPlugin;

impl Plugin for LivestockPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<GrazeMaterial>::default())
            .add_systems(Update, update_flocks);
    }
}

/// The toon material with a grazing vertex shader. The uniform block is the
/// same as the toon material's, so `toon.wesl` reads it.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct GrazeExt {
    #[uniform(100)]
    pub toon: ToonUniform,
    /// The fog, shared by all materials (`fog::FOG_BUFFER`).
    #[storage(101, read_only)]
    pub fog: Handle<ShaderBuffer>,
}

impl MaterialExtension for GrazeExt {
    fn vertex_shader() -> ShaderRef {
        "shaders/graze.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/toon.wesl".into()
    }
}

/// Sheep a pasture with `grass` (0..1) carries.
pub fn flock_size(grass: f32) -> usize {
    4 + (grass.clamp(0.0, 1.0) * 5.0).round() as usize
}

/// A small random number for sheep `k` of a flock at `p`, 0..1.
fn unit(p: Vec2, k: usize, salt: u32) -> f32 {
    let h =
        (p.x * 12.9898 + p.y * 78.233 + k as f32 * 37.719 + salt as f32 * 4.1).sin() * 43758.547;
    h.fract().abs()
}

/// One sheep: (part centre, half size, colour, head?) in its own frame
/// (x across, y up, z forward; about 0.4 long).
fn sheep_parts(wool: [f32; 4]) -> Vec<(Vec3, Vec3, [f32; 4], bool)> {
    let face = [0.06, 0.05, 0.05, 1.0];
    let mut parts = vec![
        (
            Vec3::new(0.0, 0.2, 0.0),
            Vec3::new(0.11, 0.09, 0.17),
            wool,
            false,
        ),
        (
            Vec3::new(0.0, 0.25, 0.2),
            Vec3::new(0.05, 0.05, 0.06),
            face,
            true,
        ),
    ];
    for (x, z) in [(0.07, 0.11), (-0.07, 0.11), (0.07, -0.11), (-0.07, -0.11)] {
        parts.push((
            Vec3::new(x, 0.06, z),
            Vec3::new(0.02, 0.06, 0.02),
            face,
            false,
        ));
    }
    parts
}

/// The mesh of a town's flocks: each sheep's boxes, already in world space,
/// with its home spot in UV 1 and 1 in UV 0 x for the head.
pub fn flock_mesh(sheep: &[(Vec3, f32, [f32; 4])]) -> Mesh {
    let cube: Mesh = Cuboid::new(2.0, 2.0, 2.0).into();
    let cpos = cube
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .unwrap()
        .as_float3()
        .unwrap()
        .to_vec();
    let cnrm = match cube.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(bevy::mesh::VertexAttributeValues::Float32x3(n)) => n.clone(),
        _ => Vec::new(),
    };
    let cidx: Vec<u32> = cube.indices().unwrap().iter().map(|i| i as u32).collect();
    let (mut pos, mut nrm, mut col, mut uv0, mut uv1, mut idx) = (
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    for &(at, heading, wool) in sheep {
        let turn = Quat::from_rotation_y(heading);
        for (centre, half, colour, head) in sheep_parts(wool) {
            let base = pos.len() as u32;
            for (p, n) in cpos.iter().zip(&cnrm) {
                let local = centre + Vec3::from(*p) * half;
                pos.push((at + turn * local).to_array());
                nrm.push((turn * Vec3::from(*n)).to_array());
                col.push(colour);
                uv0.push([if head { 1.0 } else { 0.0 }, 0.0]);
                uv1.push([at.x, at.z]);
            }
            idx.extend(cidx.iter().map(|i| base + i));
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv0);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv1);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// A town's flock entity and the pastures it was built for.
#[derive(Component)]
struct Flock {
    town: usize,
    built: Vec<((f32, f32), usize)>,
}

#[allow(clippy::too_many_arguments)]
/// Build a town's flocks again when its pastures (or their flock sizes)
/// change.
fn update_flocks(
    mut commands: Commands,
    live: Option<Res<LiveSim>>,
    world: Option<Res<World3d>>,
    mut last_tick: Local<Option<u64>>,
    mut material: Local<Option<Handle<GrazeMaterial>>>,
    mut materials: ResMut<Assets<GrazeMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut flocks: Query<(&mut Flock, &mut Mesh3d)>,
) {
    let (Some(live), Some(world)) = (live, world) else {
        return;
    };
    if *last_tick == Some(live.snapshot.tick) {
        return;
    }
    *last_tick = Some(live.snapshot.tick);
    let material = material
        .get_or_insert_with(|| {
            materials.add(ExtendedMaterial {
                base: StandardMaterial {
                    base_color: Color::WHITE,
                    perceptual_roughness: 1.0,
                    ..default()
                },
                extension: GrazeExt {
                    toon: ToonUniform::default(),
                    fog: FOG_BUFFER,
                },
            })
        })
        .clone();
    let t = &world.terrain;
    for (ti, town) in live.snapshot.towns.iter().enumerate() {
        let want: Vec<((f32, f32), usize)> = town
            .pastures
            .iter()
            .map(|&(p, grass)| (p, flock_size(grass)))
            .collect();
        let existing = flocks.iter_mut().find(|(f, _)| f.town == ti);
        if existing.as_ref().is_some_and(|(f, _)| f.built == want)
            || (existing.is_none() && want.is_empty())
        {
            continue;
        }
        // Each flock spreads over a couple of units around its pasture.
        let mut sheep = Vec::new();
        for &(p, n) in &want {
            let centre = Vec2::new(p.0, -p.1) / SCALE;
            for k in 0..n {
                let a = unit(centre, k, 1) * std::f32::consts::TAU;
                let r = 0.4 + 2.0 * unit(centre, k, 2).sqrt();
                let spot = centre + Vec2::new(a.cos(), a.sin()) * r;
                let map = (spot.x * SCALE, -spot.y * SCALE);
                // Not in the river.
                if t.cell_at(map).is_some_and(|c| t.water[c]) {
                    continue;
                }
                let at = Vec3::new(spot.x, height(t, map), spot.y);
                // One in seven is black.
                let wool = if unit(centre, k, 3) < 0.14 {
                    [0.08, 0.07, 0.07, 1.0]
                } else {
                    [0.86, 0.84, 0.78, 1.0]
                };
                sheep.push((at, unit(centre, k, 4) * std::f32::consts::TAU, wool));
            }
        }
        let mesh = meshes.add(flock_mesh(&sheep));
        match existing {
            Some((mut f, mut m)) => {
                f.built = want;
                m.0 = mesh;
            }
            None => {
                commands.spawn((
                    Flock {
                        town: ti,
                        built: want,
                    },
                    Mesh3d(mesh),
                    MeshMaterial3d(material.clone()),
                    Transform::default(),
                    // Too small to matter in the sun's shadow map.
                    NotShadowCaster,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn more_grass_carries_a_bigger_flock() {
        assert_eq!(flock_size(0.0), 4);
        assert_eq!(flock_size(1.0), 9);
        assert!(flock_size(0.5) > flock_size(0.1));
    }

    #[test]
    fn every_sheep_is_six_boxes_with_its_spot_and_a_marked_head() {
        let m = flock_mesh(&[(Vec3::new(1.0, 0.0, 2.0), 0.0, [1.0; 4])]);
        assert_eq!(m.count_vertices(), 6 * 24);
        let Some(bevy::mesh::VertexAttributeValues::Float32x2(uv0)) =
            m.attribute(Mesh::ATTRIBUTE_UV_0)
        else {
            panic!("uv0");
        };
        // The head (second box) is marked, the body not.
        assert_eq!(uv0[0][0], 0.0);
        assert_eq!(uv0[24][0], 1.0);
        let Some(bevy::mesh::VertexAttributeValues::Float32x2(uv1)) =
            m.attribute(Mesh::ATTRIBUTE_UV_1)
        else {
            panic!("uv1");
        };
        assert!(uv1.iter().all(|s| *s == [1.0, 2.0]));
    }
}
