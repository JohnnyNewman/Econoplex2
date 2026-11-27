//! `--render-scale F` (0.25..1): draw the 3D view at a fraction of the window's
//! resolution and stretch it over the window.
//!
//! The 3D camera renders into an image `F` times the window's size in each
//! direction, so at 0.75 the GPU shades a bit more than half the pixels. A 2D
//! camera then draws that image over the whole window and carries the panels,
//! which stay sharp; the help text is drawn with the 3D view, under them. This helps most on a hot laptop GPU, where
//! the frame is bound by pixel work (see "Performance" in graphics-ui.md).
//!
//! At 1 none of this happens: the 3D camera draws to the window.
//!
//! **Dynamic resolution** (`AutoScale`, on by default; off with
//! `--render-scale F`, `--no-auto-scale`, `--shot` and `--fps`): the scale
//! follows the frame time. When frames take clearly longer than the target
//! (60 per second) the scale steps down by 0.05, at most once a second, to
//! 0.6 at the least. When they keep up, it tries a step up after a few
//! seconds; if that was too much, it comes back down and waits twice as long
//! before trying again (up to half a minute), so it doesn't flicker between
//! two scales. A hot laptop that throttles keeps its frame rate and loses a
//! little sharpness instead. It only watches while the window has the
//! focus: a window in the background may be slowed down by the system.

use crate::camera::WorldCamera;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::window::{PrimaryWindow, WindowResized};
use bevy_egui::{EguiGlobalSettings, PrimaryEguiContext};

pub struct RenderScalePlugin;

impl Plugin for RenderScalePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AutoScale>()
            .add_systems(PostStartup, set_up)
            .add_systems(Update, (adjust_scale, follow_window).chain());
    }
}

/// Dynamic resolution: the scale follows the frame time.
#[derive(Resource, Clone, Copy, Debug)]
pub struct AutoScale {
    pub on: bool,
    /// Frame time to hold (seconds).
    pub target: f32,
    /// Smoothed frame time.
    pub smoothed: f32,
    /// When the scale last changed, and how long to keep up before trying a
    /// step up (seconds).
    pub changed_at: f32,
    pub wait: f32,
    /// The last change was a step up (a failed one doubles the wait).
    pub probing: bool,
}

impl Default for AutoScale {
    fn default() -> Self {
        Self::new(false)
    }
}

/// The smallest scale dynamic resolution goes to, and its step.
pub const MIN_AUTO: f32 = 0.6;
pub const STEP: f32 = 0.05;

impl AutoScale {
    pub fn new(on: bool) -> Self {
        AutoScale {
            on,
            target: 1.0 / 60.0,
            smoothed: 1.0 / 60.0,
            changed_at: 0.0,
            wait: 4.0,
            probing: false,
        }
    }

    /// The next scale at time `now` (seconds), or `None` to keep `scale`.
    pub fn next(&mut self, scale: f32, now: f32) -> Option<f32> {
        let since = now - self.changed_at;
        if since < 1.0 {
            return None;
        }
        let slow = self.smoothed > self.target * 1.08;
        if slow && scale > MIN_AUTO + 1e-3 {
            // A step up that didn't hold: wait longer before the next try.
            if self.probing && since < 3.0 {
                self.wait = (self.wait * 2.0).min(30.0);
            }
            self.probing = false;
            self.changed_at = now;
            return Some((scale - STEP).max(MIN_AUTO));
        }
        if !slow && self.smoothed < self.target * 1.02 && scale < 1.0 - 1e-3 && since > self.wait {
            self.probing = true;
            self.changed_at = now;
            return Some((scale + STEP).min(1.0));
        }
        // Long calm: try again sooner next time.
        if since > 60.0 {
            self.wait = 4.0;
        }
        None
    }
}

/// The fraction of the window's resolution the 3D view is drawn at.
#[derive(Resource, Clone, Copy)]
pub struct RenderScale(pub f32);

impl RenderScale {
    pub fn on(&self) -> bool {
        self.0 < 0.999
    }
}

/// The image the 3D view is drawn into, and the sprite that shows it.
#[derive(Resource)]
struct Scaled {
    image: Handle<Image>,
    sprite: Entity,
}

/// The image size for a window of `physical` pixels at `scale`.
pub fn target_size(physical: UVec2, scale: f32) -> UVec2 {
    (physical.as_vec2() * scale.clamp(0.25, 1.0))
        .round()
        .as_uvec2()
        .max(UVec2::ONE)
}

fn target_image(size: UVec2) -> Image {
    Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None)
}

#[allow(clippy::too_many_arguments)]
fn set_up(
    mut commands: Commands,
    scale: Option<Res<RenderScale>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cams: Query<Entity, With<WorldCamera>>,
    mut images: ResMut<Assets<Image>>,
    mut egui: ResMut<EguiGlobalSettings>,
    auto: Option<Res<AutoScale>>,
) {
    let auto = auto.is_some_and(|a| a.on);
    let Some(scale) = scale.filter(|s| s.on() || auto) else {
        return;
    };
    let Ok(window) = windows.single() else { return };
    let image = images.add(target_image(target_size(window.physical_size(), scale.0)));
    for cam in &cams {
        // The help text stays with the 3D view (drawn into the image, under
        // the panels, as before).
        commands
            .entity(cam)
            .insert((RenderTarget::Image(image.clone().into()), IsDefaultUiCamera));
    }
    // The 2D camera that shows the image carries the panels.
    egui.auto_create_primary_context = false;
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            ..default()
        },
        PrimaryEguiContext,
    ));
    let sprite = commands
        .spawn(Sprite {
            image: image.clone(),
            custom_size: Some(window.size()),
            ..default()
        })
        .id();
    commands.insert_resource(Scaled { image, sprite });
}

/// Dynamic resolution: smooth the frame time and step the scale.
fn adjust_scale(
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut auto: ResMut<AutoScale>,
    mut scale: ResMut<RenderScale>,
) {
    if !auto.on {
        return;
    }
    // A window in the background or covered may be slowed down by the
    // system: that says nothing about the GPU.
    if !windows.single().is_ok_and(|w| w.focused) {
        return;
    }
    let dt = time.delta_secs();
    // Skip hitches (loading, a window drag) and the first frames.
    if dt <= 0.0 || dt > 0.25 || time.elapsed_secs() < 3.0 {
        return;
    }
    auto.smoothed += (dt - auto.smoothed) * 0.05;
    if let Some(s) = auto.next(scale.0, time.elapsed_secs()) {
        info!(
            "render scale {s:.2} (frame {:.1} ms)",
            auto.smoothed * 1000.0
        );
        scale.0 = s;
    }
}

/// A resized window, or a new scale, gets a new image of the right size.
fn follow_window(
    mut resized: MessageReader<WindowResized>,
    scale: Option<Res<RenderScale>>,
    scaled: Option<Res<Scaled>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut sprites: Query<&mut Sprite>,
) {
    let resized = resized.read().last().is_some();
    let (Some(scale), Some(scaled)) = (scale, scaled) else {
        return;
    };
    if !resized && !scale.is_changed() {
        return;
    }
    let Ok(window) = windows.single() else { return };
    let size = target_size(window.physical_size(), scale.0);
    if let Some(mut image) = images.get_mut(&scaled.image) {
        image.resize(bevy::render::render_resource::Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        });
    }
    if let Ok(mut sprite) = sprites.get_mut(scaled.sprite) {
        sprite.custom_size = Some(window.size());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_image_is_the_window_times_the_scale_within_bounds() {
        assert_eq!(
            target_size(UVec2::new(2400, 1350), 0.75),
            UVec2::new(1800, 1013)
        );
        assert_eq!(
            target_size(UVec2::new(2400, 1350), 1.0),
            UVec2::new(2400, 1350)
        );
        // Clamped to a quarter, and never zero.
        assert_eq!(
            target_size(UVec2::new(2400, 1350), 0.01),
            UVec2::new(600, 338)
        );
        assert_eq!(target_size(UVec2::new(0, 0), 0.5), UVec2::ONE);
        assert!(!RenderScale(1.0).on() && RenderScale(0.75).on());
    }

    #[test]
    fn slow_frames_step_the_scale_down_and_a_failed_try_waits_longer() {
        let mut a = AutoScale::new(true);
        a.smoothed = 0.025; // 40 fps against 60
        assert_eq!(a.next(1.0, 10.0), Some(0.95));
        // Not again within a second.
        assert_eq!(a.next(0.95, 10.5), None);
        assert!((a.next(0.95, 11.2).unwrap() - 0.9).abs() < 1e-6);
        // Down to the floor and no further.
        assert_eq!(a.next(MIN_AUTO, 20.0), None);
        // Keeping up: a step up after the wait.
        a.smoothed = 1.0 / 60.0;
        assert_eq!(a.next(0.9, 13.0), None);
        assert!((a.next(0.9, 16.0).unwrap() - 0.95).abs() < 1e-6);
        // Too much: back down, and the next try waits twice as long.
        a.smoothed = 0.02;
        assert!((a.next(0.95, 17.5).unwrap() - 0.9).abs() < 1e-6);
        assert_eq!(a.wait, 8.0);
    }
}
