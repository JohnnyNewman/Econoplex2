//! Step 12: lanterns and fires that light their surroundings at night
//! (docs/shaders/12-town-lights.md).
//!
//! Windows, lanterns and fire mouths glow by themselves (a vertex-colour marker
//! read by the toon shader, lights.wesl). The lamps and fires nearest the
//! camera also throw light: each frame up to `MAX_LAMPS` of them go into the
//! tail of the shared fog buffer (`fog::FOG_BUFFER`), and the toon and terrain
//! shaders add a soft pool of light around each. Only the buffer's contents
//! change, so no material does; and they are our own lights, not Bevy's, so
//! they add nothing to Bevy's light clustering.

use crate::fog::{FogState, FOG_BUFFER, FOG_VEC4S};
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;

/// How many lamps the shaders loop over (`array<Lamp, 24>` in lights.wesl).
pub const MAX_LAMPS: usize = 24;
/// The vec4s the lamps take in the shared buffer: one with the count, then
/// two per lamp.
pub const LAMP_VEC4S: usize = 1 + 2 * MAX_LAMPS;

pub struct LightsPlugin;

impl Plugin for LightsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, update_lamps.after(TransformSystems::Propagate));
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LampKind {
    Lantern,
    Fire,
}

impl LampKind {
    /// Colour times brightness at full night, and the reach (3D units).
    fn light(self) -> (Vec3, f32) {
        match self {
            LampKind::Lantern => (Vec3::new(1.0, 0.72, 0.38) * 1.6, 2.6),
            LampKind::Fire => (Vec3::new(1.0, 0.45, 0.15) * 2.2, 3.2),
        }
    }
}

/// The lamps on a building, in its own frame (put there when it is baked).
#[derive(Component, Clone, Debug, Default)]
pub struct Lamps(pub Vec<(Vec3, LampKind)>);

/// 0 by day, 1 once the sun is well below the horizon (as `night` in
/// lights.wesl).
fn night(sun_y: f32) -> f32 {
    let t = ((sun_y - -0.10) / (0.06 - -0.10)).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// Pick the lamps nearest the camera and write them into the shared buffer.
fn update_lamps(
    fog: Res<FogState>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    lamps: Query<(&GlobalTransform, &Lamps)>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut last: Local<Vec<[f32; 4]>>,
) {
    let dark = night(fog.0.sun_dir.y);
    let mut out = vec![[0.0f32; 4]; LAMP_VEC4S];
    if dark > 0.0 {
        if let Some(eye) = camera.iter().next().map(|c| c.translation()) {
            let mut near: Vec<(f32, Vec3, LampKind)> = lamps
                .iter()
                .flat_map(|(t, l)| {
                    l.0.iter()
                        .map(move |&(at, kind)| (t.transform_point(at), kind))
                })
                .map(|(p, kind)| (p.distance_squared(eye), p, kind))
                .collect();
            let n = near.len().min(MAX_LAMPS);
            if n > 0 {
                near.select_nth_unstable_by(n - 1, |a, b| a.0.total_cmp(&b.0));
            }
            out[0][0] = n as f32;
            for (i, &(_, p, kind)) in near.iter().take(n).enumerate() {
                let (colour, reach) = kind.light();
                let c = colour * dark;
                // A stable random number per lamp, for its flicker.
                let seed = (p.x * 12.9898 + p.z * 78.233).sin().abs().fract();
                out[1 + 2 * i] = [p.x, p.y, p.z, reach];
                out[2 + 2 * i] = [c.x, c.y, c.z, seed];
            }
        }
    }
    // By day, and while nothing moves, the buffer stays as it is.
    if *last == out {
        return;
    }
    if let Some(mut buffer) = buffers.get_mut(&FOG_BUFFER) {
        if let Some(data) = buffer.cast_slice_mut::<[f32; 4]>() {
            data[FOG_VEC4S..].copy_from_slice(&out);
        }
    }
    *last = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn night_is_zero_by_day_and_one_at_night() {
        assert_eq!(night(0.5), 0.0);
        assert_eq!(night(-0.5), 1.0);
        let dusk = night(-0.02);
        assert!(dusk > 0.0 && dusk < 1.0);
    }
}
