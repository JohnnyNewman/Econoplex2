//! A strategy-game camera for the world view: it looks at a point on the
//! ground from a yaw, a pitch and a distance.
//!
//! - `W` `A` `S` `D` pan (along the ground, relative to where the camera faces);
//! - `Q` / `E` rotate around the focus;
//! - right mouse drag: rotate and tilt;
//! - middle mouse drag: pan;
//! - mouse wheel: zoom (toward the focus);
//! - `Home`: back to the start view.
//!
//! Panning speed grows with the distance, so it feels the same zoomed in and out.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;

pub struct WorldCameraPlugin;

impl Plugin for WorldCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, world_camera);
    }
}

/// Set by the UI each frame: whether the panels have the pointer or the
/// keyboard (then the camera ignores them).
#[derive(Resource, Default)]
pub struct UiCapture {
    pub pointer: bool,
    pub keyboard: bool,
}

/// Where the camera looks from. Attach it to the `Camera3d` entity.
#[derive(Component, Clone, Copy, Debug)]
pub struct WorldCamera {
    /// The point on the ground the camera looks at.
    pub focus: Vec3,
    /// Rotation around the vertical axis (radians).
    pub yaw: f32,
    /// Angle above the ground (radians).
    pub pitch: f32,
    /// Distance from the focus.
    pub distance: f32,
    /// Limits: the map's extent on the ground (min x, min z, max x, max z).
    pub bounds: Vec4,
    /// The start view, for `Home`.
    pub home: Option<(Vec3, f32, f32, f32)>,
}

impl WorldCamera {
    pub fn new(focus: Vec3, yaw: f32, pitch: f32, distance: f32, bounds: Vec4) -> Self {
        Self {
            focus,
            yaw,
            pitch,
            distance,
            bounds,
            home: Some((focus, yaw, pitch, distance)),
        }
    }

    /// The camera transform for this view.
    pub fn transform(&self) -> Transform {
        let offset = Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.cos() * self.pitch.cos(),
        ) * self.distance;
        Transform::from_translation(self.focus + offset).looking_at(self.focus, Vec3::Y)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn world_camera(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    mut cams: Query<(&mut WorldCamera, &mut Transform)>,
    ui: Option<Res<UiCapture>>,
) {
    // While the pointer or the keyboard is on a panel, the camera stays put.
    let (pointer_free, keys_free) = ui.map_or((true, true), |u| (!u.pointer, !u.keyboard));
    let dt = time.delta_secs();
    for (mut c, mut tf) in &mut cams {
        if keys_free && keys.just_pressed(KeyCode::Home) {
            if let Some((f, y, p, d)) = c.home {
                (c.focus, c.yaw, c.pitch, c.distance) = (f, y, p, d);
            }
        }
        // Directions on the ground: "forward" is where the camera faces.
        let forward = -Vec3::new(c.yaw.sin(), 0.0, c.yaw.cos());
        let right = Vec3::new(-forward.z, 0.0, forward.x);
        let key = |k: KeyCode| (keys_free && keys.pressed(k)) as i32 as f32;
        let speed = c.distance * 0.9;
        let mut pan = (forward * (key(KeyCode::KeyW) - key(KeyCode::KeyS))
            + right * (key(KeyCode::KeyD) - key(KeyCode::KeyA)))
            * speed
            * dt;
        if pointer_free && mouse.pressed(MouseButton::Middle) {
            let px = c.distance * 0.0015;
            pan += (-right * motion.delta.x + forward * motion.delta.y) * px;
        }
        c.focus += pan;
        c.focus.x = c.focus.x.clamp(c.bounds.x, c.bounds.z);
        c.focus.z = c.focus.z.clamp(c.bounds.y, c.bounds.w);

        c.yaw += (key(KeyCode::KeyQ) - key(KeyCode::KeyE)) * 1.5 * dt;
        if pointer_free && mouse.pressed(MouseButton::Right) {
            c.yaw -= motion.delta.x * 0.005;
            c.pitch += motion.delta.y * 0.005;
        }
        c.pitch = c.pitch.clamp(0.12, 1.45);

        if pointer_free && scroll.delta.y != 0.0 {
            c.distance *= (1.0 - scroll.delta.y * 0.1).clamp(0.5, 1.5);
        }
        c.distance = c.distance.clamp(8.0, 260.0);

        *tf = c.transform();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> (App, Entity) {
        let mut app = App::new();
        // The input resources without Bevy's input systems, so the test
        // decides what is pressed in each frame.
        app.add_plugins((MinimalPlugins, WorldCameraPlugin))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<AccumulatedMouseScroll>();
        let cam = WorldCamera::new(
            Vec3::ZERO,
            0.0,
            0.5,
            50.0,
            Vec4::new(-100.0, -100.0, 100.0, 100.0),
        );
        let e = app.world_mut().spawn((cam, cam.transform())).id();
        app.update();
        (app, e)
    }

    fn press(app: &mut App, key: KeyCode, frames: usize) {
        for _ in 0..frames {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(key);
            app.update();
            // What Bevy's input system does at the start of each frame.
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .clear();
        }
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(key);
        app.update();
    }

    #[test]
    fn the_camera_looks_at_its_focus() {
        let cam = WorldCamera::new(Vec3::new(3.0, 0.0, -2.0), 0.7, 0.6, 40.0, Vec4::splat(0.0));
        let tf = cam.transform();
        let to_focus = (cam.focus - tf.translation).normalize();
        assert!(tf.forward().dot(to_focus) > 0.9999);
        assert!((tf.translation.distance(cam.focus) - 40.0).abs() < 1e-3);
    }

    #[test]
    fn keys_pan_rotate_and_reset() {
        let (mut app, e) = app();
        // W moves the focus away from the camera (yaw 0: the camera sits at +z).
        press(&mut app, KeyCode::KeyW, 10);
        let c = *app.world().get::<WorldCamera>(e).unwrap();
        assert!(
            c.focus.z < 0.0 && c.focus.x.abs() < 1e-3,
            "W pans forward: {:?}",
            c.focus
        );
        // Q rotates.
        press(&mut app, KeyCode::KeyQ, 10);
        let c = *app.world().get::<WorldCamera>(e).unwrap();
        assert!(c.yaw > 0.0);
        // Home goes back to the start view.
        press(&mut app, KeyCode::Home, 1);
        let c = *app.world().get::<WorldCamera>(e).unwrap();
        assert_eq!((c.focus, c.yaw), (Vec3::ZERO, 0.0));
        // The transform follows.
        let tf = *app.world().get::<Transform>(e).unwrap();
        assert!(tf.translation.distance(c.transform().translation) < 1e-4);
    }

    #[test]
    fn the_focus_stays_on_the_map() {
        let (mut app, e) = app();
        press(&mut app, KeyCode::KeyA, 2000);
        let c = *app.world().get::<WorldCamera>(e).unwrap();
        assert!(c.focus.x >= -100.0 && c.focus.x <= 100.0);
    }
}
