//! `--render-scale F` (0.25..1): draw the 3D view at a fraction of the window's
//! resolution and stretch it over the window.
//!
//! The 3D camera renders into an image `F` times the window's size in each
//! direction, so at 0.75 the GPU shades a bit more than half the pixels. A 2D
//! camera then draws that image over the whole window and carries the panels,
//! which stay sharp; the help text is drawn with the 3D view, under them. This helps most on a hot laptop GPU, where
//! the frame is bound by pixel work (see "Performance" in graphics-ui.md).
//!
//! At 1 (the default) none of this happens: the 3D camera draws to the window.

use crate::camera::WorldCamera;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::window::{PrimaryWindow, WindowResized};
use bevy_egui::{EguiGlobalSettings, PrimaryEguiContext};

pub struct RenderScalePlugin;

impl Plugin for RenderScalePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, set_up)
            .add_systems(Update, follow_window);
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
) {
    let Some(scale) = scale.filter(|s| s.on()) else {
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

/// A resized window gets a new image of the right size.
fn follow_window(
    mut resized: MessageReader<WindowResized>,
    scale: Option<Res<RenderScale>>,
    scaled: Option<Res<Scaled>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut sprites: Query<&mut Sprite>,
) {
    if resized.read().last().is_none() {
        return;
    }
    let (Some(scale), Some(scaled)) = (scale, scaled) else {
        return;
    };
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
}
