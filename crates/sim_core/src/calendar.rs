//! The calendar: one simulated day per month.
//!
//! A year is `time.months_per_year` simulated days (12). Each simulated day is
//! one representative day that stands for a whole month of
//! `time.days_per_month` days (10): the economy runs once on it, and whatever
//! the model writes as a daily rate comes out per month through this one
//! helper, so yearly totals do not depend on how finely time is cut.
//!
//! Rules (docs/design/time-and-weather.md has the full list):
//! - a daily flow (food eaten, goods made, wages, rent, regrowth) is
//!   [`Calendar::flow`]: the day's amount times the days in the month;
//! - a daily share (price moves, learning, forgetting, smoothing toward a
//!   target, spoilage per day) compounds: [`Calendar::share`], [`Calendar::keep`],
//!   [`Calendar::smoothing`];
//! - a yearly rate divides by [`Calendar::per_year`] simulated days;
//! - counts of events in config (intervals, cooldowns) are in months, that is
//!   simulated days, and are compared with `SimClock::tick` directly;
//! - time constants in config written in days (horizons, memories, cover and
//!   reserve days) stay in days and go through [`Calendar::months`] or
//!   [`Calendar::smoothing`].

use serde::{Deserialize, Serialize};

/// How long a simulated day is. Built from `time` in models.ron.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Calendar {
    /// Days a simulated day stands for (`time.days_per_month`).
    pub days: f32,
    /// Simulated days in a year (`time.months_per_year`).
    pub per_year: u32,
}

impl Default for Calendar {
    fn default() -> Self {
        Calendar {
            days: 10.0,
            per_year: 12,
        }
    }
}

impl Calendar {
    pub fn new(time: &crate::env::TimeParams) -> Self {
        Calendar {
            days: time.days_per_month.max(1) as f32,
            per_year: time.months_per_year.max(1),
        }
    }

    /// Days a simulated day stands for.
    pub fn days(&self) -> f32 {
        self.days
    }

    /// Whole days a simulated day stands for (for day counters).
    pub fn whole_days(&self) -> u32 {
        self.days.round().max(1.0) as u32
    }

    /// Simulated days in a year.
    pub fn per_year(&self) -> u32 {
        self.per_year
    }

    /// Days in a year.
    pub fn days_per_year(&self) -> f32 {
        self.days * self.per_year as f32
    }

    /// A daily flow over one simulated day.
    pub fn flow(&self, per_day: f32) -> f32 {
        per_day * self.days
    }

    /// The share a daily share `per_day` (0..1) adds up to over a simulated
    /// day: `1 - (1 - per_day)^days`.
    pub fn share(&self, per_day: f32) -> f32 {
        1.0 - self.keep(1.0 - per_day.clamp(0.0, 1.0))
    }

    /// What is left of a quantity that keeps `keep_per_day` of itself each day.
    pub fn keep(&self, keep_per_day: f32) -> f32 {
        keep_per_day.max(0.0).powf(self.days)
    }

    /// Weight of one simulated day in a moving average with a time constant
    /// of `memory_days` days.
    pub fn smoothing(&self, memory_days: f32) -> f32 {
        self.share(1.0 / memory_days.max(1.0))
    }

    /// A kick `per_day` that a state gets each day while it relaxes at the
    /// daily share `rate`, over a simulated day: scaled so that the state's
    /// balance between kicks and relaxation stays where it is.
    pub fn kick(&self, per_day: f32, rate: f32) -> f32 {
        if rate <= 0.0 {
            return self.flow(per_day);
        }
        per_day * self.share(rate) / rate.min(1.0)
    }

    /// Simulated days (months) in `days` days, unrounded.
    pub fn months(&self, days: f32) -> f32 {
        days / self.days
    }
}

/// Month names for a twelve-month year.
pub const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The date of a tick as the clients show it, "y3 Mar": the year from 0 and
/// the month (a month number when a year does not have twelve).
pub fn date(tick: u64, per_year: u32) -> String {
    let per_year = per_year.max(1) as u64;
    format!(
        "y{} {}",
        tick / per_year,
        month_name((tick % per_year) as u32, per_year as u32)
    )
}

/// The name of month `month` (from 0) in a year of `per_year` months.
pub fn month_name(month: u32, per_year: u32) -> String {
    match MONTHS.get(month as usize) {
        Some(m) if per_year == 12 => m.to_string(),
        _ => format!("m{}", month + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_month_compounds_ten_days() {
        let c = Calendar::default();
        assert!((c.flow(0.12) - 1.2).abs() < 1e-6);
        assert!((c.share(0.05) - (1.0 - 0.95f32.powi(10))).abs() < 1e-6);
        assert!((c.keep(0.9) - 0.9f32.powi(10)).abs() < 1e-6);
        assert_eq!(c.days_per_year(), 120.0);
        assert_eq!(c.months(30.0), 3.0);
        // A kick against relaxation keeps the balance: kick / share = per_day / rate.
        let k = c.kick(0.03, 0.05);
        assert!((k / c.share(0.05) - 0.03 / 0.05).abs() < 1e-5);
        // One day per month is the old daily clock.
        let d = Calendar {
            days: 1.0,
            per_year: 120,
        };
        assert!((d.share(0.05) - 0.05).abs() < 1e-6 && d.flow(2.0) == 2.0);
        assert_eq!(date(14, 12), "y1 Mar");
        assert_eq!(date(5, 8), "y0 m6");
    }
}
