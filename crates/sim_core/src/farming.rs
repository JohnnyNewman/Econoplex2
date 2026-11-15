//! Seasons in farming (economy redesign, step 6).
//!
//! `farming` in models.ron. Nature sites that are not crops regrow the same
//! amount every day. Crops (the nature kinds listed in `crops`: fields, flax,
//! pasture) follow the weather:
//!
//! - **Growth** follows the day's mean temperature: nothing below about 4 °C,
//!   full growth from about 14 °C, less again in great heat. Growth is scaled so
//!   that a year of the town's climate grows as much as a year of the site's
//!   daily `regrowth`, so the yearly harvest is set by the content while its
//!   timing follows the seasons.
//! - **Standing crop**: a site can hold `crop_capacity` times its capacity,
//!   so a season's growth stands in the field until it is reaped.
//! - **Losses**: frost (below 0 °C) damages a growing crop; standing crop rots
//!   on wet ground and in the cold. A crop not reaped by late autumn is lost,
//!   so the harvest needs hands at the right time.
//!
//! Food in winter then has to come from store: the hungry gap before the next
//! harvest is the town's (and its ruler's) to bridge.

use crate::components::NaturalResource;
use crate::db::Db;
use crate::world::{Params, Towns};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FarmingParams {
    /// Nature kinds (by id) that grow as crops.
    pub crops: Vec<String>,
    /// Standing crop a site can hold, as a multiple of its capacity.
    pub crop_capacity: f32,
    /// Share of a growing crop lost per day of frost.
    pub frost_loss: f32,
    /// Share of a standing crop that rots per day on soaked ground.
    pub rot_wet: f32,
    /// Share that rots per day in the cold (below 3 °C).
    pub rot_cold: f32,
}

impl Default for FarmingParams {
    fn default() -> Self {
        FarmingParams {
            crops: ["field", "flax_field", "pasture"]
                .map(String::from)
                .to_vec(),
            crop_capacity: 15.0,
            frost_loss: 0.05,
            rot_wet: 0.02,
            rot_cold: 0.01,
        }
    }
}

/// How fast crops grow at a day's mean temperature, 0..1.
pub fn growth(mean_c: f32) -> f32 {
    crate::env::smooth(4.0, 14.0, mean_c) * (1.0 - crate::env::smooth(28.0, 36.0, mean_c))
}

/// The factor that makes a year of growth in this climate equal a year of
/// daily regrowth (at most 6).
pub fn year_scale(params: &Params) -> f32 {
    let n = 360;
    let mean: f32 = (0..n)
        .map(|d| growth(crate::env::climate_c(&params.climate, d as f32 / n as f32)))
        .sum::<f32>()
        / n as f32;
    (1.0 / mean.max(1e-3)).min(6.0)
}

/// Regrows every town's nature sites for a day.
pub fn regrow(
    towns: &Towns,
    db: &Db,
    params: &Params,
    env: &crate::env::Environment,
    sites: &mut Query<&mut NaturalResource>,
) {
    let FarmingParams {
        crops,
        crop_capacity,
        frost_loss,
        rot_wet,
        rot_cold,
    } = &params.farming;
    let is_crop: Vec<bool> = db
        .content
        .nature
        .iter()
        .map(|k| crops.contains(&k.id))
        .collect();
    let scale = year_scale(params);
    for (t, town) in towns.0.iter().enumerate() {
        let local = env.at(t);
        let mean = local.day_mean_c;
        for &e in &town.nature_sites {
            let Ok(mut n) = sites.get_mut(e) else {
                continue;
            };
            if !is_crop.get(n.kind as usize).copied().unwrap_or(false) {
                n.amount = (n.amount + n.regrowth).min(n.capacity);
                continue;
            }
            let g = growth(mean);
            let cap = n.capacity * crop_capacity;
            let mut amount = n.amount + n.regrowth * g * scale;
            if mean < 0.0 {
                // A frost on a crop still in the field.
                amount *= 1.0 - frost_loss;
            }
            let rot = rot_wet * local.ground.wet + rot_cold * (mean < 3.0) as u8 as f32;
            amount *= 1.0 - rot.clamp(0.0, 1.0);
            n.amount = amount.clamp(0.0, cap);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_grow_in_the_warm_and_not_in_the_frost() {
        assert_eq!(growth(-3.0), 0.0);
        assert_eq!(growth(4.0), 0.0);
        assert!(growth(9.0) > 0.3 && growth(9.0) < 0.7);
        assert!((growth(18.0) - 1.0).abs() < 1e-6);
        assert!(growth(36.0) < 1e-6);
    }
}
