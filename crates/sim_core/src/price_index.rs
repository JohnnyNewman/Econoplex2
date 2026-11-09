//! Price level and currency strength, as metrics only (nothing reads them back).
//!
//! **Consumer price index** per town, over the basket of what its residents
//! actually consume (food eaten from the town's stores, tools bought), at each
//! year's average prices:
//! - **Laspeyres** with the first year's quantities: P_t·Q_0 / P_0·Q_0 × 100.
//! - **Chained Fisher**, so the basket can drift: each year the index moves by
//!   √(Laspeyres × Paasche) between this year and last.
//! - **Sub-indices** per price class. Essentials are a Laspeyres over the
//!   essential goods of the consumer basket. Commodities are a Laspeyres over the
//!   commodity inputs the town's trades used. Labour is pay per worker-day against
//!   the first year's.
//! - **Inflation:** the yearly change of the chained index.
//! - **Empire** (a town and the towns paying it tribute): the members' chained
//!   indices weighted by their nominal consumption.
//!
//! **Currency.** All towns use one coin (treasuries and residents hold it, every
//! town's market takes it; a mint adds to the same money), so currency strength is
//! about price levels:
//! - **purchasing power** = 100 / CPI;
//! - **real exchange rate vs the world** (PPP): the world basket's cost at this
//!   town's prices against its cost at world-average prices (above 1: dearer than
//!   the average, so its coin buys less there);
//! - **money stock** (treasury plus residents' wealth), **velocity** (residents'
//!   market spending over the year / money stock) and the **minted share** (coin
//!   struck this year / money stock).

use serde::{Deserialize, Serialize};

/// What one town's market looked like over a year.
#[derive(Debug, Clone, Default)]
pub struct YearPrices {
    /// Average price per product.
    pub price: Vec<f32>,
    /// Quantities residents consumed, per product.
    pub consumed: Vec<f32>,
    /// Quantities trades used as inputs, per product.
    pub inputs: Vec<f32>,
    /// Pay per worker-day.
    pub wage: f32,
}

/// The index state kept between years for one town.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexState {
    pub base_price: Vec<f32>,
    pub base_consumed: Vec<f32>,
    pub base_inputs: Vec<f32>,
    pub base_wage: f32,
    pub last_price: Vec<f32>,
    pub last_consumed: Vec<f32>,
    pub chained: f32,
}

/// A year's index values for one town.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TownIndex {
    pub laspeyres: f32,
    pub fisher: f32,
    /// Yearly change of the chained index (0.03 = 3 %).
    pub inflation: f32,
    pub essentials: f32,
    pub commodities: f32,
    pub labour: f32,
}

fn dot(p: &[f32], q: &[f32]) -> f64 {
    p.iter().zip(q).map(|(a, b)| *a as f64 * *b as f64).sum()
}

fn masked(q: &[f32], keep: &[bool]) -> Vec<f32> {
    q.iter()
        .zip(keep)
        .map(|(q, &k)| if k { *q } else { 0.0 })
        .collect()
}

/// Laspeyres index (× 100) of prices `p` against `p0` with quantities `q0`; 100
/// when the basket is empty.
pub fn laspeyres(p: &[f32], p0: &[f32], q0: &[f32]) -> f32 {
    let base = dot(p0, q0);
    if base <= 0.0 {
        100.0
    } else {
        (100.0 * dot(p, q0) / base) as f32
    }
}

/// Fisher price relative from (p0, q0) to (p1, q1); 1 when either basket is empty.
pub fn fisher(p0: &[f32], q0: &[f32], p1: &[f32], q1: &[f32]) -> f32 {
    let (a, b) = (dot(p0, q0), dot(p0, q1));
    if a <= 0.0 || b <= 0.0 {
        return 1.0;
    }
    let l = dot(p1, q0) / a;
    let pa = dot(p1, q1) / b;
    (l * pa).sqrt() as f32
}

impl IndexState {
    /// Index values for this year; the first year sets the base (all 100).
    /// `essential[p]`: whether product p is an essential (else a commodity).
    pub fn update(&mut self, y: &YearPrices, essential: &[bool]) -> TownIndex {
        if self.base_price.is_empty() {
            self.base_price = y.price.clone();
            self.base_consumed = y.consumed.clone();
            self.base_inputs = y.inputs.clone();
            self.base_wage = y.wage;
            self.last_price = y.price.clone();
            self.last_consumed = y.consumed.clone();
            self.chained = 100.0;
            return TownIndex {
                laspeyres: 100.0,
                fisher: 100.0,
                inflation: 0.0,
                essentials: 100.0,
                commodities: 100.0,
                labour: 100.0,
            };
        }
        let before = self.chained;
        self.chained *= fisher(&self.last_price, &self.last_consumed, &y.price, &y.consumed);
        let commodity: Vec<bool> = essential.iter().map(|e| !e).collect();
        let ix = TownIndex {
            laspeyres: laspeyres(&y.price, &self.base_price, &self.base_consumed),
            fisher: self.chained,
            inflation: self.chained / before.max(1e-9) - 1.0,
            essentials: laspeyres(
                &y.price,
                &self.base_price,
                &masked(&self.base_consumed, essential),
            ),
            commodities: laspeyres(
                &y.price,
                &self.base_price,
                &masked(&self.base_inputs, &commodity),
            ),
            labour: if self.base_wage > 0.0 {
                100.0 * y.wage / self.base_wage
            } else {
                100.0
            },
        };
        self.last_price = y.price.clone();
        self.last_consumed = y.consumed.clone();
        ix
    }
}

/// World-average prices: each good's price averaged over towns, weighted by how
/// much each town consumed of it (equal weights if nobody did).
pub fn world_prices(towns: &[YearPrices]) -> (Vec<f32>, Vec<f32>) {
    let np = towns.first().map_or(0, |t| t.price.len());
    let mut price = vec![0.0f32; np];
    let mut q = vec![0.0f32; np];
    for p in 0..np {
        let total: f32 = towns.iter().map(|t| t.consumed[p]).sum();
        price[p] = if total > 0.0 {
            towns
                .iter()
                .map(|t| t.price[p] * t.consumed[p])
                .sum::<f32>()
                / total
        } else {
            towns.iter().map(|t| t.price[p]).sum::<f32>() / towns.len().max(1) as f32
        };
        q[p] = total;
    }
    (price, q)
}

/// Real exchange rate of a town against the world (PPP): the world basket's cost
/// at the town's prices over its cost at world prices.
pub fn real_exchange_rate(town_price: &[f32], world_price: &[f32], world_q: &[f32]) -> f32 {
    let w = dot(world_price, world_q);
    if w <= 0.0 {
        1.0
    } else {
        (dot(town_price, world_q) / w) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_track_a_uniform_price_rise() {
        let mut s = IndexState::default();
        let y0 = YearPrices {
            price: vec![1.0, 2.0],
            consumed: vec![10.0, 5.0],
            inputs: vec![0.0, 4.0],
            wage: 2.0,
        };
        let first = s.update(&y0, &[true, false]);
        assert_eq!(first.laspeyres, 100.0);
        let y1 = YearPrices {
            price: vec![1.1, 2.2],
            consumed: vec![8.0, 6.0],
            inputs: vec![0.0, 4.0],
            wage: 2.4,
        };
        let ix = s.update(&y1, &[true, false]);
        assert!((ix.laspeyres - 110.0).abs() < 1e-3);
        assert!((ix.fisher - 110.0).abs() < 1e-3);
        assert!((ix.inflation - 0.10).abs() < 1e-4);
        assert!((ix.essentials - 110.0).abs() < 1e-3);
        assert!((ix.commodities - 110.0).abs() < 1e-3);
        assert!((ix.labour - 120.0).abs() < 1e-3);
    }

    #[test]
    fn fisher_lies_between_laspeyres_and_paasche() {
        let (p0, q0) = (vec![1.0, 1.0], vec![10.0, 10.0]);
        // Good 0 gets dear; people switch to good 1.
        let (p1, q1) = (vec![2.0, 1.0], vec![2.0, 18.0]);
        let l = dot(&p1, &q0) / dot(&p0, &q0);
        let pa = dot(&p1, &q1) / dot(&p0, &q1);
        let f = fisher(&p0, &q0, &p1, &q1) as f64;
        assert!(f <= l + 1e-9 && f >= pa - 1e-9);
    }

    #[test]
    fn a_dear_town_has_a_real_exchange_rate_above_one() {
        let a = YearPrices {
            price: vec![1.0, 1.0],
            consumed: vec![5.0, 5.0],
            ..Default::default()
        };
        let b = YearPrices {
            price: vec![2.0, 2.0],
            consumed: vec![5.0, 5.0],
            ..Default::default()
        };
        let (wp, wq) = world_prices(&[a.clone(), b.clone()]);
        assert!(real_exchange_rate(&b.price, &wp, &wq) > 1.0);
        assert!(real_exchange_rate(&a.price, &wp, &wq) < 1.0);
    }
}
