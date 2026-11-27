//! Spoilage: goods that rot in store.
//!
//! A good with `keeping` days (content) loses 1 / days of itself a day at
//! `market.spoilage.reference_c`, faster in the warmth and slower in the cold
//! (`SpoilageParams::speed`, from the town's mean temperature of the day). It
//! rots wherever it is kept: the market, the granary (`granary_spoilage` of
//! the rate), firms' stock and people's larders. Fresh fish and milk are gone
//! in days, bread in a week or two, cheese and salted food keep for months: so
//! preserving is how food outlasts the season. What rots is counted as
//! `Flow::Spoiled`.

use crate::calendar::Calendar;
use crate::config::MarketParams;
use crate::db::Db;
use crate::env::Environment;
use crate::goods::Flow;
use crate::store::AgentStore;
use crate::world::Towns;

/// Days a product keeps at the reference temperature (0: it does not rot).
pub fn keep_days(db: &Db, mp: &MarketParams, product: usize) -> f32 {
    match db.keep_days.get(product).copied().unwrap_or(0.0) {
        d if d > 0.0 => d,
        _ if db.food_value.get(product).is_some_and(|&v| v > 0.0) => mp.spoilage.food_days,
        _ => 0.0,
    }
}

/// One step of rot in every store of every town.
pub fn rot(
    towns: &mut Towns,
    store: &mut AgentStore,
    db: &Db,
    mp: &MarketParams,
    env: &Environment,
    cal: &Calendar,
) {
    let np = db.content.products.len();
    let perishable: Vec<(usize, f32)> = (0..np)
        .filter(|&p| !db.category(p as u32).is_item())
        .map(|p| (p, keep_days(db, mp, p)))
        .filter(|&(_, d)| d > 0.0)
        .collect();
    if perishable.is_empty() {
        return;
    }
    // Per town, the share of each perishable good that is left after the step.
    let left: Vec<Vec<f32>> = (0..towns.0.len())
        .map(|t| {
            let speed = mp.spoilage.speed(env.at(t).day_mean_c);
            let mut v = vec![1.0f32; np];
            for &(p, d) in &perishable {
                v[p] = (-cal.days() * speed / d).exp();
            }
            v
        })
        .collect();
    let granary = mp.granary_spoilage.max(0.0);
    for (t, town) in towns.0.iter_mut().enumerate() {
        let left = &left[t];
        for &(p, _) in &perishable {
            let mut rotted = 0.0;
            if let Some(s) = town.stock.get_mut(p) {
                rotted += *s * (1.0 - left[p]);
                *s *= left[p];
            }
            if let Some(s) = town.granary.get_mut(p) {
                let keep = left[p].powf(granary);
                rotted += *s * (1.0 - keep);
                *s *= keep;
            }
            for firm in town.firms.iter_mut() {
                if let Some(s) = firm.stock.get_mut(p) {
                    rotted += *s * (1.0 - left[p]);
                    *s *= left[p];
                }
            }
            if rotted > 0.0 {
                town.flows.add(Flow::Spoiled, p, rotted);
            }
        }
    }
    for i in 0..store.len() {
        let t = store.town[i] as usize;
        let Some(left) = left.get(t) else { continue };
        let mut rotted: Vec<(usize, f32)> = Vec::new();
        for (g, q) in store.larder[i].iter_mut() {
            let k = left[*g as usize];
            if k < 1.0 {
                rotted.push((*g as usize, *q * (1.0 - k)));
                *q *= k;
            }
        }
        store.larder[i].retain(|&(_, q)| q > 1e-4);
        if let Some(town) = towns.0.get_mut(t) {
            for (g, q) in rotted {
                town.flows.add(Flow::Spoiled, g, q);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::SpoilageParams;

    #[test]
    fn warmth_speeds_rot_and_cold_slows_it_down_to_the_floor() {
        let p = SpoilageParams::default();
        assert!((p.speed(15.0) - 1.0).abs() < 1e-6);
        assert!((p.speed(25.0) - 2.0).abs() < 1e-5);
        assert!(p.speed(5.0) < 1.0);
        assert_eq!(p.speed(-10.0), p.speed(0.0));
    }
}
