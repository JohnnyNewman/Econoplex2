//! Fields through the farming year (shader step 15).
//!
//! The terrain shader draws each 6 × 6 plot of the fields biome as a crop in
//! rows that follows the calendar: ploughed soil in winter, green rows that
//! sprout in spring and close up, grain that ripens to gold in summer,
//! stubble after the harvest, ploughed again in late autumn. The calendar
//! (the year's fraction) reaches the shader through the shared buffer
//! (`fog.debug.y`); what the sim knows goes into the terrain's uniform:
//!
//! - **How much of the harvest is in**, per town (`towns[k].w`): the sim's
//!   standing crop (`TownView::crops`) against this year's peak. The plots
//!   nearest each town are reaped one by one as its crop is taken in.
//!
//! A town whose crop never stands (reaped as it grows), or no sim at all,
//! falls back to the calendar's harvest.

use crate::env::Environment;
use crate::live::LiveSim;
use crate::terrain::{TerrainMaterial, MAX_TINTED};
use crate::SCALE;
use bevy::prelude::*;

pub struct FieldsPlugin;

impl Plugin for FieldsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, field_state);
    }
}

/// Per town: the highest standing crop of this year so far, and the year's
/// fraction last seen (a wrap past midwinter starts a new year).
#[derive(Default)]
struct CropYear {
    peak: Vec<f32>,
    season: f32,
}

/// The harvest by the calendar alone: none before late summer, all in by
/// mid-autumn.
pub fn calendar_reaped(season: f32) -> f32 {
    smoothstep(0.55, 0.7, season)
}

/// Share of a town's fields reaped: how far its standing crop fell below this
/// year's peak, once the peak is past (after midsummer). With no crop worth
/// the name ever standing, the calendar's.
pub fn reaped(crops: f32, peak: f32, season: f32) -> f32 {
    if peak < 0.02 {
        return calendar_reaped(season);
    }
    if season < 0.45 {
        return 0.0;
    }
    (1.0 - crops / peak).clamp(0.0, 1.0)
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Each town's harvest into the terrain uniform, written only when a value
/// moved by more than a 64th (a few times a season): the material stays put
/// the rest of the time.
fn field_state(
    live: Option<Res<LiveSim>>,
    env: Res<Environment>,
    mut year: Local<CropYear>,
    terrains: Query<&MeshMaterial3d<TerrainMaterial>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
) {
    let towns: Vec<(Vec2, f32)> = live
        .as_ref()
        .map(|l| {
            l.snapshot
                .towns
                .iter()
                .take(MAX_TINTED)
                .map(|t| (Vec2::new(t.pos.0 / SCALE, -t.pos.1 / SCALE), t.crops))
                .collect()
        })
        .unwrap_or_default();
    if env.season < year.season - 0.5 {
        year.peak.clear();
    }
    year.season = env.season;
    year.peak.resize(towns.len(), 0.0);
    let want: Vec<f32> = towns
        .iter()
        .zip(year.peak.iter_mut())
        .map(|(&(_, crops), peak)| {
            *peak = peak.max(crops);
            (reaped(crops, *peak, env.season) * 64.0).round() / 64.0
        })
        .collect();
    for handle in &terrains {
        let Some(m) = materials.get(handle) else {
            continue;
        };
        let u = &m.extension.terrain;
        let same = u.fields.x as usize == towns.len()
            && towns
                .iter()
                .zip(&want)
                .enumerate()
                .all(|(k, (&(at, _), &r))| u.towns[k].xy() == at && u.towns[k].w == r);
        if same {
            continue;
        }
        let Some(mut m) = materials.get_mut(handle) else {
            continue;
        };
        let u = &mut m.extension.terrain;
        u.fields.x = towns.len() as f32;
        for (k, (&(at, _), &r)) in towns.iter().zip(&want).enumerate() {
            u.towns[k].x = at.x;
            u.towns[k].y = at.y;
            u.towns[k].w = r;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_reaped_before_the_peak_and_all_once_the_crop_is_in() {
        // Spring: the crop grows, nothing reaped whatever stands.
        assert_eq!(reaped(0.1, 0.3, 0.3), 0.0);
        // Late summer: half the peak taken in.
        assert!((reaped(0.4, 0.8, 0.6) - 0.5).abs() < 1e-6);
        assert_eq!(reaped(0.0, 0.8, 0.7), 1.0);
    }

    #[test]
    fn a_town_without_a_standing_crop_follows_the_calendar() {
        assert_eq!(reaped(0.0, 0.0, 0.3), 0.0);
        assert_eq!(reaped(0.0, 0.0, 0.8), 1.0);
        assert!(reaped(0.0, 0.0, 0.62) > 0.0 && reaped(0.0, 0.0, 0.62) < 1.0);
    }
}
