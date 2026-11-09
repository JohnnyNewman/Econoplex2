//! Wage adjustment: how a job's pay moves with the balance of workers and places.
//!
//! Interface: the job's current wage factor, how many took the job today and how
//! many places it had at the start of the day. Output: the new factor. The factor
//! multiplies the town's base wage share (the `wages` lever) for that job, so a job
//! everyone wants pays less and a job nobody takes pays more.

use serde::{Deserialize, Serialize};

pub trait WageAdjustment: Send + Sync {
    fn adjust(&self, factor: f32, takers: f32, places: f32) -> f32;
}

/// `decision.wages` in models.ron.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WageParams {
    /// Pay is the base share for every job.
    Fixed,
    /// Each day the factor moves by `step × (target_fill − takers / places)`,
    /// clamped to `min..max`.
    ExcessDemand {
        step: f32,
        target_fill: f32,
        min: f32,
        max: f32,
    },
}

impl Default for WageParams {
    fn default() -> Self {
        WageParams::Fixed
    }
}

pub fn build(p: &WageParams) -> Box<dyn WageAdjustment> {
    match *p {
        WageParams::Fixed => Box::new(Fixed),
        WageParams::ExcessDemand {
            step,
            target_fill,
            min,
            max,
        } => Box::new(ExcessDemand {
            step,
            target_fill,
            min,
            max,
        }),
    }
}

pub struct Fixed;

impl WageAdjustment for Fixed {
    fn adjust(&self, _factor: f32, _takers: f32, _places: f32) -> f32 {
        1.0
    }
}

pub struct ExcessDemand {
    pub step: f32,
    pub target_fill: f32,
    pub min: f32,
    pub max: f32,
}

impl WageAdjustment for ExcessDemand {
    fn adjust(&self, factor: f32, takers: f32, places: f32) -> f32 {
        if places < 1.0 {
            return factor.clamp(self.min, self.max);
        }
        let fill = (takers / places).clamp(0.0, 1.0);
        (factor + self.step * (self.target_fill - fill)).clamp(self.min, self.max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule() -> ExcessDemand {
        ExcessDemand {
            step: 0.05,
            target_fill: 0.8,
            min: 0.5,
            max: 2.0,
        }
    }

    #[test]
    fn a_full_job_pays_less_and_an_empty_one_more() {
        assert!(rule().adjust(1.0, 10.0, 10.0) < 1.0);
        assert!(rule().adjust(1.0, 1.0, 10.0) > 1.0);
        assert_eq!(rule().adjust(1.0, 8.0, 10.0), 1.0);
    }

    #[test]
    fn pay_stays_in_its_range() {
        let mut f = 1.0;
        for _ in 0..1000 {
            f = rule().adjust(f, 0.0, 10.0);
        }
        assert_eq!(f, 2.0);
        for _ in 0..1000 {
            f = rule().adjust(f, 10.0, 10.0);
        }
        assert_eq!(f, 0.5);
    }

    #[test]
    fn fixed_pay_never_moves() {
        assert_eq!(Fixed.adjust(1.7, 0.0, 10.0), 1.0);
    }
}
