//! M6: overlays on the map. `V` cycles: off, trade.
//!
//! **Trade:** an arc from each town to each town it ships goods to, as high as
//! the towns are far apart, brighter and wider the more it ships (the daily
//! average over about a month, from the sim's ledger). Small lights travel along
//! the arcs, more of them for more trade: the caravans. The colours are bright
//! HDR values (above 1), so the camera's bloom makes them glow.

use crate::live::LiveSim;
use crate::{height, World3d, SCALE};
use bevy::prelude::*;

pub struct OverlayPlugin;

impl Plugin for OverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Overlay>()
            .add_systems(Startup, configure_lines)
            .add_systems(Update, (overlay_keys, draw_trade, draw_why).chain());
    }
}

/// Which overlay is shown.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overlay {
    #[default]
    Off,
    Trade,
}

fn configure_lines(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<DefaultGizmoConfigGroup>();
    config.line.width = 3.0;
}

fn overlay_keys(keys: Res<ButtonInput<KeyCode>>, mut overlay: ResMut<Overlay>) {
    if keys.just_pressed(KeyCode::KeyV) {
        *overlay = match *overlay {
            Overlay::Off => Overlay::Trade,
            Overlay::Trade => Overlay::Off,
        };
    }
}

/// A point on the arc from `a` to `b` at `s` (0..1): a parabola lifted by a
/// quarter of the distance, plus the ground height along the way.
pub fn arc_point(a: Vec3, b: Vec3, s: f32) -> Vec3 {
    let lift = a.distance(b) * 0.25;
    a.lerp(b, s) + Vec3::Y * (4.0 * s * (1.0 - s) * lift + 1.5)
}

fn draw_trade(
    mut gizmos: Gizmos,
    overlay: Res<Overlay>,
    live: Res<LiveSim>,
    world: Res<World3d>,
    time: Res<Time>,
) {
    if *overlay != Overlay::Trade {
        return;
    }
    let towns = &live.snapshot.towns;
    let t = &world.terrain;
    let at = |p: (f32, f32)| Vec3::new(p.0 / SCALE, height(t, p), -p.1 / SCALE);
    let max = towns
        .iter()
        .flat_map(|t| t.exports.iter().copied())
        .fold(1.0f32, f32::max);
    for (i, from) in towns.iter().enumerate() {
        for (j, &value) in from.exports.iter().enumerate() {
            if j == i || value < 0.5 || j >= towns.len() {
                continue;
            }
            let (a, b) = (at(from.pos), at(towns[j].pos));
            // Two-way routes sit side by side.
            let side = (b - a).cross(Vec3::Y).normalize_or_zero() * 0.8;
            let (a, b) = (a + side, b + side);
            let w = (value / max).sqrt(); // 0..1
                                          // Bright, warm, HDR (> 1): the bloom turns it into a glow.
            let glow = 1.0 + 5.0 * w;
            let color = Color::linear_rgb(1.0 * glow, 0.62 * glow, 0.22 * glow);
            let points: Vec<Vec3> = (0..=24).map(|k| arc_point(a, b, k as f32 / 24.0)).collect();
            gizmos.linestrip(points, color);
            // Caravans: lights moving from seller to buyer.
            let n = 1 + (w * 5.0).round() as usize;
            for k in 0..n {
                let s = (time.elapsed_secs() * 0.08 + k as f32 / n as f32).fract();
                let p = arc_point(a, b, s);
                gizmos.sphere(
                    Isometry3d::from_translation(p),
                    0.35,
                    color.mix(&Color::WHITE, 0.3),
                );
            }
        }
    }
}

/// The "why" view (M7) rings the selected town's buildings of the types its
/// findings name: a pulsing circle on the ground around each.
fn draw_why(
    mut gizmos: Gizmos,
    live: Res<LiveSim>,
    world: Res<World3d>,
    time: Res<Time>,
    highlight: Option<Res<crate::ui::WhyHighlight>>,
) {
    let Some(h) = highlight else { return };
    let Some(town) = live.snapshot.towns.get(h.town) else {
        return;
    };
    let t = &world.terrain;
    let pulse = 1.0 + 0.15 * (time.elapsed_secs() * 4.0).sin();
    for b in town.buildings.iter().filter(|b| h.kinds.contains(&b.kind)) {
        let at = Vec3::new(b.pos.0 / SCALE, height(t, b.pos) + 0.2, -b.pos.1 / SCALE);
        let iso = Isometry3d::new(at, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2));
        gizmos.circle(iso, 2.4 * pulse, Color::linear_rgb(3.0, 0.6, 0.3));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arcs_start_and_end_above_the_towns_and_rise_in_between() {
        let (a, b) = (Vec3::ZERO, Vec3::new(40.0, 0.0, 0.0));
        assert!((arc_point(a, b, 0.0) - Vec3::Y * 1.5).length() < 1e-4);
        assert!((arc_point(a, b, 1.0) - (b + Vec3::Y * 1.5)).length() < 1e-4);
        assert!((arc_point(a, b, 0.5).y - (10.0 + 1.5)).abs() < 1e-4);
    }
}
