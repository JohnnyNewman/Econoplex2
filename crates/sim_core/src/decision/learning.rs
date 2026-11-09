//! Learning value: what the skill a job teaches is worth, as future income.
//!
//! Interface: the agent's chance of success at the job now and after a day of
//! practice, the job's expected pay per worker-day (for a typical worker), the
//! agent's age, and the typical success chance the pay refers to. Output: the
//! present value of the extra pay that practice unlocks, in money. The utility model
//! adds it to the job's worth, so agents invest in skills they will use.

use serde::{Deserialize, Serialize};

pub struct LearnInputs {
    pub success_now: f32,
    pub success_after: f32,
    pub pay_per_day: f32,
    pub typical_success: f32,
    pub age_years: f32,
}

pub trait LearningValue: Send + Sync {
    fn value(&self, x: &LearnInputs) -> f32;
}

/// `decision.learning` in models.ron.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum LearningValueParams {
    /// Skill gains are not weighed (the job is valued for today's pay only).
    #[default]
    None,
    /// The extra pay from a higher success chance, over `horizon_days` discounted by
    /// `discount` per day, shrinking to nothing as the agent nears `retire_age`.
    Discounted {
        horizon_days: f32,
        discount: f32,
        retire_age: f32,
    },
}

pub fn build(p: &LearningValueParams) -> Box<dyn LearningValue> {
    match *p {
        LearningValueParams::None => Box::new(NoValue),
        LearningValueParams::Discounted {
            horizon_days,
            discount,
            retire_age,
        } => Box::new(Discounted {
            horizon_days,
            discount,
            retire_age,
        }),
    }
}

pub struct NoValue;

impl LearningValue for NoValue {
    fn value(&self, _x: &LearnInputs) -> f32 {
        0.0
    }
}

pub struct Discounted {
    pub horizon_days: f32,
    pub discount: f32,
    pub retire_age: f32,
}

impl Discounted {
    /// Sum of discount factors over the days the agent still has to use the skill.
    fn weight(&self, age_years: f32, days_per_year: f32) -> f32 {
        let left = ((self.retire_age - age_years) * days_per_year).max(0.0);
        let n = self.horizon_days.min(left);
        let b = self.discount.clamp(0.0, 1.0);
        if n <= 0.0 {
            0.0
        } else if (1.0 - b).abs() < 1e-6 {
            n
        } else {
            (1.0 - b.powf(n)) / (1.0 - b)
        }
    }
}

/// Days per year assumed when counting the years an agent has left (the horizon is
/// in days; ages are in years).
const YEAR_DAYS: f32 = 120.0;

impl LearningValue for Discounted {
    fn value(&self, x: &LearnInputs) -> f32 {
        let gain = (x.success_after - x.success_now).max(0.0);
        let extra_per_day = x.pay_per_day.max(0.0) * gain / x.typical_success.max(1e-3);
        extra_per_day * self.weight(x.age_years, YEAR_DAYS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule() -> Discounted {
        Discounted {
            horizon_days: 360.0,
            discount: 0.995,
            retire_age: 60.0,
        }
    }

    fn inputs(age: f32, now: f32, after: f32) -> LearnInputs {
        LearnInputs {
            success_now: now,
            success_after: after,
            pay_per_day: 1.0,
            typical_success: 0.7,
            age_years: age,
        }
    }

    #[test]
    fn a_skill_is_worth_more_to_the_young() {
        let young = rule().value(&inputs(20.0, 0.3, 0.32));
        let old = rule().value(&inputs(58.0, 0.3, 0.32));
        assert!(young > old && old > 0.0);
        assert_eq!(rule().value(&inputs(65.0, 0.3, 0.32)), 0.0);
    }

    #[test]
    fn mastered_work_teaches_nothing_of_value() {
        assert_eq!(rule().value(&inputs(20.0, 0.95, 0.95)), 0.0);
        assert_eq!(NoValue.value(&inputs(20.0, 0.3, 0.5)), 0.0);
    }
}
