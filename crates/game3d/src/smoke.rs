//! Chimney smoke (shader step 16, docs/shaders/16-building-life.md).
//!
//! Smoke rises from the chimneys of the two-storey houses (hearths: a thin
//! pale wisp in summer, more in autumn and winter, when every house heats)
//! and from workplaces with a chimney while they work (bakeries pale,
//! smithies and furnaces dark and thick).
//!
//! Like rain (precip.rs), the smoke is one mesh of small quads per town whose
//! vertices never move on the CPU: each quad is one puff, and all its four
//! vertices hold the chimney top (the position), which corner they are (the
//! UV) and the puff's traits (the colour: how dark, a random seed, its slot
//! in the plume, hearth or workplace). The vertex shader
//! (`assets/shaders/smoke.wesl`) lifts each puff from the chimney over a few
//! seconds, grows it, drifts it with the wind and fades it out, then starts
//! it again at the top: a plume of a few puffs at different ages.
//!
//! Wind, season and daylight come from the shared buffer (`fog::FOG_BUFFER`)
//! and the time from Bevy's globals, so the material never changes. A town's
//! mesh is built again only when its chimneys change or a workplace starts or
//! stops (a few times a day).

use crate::fog::FOG_BUFFER;
use crate::live::LiveSim;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use std::collections::HashMap;

const SHADER: &str = "shaders/smoke.wesl";

/// Puffs in one plume.
pub const PUFFS: usize = 6;

pub struct SmokePlugin;

impl Plugin for SmokePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<SmokeMaterial>::default())
            .init_resource::<Chimneys>()
            .add_systems(Update, update_smoke);
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct SmokeMaterial {
    /// The shared fog, sun, wind and season (`fog::FOG_BUFFER`).
    #[storage(0, read_only)]
    pub env: Handle<ShaderBuffer>,
}

impl Material for SmokeMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

/// The chimney tops in the scene (3D), kept by live.rs as houses and
/// buildings come and go.
#[derive(Resource, Default)]
pub struct Chimneys {
    /// Per town, its houses' chimneys (hearths).
    pub houses: Vec<Vec<Vec3>>,
    /// Workplaces with a chimney by sim id: (town, chimney top, how dark the
    /// smoke is 0..1).
    pub buildings: HashMap<u64, (usize, Vec3, f32)>,
    /// Something changed since the meshes were built.
    pub dirty: bool,
}

/// One plume: where it rises, how dark (0..1), and whether it is a
/// workplace's (always full) or a hearth's (by the season).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plume {
    pub at: Vec3,
    pub dark: f32,
    pub workplace: bool,
}

/// The mesh for a town's plumes: `PUFFS` quads each, see the module doc.
pub fn smoke_mesh(plumes: &[Plume]) -> Mesh {
    let n = plumes.len() * PUFFS;
    let mut pos = Vec::with_capacity(n * 4);
    let mut uv = Vec::with_capacity(n * 4);
    let mut col = Vec::with_capacity(n * 4);
    let mut idx = Vec::with_capacity(n * 6);
    for (i, p) in plumes.iter().enumerate() {
        // A seed from the chimney's place: steady across rebuilds.
        let seed = (p.at.x * 12.9898 + p.at.z * 78.233).sin().abs() * 43758.547 % 1.0;
        for k in 0..PUFFS {
            let data = [
                p.dark,
                seed,
                k as f32 / PUFFS as f32,
                if p.workplace { 1.0 } else { 0.0 },
            ];
            for c in [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
                pos.push(p.at.to_array());
                uv.push(c);
                col.push(data);
            }
            let b = ((i * PUFFS + k) * 4) as u32;
            idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n * 4]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// The plumes of every town: all hearths, and the workplaces at work.
pub fn plumes(chimneys: &Chimneys, working: &HashMap<u64, bool>, towns: usize) -> Vec<Vec<Plume>> {
    let mut out: Vec<Vec<Plume>> = (0..towns.max(chimneys.houses.len()))
        .map(|t| {
            chimneys
                .houses
                .get(t)
                .map(|h| {
                    h.iter()
                        .map(|&at| Plume {
                            at,
                            dark: 0.0,
                            workplace: false,
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();
    let mut ids: Vec<&u64> = chimneys.buildings.keys().collect();
    ids.sort();
    for id in ids {
        let (t, at, dark) = chimneys.buildings[id];
        if working.get(id).copied().unwrap_or(false) && t < out.len() {
            out[t].push(Plume {
                at,
                dark,
                workplace: true,
            });
        }
    }
    out
}

/// The smoke of one town.
#[derive(Component)]
struct TownSmoke(usize);

/// Build a town's smoke again when its plumes changed.
#[allow(clippy::too_many_arguments)]
fn update_smoke(
    mut commands: Commands,
    live: Option<Res<LiveSim>>,
    mut chimneys: ResMut<Chimneys>,
    mut last: Local<(Option<u64>, Vec<Vec<Plume>>)>,
    mut material: Local<Option<Handle<SmokeMaterial>>>,
    mut materials: ResMut<Assets<SmokeMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut smoke: Query<(&TownSmoke, &mut Mesh3d, &mut Visibility)>,
) {
    let Some(live) = live else { return };
    let tick = live.snapshot.tick;
    if last.0 == Some(tick) && !chimneys.dirty {
        return;
    }
    last.0 = Some(tick);
    chimneys.dirty = false;
    let working: HashMap<u64, bool> = live
        .snapshot
        .towns
        .iter()
        .flat_map(|t| t.buildings.iter().map(|b| (b.id, b.progress.is_some())))
        .collect();
    let now = plumes(&chimneys, &working, live.snapshot.towns.len());
    let material = material
        .get_or_insert_with(|| materials.add(SmokeMaterial { env: FOG_BUFFER }))
        .clone();
    for (t, p) in now.iter().enumerate() {
        if last.1.get(t) == Some(p) {
            continue;
        }
        // A town without smoke keeps no mesh (an empty one is an error).
        let existing = smoke.iter_mut().find(|(s, _, _)| s.0 == t);
        if p.is_empty() {
            if let Some((_, _, mut vis)) = existing {
                *vis = Visibility::Hidden;
            }
            continue;
        }
        let mesh = meshes.add(smoke_mesh(p));
        if let Some((_, mut m, mut vis)) = existing {
            m.0 = mesh;
            *vis = Visibility::Inherited;
        } else {
            commands.spawn((
                TownSmoke(t),
                Mesh3d(mesh),
                MeshMaterial3d(material.clone()),
                // The shader moves the puffs: no culling by the chimneys'
                // box, no shadows.
                NoFrustumCulling,
                NotShadowCaster,
                NotShadowReceiver,
                Transform::default(),
                Visibility::Inherited,
            ));
        }
    }
    last.1 = now;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_plume_is_a_few_puffs_of_four_corners() {
        let p = Plume {
            at: Vec3::new(1.0, 2.0, 3.0),
            dark: 0.5,
            workplace: true,
        };
        let m = smoke_mesh(&[p, p]);
        assert_eq!(m.count_vertices(), 2 * PUFFS * 4);
        assert_eq!(m.indices().unwrap().len(), 2 * PUFFS * 6);
    }

    #[test]
    fn workplaces_smoke_only_while_they_work() {
        let mut c = Chimneys {
            houses: vec![vec![Vec3::ZERO]],
            ..default()
        };
        c.buildings.insert(7, (0, Vec3::X, 1.0));
        c.buildings.insert(8, (0, Vec3::Y, 0.2));
        let working = HashMap::from([(7, true), (8, false)]);
        let p = plumes(&c, &working, 1);
        assert_eq!(p[0].len(), 2);
        assert!(p[0].iter().any(|q| q.workplace && q.at == Vec3::X));
        assert!(!p[0].iter().any(|q| q.at == Vec3::Y));
    }
}
