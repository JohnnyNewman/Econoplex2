//! Expectations: what an agent thinks a price or a day's pay will be, learned from
//! what it observes.
//!
//! Interface: the previous expectation (or `None` if the agent has none yet), an
//! observation, and the agent's curiosity in 0..1 (curious agents update faster).
//! Output: the new expectation. Nothing else is read.

use serde::{Deserialize, Serialize};

pub trait Expectations: Send + Sync {
    fn update(&self, previous: Option<f32>, observed: f32, curiosity: f32) -> f32;
}

/// Which expectation rule to use, with its parameters (`decision.expectations`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExpectationsParams {
    /// Expect exactly the last observation.
    Fixed,
    /// Move a share of the way toward each observation (an exponential moving
    /// average); the share is `base_rate + curiosity_rate × curiosity`.
    Adaptive { base_rate: f32, curiosity_rate: f32 },
}

impl Default for ExpectationsParams {
    fn default() -> Self {
        ExpectationsParams::Adaptive {
            base_rate: 0.05,
            curiosity_rate: 0.2,
        }
    }
}

pub fn build(p: &ExpectationsParams) -> Box<dyn Expectations> {
    match *p {
        ExpectationsParams::Fixed => Box::new(Fixed),
        ExpectationsParams::Adaptive {
            base_rate,
            curiosity_rate,
        } => Box::new(Adaptive {
            base_rate,
            curiosity_rate,
        }),
    }
}

pub struct Fixed;

impl Expectations for Fixed {
    fn update(&self, _previous: Option<f32>, observed: f32, _curiosity: f32) -> f32 {
        observed
    }
}

pub struct Adaptive {
    pub base_rate: f32,
    pub curiosity_rate: f32,
}

impl Expectations for Adaptive {
    fn update(&self, previous: Option<f32>, observed: f32, curiosity: f32) -> f32 {
        match previous {
            None => observed,
            Some(e) => {
                let rate = (self.base_rate + self.curiosity_rate * curiosity).clamp(0.0, 1.0);
                e + rate * (observed - e)
            }
        }
    }
}

/// An agent's pay expectations for the jobs it has done: at most [`KEEP`] entries of
/// (recipe, expected pay per worker-day); the least recently updated goes first.
pub const KEEP: usize = 8;

pub fn remember(
    list: &mut Vec<(u32, f32)>,
    recipe: u32,
    observed: f32,
    curiosity: f32,
    rule: &dyn Expectations,
) {
    match list.iter().position(|e| e.0 == recipe) {
        Some(k) => {
            let (_, prev) = list.remove(k);
            list.push((recipe, rule.update(Some(prev), observed, curiosity)));
        }
        None => {
            if list.len() >= KEEP {
                list.remove(0);
            }
            list.push((recipe, rule.update(None, observed, curiosity)));
        }
    }
}

/// What an agent expects a worker-day at `recipe` to pay: its own experience if it
/// has any, otherwise the town's public pay rate.
pub fn expected_pay(own: &[(u32, f32)], recipe: u32, public: f32) -> f32 {
    own.iter().find(|e| e.0 == recipe).map_or(public, |e| e.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_expectations_converge_to_a_steady_value() {
        let rule = Adaptive {
            base_rate: 0.05,
            curiosity_rate: 0.2,
        };
        let mut e = rule.update(None, 1.0, 0.5);
        for _ in 0..200 {
            e = rule.update(Some(e), 4.0, 0.5);
        }
        assert!((e - 4.0).abs() < 1e-3);
    }

    #[test]
    fn curious_agents_learn_faster() {
        let rule = Adaptive {
            base_rate: 0.05,
            curiosity_rate: 0.2,
        };
        let slow = rule.update(Some(0.0), 10.0, 0.0);
        let fast = rule.update(Some(0.0), 10.0, 1.0);
        assert!(fast > slow && slow > 0.0);
    }

    #[test]
    fn remembering_keeps_the_most_recent_jobs() {
        let rule = Fixed;
        let mut list = Vec::new();
        for r in 0..(KEEP as u32 + 3) {
            remember(&mut list, r, r as f32, 0.5, &rule);
        }
        assert_eq!(list.len(), KEEP);
        assert!(list.iter().all(|e| e.0 >= 3));
        assert_eq!(expected_pay(&list, 5, 99.0), 5.0);
        assert_eq!(expected_pay(&list, 0, 99.0), 99.0);
    }
}
