//! M4: the people of the simulation as one crowd.
//!
//! Every frame:
//! - each person walks toward their target from the newest snapshot (their
//!   workplace, home or the squad's goal). The sim only knows where people are
//!   heading once per tick; the view moves them smoothly in between;
//! - the CPU writes one entry per person into a storage buffer: feet position,
//!   heading, colour, size, and whether they are walking;
//! - `people.wesl` draws them all from one mesh (vertex pulling, as for the
//!   grass) and animates the walk.
//!
//! Colours: workers by the skill domain of what they worked at today, soldiers
//! dark red, children small and pale, idle people light, resting ones blue-grey.
//! Key: `P` shows or hides the people.

use crate::fog::FogState;
use crate::live::LiveSim;
use crate::toon::{ToonSettings, ToonUniform};
use crate::{height, World3d, SCALE};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use sim_core::view::Doing;
use std::collections::HashMap;

pub type CrowdMaterial = ExtendedMaterial<StandardMaterial, CrowdExt>;

/// Vertices of one figure (must match `VERTS_PER_PERSON` in people.wesl).
pub const VERTS_PER_PERSON: usize = 60;
/// Walking speed in 3D units per second (a house is 1 unit).
const WALK: f32 = 3.0;

pub struct CrowdPlugin;

impl Plugin for CrowdPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<CrowdMaterial>::default())
            .init_resource::<Walkers>()
            .add_systems(Startup, spawn_crowd)
            .add_systems(Update, (move_people, update_crowd_material));
    }
}

#[derive(ShaderType, Reflect, Clone, Copy, Debug)]
pub struct CrowdUniform {
    pub toon: ToonUniform,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct CrowdExt {
    #[uniform(100)]
    pub crowd: CrowdUniform,
    /// 8 floats per person: feet xyz, heading, colour rgb, size (negative =
    /// walking, 0 = empty slot).
    #[storage(101, read_only)]
    pub people: Handle<ShaderBuffer>,
}

impl MaterialExtension for CrowdExt {
    fn vertex_shader() -> ShaderRef {
        "shaders/people.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/people.wesl".into()
    }
}

/// The figure: a tapered six-sided body (36 vertices) and an octahedron head
/// (24), flat-shaded, about 0.75 units tall, facing +z.
fn figure() -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let mut pos = Vec::with_capacity(VERTS_PER_PERSON);
    let mut nrm = Vec::with_capacity(VERTS_PER_PERSON);
    let mut tri = |a: Vec3, b: Vec3, c: Vec3| {
        // Listed clockwise as seen from outside; emitted counter-clockwise (the
        // front face) with the normal pointing out.
        let n = (c - a).cross(b - a).normalize_or_zero();
        for p in [a, c, b] {
            pos.push(p.to_array());
            nrm.push(n.to_array());
        }
    };
    let ring = |r: f32, y: f32, k: usize| {
        let a = k as f32 * std::f32::consts::TAU / 6.0;
        Vec3::new(r * a.sin(), y, r * a.cos())
    };
    for k in 0..6 {
        let (b0, b1) = (ring(0.10, 0.0, k), ring(0.10, 0.0, k + 1));
        let (t0, t1) = (ring(0.14, 0.5, k), ring(0.14, 0.5, k + 1));
        tri(b0, b1, t1);
        tri(b0, t1, t0);
    }
    let head = Vec3::new(0.0, 0.64, 0.0);
    let r = 0.11;
    let ax = [Vec3::X, Vec3::Z, -Vec3::X, -Vec3::Z];
    for k in 0..4 {
        let (a, b) = (head + ax[k] * r, head + ax[(k + 1) % 4] * r);
        tri(a, b, head + Vec3::Y * r);
        tri(b, a, head - Vec3::Y * r);
    }
    assert_eq!(pos.len(), VERTS_PER_PERSON);
    (pos, nrm)
}

/// A mesh with room for `n` people: n copies of the figure.
fn crowd_mesh(n: usize) -> Mesh {
    let (fp, fnrm) = figure();
    let mut pos = Vec::with_capacity(n * VERTS_PER_PERSON);
    let mut nrm = Vec::with_capacity(n * VERTS_PER_PERSON);
    for _ in 0..n {
        pos.extend_from_slice(&fp);
        nrm.extend_from_slice(&fnrm);
    }
    let count = pos.len();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; count]);
    mesh.insert_indices(Indices::U32((0..count as u32).collect()));
    mesh
}

/// The crowd entity, its buffer, and how many people its mesh has room for.
#[derive(Resource)]
struct CrowdHandles {
    entity: Entity,
    buffer: Handle<ShaderBuffer>,
    capacity: usize,
}

/// Where each person is right now (by agent id) and which way they face.
#[derive(Resource, Default)]
struct Walkers(HashMap<u32, (Vec3, f32)>);

fn spawn_crowd(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut materials: ResMut<Assets<CrowdMaterial>>,
) {
    let capacity = 4096;
    let buffer = buffers.add(ShaderBuffer::from(vec![[0.0f32; 8]; capacity]));
    let material = materials.add(ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            ..default()
        },
        extension: CrowdExt {
            crowd: CrowdUniform {
                toon: ToonUniform::default(),
            },
            people: buffer.clone(),
        },
    });
    let entity = commands
        .spawn((
            Mesh3d(meshes.add(crowd_mesh(capacity))),
            MeshMaterial3d(material),
            Transform::default(),
            // The mesh's own bounds are one figure at the origin.
            NoFrustumCulling,
            NotShadowCaster,
        ))
        .id();
    commands.insert_resource(CrowdHandles {
        entity,
        buffer,
        capacity,
    });
}

/// A small fixed offset per person, so people heading for the same building
/// stand around it instead of on one spot.
fn jitter(id: u32) -> Vec2 {
    let mut h = id.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    let a = (h & 0xFFFF) as f32 / 65536.0 * std::f32::consts::TAU;
    let r = 0.7 + ((h >> 16) & 0xFF) as f32 / 255.0 * 1.3;
    Vec2::new(a.cos(), a.sin()) * r
}

/// The colour of a person (linear RGB) and their size.
fn look(doing: Doing, domains: u32) -> ([f32; 3], f32) {
    let lin = |c: Color| c.to_linear().to_f32_array_no_alpha();
    match doing {
        Doing::Working(d) => (
            lin(Color::hsl(
                360.0 * d as f32 / domains.max(1) as f32,
                0.55,
                0.5,
            )),
            1.0,
        ),
        Doing::Child => (lin(Color::srgb(0.92, 0.90, 0.84)), 0.65),
        Doing::Idle => (lin(Color::srgb(0.86, 0.82, 0.74)), 1.0),
        Doing::Resting => (lin(Color::srgb(0.55, 0.58, 0.70)), 1.0),
        Doing::Soldier => (lin(Color::srgb(0.62, 0.14, 0.12)), 1.0),
        Doing::Captive => (lin(Color::srgb(0.30, 0.30, 0.32)), 0.95),
    }
}

#[allow(clippy::too_many_arguments)]
fn move_people(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    live: Res<LiveSim>,
    world: Res<World3d>,
    mut walkers: ResMut<Walkers>,
    handles: Option<ResMut<CrowdHandles>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut crowd: Query<(&mut Mesh3d, &mut Visibility)>,
) {
    let Some(mut handles) = handles else { return };
    if keys.just_pressed(KeyCode::KeyP) {
        if let Ok((_, mut v)) = crowd.get_mut(handles.entity) {
            v.toggle_visible_hidden();
        }
    }
    let dt = time.delta_secs().min(0.1);
    let snap = live.snapshot.clone();
    let people = &snap.people;
    let t = &world.terrain;
    let mut data: Vec<[f32; 8]> = Vec::with_capacity(people.id.len());
    let mut seen = Vec::with_capacity(people.id.len());
    for k in 0..people.id.len() {
        let id = people.id[k];
        seen.push(id);
        let target = Vec2::new(people.target[k].0, -people.target[k].1) / SCALE + jitter(id);
        let entry = walkers
            .0
            .entry(id)
            .or_insert_with(|| (Vec3::new(target.x, 0.0, target.y), 0.0));
        let here = Vec2::new(entry.0.x, entry.0.z);
        let to = target - here;
        let dist = to.length();
        // Walk; far-off targets (a new home, a raid) are reached quicker.
        let step = (WALK.max(dist * 0.3) * dt).min(dist);
        let walking = dist > 0.05;
        let next = if walking {
            here + to / dist * step
        } else {
            here
        };
        if walking {
            entry.1 = to.x.atan2(to.y);
        }
        let ground = height(t, (next.x * SCALE, -next.y * SCALE));
        entry.0 = Vec3::new(next.x, ground, next.y);
        let (rgb, size) = look(people.doing[k], snap.domains);
        data.push([
            entry.0.x,
            entry.0.y,
            entry.0.z,
            entry.1,
            rgb[0],
            rgb[1],
            rgb[2],
            if walking { -size } else { size },
        ]);
    }
    // Forget people who are gone (died, or no longer in the snapshot).
    if walkers.0.len() > seen.len() {
        let keep: std::collections::HashSet<u32> = seen.into_iter().collect();
        walkers.0.retain(|id, _| keep.contains(id));
    }
    // Grow the mesh when the population outgrows it.
    if data.len() > handles.capacity {
        handles.capacity = (data.len() * 3 / 2).next_power_of_two();
        if let Ok((mut mesh, _)) = crowd.get_mut(handles.entity) {
            mesh.0 = meshes.add(crowd_mesh(handles.capacity));
        }
    }
    // Empty slots: size 0, so their figures collapse to a point.
    data.resize(handles.capacity, [0.0; 8]);
    if let Some(mut buffer) = buffers.get_mut(&handles.buffer) {
        buffer.clear();
        buffer.extend_from_slice(&data);
    }
}

fn update_crowd_material(
    toon: Res<ToonSettings>,
    fog: Res<FogState>,
    mut materials: ResMut<Assets<CrowdMaterial>>,
) {
    if !toon.is_changed() && !fog.is_changed() {
        return;
    }
    for (_, m) in materials.iter_mut() {
        m.extension.crowd.toon.fog = fog.0;
        m.extension.crowd.toon.debug.y = toon.on as u32 as f32;
        m.extension.crowd.toon.rim.x = if toon.rim { 1.0 } else { 0.0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_figure_has_the_vertex_count_the_shader_expects() {
        assert_eq!(figure().0.len(), VERTS_PER_PERSON);
        assert_eq!(crowd_mesh(3).count_vertices(), 3 * VERTS_PER_PERSON);
    }

    #[test]
    fn jitter_spreads_people_around_their_target() {
        for id in 0..200 {
            let j = jitter(id).length();
            assert!((0.69..=2.01).contains(&j), "{j}");
        }
        assert_ne!(jitter(1), jitter(2));
    }
}
