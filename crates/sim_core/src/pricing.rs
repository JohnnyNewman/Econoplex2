//! Price dynamics: how a good's price moves toward what its market wants.
//!
//! Each day the market sees a good's **pressure**, -1..1 (stock below its target
//! is positive; see `market.pricing` for the target). How the price responds is
//! set by the good's **price class** (from the content, `price_class`, or its
//! category), with per-class parameters in `market.price_classes` and per-good
//! overrides in `market.price_overrides`:
//!
//! - **Essential** goods (food, fuel, clothing): moderate smoothing and a tight
//!   daily cap, so subsistence prices don't spike.
//! - **Commodities** (ore, metals, wood, wool, trade goods): faster and more
//!   flexible; trade between towns evens them out.
//! - **Labour** (each job's wage factor): sticky, especially downward.
//!
//! The response itself is a swappable [`Inertia`] model:
//! - `Partial { speed }`: the log price moves `speed` × pressure a day (the
//!   original rule, with speed `price_adjust`);
//! - `Sticky { up, down }`: different speeds up and down;
//! - `Smoothed { speed, memory_days }`: like `Partial` on a pressure averaged over
//!   `memory_days`, so one odd day barely moves the price.
//!
//! Every class also caps the daily change (`max_daily`, as a share of the price)
//! and bounds the price to `floor`..`ceiling` × base price.

use serde::{Deserialize, Serialize};
pub use sim_data::PriceClass;

/// How a price responds to pressure.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Inertia {
    Partial { speed: f32 },
    Sticky { up: f32, down: f32 },
    Smoothed { speed: f32, memory_days: f32 },
}

impl Inertia {
    /// Wanted change of the log price today, from today's pressure and the
    /// smoothed pressure so far (which this updates).
    pub fn wanted(&self, pressure: f32, smoothed: &mut f32) -> f32 {
        match *self {
            Inertia::Partial { speed } => speed * pressure,
            Inertia::Sticky { up, down } => {
                if pressure >= 0.0 {
                    up * pressure
                } else {
                    down * pressure
                }
            }
            Inertia::Smoothed { speed, memory_days } => {
                *smoothed += (pressure - *smoothed) / memory_days.max(1.0);
                speed * *smoothed
            }
        }
    }
}

/// One price class's dynamics.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ClassParams {
    pub inertia: Inertia,
    /// Largest daily change, as a share of the price (e.g. 0.03 = 3 %).
    pub max_daily: f32,
    /// Bounds, as factors of the base price.
    pub floor: f32,
    pub ceiling: f32,
}

impl ClassParams {
    /// Today's price from yesterday's, the pressure and the smoothed pressure.
    pub fn step(&self, price: f32, base: f32, pressure: f32, smoothed: &mut f32) -> f32 {
        let m = self.max_daily.max(0.0);
        let d = self.inertia.wanted(pressure, smoothed).clamp(-m, m);
        (price * d.exp()).clamp(base * self.floor, base * self.ceiling.max(self.floor))
    }
}

/// `market.price_classes` in models.ron. Missing (the legacy rule): every good
/// moves by `price_adjust` × pressure within `min_price_factor`..`max_price_factor`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceClasses {
    pub essential: ClassParams,
    pub commodity: ClassParams,
    pub labour: ClassParams,
}

impl PriceClasses {
    pub fn get(&self, c: PriceClass) -> &ClassParams {
        match c {
            PriceClass::Essential => &self.essential,
            PriceClass::Commodity => &self.commodity,
            PriceClass::Labour => &self.labour,
        }
    }
}

/// Mean absolute daily change of the log price, and its 95th percentile, from a
/// histogram of daily changes (bins of `VOL_BIN`).
pub const VOL_BINS: usize = 100;
pub const VOL_BIN: f32 = 0.002;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Volatility {
    pub sum: f64,
    pub n: u64,
    pub hist: Vec<u64>,
}

impl Volatility {
    pub fn add(&mut self, abs_dlog: f32) {
        if self.hist.len() != VOL_BINS {
            self.hist = vec![0; VOL_BINS];
        }
        self.sum += abs_dlog as f64;
        self.n += 1;
        let k = ((abs_dlog / VOL_BIN) as usize).min(VOL_BINS - 1);
        self.hist[k] += 1;
    }

    pub fn mean(&self) -> f32 {
        if self.n == 0 {
            0.0
        } else {
            (self.sum / self.n as f64) as f32
        }
    }

    pub fn p95(&self) -> f32 {
        let need = (self.n as f64 * 0.95).ceil() as u64;
        let mut seen = 0;
        for (k, &c) in self.hist.iter().enumerate() {
            seen += c;
            if seen >= need && need > 0 {
                return (k + 1) as f32 * VOL_BIN;
            }
        }
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(inertia: Inertia, max_daily: f32) -> ClassParams {
        ClassParams {
            inertia,
            max_daily,
            floor: 0.2,
            ceiling: 5.0,
        }
    }

    #[test]
    fn partial_adjustment_follows_pressure_within_its_daily_cap() {
        let c = class(Inertia::Partial { speed: 0.05 }, 1.0);
        let mut s = 0.0;
        assert!(c.step(10.0, 10.0, 1.0, &mut s) > 10.0);
        assert!(c.step(10.0, 10.0, -1.0, &mut s) < 10.0);
        let tight = class(Inertia::Partial { speed: 0.5 }, 0.02);
        let p = tight.step(10.0, 10.0, 1.0, &mut s);
        assert!((p / 10.0).ln() <= 0.02 + 1e-6);
        // Bounds hold.
        assert!(c.step(49.9, 10.0, 1.0, &mut s) <= 50.0);
        assert!(c.step(2.01, 10.0, -1.0, &mut s) >= 2.0);
    }

    #[test]
    fn sticky_prices_fall_slower_than_they_rise() {
        let c = class(
            Inertia::Sticky {
                up: 0.05,
                down: 0.01,
            },
            1.0,
        );
        let mut s = 0.0;
        let up = c.step(10.0, 10.0, 1.0, &mut s) / 10.0;
        let down = 10.0 / c.step(10.0, 10.0, -1.0, &mut s);
        assert!(up.ln() > 4.0 * down.ln());
    }

    #[test]
    fn smoothed_prices_shrug_off_one_odd_day() {
        let smooth = class(
            Inertia::Smoothed {
                speed: 0.05,
                memory_days: 10.0,
            },
            1.0,
        );
        let plain = class(Inertia::Partial { speed: 0.05 }, 1.0);
        let (mut a, mut b) = (0.0, 0.0);
        let one_day_smooth = smooth.step(10.0, 10.0, 1.0, &mut a);
        let one_day_plain = plain.step(10.0, 10.0, 1.0, &mut b);
        assert!(one_day_smooth < one_day_plain);
        // A lasting pressure still moves it.
        let mut p = 10.0;
        for _ in 0..100 {
            p = smooth.step(p, 10.0, 1.0, &mut a);
        }
        assert!(p > 30.0);
    }

    #[test]
    fn volatility_reports_mean_and_tail() {
        let mut v = Volatility::default();
        for _ in 0..95 {
            v.add(0.001);
        }
        for _ in 0..5 {
            v.add(0.05);
        }
        assert!((v.mean() - (0.095 + 0.25) / 100.0).abs() < 1e-5);
        assert!(v.p95() <= 0.004);
        v.add(0.05);
        assert!(v.p95() > 0.04);
    }
}
