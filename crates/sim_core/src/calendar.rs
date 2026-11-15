//! The calendar: one simulated day per month, cut into economy steps.
//!
//! A year is `time.months_per_year` simulated days (12). Each simulated day is
//! one representative day that stands for a whole month of
//! `time.days_per_month` days (10). The day is cut into `time.steps_per_day`
//! economy steps; each step is one slice of the representative day's 24 hours
//! and stands for that slice of every day of the month. The whole economy
//! (work, production, market, eating, decisions) runs every step, and whatever
//! the model writes as a daily rate comes out per step through this one
//! helper, so yearly totals do not depend on how finely time is cut.
//!
//! Rules (docs/design/time-and-weather.md has the full list):
//! - a daily flow (food eaten, wages, rent, regrowth) is [`Calendar::flow`]:
//!   the day's amount times the days a step stands for;
//! - a daily share (price moves, learning, forgetting, smoothing toward a
//!   target, spoilage per day) compounds: [`Calendar::share`],
//!   [`Calendar::keep`], [`Calendar::smoothing`];
//! - a yearly rate divides by [`Calendar::per_year`] steps;
//! - counts of events in config (intervals, cooldowns) are in months, that is
//!   simulated days: they happen on the first step of a simulated day and are
//!   compared with `SimClock::tick`;
//! - time constants in config written in days (horizons, memories, cover and
//!   reserve days) stay in days and go through [`Calendar::steps`] or
//!   [`Calendar::smoothing`];
//! - work goes on in the step's share of the working hours
//!   ([`Calendar::work_share`]), so a day's work is done in the day's light.

use serde::{Deserialize, Serialize};

/// How long an economy step is. Built from `time` in models.ron.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Calendar {
    /// Days of time one step stands for: `days_per_month / steps_per_day`.
    pub days: f32,
    /// Economy steps in a year: `months_per_year * steps_per_day`.
    pub per_year: u32,
    /// Economy steps in a simulated day (`time.steps_per_day`).
    pub steps: u32,
}

impl Default for Calendar {
    fn default() -> Self {
        Calendar::of(10, 12, 300)
    }
}

impl Calendar {
    /// A month of `days_per_month` days, `months_per_year` of them, each
    /// simulated day cut into `steps_per_day` steps.
    pub fn of(days_per_month: u32, months_per_year: u32, steps_per_day: u32) -> Self {
        let steps = steps_per_day.max(1);
        Calendar {
            days: days_per_month.max(1) as f32 / steps as f32,
            per_year: months_per_year.max(1) * steps,
            steps,
        }
    }

    pub fn new(time: &crate::env::TimeParams) -> Self {
        Calendar::of(
            time.days_per_month,
            time.months_per_year,
            time.steps_per_day,
        )
    }

    /// Days of time a step stands for.
    pub fn days(&self) -> f32 {
        self.days
    }

    /// Days a simulated day (a month) stands for.
    pub fn month_days(&self) -> f32 {
        self.days * self.steps as f32
    }

    /// Steps in a year.
    pub fn per_year(&self) -> u32 {
        self.per_year
    }

    /// Simulated days (months) in a year.
    pub fn months_per_year(&self) -> u32 {
        (self.per_year / self.steps.max(1)).max(1)
    }

    /// Days in a year.
    pub fn days_per_year(&self) -> f32 {
        self.days * self.per_year as f32
    }

    /// Hours of the representative day one step covers.
    pub fn step_hours(&self) -> f32 {
        24.0 / self.steps.max(1) as f32
    }

    /// A daily flow over one step.
    pub fn flow(&self, per_day: f32) -> f32 {
        per_day * self.days
    }

    /// The share a daily share `per_day` (0..1) adds up to over a step:
    /// `1 - (1 - per_day)^days`.
    pub fn share(&self, per_day: f32) -> f32 {
        1.0 - self.keep(1.0 - per_day.clamp(0.0, 1.0))
    }

    /// What is left of a quantity that keeps `keep_per_day` of itself each day.
    pub fn keep(&self, keep_per_day: f32) -> f32 {
        keep_per_day.max(0.0).powf(self.days)
    }

    /// Weight of one step in a moving average with a time constant of
    /// `memory_days` days.
    pub fn smoothing(&self, memory_days: f32) -> f32 {
        self.share(1.0 / memory_days.max(1.0))
    }

    /// A kick `per_day` that a state gets each day while it relaxes at the
    /// daily share `rate`, over a step: scaled so that the state's balance
    /// between kicks and relaxation stays where it is.
    pub fn kick(&self, per_day: f32, rate: f32) -> f32 {
        if rate <= 0.0 {
            return self.flow(per_day);
        }
        per_day * self.share(rate) / rate.min(1.0)
    }

    /// Days of work a worker does in a step inside the working hours (at
    /// least one: a batch or a job place is never less than a day's work).
    pub fn work_step_days(&self, equinox_hours: f32) -> f32 {
        (self.month_days() * self.step_hours() / equinox_hours.max(1e-3)).max(1.0)
    }

    /// The calendar of `k` steps taken together (for slow processes that
    /// run once every `k` steps).
    pub fn every(&self, k: u32) -> Calendar {
        let k = k.max(1);
        Calendar {
            days: self.days * k as f32,
            per_year: (self.per_year / k).max(1),
            steps: (self.steps / k).max(1),
        }
    }

    /// Steps in `days` days, unrounded.
    pub fn steps(&self, days: f32) -> f32 {
        days / self.days
    }

    /// The share of a day's work done in step `step` (0 .. steps) of the
    /// representative day, for work hours from `start` to `end` (hours, 0..24):
    /// the part of the step inside them over `equinox_hours`, the length of a
    /// full working day. Summed over a day's steps it is `(end - start) /
    /// equinox_hours`; times [`Calendar::month_days`] it is the work a step
    /// stands for, in days.
    pub fn work_share(&self, step: u32, start: f32, end: f32, equinox_hours: f32) -> f32 {
        let h = self.step_hours();
        let (a, b) = (step as f32 * h, (step + 1) as f32 * h);
        let inside = (b.min(end) - a.max(start)).max(0.0);
        inside / equinox_hours.max(1e-3)
    }
}

/// A game speed as people read it: `5 min/day`, `12 s/day`, `30 days/s`
/// (`days_per_sec` simulated days a real second).
pub fn speed_label(days_per_sec: f32) -> String {
    if days_per_sec >= 1.0 {
        format!("{days_per_sec:.0} days/s")
    } else {
        let secs = 1.0 / days_per_sec.max(1e-6);
        if secs >= 60.0 {
            format!("{:.0} min/day", secs / 60.0)
        } else {
            format!("{secs:.0} s/day")
        }
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
    fn a_step_compounds_its_days() {
        // Ten steps a simulated day of ten days: one day a step.
        let d = Calendar::of(10, 12, 10);
        assert!((d.share(0.05) - 0.05).abs() < 1e-6 && d.flow(2.0) == 2.0);
        assert_eq!(d.per_year(), 120);
        assert_eq!(d.months_per_year(), 12);
        // Three hundred steps: a thirtieth of a day each.
        let c = Calendar::default();
        assert!((c.flow(0.12) * 300.0 - 1.2).abs() < 1e-5);
        assert!((c.share(0.05) - (1.0 - 0.95f32.powf(1.0 / 30.0))).abs() < 1e-6);
        assert!((c.keep(0.9).powi(300) - 0.9f32.powi(10)).abs() < 1e-4);
        assert!((c.days_per_year() - 120.0).abs() < 1e-3);
        assert!((c.month_days() - 10.0).abs() < 1e-5);
        assert!((c.steps(1.0) - 30.0).abs() < 1e-3);
        // A kick against relaxation keeps the balance: kick / share = per_day / rate.
        let k = c.kick(0.03, 0.05);
        assert!((k / c.share(0.05) - 0.03 / 0.05).abs() < 1e-4);
        assert_eq!(date(14, 12), "y1 Mar");
        assert_eq!(date(5, 8), "y0 m6");
    }

    #[test]
    fn a_day_of_steps_does_a_day_of_work() {
        for steps in [10, 24, 60, 300] {
            let c = Calendar::of(10, 12, steps);
            let indoor: f32 = (0..steps).map(|s| c.work_share(s, 6.0, 18.0, 12.0)).sum();
            assert!((indoor - 1.0).abs() < 1e-4, "{steps}: {indoor}");
            let winter: f32 = (0..steps).map(|s| c.work_share(s, 8.0, 16.0, 12.0)).sum();
            assert!((winter - 8.0 / 12.0).abs() < 1e-4, "{steps}: {winter}");
            // Nobody works at midnight.
            assert_eq!(c.work_share(0, 6.0, 18.0, 12.0), 0.0);
        }
    }
}
