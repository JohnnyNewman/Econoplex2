//! Valuation parameters shared by job choice, pay and migration: how far ahead
//! agents look, the success chance of a typical worker, and how towns post pay
//! for jobs nobody has tried yet.

use serde::{Deserialize, Serialize};

/// `decision.valuation` in models.ron.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ValuationParams {
    /// Days ahead an agent looks (weeks).
    pub horizon_days: f32,
    /// Daily discount factor over the horizon.
    pub discount: f32,
    /// Success chance a job's typical worker has; pay per worker-day counts
    /// failures at about this rate.
    pub typical_success: f32,
    /// Days over which a town's public pay rate drifts toward what a job would pay
    /// at today's prices (0: only realised pay counts, and an untried job shows
    /// none).
    pub posted_pay_days: f32,
}

impl Default for ValuationParams {
    fn default() -> Self {
        ValuationParams {
            horizon_days: 30.0,
            discount: 0.97,
            typical_success: 0.7,
            posted_pay_days: 90.0,
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
