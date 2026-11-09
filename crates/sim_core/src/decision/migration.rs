//! Migration value: what living in a town is worth to an agent weighing a move.
//!
//! Interface: a view of one town as the agent sees it (food per resident, pay of the
//! agent's trade, rent, cultural fit, the agent's social network there, how safe it
//! is, the road from home) and the agent's own situation. Output: a value. The
//! migrate system sums the values of a household's members and moves the household
//! to the best town it knows if that beats home by `min_gain` per member.
//!
//! `Weighted` is the original rule (a weighted sum, `migration` in models.ron).
//! `Utility` has four weighted terms:
//! - **economic strength**: the utility of the agent's needs there, with pay of its
//!   own trade minus rent over the horizon, and food per head;
//! - **culture fit** with the town's culture (`decision::culture`);
//! - **social network**: trust, friends and family living there;
//! - **safety**: recent raids and losses, offset by the town's soldiers.
//!
//! Economic strength and culture fit are the main terms; the road's cost and
//! `min_gain` are the threshold that keeps households from hopping.

use super::utility::{needs, Needs, Utility, FOOD};
use serde::{Deserialize, Serialize};

/// One town, as an agent weighing a move sees it.
pub struct TownView {
    pub food_per_capita: f32,
    /// 1 if the town was beaten in a raid this year.
    pub danger: f32,
    /// 0..1: how safe the town is (raids, losses, soldiers).
    pub safety: f32,
    /// -1..1: how well the agent fits the town's culture (`decision::culture::fit`).
    pub culture_fit: f32,
    /// Trust the agent places in people living there (capped at 1).
    pub friends: f32,
    /// 0..1: strength of the agent's social network there (trust and family).
    pub social: f32,
    /// How the agent's trade sells there against the average town, clamped -1..2.
    pub sells: f32,
    /// What a worker-day of the agent's trade pays there.
    pub pay: f32,
    /// Daily rent there (see `decision::crowding`).
    pub rent: f32,
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
    /// How much better than home a town must be, per household member, to move.
    fn min_gain(&self) -> f32;
}

/// `decision.migration` in models.ron.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum MigrationValueParams {
    /// The original weighted sum, with the weights of the `migration` section.
    #[default]
    Weighted,
    /// `economy_weight` × economic strength + `culture_weight` × culture fit +
    /// `social_weight` × social network + `safety_weight` × safety + the welcome,
    /// minus `distance_cost` per 1000 road units.
    Utility(UtilityMigrationParams),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UtilityMigrationParams {
    pub economy_weight: f32,
    pub culture_weight: f32,
    pub social_weight: f32,
    pub safety_weight: f32,
    pub distance_cost: f32,
    pub min_gain: f32,
    /// Food per resident at which a town's food feels secure.
    pub food_reference: f32,
}

impl Default for UtilityMigrationParams {
    fn default() -> Self {
        UtilityMigrationParams {
            economy_weight: 1.0,
            culture_weight: 1.0,
            social_weight: 0.4,
            safety_weight: 0.4,
            distance_cost: 0.1,
            min_gain: 0.3,
            food_reference: 3.0,
        }
    }
}

/// The weights of the original rule (from the `migration` section).
pub struct Weighted {
    pub food_weight: f32,
    pub food_reference: f32,
    pub danger_weight: f32,
    pub culture_weight: f32,
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
            + self.culture_weight * t.culture_fit
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

pub struct UtilityMigration(pub UtilityMigrationParams);

impl UtilityMigration {
    /// Economic strength of a town for this agent: the utility of its needs there.
    pub fn economy(&self, me: &MoverView, t: &TownView, u: &dyn Utility, horizon: f32) -> f32 {
        let mut n = needs(
            me.state,
            (me.wealth + (t.pay.max(0.0) - t.rent) * horizon).max(0.0),
            me.median_wealth,
        );
        // Food feels as secure as the town's stores.
        n[FOOD] = n[FOOD].min((t.food_per_capita / self.0.food_reference.max(1e-6)).min(1.0));
        u.value(&n, me.weights)
    }
}

impl Migration for UtilityMigration {
    fn value(&self, me: &MoverView, t: &TownView, u: &dyn Utility, horizon: f32) -> f32 {
        let p = &self.0;
        p.economy_weight * self.economy(me, t, u, horizon)
            + p.culture_weight * t.culture_fit
            + p.social_weight * t.social
            + p.safety_weight * t.safety
            + t.welcome
            - p.distance_cost * t.road / 1000.0
    }

    fn min_gain(&self) -> f32 {
        self.0.min_gain
    }
}

pub fn build(
    p: &MigrationValueParams,
    m: &crate::config::MigrationParams,
    trust_weight: f32,
) -> Box<dyn Migration> {
    match p {
        MigrationValueParams::Weighted => Box::new(Weighted {
            food_weight: m.food_weight,
            food_reference: m.food_reference,
            danger_weight: m.danger_weight,
            culture_weight: m.culture_weight,
            trade_weight: m.trade_weight,
            trust_weight,
            distance_cost: m.distance_cost,
            min_gain: m.min_gain,
        }),
        MigrationValueParams::Utility(p) => Box::new(UtilityMigration(p.clone())),
    }
}

/// How safe a town is, 0..1: a raid it lost this year and people it lost (fallen or
/// carried off, per resident) threaten it; its soldiers (per resident, against
/// `guard_share`) halve the threat at full strength.
pub fn safety(
    beaten: bool,
    losses: u32,
    residents: usize,
    soldiers: usize,
    guard_share: f32,
) -> f32 {
    let n = residents.max(1) as f32;
    let threat = (if beaten { 0.6 } else { 0.0 } + 10.0 * losses as f32 / n).min(1.0);
    let protection = (soldiers as f32 / (guard_share.max(1e-3) * n)).min(1.0);
    (1.0 - threat * (1.0 - 0.5 * protection)).clamp(0.0, 1.0)
}

/// Strength of a social network, 0..1, from the trust and family ties in a town.
pub fn network(trust: f32, kin: u32, kin_trust: f32) -> f32 {
    1.0 - (-(trust.max(0.0) + kin as f32 * kin_trust)).exp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::utility::Crra;

    fn town(pay: f32, food: f32, road: f32) -> TownView {
        TownView {
            food_per_capita: food,
            danger: 0.0,
            safety: 1.0,
            culture_fit: 0.5,
            friends: 0.0,
            social: 0.0,
            sells: 0.0,
            pay,
            rent: 0.0,
            welcome: 0.0,
            road,
        }
    }

    fn rule() -> UtilityMigration {
        UtilityMigration(UtilityMigrationParams {
            social_weight: 0.3,
            min_gain: 0.05,
            ..Default::default()
        })
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
        // A crowded town's rent eats into its pay.
        let crowded = TownView {
            rent: 0.5,
            ..town(1.0, 3.0, 0.0)
        };
        assert!(v(&town(1.0, 3.0, 0.0)) > v(&crowded));
        // Culture fit, a social network and safety each draw, all else equal.
        let base = v(&town(1.0, 3.0, 0.0));
        let fits = TownView {
            culture_fit: 0.9,
            ..town(1.0, 3.0, 0.0)
        };
        let friends = TownView {
            social: 0.8,
            ..town(1.0, 3.0, 0.0)
        };
        let unsafe_town = TownView {
            safety: 0.2,
            ..town(1.0, 3.0, 0.0)
        };
        assert!(v(&fits) > base);
        assert!(v(&friends) > base);
        assert!(v(&unsafe_town) < base);
    }

    #[test]
    fn raids_and_losses_make_a_town_unsafe_and_soldiers_help() {
        assert_eq!(safety(false, 0, 100, 0, 0.05), 1.0);
        assert!(safety(true, 0, 100, 0, 0.05) < 0.5);
        assert!(safety(true, 0, 100, 5, 0.05) > safety(true, 0, 100, 0, 0.05));
        assert!(safety(false, 5, 100, 0, 0.05) < safety(false, 1, 100, 0, 0.05));
        assert!((0.0..=1.0).contains(&safety(true, 500, 100, 0, 0.05)));
    }

    #[test]
    fn family_and_trust_build_a_network() {
        assert_eq!(network(0.0, 0, 0.5), 0.0);
        assert!(network(0.0, 2, 0.5) > network(0.0, 1, 0.5));
        assert!(network(1.0, 0, 0.5) > network(0.3, 0, 0.5));
        assert!(network(50.0, 10, 0.5) <= 1.0);
    }
}
