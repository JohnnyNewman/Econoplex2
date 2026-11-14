//! M4: the people of the simulation as one crowd.
//!
//! - **Per tick (CPU):** every person gets a fixed slot. When a new snapshot
//!   arrives, the CPU writes each slot's target (their workplace, home or the
//!   squad's goal), colour and size into the *targets* buffer.
//! - **Per frame (GPU, step 07):** a compute shader (`walk.wesl`, run by
//!   `crowd_compute.rs`) moves everyone toward their target and writes position,
//!   heading, colour and size into the *people* buffer. The sim only says where
//!   people are heading once per tick; the GPU moves them smoothly in between.
//! - **Drawing:** `people.wesl` draws everyone from one mesh (vertex pulling,
//!   as for the grass) straight from the people buffer, and animates them
//!   (step 09): legs and arms are rigid parts turned about their hip or
//!   shoulder. Walkers swing them in step with the distance walked (the compute
//!   pass keeps each person's stride phase, so feet don't slide); workers
//!   standing at their workplace make their trade's motion (`Motion`).
//!
//! - **Level of detail (step 10):** close up, the full figure; farther away, a
//!   card facing the camera with the silhouette drawn by a distance function
//!   in the fragment shader; beyond `far`, nothing. Levels cross-fade with a
//!   dither, so nobody pops. `K` tints people by level; `--no-lod` draws
//!   full figures everywhere (for comparison).
//!
//! Colours: workers by the skill domain of what they worked at today, soldiers
//! dark red, children small and pale, idle people light, resting ones blue-grey.
//! Key: `P` shows or hides the people.

use crate::crowd_compute::{CrowdCompute, WalkParams};
use crate::fog::FOG_BUFFER;
use crate::live::LiveSim;
use crate::toon::{ToonSettings, ToonUniform};
use crate::SCALE;
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

/// Vertices of the full figure.
pub const FIGURE_VERTS: usize = 180;
/// Vertices of one person in the crowd mesh: the figure, then the card (two
/// triangles) for the middle distance. Must match `VERTS_PER_PERSON` in
/// people.wesl.
pub const VERTS_PER_PERSON: usize = FIGURE_VERTS + 6;
/// Floats per person in the people buffer (must match `Person` in people.wesl
/// and walk.wesl).
pub const PERSON_FLOATS: usize = 12;

/// What a person does with their arms when standing still (`Target.motion`,
/// read by people.wesl). Chosen per skill domain by name, so new content gets a
/// motion by naming its domain; unknown domains get `Craft`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    None = 0,
    Scythe = 1,
    Hammer = 2,
    Axe = 3,
    Craft = 4,
    Guard = 5,
}

impl Motion {
    pub fn of_domain(name: &str) -> Motion {
        match name {
            "agriculture" => Motion::Scythe,
            "smithing" | "metallurgy" | "masonry" | "mining" => Motion::Hammer,
            "forestry" | "woodworking" | "construction" => Motion::Axe,
            "military" => Motion::Guard,
            _ => Motion::Craft,
        }
    }

    pub fn of(doing: Doing, domain_names: &[String]) -> Motion {
        match doing {
            Doing::Working(d) => domain_names
                .get(d as usize)
                .map_or(Motion::Craft, |n| Motion::of_domain(n)),
            Doing::Soldier => Motion::Guard,
            _ => Motion::None,
        }
    }
}
/// Walking speed in 3D units per second (a house is 1 unit).
const WALK: f32 = 3.0;

pub struct CrowdPlugin;

impl Plugin for CrowdPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<CrowdMaterial>::default())
            .init_resource::<Slots>()
            .init_resource::<LodSettings>()
            .add_systems(Startup, spawn_crowd)
            .add_systems(Update, (update_targets, update_crowd_material));
    }
}

#[derive(ShaderType, Reflect, Clone, Copy, Debug)]
pub struct CrowdUniform {
    pub toon: ToonUniform,
    /// Level of detail: figure-to-card distance, far cut-off, cross-fade band,
    /// tint by level (1) or not (0).
    pub lod: Vec4,
    /// x: the selected person's slot, or -1 (inspect.rs); yzw unused.
    pub selected: Vec4,
}

/// The level-of-detail distances (3D units) and switches.
#[derive(Resource, Clone, Copy, Debug)]
pub struct LodSettings {
    pub near: f32,
    pub far: f32,
    pub band: f32,
    /// Off: full figures at every distance (`--no-lod`).
    pub on: bool,
    /// Tint people by level (`K`).
    pub show: bool,
}

impl Default for LodSettings {
    fn default() -> Self {
        LodSettings {
            near: 28.0,
            far: 260.0,
            band: 6.0,
            on: true,
            show: false,
        }
    }
}

impl LodSettings {
    pub fn uniform(&self) -> Vec4 {
        if self.on {
            Vec4::new(self.near, self.far, self.band, self.show as u32 as f32)
        } else {
            Vec4::new(1e9, 2e9, self.band, self.show as u32 as f32)
        }
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct CrowdExt {
    #[uniform(100)]
    pub crowd: CrowdUniform,
    /// 12 floats per person: feet xyz, heading, colour rgb, size (negative =
    /// walking, 0 = empty slot), then the motion: kind, stride phase, walking
    /// blend (0 standing .. 1 walking), unused.
    #[storage(101, read_only)]
    pub people: Handle<ShaderBuffer>,
    /// The fog, shared by all materials (`fog::FOG_BUFFER`).
    #[storage(102, read_only)]
    pub fog: Handle<ShaderBuffer>,
}

impl MaterialExtension for CrowdExt {
    fn vertex_shader() -> ShaderRef {
        "shaders/people.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/people.wesl".into()
    }
}

/// The figure, about 0.75 units tall, facing +z, flat-shaded, in parts the
/// shader tells apart by vertex number (`part` in people.wesl):
///
/// | vertices | part |
/// |---|---|
/// | 0..36 | body: a tapered six-sided tube |
/// | 36..60 | head: an octahedron |
/// | 60..90, 90..120 | left and right leg (hip at y 0.32) |
/// | 120..150, 150..180 | left and right arm (shoulder at y 0.53) |
pub fn figure() -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let mut pos = Vec::with_capacity(FIGURE_VERTS);
    let mut nrm = Vec::with_capacity(FIGURE_VERTS);
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
        let (b0, b1) = (ring(0.10, 0.28, k), ring(0.10, 0.28, k + 1));
        let (t0, t1) = (ring(0.14, 0.55, k), ring(0.14, 0.55, k + 1));
        tri(b1, b0, t0);
        tri(b1, t0, t1);
    }
    let head = Vec3::new(0.0, 0.66, 0.0);
    let r = 0.11;
    let ax = [Vec3::X, Vec3::Z, -Vec3::X, -Vec3::Z];
    for k in 0..4 {
        let (a, b) = (head + ax[k] * r, head + ax[(k + 1) % 4] * r);
        tri(a, b, head + Vec3::Y * r);
        tri(b, a, head - Vec3::Y * r);
    }
    // A limb: a four-sided tube from `top` down to `bottom`, `w` thick, with a
    // cap at the bottom (4 sides x 6 + 6 = 30 vertices).
    let mut limb = |top: Vec3, bottom: Vec3, w: f32| {
        let c = [
            Vec3::new(-w, 0.0, -w),
            Vec3::new(-w, 0.0, w),
            Vec3::new(w, 0.0, w),
            Vec3::new(w, 0.0, -w),
        ];
        for k in 0..4 {
            let (a, b) = (c[k], c[(k + 1) % 4]);
            tri(bottom + b, bottom + a, top + a);
            tri(bottom + b, top + a, top + b);
        }
        tri(bottom + c[0], bottom + c[1], bottom + c[2]);
        tri(bottom + c[0], bottom + c[2], bottom + c[3]);
    };
    for x in [0.055, -0.055] {
        limb(Vec3::new(x, 0.32, 0.0), Vec3::new(x, 0.0, 0.0), 0.04);
    }
    for x in [0.17, -0.17] {
        limb(Vec3::new(x, 0.53, 0.0), Vec3::new(x, 0.27, 0.0), 0.03);
    }
    assert_eq!(pos.len(), FIGURE_VERTS);
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
        // The card's corners are placed by the shader (it faces the camera).
        pos.extend_from_slice(&[[0.0; 3]; 6]);
        nrm.extend_from_slice(&[[0.0, 1.0, 0.0]; 6]);
    }
    let count = pos.len();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; count]);
    // A second UV set gives the fragment stage the card coordinates
    // (`VERTEX_UVS_B`); the values are written by the vertex shader.
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0f32, 0.0]; count]);
    mesh.insert_indices(Indices::U32((0..count as u32).collect()));
    mesh
}

/// The crowd entity, its material and buffers, and how many people they have
/// room for.
#[derive(Resource)]
pub struct CrowdHandles {
    pub entity: Entity,
    pub material: Handle<CrowdMaterial>,
    pub people: Handle<ShaderBuffer>,
    pub targets: Handle<ShaderBuffer>,
    pub capacity: usize,
}

/// The terrain heightmap and its placement, for the walking (set by main.rs
/// when the terrain is built).
#[derive(Resource, Clone)]
pub struct CrowdTerrain {
    pub heights: Handle<Image>,
    /// x, z of texel (0, 0); texel spacing; height scale.
    pub grid: Vec4,
    /// Texels across and deep.
    pub dims: Vec4,
}

/// Which slot each person (by agent id) has, and the free slots. A slot freed
/// in one tick is reused only from the next one, so the GPU sees it empty for at
/// least a frame and the next person in it appears at their goal.
#[derive(Resource, Default)]
pub struct Slots {
    of: HashMap<u32, usize>,
    free: Vec<usize>,
    cooling: Vec<usize>,
    next: usize,
    tick: Option<u64>,
}

impl Slots {
    /// The slot of this person (agent store index), if they have one.
    pub fn slot_of(&self, id: u32) -> Option<usize> {
        self.of.get(&id).copied()
    }

    /// Who is in this slot (agent store index), if anyone.
    pub fn id_of(&self, slot: usize) -> Option<u32> {
        self.of.iter().find(|(_, &s)| s == slot).map(|(&id, _)| id)
    }

    /// Bring the slots up to date with the people of a new tick: free the slots
    /// of those who are gone (reusable from the next tick), give newcomers one.
    fn assign(&mut self, ids: &[u32]) {
        let cooled = std::mem::take(&mut self.cooling);
        self.free.extend(cooled);
        let present: std::collections::HashSet<u32> = ids.iter().copied().collect();
        let gone: Vec<(u32, usize)> = self
            .of
            .iter()
            .filter(|(id, _)| !present.contains(id))
            .map(|(&id, &s)| (id, s))
            .collect();
        for (id, s) in gone {
            self.of.remove(&id);
            self.cooling.push(s);
        }
        for &id in ids {
            if !self.of.contains_key(&id) {
                let s = self.free.pop().unwrap_or_else(|| {
                    self.next += 1;
                    self.next - 1
                });
                self.of.insert(id, s);
            }
        }
    }
}

/// An empty people buffer for `n` slots.
fn zeros(n: usize) -> ShaderBuffer {
    ShaderBuffer::from(vec![[0.0f32; PERSON_FLOATS]; n])
}

/// An empty targets buffer for `n` slots (8 floats each).
fn zero_targets(n: usize) -> ShaderBuffer {
    ShaderBuffer::from(vec![[0.0f32; 8]; n])
}

fn spawn_crowd(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut materials: ResMut<Assets<CrowdMaterial>>,
) {
    let capacity = 4096;
    let people = buffers.add(zeros(capacity));
    let targets = buffers.add(zero_targets(capacity));
    let material = materials.add(ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            ..default()
        },
        extension: CrowdExt {
            crowd: CrowdUniform {
                toon: ToonUniform::default(),
                lod: LodSettings::default().uniform(),
                selected: Vec4::new(-1.0, 0.0, 0.0, 0.0),
            },
            people: people.clone(),
            fog: FOG_BUFFER,
        },
    });
    let entity = commands
        .spawn((
            Mesh3d(meshes.add(crowd_mesh(capacity))),
            MeshMaterial3d(material.clone()),
            Transform::default(),
            // The mesh's own bounds are one figure at the origin.
            NoFrustumCulling,
            NotShadowCaster,
        ))
        .id();
    commands.insert_resource(CrowdHandles {
        entity,
        material,
        people,
        targets,
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

/// 0..1 from a person's id and a salt, for the day plan's variations.
fn unit(id: u32, salt: u32) -> f32 {
    let mut h = id.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    (h & 0xFFFF) as f32 / 65536.0
}

/// Where someone is during the day (3D x, z) and whether they are out at all.
/// The simulation decides once a day what everyone does and where they work;
/// in between the view gives them a day: up in the morning, at work in
/// working hours, home in the evening, indoors at night. Those not working
/// stroll between home and the town centre. Soldiers and captives go where
/// the simulation sends them.
struct DayPlan {
    goal: Vec2,
    out: bool,
    at_work: bool,
}

/// One person's plan at `hour`. `work` is the simulation's target, `home` a
/// house of their town, `centre` the town's centre.
fn day_plan(id: u32, doing: Doing, hour: f32, work: Vec2, home: Vec2, centre: Vec2) -> DayPlan {
    // Everyone keeps their own hours, up to an hour either way.
    let h = hour - (unit(id, 1) - 0.5) * 2.0;
    let wake = 5.5;
    let bed = 21.5;
    if matches!(doing, Doing::Soldier | Doing::Captive) {
        return DayPlan {
            goal: work,
            out: true,
            at_work: true,
        };
    }
    if !(wake..bed).contains(&h) {
        return DayPlan {
            goal: home,
            out: false,
            at_work: false,
        };
    }
    if let Doing::Working(_) = doing {
        if (6.5..17.5).contains(&h) {
            return DayPlan {
                goal: work,
                out: true,
                at_work: true,
            };
        }
        return DayPlan {
            goal: home,
            out: true,
            at_work: false,
        };
    }
    // A stroll: every hour and a half somewhere new, near home or towards
    // the centre (children stay closer to home).
    let period = (h / 1.5).floor() as u32;
    let r = if doing == Doing::Child { 2.5 } else { 5.0 };
    let pick = unit(id, 10 + period);
    let a = unit(id, 100 + period) * std::f32::consts::TAU;
    let off = Vec2::new(a.cos(), a.sin()) * r * unit(id, 1000 + period);
    let base = if pick < 0.5 {
        home
    } else {
        home.lerp(centre, 0.7)
    };
    DayPlan {
        goal: base + off,
        out: true,
        at_work: false,
    }
}

/// Where a person lives and works (3D x, z), as their day plan uses them
/// (`None` if they are not in the snapshot).
pub fn spots_of(snap: &sim_core::view::ViewSnapshot, id: u32) -> Option<(Vec2, Vec2)> {
    let k = snap.people.id.iter().position(|&i| i == id)?;
    let work = Vec2::new(snap.people.target[k].0, -snap.people.target[k].1) / SCALE + jitter(id);
    let ti = snap.people.town.get(k).copied().unwrap_or(0) as usize;
    let home = snap
        .towns
        .get(ti)
        .map_or(work, |t| home_of(id, t.pos, ti, t.population));
    Some((home, work))
}

/// A house of the town for this person: a spot on the spiral the houses
/// stand on (live.rs), picked by their id, in 3D x, z.
fn home_of(id: u32, town_pos: (f32, f32), town_index: usize, population: u32) -> Vec2 {
    let n = crate::live::houses_for(population).max(1);
    let k = (unit(id, 7) * n as f32) as u32;
    let a = k as f32 * 2.399_963 + town_index as f32;
    let r = 1.8 + (k as f32).sqrt() * 1.05 + 0.6;
    Vec2::new(
        town_pos.0 / SCALE + r * a.cos(),
        -town_pos.1 / SCALE - r * a.sin(),
    )
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

/// Per tick: slots and targets. Per frame: the compute pass's parameters.
#[allow(clippy::too_many_arguments)]
fn update_targets(
    mut commands: Commands,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    live: Res<LiveSim>,
    terrain: Option<Res<CrowdTerrain>>,
    mut slots: ResMut<Slots>,
    handles: Option<ResMut<CrowdHandles>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<CrowdMaterial>>,
    mut crowd: Query<(&mut Mesh3d, &mut Visibility)>,
    overlay: Res<crate::overlay::Overlay>,
    clock: Res<crate::sky::SkyClock>,
    mut last: Local<Vec<[f32; 8]>>,
) {
    let (Some(mut handles), Some(terrain)) = (handles, terrain) else {
        return;
    };
    let by_culture = *overlay == crate::overlay::Overlay::Culture;
    if keys.just_pressed(KeyCode::KeyP) {
        if let Ok((_, mut v)) = crowd.get_mut(handles.entity) {
            v.toggle_visible_hidden();
        }
    }
    let snap = live.snapshot.clone();
    let new_tick = slots.tick != Some(snap.tick);
    // Every frame: where everyone is in their day (written only when that
    // changes, a few times a game hour). The colours change with the overlay
    // too (culture: everyone in their own).
    {
        let people = &snap.people;
        if new_tick {
            slots.tick = Some(snap.tick);
            slots.assign(&people.id);
        }
        // Outgrown: bigger buffers and mesh (everyone reappears at their goal).
        if slots.next > handles.capacity {
            handles.capacity = (slots.next * 3 / 2).next_power_of_two();
            handles.people = buffers.add(zeros(handles.capacity));
            handles.targets = buffers.add(zero_targets(handles.capacity));
            if let Some(mut m) = materials.get_mut(&handles.material) {
                m.extension.people = handles.people.clone();
            }
            if let Ok((mut mesh, _)) = crowd.get_mut(handles.entity) {
                mesh.0 = meshes.add(crowd_mesh(handles.capacity));
            }
        }
        // The targets, slot by slot (empty slots stay all zero: inactive).
        let mut data = vec![[0.0f32; 8]; handles.capacity];
        for k in 0..people.id.len() {
            let id = people.id[k];
            let doing = people.doing[k];
            let work = Vec2::new(people.target[k].0, -people.target[k].1) / SCALE + jitter(id);
            let ti = people.town.get(k).copied().unwrap_or(0) as usize;
            let (home, centre) = match snap.towns.get(ti) {
                Some(t) => (
                    home_of(id, t.pos, ti, t.population),
                    Vec2::new(t.pos.0, -t.pos.1) / SCALE,
                ),
                None => (work, work),
            };
            let plan = day_plan(id, doing, clock.hour, work, home, centre);
            let (mut rgb, size) = look(doing, snap.domains);
            if let (true, Some(c)) = (by_culture, people.culture.get(k)) {
                rgb = crate::overlay::culture_colour(c);
            }
            // The trade's motion only at work; elsewhere they just stand.
            let motion = if plan.at_work {
                Motion::of(doing, &snap.domain_names) as u32 as f32
            } else {
                0.0
            };
            let occupied = if plan.out { 1.0 } else { 0.0 };
            data[slots.of[&id]] = [
                plan.goal.x,
                plan.goal.y,
                occupied,
                size,
                rgb[0],
                rgb[1],
                rgb[2],
                motion,
            ];
        }
        if *last != data {
            if let Some(mut buffer) = buffers.get_mut(&handles.targets) {
                buffer.clear();
                buffer.extend_from_slice(&data);
            }
            *last = data;
        }
    }
    // Every frame: the compute pass's inputs (copied to the render world).
    commands.insert_resource(CrowdCompute {
        people: handles.people.clone(),
        targets: handles.targets.clone(),
        heights: terrain.heights.clone(),
        params: WalkParams {
            dt: time.delta_secs().min(0.1),
            walk: WALK,
            count: handles.capacity as u32,
            pad: 0,
            grid: terrain.grid,
            dims: terrain.dims,
        },
    });
}

fn update_crowd_material(
    toon: Res<ToonSettings>,
    keys: Res<ButtonInput<KeyCode>>,
    mut lod: ResMut<LodSettings>,
    mut materials: ResMut<Assets<CrowdMaterial>>,
    selected: Res<crate::inspect::Selected>,
    slots: Res<Slots>,
    mut lit: Local<Option<f32>>,
) {
    if keys.just_pressed(KeyCode::KeyK) {
        lod.show = !lod.show;
    }
    // The selected person's slot (it stays the same while they live).
    let slot = selected
        .0
        .and_then(|id| slots.of.get(&id))
        .map_or(-1.0, |&s| s as f32);
    if !toon.is_changed() && !lod.is_changed() && *lit == Some(slot) {
        return;
    }
    *lit = Some(slot);
    for (_, m) in materials.iter_mut() {
        m.extension.crowd.selected.x = slot;
        m.extension.crowd.lod = lod.uniform();
        m.extension.crowd.toon.debug.y = toon.on as u32 as f32;
        m.extension.crowd.toon.rim.x = if toon.rim { 1.0 } else { 0.0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_figure_has_the_vertex_count_the_shader_expects() {
        assert_eq!(figure().0.len(), FIGURE_VERTS);
        assert_eq!(crowd_mesh(3).count_vertices(), 3 * VERTS_PER_PERSON);
    }

    #[test]
    fn the_figure_has_its_parts_where_the_shader_looks() {
        let (pos, _) = figure();
        // Legs end at the feet, arms hang from the shoulders, left on +x.
        let lowest =
            |r: std::ops::Range<usize>| pos[r].iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        assert_eq!(lowest(60..90), 0.0);
        assert_eq!(lowest(90..120), 0.0);
        assert!(pos[60..90].iter().all(|p| p[0] > 0.0));
        assert!(pos[120..150].iter().all(|p| p[0] > 0.0 && p[1] >= 0.27));
        assert!(pos[150..180].iter().all(|p| p[0] < 0.0 && p[1] <= 0.53));
    }

    #[test]
    fn every_face_of_the_figure_points_outward() {
        let (pos, nrm) = figure();
        for (start, end) in [
            (0, 36),
            (36, 60),
            (60, 90),
            (90, 120),
            (120, 150),
            (150, 180),
        ] {
            let n = (end - start) as f32;
            let centre = pos[start..end].iter().map(|&p| Vec3::from(p)).sum::<Vec3>() / n;
            for t in (start..end).step_by(3) {
                let mid =
                    (Vec3::from(pos[t]) + Vec3::from(pos[t + 1]) + Vec3::from(pos[t + 2])) / 3.0;
                assert!(
                    (mid - centre).dot(Vec3::from(nrm[t])) > 0.0,
                    "triangle at vertex {t}"
                );
            }
        }
    }

    #[test]
    fn lod_off_pushes_the_switches_out_of_reach() {
        let on = LodSettings::default();
        assert!(
            on.uniform().x < on.uniform().y,
            "cards between figures and the far cut-off"
        );
        let off = LodSettings { on: false, ..on };
        assert!(off.uniform().x > 1e6 && off.uniform().y > off.uniform().x);
        let shown = LodSettings { show: true, ..on };
        assert_eq!(shown.uniform().w, 1.0);
    }

    #[test]
    fn a_day_takes_workers_to_work_and_everyone_home_at_night() {
        let (work, home, centre) = (
            Vec2::new(10.0, 0.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(3.0, 3.0),
        );
        let at = |id, doing, hour| day_plan(id, doing, hour, work, home, centre);
        for id in 0..50 {
            let noon = at(id, Doing::Working(0), 12.0);
            assert!(noon.out && noon.at_work && noon.goal == work);
            let evening = at(id, Doing::Working(0), 19.5);
            assert!(evening.out && !evening.at_work && evening.goal == home);
            assert!(!at(id, Doing::Working(0), 2.0).out);
            assert!(!at(id, Doing::Resting, 23.9).out);
            assert!(at(id, Doing::Soldier, 2.0).out);
        }
        // Strollers go somewhere new over the day.
        let spots: std::collections::HashSet<(i32, i32)> = (8..18)
            .map(|h| at(3, Doing::Resting, h as f32).goal)
            .map(|g| ((g.x * 10.0) as i32, (g.y * 10.0) as i32))
            .collect();
        assert!(spots.len() > 2);
    }

    #[test]
    fn trades_get_their_motion_and_unknown_ones_a_default() {
        let names: Vec<String> = ["agriculture", "smithing", "forestry", "brewing"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(Motion::of(Doing::Working(0), &names), Motion::Scythe);
        assert_eq!(Motion::of(Doing::Working(1), &names), Motion::Hammer);
        assert_eq!(Motion::of(Doing::Working(2), &names), Motion::Axe);
        assert_eq!(Motion::of(Doing::Working(3), &names), Motion::Craft);
        assert_eq!(Motion::of(Doing::Working(9), &names), Motion::Craft);
        assert_eq!(Motion::of(Doing::Soldier, &names), Motion::Guard);
        assert_eq!(Motion::of(Doing::Idle, &names), Motion::None);
    }

    #[test]
    fn people_keep_their_slot_and_freed_slots_wait_a_tick() {
        let mut slots = Slots::default();
        slots.assign(&[10, 11, 12]);
        let s11 = slots.of[&11];
        // 11 stays put while others come and go.
        slots.assign(&[11, 12, 13]);
        assert_eq!(slots.of[&11], s11);
        let s10_freed = 0; // 10 had slot 0
        assert!(
            !slots.of.values().any(|&s| s == s10_freed),
            "a freed slot is not reused in the same tick"
        );
        // From the next tick on, it can be.
        slots.assign(&[11, 12, 13, 14]);
        assert_eq!(slots.of[&14], s10_freed);
        // Every person has a distinct slot.
        let mut used: Vec<usize> = slots.of.values().copied().collect();
        used.sort();
        used.dedup();
        assert_eq!(used.len(), slots.of.len());
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
