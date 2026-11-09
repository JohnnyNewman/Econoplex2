//! Migration value: what living in a town is worth to an agent weighing a move.
//!
//! Interface: a view of one town as the agent sees it (food per resident, danger,
//! how close its people are in outlook, trusted people there, how well the agent's
//! trade sells there, the pay a worker-day of that trade brings there, the road from
//! home) and the agent's own situation. Output: a value; the agent moves to the best
//! town if it beats home by `min_gain`.
//!
//! `Weighted` is the original rule (a weighted sum, `migration` in models.ron).
//! `Utility` values a town through the utility of the agent's needs, the same way
//! job choice does, plus the social pull of outlook and friends.

use super::utility::{needs, Needs, Utility, FOOD, SAFETY};
use serde::{Deserialize, Serialize};

/// One town, as an agent weighing a move sees it.
pub struct TownView {
    pub food_per_capita: f32,
    /// 1 if the town was beaten in a raid this year.
    pub danger: f32,
    /// 0..1: how close the town's average outlook is to the agent's.
    pub closeness: f32,
    /// Trust the agent places in people living there (capped at 1).
    pub friends: f32,
    /// How the agent's trade sells there against the average town, clamped -1..2.
    pub sells: f32,
    /// What a worker-day of the agent's trade pays there.
    pub pay: f32,
    /// Bonus for encouraged immigration.
    pub welcome: f32,
    /// Road length from home (0 for home).
    pub road: f32,
}

/// The agent's own situation.
pub struct MoverView<'a> {
    pub state: &'a sim_data::dims::StateVec,
    pub wealth: f32,
    pub median_wealth: f32,
    pub weights: &'a Needs,
}

pub trait Migration: Send + Sync {
    fn value(&self, me: &MoverView, town: &TownView, utility: &dyn Utility, horizon: f32) -> f32;
    /// How much better than home a town must be to move.
    fn min_gain(&self) -> f32;
}

/// `decision.migration` in models.ron.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum MigrationValueParams {
    /// The original weighted sum, with the weights of the `migration` section.
    #[default]
    Weighted,
    /// Utility of needs in the town (pay over the horizon, food, safety), plus
    /// `social_weight` × (closeness + friends) and the welcome, minus
    /// `distance_cost` per 1000 road units; move when the gain exceeds `min_gain`.
    Utility {
        social_weight: f32,
        distance_cost: f32,
        min_gain: f32,
        /// Food per resident at which a town's food feels secure.
        food_reference: f32,
    },
}

/// The weights of the original rule (from the `migration` section).
pub struct Weighted {
    pub food_weight: f32,
    pub food_reference: f32,
    pub danger_weight: f32,
    pub ideology_weight: f32,
    pub trade_weight: f32,
    pub trust_weight: f32,
    pub distance_cost: f32,
    pub min_gain: f32,
}

impl Migration for Weighted {
    fn value(&self, _me: &MoverView, t: &TownView, _u: &dyn Utility, _horizon: f32) -> f32 {
        let base = self.food_weight * (t.food_per_capita / self.food_reference.max(1e-6)).min(2.0)
            - self.danger_weight * t.danger;
        let v = base
            + self.ideology_weight * t.closeness
            + self.trade_weight * t.sells
            + self.trust_weight * t.friends
            + t.welcome;
        if t.road > 0.0 {
            v - self.distance_cost * t.road / 1000.0
        } else {
            v
        }
    }

    fn min_gain(&self) -> f32 {
        self.min_gain
    }
}

pub struct UtilityMigration {
    pub social_weight: f32,
    pub distance_cost: f32,
    pub min_gain: f32,
    pub food_reference: f32,
}

impl Migration for UtilityMigration {
    fn value(&self, me: &MoverView, t: &TownView, u: &dyn Utility, horizon: f32) -> f32 {
        let mut n = needs(
            me.state,
            me.wealth + t.pay.max(0.0) * horizon,
            me.median_wealth,
        );
        // Food feels as secure as the town's stores; a town beaten in a raid feels unsafe.
        n[FOOD] = n[FOOD].min((t.food_per_capita / self.food_reference.max(1e-6)).min(1.0));
        n[SAFETY] *= 1.0 - 0.5 * t.danger;
        u.value(&n, me.weights) + self.social_weight * (t.closeness + t.friends) + t.welcome
            - self.distance_cost * t.road / 1000.0
    }

    fn min_gain(&self) -> f32 {
        self.min_gain
    }
}

pub fn build(
    p: &MigrationValueParams,
    m: &crate::config::MigrationParams,
    trust_weight: f32,
) -> Box<dyn Migration> {
    match *p {
        MigrationValueParams::Weighted => Box::new(Weighted {
            food_weight: m.food_weight,
            food_reference: m.food_reference,
            danger_weight: m.danger_weight,
            ideology_weight: m.ideology_weight,
            trade_weight: m.trade_weight,
            trust_weight,
            distance_cost: m.distance_cost,
            min_gain: m.min_gain,
        }),
        MigrationValueParams::Utility {
            social_weight,
            distance_cost,
            min_gain,
            food_reference,
        } => Box::new(UtilityMigration {
            social_weight,
            distance_cost,
            min_gain,
            food_reference,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::utility::Crra;

    fn town(pay: f32, food: f32, road: f32) -> TownView {
        TownView {
            food_per_capita: food,
            danger: 0.0,
            closeness: 0.5,
            friends: 0.0,
            sells: 0.0,
            pay,
            welcome: 0.0,
            road,
        }
    }

    fn rule() -> UtilityMigration {
        UtilityMigration {
            social_weight: 0.3,
            distance_cost: 0.1,
            min_gain: 0.05,
            food_reference: 3.0,
        }
    }

    #[test]
    fn better_pay_and_food_draw_people_and_roads_hold_them_back() {
        let u = Crra { rho: 1.5 };
        let st = [0.3, 0.2, 0.6, 1.0];
        let w = [1.0; 4];
        let me = MoverView {
            state: &st,
            wealth: 10.0,
            median_wealth: 20.0,
            weights: &w,
        };
        let v = |t: &TownView| rule().value(&me, t, &u, 25.0);
        assert!(v(&town(2.0, 3.0, 0.0)) > v(&town(0.5, 3.0, 0.0)));
        assert!(v(&town(1.0, 3.0, 0.0)) > v(&town(1.0, 0.3, 0.0)));
        assert!(v(&town(1.0, 3.0, 0.0)) > v(&town(1.0, 3.0, 2000.0)));
    }
}
