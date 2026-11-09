//! Utility: how much an agent values the state of its needs.
//!
//! Interface: the agent's need satisfactions (each 0..1) and its need weights.
//! Output: one utility value. Utility has diminishing returns per need, so a badly
//! met need (a starving agent's food) outweighs everything else.

use serde::{Deserialize, Serialize};
use sim_data::dims::{mind, state, MindVec, StateVec, MIND_DIM};

/// The needs an agent weighs.
pub const NEEDS: usize = 4;
pub const FOOD: usize = 0;
pub const REST: usize = 1;
pub const SAFETY: usize = 2;
pub const STANDING: usize = 3;
pub const NEED_NAMES: [&str; NEEDS] = ["food", "rest", "safety", "standing"];

pub type Needs = [f32; NEEDS];

pub trait Utility: Send + Sync {
    fn value(&self, needs: &Needs, weights: &Needs) -> f32;
}

/// Which utility function to use (`decision.utility.model`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UtilityModel {
    /// Constant relative risk aversion per need: `(s^(1-rho) - 1) / (1 - rho)`
    /// (the logarithm at `rho` = 1). Higher `rho`: stronger diminishing returns.
    Crra { rho: f32 },
}

/// `decision.utility` in models.ron.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UtilityParams {
    pub model: UtilityModel,
    /// Need weights: `base + mind_weights · mind`, kept positive.
    pub base: Vec<f32>,
    /// One row per need, one column per mind axis.
    pub mind_weights: Vec<Vec<f32>>,
}

impl Default for UtilityParams {
    fn default() -> Self {
        let mut mw = vec![vec![0.0; MIND_DIM]; NEEDS];
        // Ambitious and sociable agents care more about standing; risk-takers less
        // about safety; diligent ones less about rest.
        mw[STANDING][mind::AMBITION] = 0.8;
        mw[STANDING][mind::SOCIABILITY] = 0.3;
        mw[SAFETY][mind::RISK] = -0.5;
        mw[REST][mind::DILIGENCE] = -0.4;
        UtilityParams {
            model: UtilityModel::Crra { rho: 1.5 },
            base: vec![1.5, 0.8, 0.6, 0.3],
            mind_weights: mw,
        }
    }
}

pub fn build(p: &UtilityParams) -> Box<dyn Utility> {
    match p.model {
        UtilityModel::Crra { rho } => Box::new(Crra { rho }),
    }
}

/// Need weights of an agent with this mind.
pub fn need_weights(p: &UtilityParams, m: &MindVec) -> Needs {
    std::array::from_fn(|k| {
        let w: f32 = p
            .mind_weights
            .get(k)
            .map_or(0.0, |row| row.iter().zip(m).map(|(a, b)| a * b).sum());
        (p.base.get(k).copied().unwrap_or(1.0) + w).max(0.05)
    })
}

/// How well an agent's needs are met. `standing` is its wealth relative to the
/// town's median (0.5 at the median).
pub fn needs(state_v: &StateVec, wealth: f32, median_wealth: f32) -> Needs {
    let mut n = [0.0; NEEDS];
    n[FOOD] = (1.0 - state_v[state::HUNGER]).clamp(0.0, 1.0);
    n[REST] = (1.0 - state_v[state::FATIGUE]).clamp(0.0, 1.0);
    n[SAFETY] = state_v[state::HEALTH].clamp(0.0, 1.0);
    let w = wealth.max(0.0);
    n[STANDING] = w / (w + median_wealth.max(1e-3));
    n
}

pub struct Crra {
    pub rho: f32,
}

/// Floor on satisfaction so utility stays finite.
const FLOOR: f32 = 0.02;

impl Crra {
    fn per_need(&self, s: f32) -> f32 {
        let s = s.max(FLOOR);
        if (self.rho - 1.0).abs() < 1e-4 {
            s.ln()
        } else {
            (s.powf(1.0 - self.rho) - 1.0) / (1.0 - self.rho)
        }
    }
}

impl Utility for Crra {
    fn value(&self, needs: &Needs, weights: &Needs) -> f32 {
        needs
            .iter()
            .zip(weights)
            .map(|(&s, &w)| w * self.per_need(s))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u() -> Crra {
        Crra { rho: 1.5 }
    }

    #[test]
    fn more_of_a_need_is_better() {
        let w = [1.0; NEEDS];
        let mut lo = [0.5; NEEDS];
        let mut hi = lo;
        hi[FOOD] = 0.8;
        assert!(u().value(&hi, &w) > u().value(&lo, &w));
        lo[REST] = 0.2;
        assert!(u().value(&lo, &w) < u().value(&[0.5; NEEDS], &w));
    }

    #[test]
    fn returns_diminish() {
        let w = [1.0; NEEDS];
        let at = |food: f32| {
            let mut n = [0.5; NEEDS];
            n[FOOD] = food;
            u().value(&n, &w)
        };
        assert!(at(0.3) - at(0.2) > at(0.9) - at(0.8));
    }

    #[test]
    fn a_starving_agent_values_food_above_everything() {
        let w = [1.0; NEEDS];
        let mut n = [0.8; NEEDS];
        n[FOOD] = 0.05;
        let gain = |k: usize| {
            let mut m = n;
            m[k] += 0.1;
            u().value(&m, &w) - u().value(&n, &w)
        };
        assert!((1..NEEDS).all(|k| gain(FOOD) > 5.0 * gain(k)));
    }

    #[test]
    fn the_mind_shapes_need_weights() {
        let p = UtilityParams::default();
        let mut ambitious = [0.5; MIND_DIM];
        ambitious[mind::AMBITION] = 1.0;
        let mut modest = ambitious;
        modest[mind::AMBITION] = 0.0;
        assert!(need_weights(&p, &ambitious)[STANDING] > need_weights(&p, &modest)[STANDING]);
    }

    #[test]
    fn needs_come_from_the_agent_state() {
        let st = [0.9, 0.2, 0.6, 1.0]; // hunger, fatigue, happiness, health
        let n = needs(&st, 30.0, 30.0);
        assert!((n[FOOD] - 0.1).abs() < 1e-6);
        assert!((n[REST] - 0.8).abs() < 1e-6);
        assert!((n[STANDING] - 0.5).abs() < 1e-6);
    }
}
