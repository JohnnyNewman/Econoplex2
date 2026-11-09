//! Crowding: what a town's size costs the people living in it.
//!
//! Interface: a town's housing (from its land and its buildings) and its number of
//! residents. Output: the daily rent each adult resident pays the town, which rises
//! with crowding (residents per place to live). Rent is money paid to the treasury
//! (a tax by right in the ledger), so it changes nothing about the money total.
//!
//! Decisions see it through the normal modules: job choice values wealth after the
//! rent of the coming weeks, and utility migration counts a town's rent against its
//! pay, so a big town is a real trade-off and not just a magnet.

use serde::{Deserialize, Serialize};

/// `decision.crowding` in models.ron.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum CrowdingParams {
    /// No rent (the original rule).
    #[default]
    None,
    /// Housing = `base` + `per_plot` × land plots (worked or free) + `per_building`
    /// × buildings. Rent per adult per day = `rate` × (residents / housing) ^
    /// `exponent`, followed over `sticky_days`.
    Rent {
        base: f32,
        per_plot: f32,
        per_building: f32,
        rate: f32,
        exponent: f32,
        /// Days over which the rent follows crowding (rents are sticky).
        #[serde(default = "sticky_default")]
        sticky_days: f32,
    },
}

fn sticky_default() -> f32 {
    60.0
}

impl CrowdingParams {
    /// Places to live in a town with `plots` plots of land and `buildings` buildings.
    pub fn housing(&self, plots: usize, buildings: usize) -> f32 {
        match *self {
            CrowdingParams::None => f32::INFINITY,
            CrowdingParams::Rent {
                base,
                per_plot,
                per_building,
                ..
            } => (base + per_plot * plots as f32 + per_building * buildings as f32).max(1.0),
        }
    }

    /// Days over which the rent follows crowding (time constant).
    pub fn stickiness_days(&self) -> f32 {
        match *self {
            CrowdingParams::None => 1.0,
            CrowdingParams::Rent { sticky_days, .. } => sticky_days.max(1.0),
        }
    }

    /// Daily rent per adult when `residents` live in `housing` places.
    pub fn rent(&self, residents: usize, housing: f32) -> f32 {
        match *self {
            CrowdingParams::None => 0.0,
            CrowdingParams::Rent { rate, exponent, .. } => {
                rate.max(0.0) * (residents as f32 / housing.max(1.0)).powf(exponent)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rent() -> CrowdingParams {
        CrowdingParams::Rent {
            base: 20.0,
            per_plot: 10.0,
            per_building: 5.0,
            rate: 0.1,
            exponent: 2.0,
            sticky_days: 60.0,
        }
    }

    #[test]
    fn rent_rises_with_crowding_and_land_and_buildings_relieve_it() {
        let c = rent();
        let h = c.housing(10, 4);
        assert_eq!(h, 140.0);
        assert!(c.rent(280, h) > c.rent(140, h));
        assert!((c.rent(140, h) - 0.1).abs() < 1e-6);
        assert!(c.rent(140, c.housing(20, 4)) < c.rent(140, h));
        assert_eq!(CrowdingParams::None.rent(1000, 1.0), 0.0);
    }
}
