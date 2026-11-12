//! Household consumption: households buy goods for their own use.
//!
//! `market.consumption` in models.ron. `None` (the default) keeps the original
//! rule: households buy only food (when hungry) and tools (to work). `Basket`
//! makes each adult keep a small household stock of the basket's goods:
//! - each good wears out over `life_days` (ale is drunk within days, clothes
//!   last months, pots a year);
//! - how much of it the agent wants is `want` × its taste (0.5 + the mind
//!   dimension named in `taste`, if any) × its relative income (wealth against
//!   the town's median, to the power `elasticity`, so richer households want
//!   more, and more of the dearer goods);
//! - the goods are those listed in `goods`, or, if none are, every good whose
//!   content declares `household` use (so new goods join by saying so);
//! - every `interval` days it spends up to `spend_share` of its wealth on the
//!   goods it is furthest short of, one unit at a time, from the firms that
//!   hold them (with `market.firms: Workshops`) or else the market, at the town
//!   price;
//! - what it owns against what it wants is its **comfort**, which raises its
//!   happiness by up to `comfort_weight`.
//!
//! The purchases are demand: with `market.pricing: Demand` the market's target
//! stock of each basket good includes `cover_days` of what households bought.
//! Money is conserved: every purchase pays a firm or the treasury.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum ConsumptionParams {
    #[default]
    None,
    Basket {
        spend_share: f32,
        interval: u32,
        elasticity: f32,
        comfort_weight: f32,
        goods: Vec<BasketGood>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BasketGood {
    pub good: String,
    /// Units a household of median income and middling taste wants to hold.
    pub want: f32,
    /// Days a unit lasts.
    pub life_days: f32,
    /// Mind dimension (by name) that makes an agent want more of it.
    #[serde(default)]
    pub taste: Option<String>,
    /// Overrides the basket's `elasticity` for this good (luxuries above 1).
    #[serde(default)]
    pub elasticity: Option<f32>,
}

/// The basket's goods: those listed in models.ron or, if none are, every good
/// whose content declares `household` use.
pub fn basket(listed: &[BasketGood], db: &crate::db::Db) -> Vec<BasketGood> {
    if !listed.is_empty() {
        return listed.to_vec();
    }
    db.content
        .products
        .iter()
        .filter_map(|p| {
            p.household.as_ref().map(|h| BasketGood {
                good: p.id.clone(),
                want: h.want,
                life_days: h.life_days,
                taste: h.taste.clone(),
                elasticity: h.elasticity,
            })
        })
        .collect()
}

impl ConsumptionParams {
    pub fn on(&self) -> bool {
        matches!(self, ConsumptionParams::Basket { .. })
    }
}

/// How much of a good an agent wants: `want` × taste × relative income^elasticity.
pub fn desired(want: f32, taste: f32, wealth: f32, median: f32, elasticity: f32) -> f32 {
    let rel = (wealth.max(0.0) / median.max(1e-3)).clamp(0.05, 20.0);
    want * (0.5 + taste.clamp(0.0, 1.0)) * rel.powf(elasticity)
}

/// Comfort, 0..1: what the agent owns against what it wants, over the basket.
pub fn comfort(owned: &[f32], wanted: &[f32]) -> f32 {
    let w: f32 = wanted.iter().sum();
    if w <= 0.0 {
        return 1.0;
    }
    owned
        .iter()
        .zip(wanted)
        .map(|(o, w)| o.min(*w))
        .sum::<f32>()
        / w
}

/// Every `interval` days (staggered by agent), each adult tops up its household
/// stock of the basket's goods (see the module docs).
pub fn spend(
    towns: &mut crate::world::Towns,
    store: &mut crate::store::AgentStore,
    db: &crate::db::Db,
    params: &crate::world::Params,
    tick: u64,
) {
    use crate::store::Activity;
    let ConsumptionParams::Basket {
        spend_share,
        interval,
        elasticity,
        goods: listed,
        ..
    } = &params.market.consumption
    else {
        return;
    };
    let goods = &basket(listed, db);
    let ids: Vec<Option<usize>> = goods
        .iter()
        .map(|g| db.content.product(&g.good).map(|p| p as usize))
        .collect();
    let tastes: Vec<Option<usize>> = goods
        .iter()
        .map(|g| {
            g.taste.as_ref().and_then(|t| {
                sim_data::dims::MIND_NAMES
                    .iter()
                    .position(|n| n.eq_ignore_ascii_case(t))
            })
        })
        .collect();
    let iv = (*interval).max(1) as u64;
    let firms_on = params.market.firms.on();
    let medians: Vec<f32> = towns
        .0
        .iter()
        .map(|t| {
            let mut w: Vec<f32> = t
                .residents
                .iter()
                .map(|&i| store.wealth[i as usize])
                .collect();
            if w.is_empty() {
                return 1.0;
            }
            let mid = w.len() / 2;
            *w.select_nth_unstable_by(mid, f32::total_cmp).1
        })
        .collect();
    let np = db.content.products.len();
    for i in 0..store.len() {
        if !store.alive[i]
            || matches!(store.activity[i], Activity::Child | Activity::Captive)
            || !(tick + i as u64).is_multiple_of(iv)
        {
            continue;
        }
        let t = store.town[i] as usize;
        let town = &mut towns.0[t];
        town.goods_today.resize(np, 0.0);
        town.consumed_year.resize(np, 0.0);
        let mut have = vec![0.0f32; goods.len()];
        // Wear: each good loses its share of a lifetime since the last look.
        for e in store.owned[i].iter_mut() {
            if let Some(k) = ids.iter().position(|&p| p == Some(e.0 as usize)) {
                e.1 *= (1.0 - iv as f32 / goods[k].life_days.max(1.0)).max(0.0);
                have[k] = e.1;
            }
        }
        let wanted: Vec<f32> = goods
            .iter()
            .zip(&tastes)
            .map(|(g, taste)| {
                let taste = taste.map_or(0.5, |d| store.mind[i][d]);
                let e = g.elasticity.unwrap_or(*elasticity);
                desired(g.want, taste, store.wealth[i], medians[t], e)
            })
            .collect();
        let mut budget = spend_share.clamp(0.0, 1.0) * store.wealth[i].max(0.0);
        let mut out = vec![false; goods.len()];
        for _ in 0..16 {
            // The good the household is furthest short of, that it can still buy.
            let Some(k) = (0..goods.len())
                .filter(|&k| !out[k] && ids[k].is_some() && wanted[k] - have[k] >= 0.5)
                .max_by(|&a, &b| {
                    ((wanted[a] - have[a]) / wanted[a].max(1e-6))
                        .total_cmp(&((wanted[b] - have[b]) / wanted[b].max(1e-6)))
                        .then(b.cmp(&a))
                })
            else {
                break;
            };
            let p = ids[k].unwrap();
            let price = town.price[p];
            if price > budget {
                out[k] = true;
                continue;
            }
            // From a firm that holds it, else from the market.
            let firm = if firms_on {
                town.firms
                    .iter()
                    .position(|f| f.stock.get(p).is_some_and(|&s| s >= 1.0))
            } else {
                None
            };
            match firm {
                Some(f) => {
                    let firm = &mut town.firms[f];
                    firm.stock[p] -= 1.0;
                    firm.cash += price;
                    firm.book(crate::firms::FirmFlow::DirectSales, price);
                }
                None if town.stock[p] >= 1.0 => {
                    town.stock[p] -= 1.0;
                    town.treasury += price;
                    town.ledger.add(crate::ledger::Flow::HouseholdGoods, price);
                }
                None => {
                    out[k] = true;
                    continue;
                }
            }
            store.wealth[i] -= price;
            budget -= price;
            have[k] += 1.0;
            town.goods_today[p] += 1.0;
            town.consumed_year[p] += 1.0;
        }
        // Write back what the household holds.
        let owned = &mut store.owned[i];
        for (k, &p) in ids.iter().enumerate() {
            let Some(p) = p else { continue };
            match owned.iter_mut().find(|e| e.0 as usize == p) {
                Some(e) => e.1 = have[k],
                None if have[k] > 0.0 => owned.push((p as u32, have[k])),
                None => {}
            }
        }
        owned.retain(|e| e.1 > 0.01);
        store.comfort[i] = comfort(&have, &wanted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn richer_and_keener_households_want_more() {
        let base = desired(1.0, 0.5, 20.0, 20.0, 0.5);
        assert!((base - 1.0).abs() < 1e-6);
        assert!(desired(1.0, 0.5, 80.0, 20.0, 0.5) > base);
        assert!(desired(1.0, 0.9, 20.0, 20.0, 0.5) > base);
        assert!(desired(1.0, 0.5, 0.0, 20.0, 0.5) < base);
    }

    #[test]
    fn an_empty_basket_takes_the_goods_the_content_declares() {
        let paths = crate::AssetPaths::discover();
        let (db, _, _) = crate::load(&paths).unwrap();
        let from_content = basket(&[], &db);
        let ids: Vec<&str> = from_content.iter().map(|g| g.good.as_str()).collect();
        for id in ["clothes", "pot", "ale", "charcoal", "cloth", "silver"] {
            assert!(ids.contains(&id), "{id} missing from {ids:?}");
        }
        // Silver is a luxury: richer households want disproportionately more.
        let silver = from_content.iter().find(|g| g.good == "silver").unwrap();
        assert!(silver.elasticity.unwrap() > 1.0);
        // A listed basket is used as it is.
        let listed = vec![from_content[0].clone()];
        assert_eq!(basket(&listed, &db).len(), 1);
    }

    #[test]
    fn comfort_is_what_you_have_of_what_you_want() {
        assert_eq!(comfort(&[1.0, 0.0], &[1.0, 1.0]), 0.5);
        assert_eq!(comfort(&[5.0, 5.0], &[1.0, 1.0]), 1.0);
        assert_eq!(comfort(&[], &[]), 1.0);
    }
}
