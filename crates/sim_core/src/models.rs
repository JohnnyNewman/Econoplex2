//! Pluggable formulas. Each model is a trait; [`Models`] holds the implementations
//! selected in `models.ron`. To try a new formula, add an implementation and a
//! matching config variant — systems never hard-code the math.

use crate::config::*;
use bevy_ecs::prelude::Resource;
use sim_data::dims::{dot, mind, norm, CapVec, MindVec, CAP_DIM};

pub trait TeamPooling: Send + Sync {
    /// Combine the members' effective capability vectors into one team vector.
    fn pool(&self, members: &[CapVec]) -> CapVec;
    /// Proficiency penalty for a team of `n`.
    fn coordination_penalty(&self, n: usize) -> f32;
}

pub trait SuccessModel: Send + Sync {
    fn probability(&self, effective: f32, difficulty: f32) -> f32;
}

pub trait QualityModel: Send + Sync {
    fn step_quality(&self, effective: f32, difficulty: f32) -> f32;
    fn combine(&self, step: f32, inputs: f32) -> f32;
}

pub trait LearningRule: Send + Sync {
    fn practice(&self, cap: &mut CapVec, skill: &CapVec, aptitude: f32, success: bool);
}

pub trait DiffusionRule: Send + Sync {
    /// Move `apprentice` toward `master` along `skill`. Returns the master's gain factor.
    fn diffuse(&self, apprentice: &mut CapVec, master: &CapVec, skill: &CapVec) -> f32;
}

pub trait CapacityRule: Send + Sync {
    fn apply(&self, cap: &mut CapVec);
}

pub trait SocialInfluence: Send + Sync {
    /// Interact two minds; `noise` is a pair of uniform samples in [-1, 1] per ideological axis.
    fn interact(&self, a: &mut MindVec, b: &mut MindVec, noise: &[f32]);
    fn daily_contacts(&self) -> f32;
}

// ---------------------------------------------------------------- implementations

pub struct ElementwiseMax {
    pub coordination_cost: f32,
}
impl TeamPooling for ElementwiseMax {
    fn pool(&self, members: &[CapVec]) -> CapVec {
        let mut out: CapVec = [0.0; CAP_DIM];
        for m in members {
            for i in 0..CAP_DIM {
                out[i] = out[i].max(m[i]);
            }
        }
        out
    }
    fn coordination_penalty(&self, n: usize) -> f32 {
        self.coordination_cost * n.saturating_sub(1) as f32
    }
}

pub struct LeaderPlusAssistants {
    pub assist: f32,
    pub coordination_cost: f32,
}
impl TeamPooling for LeaderPlusAssistants {
    fn pool(&self, members: &[CapVec]) -> CapVec {
        let leader = members
            .iter()
            .enumerate()
            .max_by(|a, b| norm(a.1).total_cmp(&norm(b.1)).then(b.0.cmp(&a.0)))
            .map(|(i, _)| i)
            .unwrap_or(0);
        let mut out = members[leader];
        for (j, m) in members.iter().enumerate() {
            if j != leader {
                for i in 0..CAP_DIM {
                    out[i] += self.assist * m[i];
                }
            }
        }
        out
    }
    fn coordination_penalty(&self, n: usize) -> f32 {
        self.coordination_cost * n.saturating_sub(1) as f32
    }
}

pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

pub struct SigmoidSuccess {
    pub steepness: f32,
}
impl SuccessModel for SigmoidSuccess {
    fn probability(&self, effective: f32, difficulty: f32) -> f32 {
        sigmoid(self.steepness * (effective - difficulty))
    }
}

pub struct MarginQuality {
    pub steepness: f32,
    pub offset: f32,
    pub combine: QualityCombine,
}
impl QualityModel for MarginQuality {
    fn step_quality(&self, effective: f32, difficulty: f32) -> f32 {
        sigmoid(self.steepness * (effective - difficulty) + self.offset)
    }
    fn combine(&self, step: f32, inputs: f32) -> f32 {
        match self.combine {
            QualityCombine::Min => step.min(inputs),
            QualityCombine::GeometricMean => (step * inputs).max(0.0).sqrt(),
        }
    }
}

pub struct DiminishingPractice {
    pub rate: f32,
    pub mastery: f32,
    pub failure_factor: f32,
}
impl LearningRule for DiminishingPractice {
    fn practice(&self, cap: &mut CapVec, skill: &CapVec, aptitude: f32, success: bool) {
        let room = (1.0 - dot(cap, skill) / self.mastery).max(0.0);
        let k = self.rate * aptitude * room * if success { 1.0 } else { self.failure_factor };
        for i in 0..CAP_DIM {
            cap[i] += k * skill[i];
        }
    }
}

pub struct AlongSkillDiffusion {
    pub rate: f32,
    pub teacher_gain: f32,
}
impl DiffusionRule for AlongSkillDiffusion {
    fn diffuse(&self, apprentice: &mut CapVec, master: &CapVec, skill: &CapVec) -> f32 {
        let mut diff = [0.0; CAP_DIM];
        for i in 0..CAP_DIM {
            diff[i] = master[i] - apprentice[i];
        }
        let gap = dot(&diff, skill).max(0.0);
        for i in 0..CAP_DIM {
            apprentice[i] += self.rate * gap * skill[i];
        }
        self.teacher_gain * gap
    }
}

pub struct NormBudget {
    pub budget: f32,
    pub forgetting: f32,
}
impl CapacityRule for NormBudget {
    fn apply(&self, cap: &mut CapVec) {
        let keep = 1.0 - self.forgetting;
        for x in cap.iter_mut() {
            *x = (*x * keep).max(0.0);
        }
        let n = norm(cap);
        if n > self.budget {
            let s = self.budget / n;
            cap.iter_mut().for_each(|x| *x *= s);
        }
    }
}

pub struct BoundedConfidence {
    pub rate: f32,
    pub confidence: f32,
    pub repulsion_distance: f32,
    pub repulsion: f32,
    pub noise: f32,
    pub daily_contacts: f32,
}
impl SocialInfluence for BoundedConfidence {
    fn interact(&self, a: &mut MindVec, b: &mut MindVec, noise: &[f32]) {
        let s = mind::IDEOLOGY_START;
        let mut d2 = 0.0;
        for i in s..a.len() {
            d2 += (a[i] - b[i]).powi(2);
        }
        let d = d2.sqrt();
        let k = if d < self.confidence {
            self.rate
        } else if d > self.repulsion_distance {
            -self.repulsion
        } else {
            0.0
        };
        for (n, i) in (s..a.len()).enumerate() {
            let delta = b[i] - a[i];
            a[i] = (a[i] + k * delta + self.noise * noise[2 * n]).clamp(0.0, 1.0);
            b[i] = (b[i] - k * delta + self.noise * noise[2 * n + 1]).clamp(0.0, 1.0);
        }
    }
    fn daily_contacts(&self) -> f32 {
        self.daily_contacts
    }
}

/// The active set of models, built from [`ModelParams`].
#[derive(Resource)]
pub struct Models {
    pub pooling: Box<dyn TeamPooling>,
    pub success: Box<dyn SuccessModel>,
    pub quality: Box<dyn QualityModel>,
    pub learning: Box<dyn LearningRule>,
    pub diffusion: Box<dyn DiffusionRule>,
    pub capacity: Box<dyn CapacityRule>,
    pub social: Box<dyn SocialInfluence>,
}

impl Models {
    pub fn from_params(p: &ModelParams) -> Self {
        Models {
            pooling: match p.team_pooling {
                TeamPoolingParams::ElementwiseMax { coordination_cost } => {
                    Box::new(ElementwiseMax { coordination_cost })
                }
                TeamPoolingParams::LeaderPlusAssistants {
                    assist,
                    coordination_cost,
                } => Box::new(LeaderPlusAssistants {
                    assist,
                    coordination_cost,
                }),
            },
            success: match p.success {
                SuccessParams::Sigmoid { steepness } => Box::new(SigmoidSuccess { steepness }),
            },
            quality: match p.quality {
                QualityParams::Margin {
                    steepness,
                    offset,
                    combine,
                } => Box::new(MarginQuality {
                    steepness,
                    offset,
                    combine,
                }),
            },
            learning: match p.learning {
                LearningParams::Diminishing {
                    rate,
                    mastery,
                    failure_factor,
                } => Box::new(DiminishingPractice {
                    rate,
                    mastery,
                    failure_factor,
                }),
            },
            diffusion: match p.diffusion {
                DiffusionParams::AlongSkill { rate, teacher_gain } => {
                    Box::new(AlongSkillDiffusion { rate, teacher_gain })
                }
            },
            capacity: match p.capacity {
                CapacityParams::NormBudget { budget, forgetting } => {
                    Box::new(NormBudget { budget, forgetting })
                }
            },
            social: match p.social {
                SocialParams::BoundedConfidence {
                    rate,
                    confidence,
                    repulsion_distance,
                    repulsion,
                    noise,
                    daily_contacts,
                } => Box::new(BoundedConfidence {
                    rate,
                    confidence,
                    repulsion_distance,
                    repulsion,
                    noise,
                    daily_contacts,
                }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(i: usize) -> CapVec {
        let mut v = [0.0; CAP_DIM];
        v[i] = 1.0;
        v
    }

    #[test]
    fn practice_saturates_at_mastery() {
        let rule = DiminishingPractice {
            rate: 0.1,
            mastery: 1.5,
            failure_factor: 0.5,
        };
        let s = unit(3);
        let mut a = [0.0; CAP_DIM];
        for _ in 0..2000 {
            rule.practice(&mut a, &s, 1.0, true);
        }
        let p = dot(&a, &s);
        assert!(p > 1.45 && p <= 1.5 + 1e-4, "{p}");
    }

    #[test]
    fn diffusion_only_moves_along_skill() {
        let rule = AlongSkillDiffusion {
            rate: 0.5,
            teacher_gain: 0.0,
        };
        let mut app = [0.0; CAP_DIM];
        let mut master = [0.0; CAP_DIM];
        master[1] = 1.0;
        master[2] = 1.0;
        rule.diffuse(&mut app, &master, &unit(1));
        assert!(app[1] > 0.0 && app[2] == 0.0);
    }

    #[test]
    fn complementary_specialists_pool() {
        let pool = ElementwiseMax {
            coordination_cost: 0.0,
        };
        let team = pool.pool(&[unit(0), unit(1)]);
        assert_eq!(team[0], 1.0);
        assert_eq!(team[1], 1.0);
    }

    #[test]
    fn norm_budget_caps_vectors() {
        let rule = NormBudget {
            budget: 1.0,
            forgetting: 0.0,
        };
        let mut a = [1.0; CAP_DIM];
        rule.apply(&mut a);
        assert!((norm(&a) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn close_minds_converge_far_minds_ignore() {
        let m = BoundedConfidence {
            rate: 0.5,
            confidence: 0.3,
            repulsion_distance: 2.0,
            repulsion: 0.0,
            noise: 0.0,
            daily_contacts: 0.0,
        };
        let mut a = [0.5; 8];
        let mut b = [0.5; 8];
        b[mind::FAITH] = 0.7;
        m.interact(&mut a, &mut b, &[0.0; 6]);
        assert!((a[mind::FAITH] - b[mind::FAITH]).abs() < 1e-6);
        let mut c = [0.0; 8];
        let mut d = [1.0; 8];
        m.interact(&mut c, &mut d, &[0.0; 6]);
        assert_eq!(c[mind::FAITH], 0.0);
    }
}
