//! Food as the ruler's challenge (economy redesign, step 7).
//!
//! `market.food_rules` in models.ron:
//!
//! - `Town` (the default) keeps two town-wide rules that no single person
//!   would keep: seed grain and flour for a few days of bread are held out of
//!   reach of eating and of other trades (`food_reserve_days`), and people are
//!   drawn to farm work by the town's food scarcity, not only by its pay.
//! - `Ruler` drops both: people eat what the market holds and take the jobs
//!   that pay. Bridging the hungry gap is then the ruler's job, with the
//!   granary below, the seed reserve as an order (`reserve TOWN on`), and the
//!   levers they already had (food subsidies, imports, new fields).
//!
//! **The granary** is food the town keeps out of the market. Order
//! `granary TOWN DAYS` sets how many days of its people's food it aims to
//! hold. While the market holds more than its food target, the granary takes
//! the surplus (grain first: it keeps); when the market falls below half its
//! food target, the granary releases food into it. Grain keeps better in a
//! granary (`granary_spoilage` of the market's rate). Moving food between the
//! market and the granary costs nothing: both belong to the town.

use crate::db::Db;
use crate::world::{Params, Towns};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FoodRules {
    #[default]
    Town,
    Ruler,
}

impl FoodRules {
    /// Whether the town-wide food rules (seed reserve, scarcity pull) apply.
    pub fn town(self) -> bool {
        self == FoodRules::Town
    }
}

/// Whether `town` holds the inputs of its food chains (seed grain, flour)
/// back from eating and other trades: always under `Town` rules, by the
/// ruler's order (`reserve TOWN on`) under `Ruler` rules.
pub fn keeps_reserve(town: &crate::world::Town, params: &Params) -> bool {
    params.market.food_rules.town() || town.policy.food_reserve
}

/// Food value held in the granary.
pub fn held(town: &crate::world::Town, db: &Db) -> f32 {
    db.foods
        .iter()
        .map(|&(f, v)| town.granary.get(f as usize).copied().unwrap_or(0.0) * v)
        .sum()
}

/// Daily: the granary takes the market's food surplus up to its target, or
/// releases food into a hungry market; and its store slowly spoils.
pub fn tend(towns: &mut Towns, db: &Db, params: &Params, days_per_year: f32) {
    let mp = &params.market;
    let np = db.content.products.len();
    // Grain first (lowest food value per unit keeps longest), then the rest.
    let mut order: Vec<(u32, f32)> = db.foods.clone();
    order.reverse();
    for town in towns.0.iter_mut() {
        if town.granary.is_empty() && town.policy.granary_days <= 0.0 {
            continue;
        }
        town.granary.resize(np, 0.0);
        let pop = town.residents.len() as f32;
        let target = town.policy.granary_days.max(0.0) * pop * params.needs.hunger_per_day;
        let market_target = mp.food_target_per_capita * pop;
        let market = town.food_stock(db);
        let have = held(town, db);
        if have < target && market > market_target {
            let mut want = (target - have).min(market - market_target);
            for &(f, v) in &order {
                if want <= 0.0 {
                    break;
                }
                let f = f as usize;
                let q = town.stock[f].min(want / v.max(1e-6)).max(0.0);
                town.stock[f] -= q;
                town.granary[f] += q;
                want -= q * v;
            }
        } else if market < 0.5 * market_target && have > 0.0 {
            let mut want = 0.5 * market_target - market;
            for &(f, v) in db.foods.iter().rev() {
                if want <= 0.0 {
                    break;
                }
                let f = f as usize;
                let q = town.granary[f].min(want / v.max(1e-6)).max(0.0);
                town.granary[f] -= q;
                let old = town.stock[f];
                town.stock[f] = old + q;
                want -= q * v;
            }
        }
        let rate = mp.food_spoilage / days_per_year * mp.granary_spoilage;
        for &(f, _) in &db.foods {
            let f = f as usize;
            let rotted = town.granary[f] * rate;
            town.granary[f] -= rotted;
            town.flows.add(crate::goods::Flow::Spoiled, f, rotted);
        }
    }
}
