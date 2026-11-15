//! A steady-state start (implementation plan, Phase 2.1).
//!
//! Towns used to start with some finished goods and empty pipelines, so year 1
//! lived off stock and year 2 paid for it. `start: Steady(..)` in models.ron
//! starts every part of the economy as if it had been running, with amounts
//! worked out from the recipes, the needs and the population, so they stay
//! right when content changes and work for a town of any size:
//!
//! - **Food in the market**: `food_days` of what the residents eat, in the
//!   food that feeds best (bread). What a person eats per day follows the
//!   hunger rule: a whole loaf when hunger passes the threshold, so a little
//!   more than `hunger_per_day`.
//! - **Pipelines**: walking each food's recipe back, its inputs for
//!   `input_days` of the work that uses them (flour, oven wood), and goods
//!   harvested from nature for `raw_days` (grain until the next harvest).
//! - **Households** (with the needs ladder): every person's larder holds the
//!   household food stock target, every home keeps fuel for the planning days,
//!   and everyone wears the clothes they can.
//!
//! The scenario's own stock is kept where it is larger.

use crate::db::Db;
use crate::store::AgentStore;
use crate::world::Towns;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StartParams {
    /// Only the scenario's stock (the old start).
    Scenario,
    /// Start as if the economy had been running.
    Steady {
        /// Days of the residents' food in the market.
        food_days: f32,
        /// Days of work's inputs kept along each food chain.
        input_days: f32,
        /// Days of goods harvested from nature (grain to the next harvest).
        raw_days: f32,
    },
}

impl Default for StartParams {
    fn default() -> Self {
        StartParams::Steady {
            food_days: 25.0,
            input_days: 10.0,
            raw_days: 90.0,
        }
    }
}

/// Food a person eats per day, in food value: they eat when hunger passes the
/// threshold (on average half a day's hunger above it) and a loaf of value
/// `value` brings hunger to `max(0, h - value)`.
pub fn food_per_day(hunger_per_day: f32, eat_threshold: f32, value: f32) -> f32 {
    let at_eating = eat_threshold + 0.5 * hunger_per_day;
    let relief = value.min(at_eating).max(1e-3);
    // One loaf every `relief / hunger_per_day` days.
    value * hunger_per_day / relief
}

/// Stock of each good a town of `pop` people starts with (market and workplaces).
pub fn market_stock(db: &Db, params: &crate::config::ModelParams, pop: f32) -> Vec<f32> {
    let np = db.content.products.len();
    let mut stock = vec![0.0f32; np];
    let StartParams::Steady {
        food_days,
        input_days,
        raw_days,
    } = params.start
    else {
        return stock;
    };
    let Some(&(food, value)) = db.foods.first() else {
        return stock;
    };
    let n = &params.needs;
    let units_per_day = pop * food_per_day(n.hunger_per_day, n.eat_threshold, value) / value;
    stock[food as usize] += units_per_day * food_days;
    add_inputs(
        db,
        food as usize,
        units_per_day,
        input_days,
        raw_days,
        0,
        &mut stock,
    );
    stock
}

/// Adds the inputs of the recipe that makes `good` at `per_day` units a day.
fn add_inputs(
    db: &Db,
    good: usize,
    per_day: f32,
    input_days: f32,
    raw_days: f32,
    depth: u32,
    stock: &mut [f32],
) {
    if depth > 4 || per_day <= 0.0 {
        return;
    }
    let Some(m) = db
        .recipes
        .iter()
        .find(|m| m.outputs.iter().any(|o| o.0 as usize == good))
    else {
        return;
    };
    let out: f32 = m
        .outputs
        .iter()
        .filter(|o| o.0 as usize == good)
        .map(|o| o.1)
        .sum();
    for &(p, q) in &m.inputs {
        let p = p as usize;
        let need = per_day * q / out.max(1e-6);
        let harvested = db
            .recipes
            .iter()
            .any(|r| r.nature.is_some() && r.outputs.iter().any(|o| o.0 as usize == p));
        stock[p] += need * if harvested { raw_days } else { input_days };
        add_inputs(db, p, need, input_days, raw_days, depth + 1, stock);
    }
}

/// Fills households' stocks: larders, home fuel and clothes (with the needs
/// ladder), recording what was seeded in each town's goods ledger.
pub fn households(
    towns: &mut Towns,
    store: &mut AgentStore,
    db: &Db,
    params: &crate::config::ModelParams,
) {
    if !matches!(params.start, StartParams::Steady { .. }) {
        return;
    }
    let crate::consumption::ConsumptionParams::Needs(lp) = &params.market.consumption else {
        return;
    };
    let Some(&(food, value)) = db.foods.first() else {
        return;
    };
    let n = &params.needs;
    let per_day = food_per_day(n.hunger_per_day, n.eat_threshold, value) / value;
    let goods = crate::ladder::goods(db);
    // The warmest wearable and the best fuel per unit.
    let clothes = goods.iter().find(|g| {
        g.serves
            .iter()
            .any(|s| s.by == sim_data::Use::Wear && s.need == sim_data::Need::Warmth)
    });
    let fuel = goods
        .iter()
        .filter_map(|g| {
            g.serves
                .iter()
                .find(|s| s.by == sim_data::Use::Burn && s.need == sim_data::Need::Warmth)
                .map(|s| (g.good, s.amount))
        })
        .min_by(|a, b| {
            let pa = db.base_price[a.0] / a.1.max(1e-3);
            let pb = db.base_price[b.0] / b.1.max(1e-3);
            pa.total_cmp(&pb)
        });
    // Cold enough to need a fire for about half the planning days on average.
    let mean_c = match params.climate {
        crate::env::ClimateParams::Constant { temperature } => temperature,
        crate::env::ClimateParams::Seasonal { mean_c, .. } => mean_c,
    };
    let homes = crate::ladder::Homes::of(store, &goods, lp);
    for i in 0..store.len() {
        if !store.alive[i] {
            continue;
        }
        let t = store.town[i] as usize;
        if lp.larder_days > 0.0 {
            let q = (per_day * lp.larder_days).ceil();
            store.larder[i].push((food, q));
            towns.0[t]
                .flows
                .add(crate::goods::Flow::Seeded, food as usize, q);
        }
        if let Some(c) = clothes {
            store.owned[i].push((c.good as u32, lp.max_clothes));
        }
        if let (Some((f, heat)), true) = (fuel, homes.keeper[i] == i) {
            let size = homes.size_of(i);
            let need = crate::ladder::fire_need(mean_c, 0.0, size, lp) * lp.plan_days;
            let q = (need / heat.max(1e-3)).ceil();
            if q > 0.0 {
                store.owned[i].push((f as u32, q));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_loaf_at_the_threshold_eats_a_little_more_than_hunger() {
        let f = food_per_day(0.12, 0.5, 1.0);
        assert!(f > 0.12 && f < 0.25, "{f}");
        // Small portions are eaten without waste.
        assert!((food_per_day(0.12, 0.5, 0.3) - 0.12).abs() < 1e-6);
    }

    #[test]
    fn a_town_starts_with_its_bread_chain_full() {
        let paths = crate::AssetPaths::discover();
        let (db, params, _) = crate::load(&paths).unwrap();
        let p = |id: &str| db.content.product(id).unwrap() as usize;
        let s = market_stock(&db, &params, 300.0);
        let s1 = market_stock(&db, &params, 1.0);
        let (bread, flour, grain) = (p("bread"), p("flour"), p("grain"));
        // 25 days of bread, flour for 10 days of baking, grain for 90 of milling.
        let per_day = s[bread] / 25.0;
        assert!(per_day > 300.0 * 0.12, "{per_day}");
        assert!((s[flour] - per_day * 0.5 * 10.0).abs() < 1e-2, "{}", s[flour]);
        assert!((s[grain] - per_day * 0.5 * 90.0).abs() < 1e-1, "{}", s[grain]);
        // It scales with the town: an outpost of one gets a three-hundredth.
        assert!((s1[bread] * 300.0 - s[bread]).abs() < 1e-2);
    }
}
