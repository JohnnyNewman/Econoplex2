//! Picking and inspecting individual people.
//!
//! Left-click a person to select them; click empty ground (or `Esc`) to let
//! go. Where people are lives on the GPU (the walking is a compute pass,
//! crowd_compute.rs), so a click asks for one copy of the people buffer back
//! (`ReadbackOnce`) and picks the figure nearest the click's ray when it
//! arrives, a frame or two later. The selected person is lit gold (people.wesl)
//! and the simulation describes them after every tick (`sim_core::view::person`),
//! shown in the "Person" panel (ui.rs).

use crate::camera::UiCapture;
use crate::crowd::{CrowdHandles, Slots, PERSON_FLOATS};
use crate::live::LiveSim;
use bevy::prelude::*;
use bevy::render::gpu_readback::{ReadbackComplete, ReadbackOnce};

pub struct InspectPlugin;

impl Plugin for InspectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pick>()
            .add_systems(Update, (click, resolve).chain());
    }
}

/// The person selected (agent store index), if any.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct Selected(pub Option<u32>);

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
