//! Choice rule: how an agent turns the values of its options into a decision.
//!
//! Interface: the value of resting, the values of the candidate jobs (in a fixed
//! order), the agent's mind (for its temperature) and its random stream. Output: the
//! chosen job, or `None` for resting. Values may be in any units; the rule's
//! temperature is in the same units.

use rand::{Rng, RngCore};
use serde::{Deserialize, Serialize};
use sim_data::dims::{mind, MindVec};

pub trait ChoiceRule: Send + Sync {
    fn pick(
        &self,
        rest: f32,
        jobs: &[(u32, f32)],
        mind: &MindVec,
        rng: &mut dyn RngCore,
    ) -> Option<u32>;
}

/// `decision.choice` in models.ron.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChoiceParams {
    /// Always the best option (ties: resting, then the earlier job).
    Argmax,
    /// Softmax (logit) choice. Temperature = `base × (1 + risk_weight × risk −
    /// diligence_weight × diligence)`, at least a tenth of `base`. `base: None` uses
    /// `decision.temperature`.
    Logit {
        #[serde(default)]
        base: Option<f32>,
        #[serde(default)]
        risk_weight: f32,
        #[serde(default)]
        diligence_weight: f32,
    },
}

impl Default for ChoiceParams {
    fn default() -> Self {
        ChoiceParams::Logit {
            base: None,
            risk_weight: 0.0,
            diligence_weight: 0.0,
        }
    }
}

pub fn build(p: &ChoiceParams, decision_temperature: f32) -> Box<dyn ChoiceRule> {
    match *p {
        ChoiceParams::Argmax => Box::new(Argmax),
        ChoiceParams::Logit {
            base,
            risk_weight,
            diligence_weight,
        } => Box::new(Logit {
            base: base.unwrap_or(decision_temperature),
            risk_weight,
            diligence_weight,
        }),
    }
}

pub struct Argmax;

impl ChoiceRule for Argmax {
    fn pick(
        &self,
        rest: f32,
        jobs: &[(u32, f32)],
        _mind: &MindVec,
        _rng: &mut dyn RngCore,
    ) -> Option<u32> {
        let mut best = (None, rest);
        for &(r, v) in jobs {
            if v > best.1 {
                best = (Some(r), v);
            }
        }
        best.0
    }
}

pub struct Logit {
    pub base: f32,
    pub risk_weight: f32,
    pub diligence_weight: f32,
}

impl Logit {
    pub fn temperature(&self, m: &MindVec) -> f32 {
        let f = 1.0 + self.risk_weight * m[mind::RISK] - self.diligence_weight * m[mind::DILIGENCE];
        (self.base * f.max(0.1)).max(1e-4)
    }
}

impl ChoiceRule for Logit {
    fn pick(
        &self,
        rest: f32,
        jobs: &[(u32, f32)],
        m: &MindVec,
        rng: &mut dyn RngCore,
    ) -> Option<u32> {
        let max = jobs.iter().map(|j| j.1).fold(rest, f32::max);
        let temp = self.temperature(m);
        let weight = |s: f32| ((s - max) / temp).exp();
        let total = weight(rest) + jobs.iter().map(|j| weight(j.1)).sum::<f32>();
        let mut x = rng.random::<f32>() * total;
        x -= weight(rest);
        if x <= 0.0 {
            return None;
        }
        for &(r, v) in jobs {
            x -= weight(v);
            if x <= 0.0 {
                return Some(r);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn argmax_takes_the_best_option() {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(1);
        let m = [0.5; 8];
        assert_eq!(
            Argmax.pick(0.0, &[(3, 1.0), (4, 2.0)], &m, &mut rng),
            Some(4)
        );
        assert_eq!(Argmax.pick(5.0, &[(3, 1.0)], &m, &mut rng), None);
    }

    #[test]
    fn logit_prefers_better_options_and_cools_with_diligence() {
        let rule = Logit {
            base: 0.5,
            risk_weight: 0.5,
            diligence_weight: 0.8,
        };
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(2);
        let m = [0.5; 8];
        let picks = (0..2000)
            .filter(|_| rule.pick(0.0, &[(1, 1.0)], &m, &mut rng) == Some(1))
            .count();
        assert!(picks > 1400, "{picks}");
        let mut diligent = m;
        diligent[mind::DILIGENCE] = 1.0;
        let mut reckless = m;
        reckless[mind::RISK] = 1.0;
        assert!(rule.temperature(&diligent) < rule.temperature(&reckless));
    }
}
