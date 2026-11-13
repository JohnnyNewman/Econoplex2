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
//! **Stockpiles** (order `stockpile TOWN GOOD AMOUNT`) are the general form:
//! any good the town keeps in its stores against a siege or bad times. A
//! stockpile raises the market's target stock by what the store still lacks
//! (so prices and pay draw production toward it); the store fills from the
//! market's surplus and gives back when the market falls below half its
//! target. For tools and weapons the store is the armory: the target is how
//! many it keeps unissued. Targets follow the season: each town learns, month
//! by month, how much of each good it makes and uses, and a target is in full
//! in the month before the deepest shortfall (grain after the harvest,
//! firewood before the winter), down to a quarter in the leanest month.
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

/// Daily, for each good the town stockpiles (`stockpile TOWN GOOD AMOUNT`): its
/// store takes the market's surplus over the market's own target, up to the
/// stockpile, and gives back to a market that has fallen below half its
/// target. Tools and weapons stay in the armory (it is their store).
fn tend_stockpiles(towns: &mut Towns, db: &Db, params: &Params) {
    let np = db.content.products.len();
    for town in towns.0.iter_mut() {
        if town.policy.stockpile.is_empty() {
            continue;
        }
        town.granary.resize(np, 0.0);
        for k in 0..town.policy.stockpile.len() {
            let p = town.policy.stockpile[k].0 as usize;
            let target = town.stockpile_target(p);
            if db.category(p as u32).is_item() {
                continue;
            }
            // The market's own target, without what the stockpile adds to it.
            let own = {
                let saved = std::mem::take(&mut town.policy.stockpile);
                let t = crate::systems::decide::target_stock(db, town, params, p);
                town.policy.stockpile = saved;
                t
            };
            let held = town.granary[p];
            if held < target && town.stock[p] > own {
                let q = (target - held).min(town.stock[p] - own);
                town.stock[p] -= q;
                town.granary[p] += q;
            } else if town.stock[p] < 0.5 * own && held > 0.0 {
                let q = held.min(0.5 * own - town.stock[p]);
                town.granary[p] -= q;
                town.stock[p] += q;
            }
        }
    }
}

/// Months in the seasonal profile.
const MONTHS: usize = 12;

/// What a stockpile keeps in its leanest month, as a share of its target (a
/// reserve against a siege or a failed harvest).
pub const SEASON_FLOOR: f32 = 0.25;

/// Learns, for each good, how much the town makes minus uses per day in each
/// month of the year (from the goods ledger; averaged over about two years).
fn learn_profile(town: &mut crate::world::Town, np: usize, month: usize) {
    town.flow_profile.resize(np * MONTHS, 0.0);
    town.flow_seen.resize(np, 0.0);
    let first = town.flow_seen_day == 0;
    town.flow_seen_day += 1;
    for g in 0..np {
        let net = town.flows.net(g) - town.flows.get(crate::goods::Flow::Seeded, g);
        let delta = (net - town.flow_seen[g]) as f32;
        town.flow_seen[g] = net;
        if first {
            continue;
        }
        let v = &mut town.flow_profile[g * MONTHS + month];
        *v += PROFILE_RATE * (delta - *v);
    }
}

/// How fast a month's profile follows what the town did (per day of that month).
const PROFILE_RATE: f32 = 0.05;

/// Share of each stockpile target that applies this month. A store is there to
/// carry the town through the months when it uses more than it makes: grain
/// after the harvest, firewood before the winter. For each month, the store it
/// needs is the deepest shortfall the coming months add up to (from the
/// learned profile); the target scales with that, from `SEASON_FLOOR` in the
/// month that needs least to 1 in the month that needs most. Goods made and
/// used evenly keep their whole target.
pub fn season_scale(town: &crate::world::Town, np: usize, month: usize) -> Vec<f32> {
    let mut v = vec![1.0; np];
    if town.flow_profile.len() < np * MONTHS {
        return v;
    }
    for (g, out) in v.iter_mut().enumerate() {
        let prof = &town.flow_profile[g * MONTHS..(g + 1) * MONTHS];
        let need = |m: usize| {
            let (mut balance, mut deepest) = (0.0f32, 0.0f32);
            for k in 0..MONTHS {
                balance += prof[(m + k) % MONTHS];
                deepest = deepest.max(-balance);
            }
            deepest
        };
        let needs: Vec<f32> = (0..MONTHS).map(need).collect();
        let most = needs.iter().copied().fold(0.0f32, f32::max);
        if most > 1e-3 {
            *out = SEASON_FLOOR + (1.0 - SEASON_FLOOR) * needs[month] / most;
        }
    }
    v
}

/// Daily: the granary takes the market's food surplus up to its target, or
/// releases food into a hungry market; and its store slowly spoils.
pub fn tend(towns: &mut Towns, db: &Db, params: &Params, days_per_year: f32, season: f32) {
    let np = db.content.products.len();
    let month =
        ((season - params.time.year_starts_at).rem_euclid(1.0) * MONTHS as f32) as usize % MONTHS;
    for town in towns.0.iter_mut() {
        learn_profile(town, np, month);
        if !town.policy.stockpile.is_empty() {
            town.stockpile_scale = season_scale(town, np, month);
        }
    }
    tend_stockpiles(towns, db, params);
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
