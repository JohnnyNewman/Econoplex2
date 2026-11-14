//! Picking and inspecting individual people.
//!
//! Left-click a person to select them; click empty ground (or `Esc`) to let
//! go. Where people are lives on the GPU (the walking is a compute pass,
//! crowd_compute.rs), so a click asks for one copy of the people buffer back
//! (`ReadbackOnce`) and picks the figure nearest the click's ray when it
//! arrives, a frame or two later. The selected person is lit gold (people.wesl)
//! and the simulation describes them after every tick (`sim_core::view::person`),
//! shown in the "Person" panel (ui.rs).
//!
//! While someone is selected, their slot of the people buffer is read back
//! every frame (48 bytes), so the camera can follow them ("Follow" in the
//! panel; panning with the keys or the middle button stops it), and rings
//! mark their home (gold) and workplace (blue) on the ground.

use crate::camera::UiCapture;
use crate::crowd::{CrowdHandles, Slots, PERSON_FLOATS};
use crate::live::LiveSim;
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete, ReadbackOnce};

pub struct InspectPlugin;

impl Plugin for InspectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pick>()
            .init_resource::<Follow>()
            .add_systems(Startup, spawn_markers)
            .add_systems(
                Update,
                (click, resolve, track, follow, place_markers)
                    .chain()
                    .before(crate::camera::world_camera),
            );
    }
}

/// The person selected (agent store index), if any.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct Selected(pub Option<u32>);

/// The selected person's position (from the GPU, a frame late) and whether
/// the camera follows them.
#[derive(Resource, Default)]
pub struct Follow {
    pub on: bool,
    pub pos: Option<Vec3>,
    /// The slot being read back, and the entity reading it.
    reading: Option<(usize, Entity)>,
}

/// The rings on the ground at the selected person's home and workplace.
#[derive(Component)]
struct Marker {
    home: bool,
}

/// A click waiting for the people buffer to come back from the GPU.
#[derive(Resource, Default)]
struct Pick {
    ray: Option<Ray3d>,
    data: Option<Vec<u8>>,
}

#[allow(clippy::too_many_arguments)]
fn click(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    ui: Option<Res<UiCapture>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    handles: Option<Res<CrowdHandles>>,
    mut pick: ResMut<Pick>,
    mut selected: ResMut<Selected>,
    live: Res<LiveSim>,
) {
    if keys.just_pressed(KeyCode::Escape) && selected.0.is_some() {
        selected.0 = None;
        live.inspect(None);
    }
    if !mouse.just_pressed(MouseButton::Left) || ui.is_some_and(|u| u.pointer) {
        return;
    }
    let (Some(handles), Some(cursor)) = (
        handles,
        windows.iter().next().and_then(|w| w.cursor_position()),
    ) else {
        return;
    };
    let Some((camera, at)) = cameras.iter().find(|(c, _)| c.is_active) else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(at, cursor) else {
        return;
    };
    pick.ray = Some(ray);
    pick.data = None;
    commands
        .spawn(ReadbackOnce::buffer(handles.people.clone()))
        .observe(
            |ev: On<ReadbackComplete>, mut pick: ResMut<Pick>, mut commands: Commands| {
                pick.data = Some(ev.data.clone());
                commands.entity(ev.entity).despawn();
            },
        );
}

/// The slot whose figure is nearest the ray (within reach of it), if any.
/// `floats`: the people buffer, `PERSON_FLOATS` per slot: position xyz,
/// heading, colour rgb, size (0 = empty slot, negative = walking), motion.
pub fn nearest(floats: &[f32], ray: Ray3d) -> Option<usize> {
    let dir = *ray.direction;
    let mut best: Option<(f32, usize)> = None;
    for (slot, p) in floats.as_chunks::<PERSON_FLOATS>().0.iter().enumerate() {
        let size = p[7].abs();
        if size == 0.0 {
            continue;
        }
        // Aim at the middle of the figure (about 1.6 units tall for an adult).
        let centre = Vec3::new(p[0], p[1] + 0.8 * size, p[2]);
        let t = (centre - ray.origin).dot(dir);
        if t <= 0.0 {
            continue;
        }
        let off = (ray.origin + dir * t).distance(centre);
        // The figure's half height, a little generous; nearer figures win ties.
        if off > 0.9 * size {
            continue;
        }
        let score = off / size + t * 1e-3;
        if best.is_none_or(|(b, _)| score < b) {
            best = Some((score, slot));
        }
    }
    best.map(|(_, s)| s)
}

fn resolve(
    mut pick: ResMut<Pick>,
    slots: Res<Slots>,
    mut selected: ResMut<Selected>,
    live: Res<LiveSim>,
) {
    let (Some(ray), Some(data)) = (pick.ray, pick.data.take()) else {
        return;
    };
    pick.ray = None;
    let floats: Vec<f32> = data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    let id = nearest(&floats, ray).and_then(|slot| slots.id_of(slot));
    selected.0 = id;
    live.inspect(id);
}

/// Keep a readback of the selected person's slot running (48 bytes a frame).
fn track(
    mut commands: Commands,
    selected: Res<Selected>,
    slots: Res<Slots>,
    handles: Option<Res<CrowdHandles>>,
    mut follow: ResMut<Follow>,
) {
    let want = selected.0.and_then(|id| slots.slot_of(id));
    if follow.reading.map(|(s, _)| s) == want {
        return;
    }
    if let Some((_, e)) = follow.reading.take() {
        commands.entity(e).despawn();
    }
    follow.pos = None;
    let (Some(slot), Some(handles)) = (want, handles) else {
        follow.on = false;
        return;
    };
    let bytes = (PERSON_FLOATS * 4) as u64;
    let e = commands
        .spawn(Readback::buffer_range(
            handles.people.clone(),
            slot as u64 * bytes,
            bytes,
        ))
        .observe(|ev: On<ReadbackComplete>, mut follow: ResMut<Follow>| {
            let f: Vec<f32> = ev
                .data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            // An empty slot (indoors at night, or gone): keep the last place.
            if f.len() >= 8 && f[7] != 0.0 {
                follow.pos = Some(Vec3::new(f[0], f[1], f[2]));
            }
        })
        .id();
    follow.reading = Some((slot, e));
}

/// With "Follow" on: glide the camera's focus to the selected person. Panning
/// by hand stops it.
fn follow(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut follow: ResMut<Follow>,
    mut cams: Query<&mut crate::camera::WorldCamera>,
) {
    let pan = [
        KeyCode::KeyW,
        KeyCode::KeyA,
        KeyCode::KeyS,
        KeyCode::KeyD,
        KeyCode::Home,
    ];
    if keys.any_pressed(pan) || mouse.pressed(MouseButton::Middle) {
        follow.on = false;
    }
    let (true, Some(p)) = (follow.on, follow.pos) else {
        return;
    };
    let k = 1.0 - (-6.0 * time.delta_secs()).exp();
    for mut cam in &mut cams {
        cam.focus = cam.focus.lerp(p, k);
    }
}

fn spawn_markers(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let ring = meshes.add(Torus::new(0.55, 0.75));
    for (home, colour) in [
        (true, Color::srgb(1.0, 0.75, 0.2)),
        (false, Color::srgb(0.3, 0.6, 1.0)),
    ] {
        commands.spawn((
            Marker { home },
            Mesh3d(ring.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: colour,
                emissive: LinearRgba::from(colour) * 2.0,
                unlit: true,
                ..default()
            })),
            Transform::default(),
            Visibility::Hidden,
            bevy::light::NotShadowCaster,
        ));
    }
}

/// The rings at the selected person's home and workplace (hidden without a
/// selection, and the workplace one for those who don't work).
fn place_markers(
    selected: Res<Selected>,
    live: Res<LiveSim>,
    world: Res<crate::World3d>,
    mut markers: Query<(&Marker, &mut Transform, &mut Visibility)>,
) {
    let spots = selected
        .0
        .and_then(|id| crate::crowd::spots_of(&live.snapshot, id));
    let working = selected
        .0
        .and_then(|id| live.snapshot.people.id.iter().position(|&i| i == id))
        .is_some_and(|k| {
            matches!(
                live.snapshot.people.doing[k],
                sim_core::view::Doing::Working(_)
            )
        });
    for (m, mut t, mut v) in &mut markers {
        let Some((home, work)) = spots else {
            v.set_if_neq(Visibility::Hidden);
            continue;
        };
        if !m.home && !working {
            v.set_if_neq(Visibility::Hidden);
            continue;
        }
        let at = if m.home { home } else { work };
        let y = crate::ground(&world.terrain, (at.x * crate::SCALE, -at.y * crate::SCALE));
        let place = Transform::from_xyz(at.x, y + 0.08, at.y);
        if *t != place {
            *t = place;
        }
        v.set_if_neq(Visibility::Inherited);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(x: f32, z: f32, size: f32) -> [f32; PERSON_FLOATS] {
        let mut p = [0.0; PERSON_FLOATS];
        p[0] = x;
        p[2] = z;
        p[7] = size;
        p
    }

    #[test]
    fn a_click_picks_the_figure_under_it_and_skips_empty_slots() {
        let floats: Vec<f32> = [
            person(0.0, 0.0, 1.0),
            person(5.0, 0.0, 0.0),
            person(5.0, 0.0, -1.0),
        ]
        .concat();
        // Looking straight down at x = 5: slot 1 is empty, slot 2 walks there.
        let ray = Ray3d::new(Vec3::new(5.0, 20.0, 0.0), Dir3::NEG_Y);
        assert_eq!(nearest(&floats, ray), Some(2));
        // Nothing near x = 10.
        let ray = Ray3d::new(Vec3::new(10.0, 20.0, 0.0), Dir3::NEG_Y);
        assert_eq!(nearest(&floats, ray), None);
    }
}
