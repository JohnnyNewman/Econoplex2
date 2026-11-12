//! M6: overlays on the map. `V` cycles: off, trade, culture, product space.
//!
//! **Trade:** an arc from each town to each town it ships goods to, as high as
//! the towns are far apart, brighter and wider the more it ships (the daily
//! average over about a month, from the sim's ledger). Small lights travel along
//! the arcs, more of them for more trade: the caravans. The colours are bright
//! HDR values (above 1), so the camera's bloom makes them glow.

//!
//! **Culture:** the land around each town takes the colour of its culture, and
//! every person their own: tradition red, faith green, liberty blue (each axis
//! 0..1; neutral 0.5 is grey). Newcomers and minorities stand out against their
//! town's colour. The tint is drawn by the terrain shader (`terrain.wesl`).
//!
//! **Product space:** a panel (ui.rs) with the selected town's skills, as in the
//! 2D game.

use crate::live::LiveSim;
use crate::terrain::{TerrainMaterial, MAX_TINTED};
use crate::{height, World3d, SCALE};
use bevy::prelude::*;

pub struct OverlayPlugin;

impl Plugin for OverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Overlay>()
            .add_systems(Startup, configure_lines)
            .add_systems(
                Update,
                (overlay_keys, draw_trade, draw_why, tint_terrain).chain(),
            );
    }
}

/// Which overlay is shown.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overlay {
    #[default]
    Off,
    Trade,
    Culture,
    ProductSpace,
}

impl Overlay {
    pub fn next(self) -> Overlay {
        match self {
            Overlay::Off => Overlay::Trade,
            Overlay::Trade => Overlay::Culture,
            Overlay::Culture => Overlay::ProductSpace,
            Overlay::ProductSpace => Overlay::Off,
        }
    }
}

/// A culture as a colour (linear rgb): tradition red, faith green, liberty
/// blue. Each axis is stretched around its neutral 0.5 (1.6 times), so the
/// small differences real towns have show.
pub fn culture_colour(c: &[f32]) -> [f32; 3] {
    std::array::from_fn(|k| {
        let v = c.get(k).copied().unwrap_or(0.5);
        let srgb = (0.5 + (v - 0.5) * 1.6).clamp(0.05, 0.95);
        Color::srgb(srgb, srgb, srgb).to_linear().red
    })
}

fn configure_lines(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<DefaultGizmoConfigGroup>();
    config.line.width = 3.0;
}

fn overlay_keys(keys: Res<ButtonInput<KeyCode>>, mut overlay: ResMut<Overlay>) {
    if keys.just_pressed(KeyCode::KeyV) {
        *overlay = overlay.next();
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

/// The culture overlay's terrain tint: each town's centre, radius and culture
/// colour into the terrain's uniform, when the overlay or the day changes.
fn tint_terrain(
    overlay: Res<Overlay>,
    live: Res<LiveSim>,
    mut last: Local<Option<(Overlay, u64)>>,
    terrains: Query<&MeshMaterial3d<TerrainMaterial>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
) {
    let now = (*overlay, live.snapshot.tick);
    if *last == Some(now) {
        return;
    }
    *last = Some(now);
    let towns = &live.snapshot.towns;
    let on = *overlay == Overlay::Culture;
    for handle in &terrains {
        let Some(mut m) = materials.get_mut(handle) else {
            continue;
        };
        let u = &mut m.extension.terrain;
        u.dims.z = if on {
            towns.len().min(MAX_TINTED) as f32
        } else {
            0.0
        };
        for (k, t) in towns.iter().take(MAX_TINTED).enumerate() {
            u.towns[k] = Vec4::new(t.pos.0 / SCALE, -t.pos.1 / SCALE, 32.0, 0.0);
            let c = culture_colour(&t.culture);
            u.cultures[k] = Vec4::new(c[0], c[1], c[2], 1.0);
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
    fn the_overlay_key_cycles_through_all_four() {
        let mut o = Overlay::Off;
        let mut seen = Vec::new();
        for _ in 0..4 {
            o = o.next();
            seen.push(o);
        }
        assert_eq!(seen.last(), Some(&Overlay::Off));
        assert!(seen.contains(&Overlay::Culture) && seen.contains(&Overlay::ProductSpace));
    }

    #[test]
    fn neutral_culture_is_grey_and_each_axis_has_its_channel() {
        let grey = culture_colour(&[0.5, 0.5, 0.5]);
        assert!(grey.iter().all(|&v| (v - grey[0]).abs() < 1e-6));
        let traditional = culture_colour(&[0.8, 0.5, 0.5]);
        assert!(traditional[0] > traditional[1] && traditional[1] == traditional[2]);
        let liberal = culture_colour(&[0.5, 0.5, 0.8]);
        assert!(liberal[2] > liberal[0]);
    }

    #[test]
    fn arcs_start_and_end_above_the_towns_and_rise_in_between() {
        let (a, b) = (Vec3::ZERO, Vec3::new(40.0, 0.0, 0.0));
        assert!((arc_point(a, b, 0.0) - Vec3::Y * 1.5).length() < 1e-4);
        assert!((arc_point(a, b, 1.0) - (b + Vec3::Y * 1.5)).length() < 1e-4);
        assert!((arc_point(a, b, 0.5).y - (10.0 + 1.5)).abs() < 1e-4);
    }
}
