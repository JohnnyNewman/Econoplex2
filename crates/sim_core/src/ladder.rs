//! The needs ladder: what people want, and why (economy redesign, step 4).
//!
//! `market.consumption: Needs(...)` in models.ron. Each person has a
//! satisfaction from 0 to 1 for each need, on five rungs:
//!
//! | Rung | Needs |
//! | --- | --- |
//! | physiological | food, warmth, rest |
//! | safety | shelter, health, savings |
//! | belonging | belonging |
//! | esteem | esteem |
//! | self-actualisation | growth |
//!
//! A rung counts only as the rungs below it are met: its weight is the product
//! of a smooth gate (from `gate_at - gate_width` to `gate_at + gate_width`) over
//! the satisfaction of every rung below. Goods declare in the content which
//! needs they serve (`household.serves`). Every `interval` days a household
//! buys, one unit at a time, the good that raises its weighted satisfaction
//! most per coin, and stops when nothing is worth more than the money it would
//! keep: money serves savings, so the budget comes out of the same rule
//! instead of a spending share.
//!
//! Warmth is physical. Each day a person loses heat by the degrees the day's
//! mean temperature falls short of `comfort_c`, after the house (`house_c`)
//! and the clothes worn (their insulation, up to `max_clothes` pieces). The
//! rest has to come from fuel burned at the hearth, shared by
//! `people_per_hearth`. Without it people get cold, and cold hurts health.
//!
//! Food is still eaten by the hunger rule in `needs.rs`; its satisfaction is
//! `1 - hunger`.

use crate::db::Db;
use crate::store::{Activity, AgentStore};
use crate::world::{Params, Towns};
use serde::{Deserialize, Serialize};
use sim_data::{Need, Serves, Use};

/// `market.consumption: Needs(...)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LadderParams {
    /// Days between a household's shopping trips (staggered by person).
    pub interval: u32,
    /// A rung opens as the rungs below pass `gate_at`, over ± `gate_width`.
    pub gate_at: f32,
    pub gate_width: f32,
    /// Air temperature people are comfortable in, °C.
    pub comfort_c: f32,
    /// What walls and a roof make up, °C.
    pub house_c: f32,
    /// Pieces of clothing worn at once.
    pub max_clothes: f32,
    /// People sharing one hearth's fire.
    pub people_per_hearth: f32,
    /// Days of food a household wants covered by its money and larder.
    pub savings_days: f32,
    /// Health lost per day of bitter cold (10 °C short) with no fire.
    pub cold_damage: f32,
    /// Days ahead a household plans its fuel and clothes for, from what it
    /// knows of the seasons (today's warm or cold spell fades over a few days).
    #[serde(default = "plan_days")]
    pub plan_days: f32,
}

fn plan_days() -> f32 {
    30.0
}

/// One household good and the needs it serves.
#[derive(Debug, Clone)]
pub struct LadderGood {
    pub good: usize,
    pub life_days: f32,
    pub serves: Vec<Serves>,
}

/// Every good whose content says which needs it serves.
pub fn goods(db: &Db) -> Vec<LadderGood> {
    db.content
        .products
        .iter()
        .enumerate()
        .filter_map(|(p, def)| {
            let h = def.household.as_ref()?;
            (!h.serves.is_empty()).then(|| LadderGood {
                good: p,
                life_days: h.life_days.max(1.0),
                serves: h.serves.clone(),
            })
        })
        .collect()
}

const N: usize = Need::ALL.len();

/// Satisfaction per need, 0..1, in `Need::ALL` order.
pub type Sat = [f32; N];

fn idx(n: Need) -> usize {
    Need::ALL.iter().position(|&m| m == n).unwrap()
}

/// Diminishing satisfaction from points held (0.3 points give about 0.63).
fn held(points: f32) -> f32 {
    1.0 - (-points.max(0.0) / 0.3).exp()
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Weight of each rung: 1 for the bottom one, then the product of the gates
/// over the rungs below.
pub fn gates(sat: &Sat, lp: &LadderParams) -> [f32; 5] {
    let mut level = [0.0f32; 5];
    let mut count = [0.0f32; 5];
    for (k, &n) in Need::ALL.iter().enumerate() {
        level[n.level()] += sat[k];
        count[n.level()] += 1.0;
    }
    let mut g = [1.0f32; 5];
    for l in 1..5 {
        let below = level[l - 1] / count[l - 1].max(1.0);
        g[l] = g[l - 1]
            * smooth(
                lp.gate_at - lp.gate_width,
                lp.gate_at + lp.gate_width,
                below,
            );
    }
    g
}

/// Weighted satisfaction: what a household tries to raise.
pub fn score(sat: &Sat, lp: &LadderParams) -> f32 {
    let g = gates(sat, lp);
    Need::ALL
        .iter()
        .enumerate()
        .map(|(k, n)| g[n.level()] * sat[k])
        .sum()
}

/// Happiness the ladder gives, 0..1: each rung's satisfaction weighted by its
/// gate, over the five rungs.
pub fn happiness(sat: &Sat, lp: &LadderParams) -> f32 {
    let g = gates(sat, lp);
    let mut level = [0.0f32; 5];
    let mut count = [0.0f32; 5];
    for (k, &n) in Need::ALL.iter().enumerate() {
        level[n.level()] += sat[k];
        count[n.level()] += 1.0;
    }
    (0..5)
        .map(|l| g[l] * level[l] / count[l].max(1.0))
        .sum::<f32>()
        / 5.0
}

/// What a person holds that serves needs, summed by kind.
#[derive(Debug, Clone, Copy, Default)]
struct Holdings {
    /// Degree-days of heat in fuel.
    fuel: f32,
    /// Insulation of the clothes worn, °C.
    clothes: f32,
    /// Points held per need (Own and Consume goods).
    points: [f32; N],
}

fn holdings(owned: &[(u32, f32)], goods: &[LadderGood], lp: &LadderParams) -> Holdings {
    let mut h = Holdings::default();
    let mut pieces = 0.0f32;
    for &(p, q) in owned {
        let Some(g) = goods.iter().find(|g| g.good == p as usize) else {
            continue;
        };
        for s in &g.serves {
            match (s.by, s.need) {
                (Use::Burn, Need::Warmth) => h.fuel += q * s.amount,
                (Use::Wear, Need::Warmth) => {
                    // The warmest pieces are the ones worn.
                    let wear = q.min((lp.max_clothes - pieces).max(0.0));
                    pieces += wear;
                    h.clothes += wear * s.amount;
                }
                (Use::Burn | Use::Wear, _) => {}
                (Use::Own | Use::Consume, n) => h.points[idx(n)] += q * s.amount,
            }
        }
    }
    h
}

/// Heat a person has to get from the fire today, degree-days (shared hearth).
pub fn heat_need(mean_c: f32, clothes_c: f32, lp: &LadderParams) -> f32 {
    (lp.comfort_c - (mean_c + lp.house_c + clothes_c)).max(0.0) / lp.people_per_hearth.max(1.0)
}

/// Share of the next `days` days' heat that `fuel` covers.
fn coverage(fuel: f32, need_per_day: f32, days: f32) -> f32 {
    if need_per_day <= 1e-6 {
        1.0
    } else {
        (fuel / (need_per_day * days)).clamp(0.0, 1.0)
    }
}

/// What the town gives everyone alike: housing for its people (0..1) and the
/// cost of a day's food at its cheapest.
#[derive(Debug, Clone)]
struct TownView {
    housed: f32,
    day_food_cost: f32,
    mean_c: f32,
    hunger_per_day: f32,
    /// Expected mean temperature over the planning days, °C.
    ahead: Vec<f32>,
}

/// The climate's mean temperature at a point of the year.
fn climate_c(climate: &crate::env::ClimateParams, season: f32) -> f32 {
    match *climate {
        crate::env::ClimateParams::Constant { temperature } => temperature,
        crate::env::ClimateParams::Seasonal {
            mean_c,
            seasonal_swing_c,
            ..
        } => mean_c - seasonal_swing_c * (std::f32::consts::TAU * season).cos(),
    }
}

/// Expected daily mean temperatures over the planning days: the seasons as
/// everyone knows them, with today's spell fading over about five days.
fn expected_temps(params: &Params, env: &crate::env::Environment, today: f32) -> Vec<f32> {
    let lp_days = match &params.market.consumption {
        crate::consumption::ConsumptionParams::Needs(lp) => lp.plan_days,
        _ => 30.0,
    };
    let dpy = params.life.days_per_year.max(1) as f32;
    let spell = today - climate_c(&params.climate, env.season);
    (0..lp_days.max(1.0) as usize)
        .map(|d| {
            let d = d as f32;
            climate_c(&params.climate, env.season + d / dpy) + spell * (-d / 5.0).exp()
        })
        .collect()
}

fn town_views(
    towns: &Towns,
    db: &Db,
    params: &Params,
    env: &crate::env::Environment,
) -> Vec<TownView> {
    let crowding = &params.decision.crowding;
    towns
        .0
        .iter()
        .enumerate()
        .map(|(t, town)| {
            let housing = crowding.housing(
                town.nature_sites.len() + town.free_land as usize,
                town.buildings.len(),
            );
            let housed = (housing / town.residents.len().max(1) as f32).min(1.0);
            let per_hunger = db
                .foods
                .iter()
                .map(|&(f, v)| town.price[f as usize] / v.max(1e-3))
                .fold(f32::INFINITY, f32::min);
            let day_food_cost = if per_hunger.is_finite() {
                per_hunger * params.needs.hunger_per_day
            } else {
                1.0
            };
            TownView {
                housed,
                day_food_cost: day_food_cost.max(1e-3),
                mean_c: env.at(t).day_mean_c,
                hunger_per_day: params.needs.hunger_per_day.max(1e-3),
                ahead: expected_temps(params, env, env.at(t).day_mean_c),
            }
        })
        .collect()
}

/// A person's satisfaction, with `h` held and `wealth` in hand, looking
/// `days` ahead for warmth.
#[allow(clippy::too_many_arguments)]
fn assess(
    store: &AgentStore,
    i: usize,
    h: &Holdings,
    wealth: f32,
    view: &TownView,
    lp: &LadderParams,
    db: &Db,
    plan: bool,
) -> Sat {
    use sim_data::dims::state::*;
    let st = &store.state[i];
    let mut s = [0.0f32; N];
    s[idx(Need::Food)] = (1.0 - st[HUNGER]).clamp(0.0, 1.0);
    s[idx(Need::Warmth)] = if plan {
        let need: f32 = view
            .ahead
            .iter()
            .map(|&c| heat_need(c, h.clothes, lp))
            .sum();
        coverage(h.fuel, need, 1.0)
    } else {
        coverage(h.fuel, heat_need(view.mean_c, h.clothes, lp), 1.0)
    };
    s[idx(Need::Rest)] = (1.0 - st[FATIGUE]).clamp(0.0, 1.0);
    s[idx(Need::Shelter)] = view.housed * (0.7 + 0.3 * held(h.points[idx(Need::Shelter)]));
    s[idx(Need::Health)] = st[HEALTH].clamp(0.0, 1.0);
    let larder: f32 = store.larder[i]
        .iter()
        .map(|&(f, q)| q * db.foods.iter().find(|x| x.0 == f).map_or(0.0, |x| x.1))
        .sum();
    let larder_days = larder / view.hunger_per_day;
    let money_days = wealth.max(0.0) / view.day_food_cost;
    s[idx(Need::Savings)] = ((larder_days + money_days) / lp.savings_days.max(1.0)).min(1.0);
    let ties = &store.ties[i];
    let bonds = ties.iter().map(|t| t.value.max(0.0)).sum::<f32>() / ties.len().max(1) as f32;
    s[idx(Need::Belonging)] =
        (0.6 * bonds.min(1.0) + 0.4 * held(h.points[idx(Need::Belonging)])).min(1.0);
    s[idx(Need::Esteem)] = held(h.points[idx(Need::Esteem)]);
    // Nothing serves growth yet (learning and faith come later): neutral.
    s[idx(Need::Growth)] = 0.5;
    s
}

/// Daily: clothes and household goods wear, fuel burns as the cold asks, cold
/// hurts, and happiness follows the ladder.
pub fn daily(
    store: &mut AgentStore,
    towns: &mut Towns,
    db: &Db,
    params: &Params,
    env: &crate::env::Environment,
    lp: &LadderParams,
) {
    use sim_data::dims::state::*;
    let goods = goods(db);
    let views = town_views(towns, db, params, env);
    let rate = params.needs.happiness_rate;
    for i in 0..store.len() {
        if !store.alive[i] || matches!(store.activity[i], Activity::Captive) {
            continue;
        }
        let view = &views[store.town[i] as usize];
        // Wear and use-up over each good's life.
        for e in store.owned[i].iter_mut() {
            if let Some(g) = goods.iter().find(|g| g.good == e.0 as usize) {
                if g.serves.iter().any(|s| s.by != Use::Burn) {
                    e.1 *= 1.0 - 1.0 / g.life_days;
                }
            }
        }
        let h = holdings(&store.owned[i], &goods, lp);
        let need = heat_need(view.mean_c, h.clothes, lp);
        // Burn what the day needs, fuel by fuel.
        let mut left = need;
        for e in store.owned[i].iter_mut() {
            if left <= 0.0 {
                break;
            }
            let Some(g) = goods.iter().find(|g| g.good == e.0 as usize) else {
                continue;
            };
            let Some(heat) = g
                .serves
                .iter()
                .find(|s| s.by == Use::Burn && s.need == Need::Warmth)
                .map(|s| s.amount.max(1e-3))
            else {
                continue;
            };
            let burn = e.1.min(left / heat);
            e.1 -= burn;
            left -= burn * heat;
        }
        store.owned[i].retain(|e| e.1 > 0.01);
        let warm = if need > 0.0 { 1.0 - left / need } else { 1.0 };
        if warm < 0.5 {
            towns.0[store.town[i] as usize].cold_days += 1;
        }
        let short_c = (lp.comfort_c - (view.mean_c + lp.house_c + h.clothes)).max(0.0);
        let st = &mut store.state[i];
        st[HEALTH] -= lp.cold_damage * (1.0 - warm) * (short_c / 10.0).min(1.5);
        let h = holdings(&store.owned[i], &goods, lp);
        let mut sat = assess(store, i, &h, store.wealth[i], view, lp, db, false);
        sat[idx(Need::Warmth)] = warm;
        let target = happiness(&sat, lp);
        let st = &mut store.state[i];
        st[HAPPINESS] = (st[HAPPINESS] + rate * (target - st[HAPPINESS])).clamp(0.0, 1.0);
    }
}

/// Every living person's satisfaction per need today (diagnostics).
pub fn all(
    store: &AgentStore,
    towns: &Towns,
    db: &Db,
    params: &Params,
    env: &crate::env::Environment,
    lp: &LadderParams,
) -> Vec<Option<Sat>> {
    let goods = goods(db);
    let views = town_views(towns, db, params, env);
    (0..store.len())
        .map(|i| {
            store.alive[i].then(|| {
                let h = holdings(&store.owned[i], &goods, lp);
                assess(
                    store,
                    i,
                    &h,
                    store.wealth[i],
                    &views[store.town[i] as usize],
                    lp,
                    db,
                    false,
                )
            })
        })
        .collect()
}

/// Every `interval` days (staggered), each adult buys what raises its weighted
/// satisfaction most per coin, while that beats keeping the money.
pub fn spend(
    towns: &mut Towns,
    store: &mut AgentStore,
    db: &Db,
    params: &Params,
    env: &crate::env::Environment,
    lp: &LadderParams,
    tick: u64,
) {
    let goods = goods(db);
    if goods.is_empty() {
        return;
    }
    let views = town_views(towns, db, params, env);
    let iv = lp.interval.max(1) as u64;
    let firms_on = params.market.firms.on();
    for i in 0..store.len() {
        if !store.alive[i]
            || matches!(store.activity[i], Activity::Child | Activity::Captive)
            || !(tick + i as u64).is_multiple_of(iv)
        {
            continue;
        }
        let t = store.town[i] as usize;
        let view = &views[t];
        for _ in 0..16 {
            let h = holdings(&store.owned[i], &goods, lp);
            let now = score(
                &assess(store, i, &h, store.wealth[i], view, lp, db, true),
                lp,
            );
            let town = &towns.0[t];
            // The good worth most per coin, counting the savings the money is worth.
            let mut best: Option<(usize, f32)> = None;
            for (k, g) in goods.iter().enumerate() {
                let p = g.good;
                let price = town.price[p];
                if price <= 0.0 || price > store.wealth[i] {
                    continue;
                }
                let available = town.stock[p] >= 1.0
                    || (firms_on
                        && town
                            .firms
                            .iter()
                            .any(|f| f.stock.get(p).is_some_and(|&s| s >= 1.0)));
                if !available {
                    continue;
                }
                let mut owned = store.owned[i].clone();
                match owned.iter_mut().find(|e| e.0 as usize == p) {
                    Some(e) => e.1 += 1.0,
                    None => owned.push((p as u32, 1.0)),
                }
                let h2 = holdings(&owned, &goods, lp);
                let after = score(
                    &assess(store, i, &h2, store.wealth[i] - price, view, lp, db, true),
                    lp,
                );
                let gain = (after - now) / price;
                if gain > 1e-6 && best.is_none_or(|b| gain > b.1) {
                    best = Some((k, gain));
                }
            }
            let Some((k, _)) = best else { break };
            let p = goods[k].good;
            let town = &mut towns.0[t];
            let price = town.price[p];
            let firm = if firms_on {
                town.firms
                    .iter()
                    .position(|f| f.stock.get(p).is_some_and(|&s| s >= 1.0))
            } else {
                None
            };
            match firm {
                Some(f) => {
                    town.firms[f].stock[p] -= 1.0;
                    town.firms[f].cash += price;
                    town.firms[f].book(crate::firms::FirmFlow::DirectSales, price);
                }
                None => {
                    town.stock[p] -= 1.0;
                    town.treasury += price;
                    town.ledger.add(crate::ledger::Flow::HouseholdGoods, price);
                }
            }
            town.flows.add(crate::goods::Flow::Bought, p, 1.0);
            let np = db.content.products.len();
            town.goods_today.resize(np, 0.0);
            town.consumed_year.resize(np, 0.0);
            town.goods_today[p] += 1.0;
            town.consumed_year[p] += 1.0;
            store.wealth[i] -= price;
            match store.owned[i].iter_mut().find(|e| e.0 as usize == p) {
                Some(e) => e.1 += 1.0,
                None => store.owned[i].push((p as u32, 1.0)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lp() -> LadderParams {
        LadderParams {
            interval: 7,
            gate_at: 0.7,
            gate_width: 0.15,
            comfort_c: 16.0,
            house_c: 6.0,
            max_clothes: 2.0,
            people_per_hearth: 4.0,
            savings_days: 60.0,
            cold_damage: 0.02,
            plan_days: 30.0,
        }
    }

    #[test]
    fn upper_rungs_wait_for_the_lower_ones() {
        let lp = lp();
        let mut sat = [1.0; N];
        let full = gates(&sat, &lp);
        assert!(full.iter().all(|&g| g > 0.99));
        sat[idx(Need::Food)] = 0.2;
        sat[idx(Need::Warmth)] = 0.3;
        let hungry = gates(&sat, &lp);
        assert!(hungry[1] < 0.05 && hungry[4] < 0.05, "{hungry:?}");
        // So a hungry person gains more from warmth than from esteem.
        let mut warmer = sat;
        warmer[idx(Need::Warmth)] += 0.2;
        let mut prouder = sat;
        prouder[idx(Need::Esteem)] = 1.0;
        assert!(score(&warmer, &lp) - score(&sat, &lp) > score(&prouder, &lp) - score(&sat, &lp));
    }

    #[test]
    fn cold_days_need_fire_and_clothes_cut_it() {
        let lp = lp();
        assert_eq!(heat_need(15.0, 0.0, &lp), 0.0);
        let bare = heat_need(-5.0, 0.0, &lp);
        let dressed = heat_need(-5.0, 6.0, &lp);
        assert!((bare - 15.0 / 4.0).abs() < 1e-5 && dressed < bare);
        assert_eq!(coverage(0.0, bare, 7.0), 0.0);
        assert!((coverage(bare * 7.0, bare, 7.0) - 1.0).abs() < 1e-5);
    }
}
