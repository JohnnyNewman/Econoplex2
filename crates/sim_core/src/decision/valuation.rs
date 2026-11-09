//! Valuation for the utility model: what working a job, or resting, is worth to an
//! agent over the next few weeks.
//!
//! Inputs: the agent's needs today, its need weights, its wealth and the town's
//! median wealth, the expected pay of a worker-day at the job (from the expectations
//! component), the agent's chance of success, and the fatigue the work costs.
//! Output: utility, from the utility component. Nothing else is read.
//!
//! Working changes two things: fatigue tomorrow (the rest need), and wealth over the
//! horizon if the agent keeps at this kind of work (the standing need). Resting
//! recovers fatigue and earns nothing. Hunger is the same either way (agents eat from
//! the town's stores whether or not they work), unless the household keeps part of
//! the food it makes (`decision::subsistence`): then a food job also covers the
//! worker's own food need, by the share `food_cover`.

use super::utility::{needs, Needs, Utility, FOOD, REST};
use serde::{Deserialize, Serialize};

/// `decision.valuation` in models.ron.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ValuationParams {
    /// Days ahead an agent looks (weeks).
    pub horizon_days: f32,
    /// Daily discount factor over the horizon.
    pub discount: f32,
    /// Success chance a job's typical worker has; an agent's expected pay scales by
    /// its own chance against this.
    pub typical_success: f32,
}

impl Default for ValuationParams {
    fn default() -> Self {
        ValuationParams {
            horizon_days: 30.0,
            discount: 0.97,
            typical_success: 0.7,
        }
    }
}

impl ValuationParams {
    /// Sum of discount factors over the horizon.
    pub fn horizon_weight(&self) -> f32 {
        let n = self.horizon_days.max(1.0);
        let b = self.discount.clamp(0.0, 1.0);
        if (1.0 - b).abs() < 1e-6 {
            n
        } else {
            (1.0 - b.powf(n)) / (1.0 - b)
        }
    }
}

/// One agent's situation, as the valuation sees it.
pub struct Situation<'a> {
    pub state: &'a sim_data::dims::StateVec,
    pub wealth: f32,
    pub median_wealth: f32,
    pub weights: &'a Needs,
}

pub fn rest_value(u: &dyn Utility, s: &Situation, fatigue_recovery: f32) -> f32 {
    let mut n = needs(s.state, s.wealth, s.median_wealth);
    n[REST] = (n[REST] + fatigue_recovery).min(1.0);
    u.value(&n, s.weights)
}

/// Value of working a job paying `expected_pay` per worker-day to a typical worker,
/// with success chance `success` for this agent, costing `fatigue_cost` per day.
pub fn job_value(
    u: &dyn Utility,
    p: &ValuationParams,
    s: &Situation,
    expected_pay: f32,
    success: f32,
    fatigue_cost: f32,
    learning_value: f32,
) -> f32 {
    job_value_fed(
        u,
        p,
        s,
        expected_pay,
        success,
        fatigue_cost,
        learning_value,
        0.0,
    )
}

/// `job_value` for a job whose kept output covers `food_cover` (0..1) of the
/// worker's food need over the horizon.
#[allow(clippy::too_many_arguments)]
pub fn job_value_fed(
    u: &dyn Utility,
    p: &ValuationParams,
    s: &Situation,
    expected_pay: f32,
    success: f32,
    fatigue_cost: f32,
    learning_value: f32,
    food_cover: f32,
) -> f32 {
    let income = expected_pay.max(0.0) * (success / p.typical_success.max(1e-3)).clamp(0.0, 1.5);
    let mut n = needs(
        s.state,
        s.wealth + income * p.horizon_weight() + learning_value.max(0.0),
        s.median_wealth,
    );
    n[REST] = (n[REST] - fatigue_cost).max(0.0);
    n[FOOD] =
        n[FOOD].max(food_cover.clamp(0.0, 1.0) * (success / p.typical_success.max(1e-3)).min(1.0));
    u.value(&n, s.weights)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::utility::Crra;

    fn sit<'a>(state: &'a sim_data::dims::StateVec, wealth: f32, w: &'a Needs) -> Situation<'a> {
        Situation {
            state,
            wealth,
            median_wealth: 30.0,
            weights: w,
        }
    }

    #[test]
    fn keeping_food_makes_a_food_job_worth_more_to_the_hungry() {
        let u = Crra { rho: 1.5 };
        let p = ValuationParams::default();
        let st = [0.9, 0.2, 0.6, 1.0];
        let w = [1.0; 4];
        let s = sit(&st, 20.0, &w);
        let plain = job_value(&u, &p, &s, 1.0, 0.7, 0.1, 0.0);
        assert!(job_value_fed(&u, &p, &s, 1.0, 0.7, 0.1, 0.0, 0.8) > plain);
        assert_eq!(job_value_fed(&u, &p, &s, 1.0, 0.7, 0.1, 0.0, 0.0), plain);
    }

    #[test]
    fn better_paid_and_easier_work_is_worth_more() {
        let u = Crra { rho: 1.5 };
        let p = ValuationParams::default();
        let st = [0.3, 0.2, 0.6, 1.0];
        let w = [1.0; 4];
        let s = sit(&st, 20.0, &w);
        assert!(
            job_value(&u, &p, &s, 2.0, 0.7, 0.1, 0.0) > job_value(&u, &p, &s, 1.0, 0.7, 0.1, 0.0)
        );
        assert!(
            job_value(&u, &p, &s, 1.0, 0.7, 0.1, 0.0) > job_value(&u, &p, &s, 1.0, 0.7, 0.3, 0.0)
        );
        assert!(
            job_value(&u, &p, &s, 1.0, 0.9, 0.1, 0.0) > job_value(&u, &p, &s, 1.0, 0.3, 0.1, 0.0)
        );
    }

    #[test]
    fn the_rich_and_the_tired_rest_more_readily() {
        let u = Crra { rho: 1.5 };
        let p = ValuationParams::default();
        let w = [1.0; 4];
        let fresh = [0.3, 0.1, 0.6, 1.0];
        let tired = [0.3, 0.8, 0.6, 1.0];
        // Gain from working over resting, for a poor and a rich agent.
        let gain = |st: &sim_data::dims::StateVec, wealth: f32| {
            let s = sit(st, wealth, &w);
            job_value(&u, &p, &s, 1.0, 0.7, 0.15, 0.0) - rest_value(&u, &s, 0.4)
        };
        assert!(gain(&fresh, 5.0) > gain(&fresh, 300.0));
        assert!(gain(&fresh, 20.0) > gain(&tired, 20.0));
    }
}
