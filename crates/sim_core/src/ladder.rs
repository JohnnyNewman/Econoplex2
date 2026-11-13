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
//! rest has to come from fuel burned at the household's hearth. Without it
//! people get cold, and cold hurts health.
//!
//! Households (step 5): a couple and their children share one home. Its keeper
//! holds the fuel and the home goods (pots, linen) and buys them for everyone
//! at the hearth; clothes, ale and silver stay each person's own. One fire
//! warms the whole family, so a family burns less per head than people alone.
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
    /// Fuel one hearth burns per degree the room falls short, degree-days a day.
    pub fire_per_c: f32,
    /// How much more fuel each further person at the hearth takes (0.25: a
    /// family of four burns 1.75 times what one person alone does).
    pub fire_per_extra_person: f32,
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
    /// Kept for the whole household (fuel, and goods that serve shelter).
    pub home: bool,
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
                home: h
                    .serves
                    .iter()
                    .any(|s| s.by == Use::Burn || s.need == Need::Shelter),
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

/// What a person can draw on: their own clothes and goods, and the hearth and
/// home goods of their household.
#[derive(Debug, Clone, Copy, Default)]
struct Holdings {
    /// Degree-days of heat in the household's fuel.
    fuel: f32,
    /// Insulation of the clothes this person wears, °C.
    clothes: f32,
    /// People at the hearth, and the mean insulation of their clothes.
    size: f32,
    room_clothes: f32,
    /// Points held per need (Own and Consume goods; shelter from the home).
    points: [f32; N],
}

/// Adds what `owned` holds to `h`: only home goods (fuel, shelter) when
/// `home`, only personal goods otherwise.
fn add_holdings(
    h: &mut Holdings,
    owned: &[(u32, f32)],
    goods: &[LadderGood],
    lp: &LadderParams,
    home: bool,
) {
    let mut pieces = 0.0f32;
    for &(p, q) in owned {
        let Some(g) = goods.iter().find(|g| g.good == p as usize) else {
            continue;
        };
        if g.home != home {
            continue;
        }
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
}

/// Households: people who share a hearth, a larder of home goods and the
/// fire. A couple keeps one home (kept by the partner with the lower index);
/// children live with their parents; everyone else keeps their own.
#[derive(Debug, Clone)]
pub struct Homes {
    /// Per person: who keeps their home (holds the fuel and home goods).
    pub keeper: Vec<usize>,
    /// Per keeper: people at the hearth, and the sum of their clothes' insulation.
    size: Vec<f32>,
    clothes: Vec<f32>,
}

impl Homes {
    pub fn of(store: &AgentStore, goods: &[LadderGood], lp: &LadderParams) -> Homes {
        let n = store.len();
        let alive = |k: usize| k < n && store.alive[k];
        let adult_home = |i: usize| match store.partner[i] {
            Some(p) if alive(p as usize) && (p as usize) < i => p as usize,
            _ => i,
        };
        let keeper: Vec<usize> = (0..n)
            .map(|i| {
                if matches!(store.activity[i], Activity::Child) {
                    if let Some((a, b)) = store.parents[i] {
                        for p in [a as usize, b as usize] {
                            if alive(p) && store.town[p] == store.town[i] {
                                return adult_home(p);
                            }
                        }
                    }
                }
                adult_home(i)
            })
            .collect();
        let mut size = vec![0.0f32; n];
        let mut clothes = vec![0.0f32; n];
        for i in 0..n {
            if !store.alive[i] {
                continue;
            }
            let mut h = Holdings::default();
            add_holdings(&mut h, &store.owned[i], goods, lp, false);
            size[keeper[i]] += 1.0;
            clothes[keeper[i]] += h.clothes;
        }
        Homes {
            keeper,
            size,
            clothes,
        }
    }

    /// What person `i` draws on, with `own` and `home` standing in for their
    /// own goods and their keeper's.
    fn holdings_with(
        &self,
        i: usize,
        own: &[(u32, f32)],
        home: &[(u32, f32)],
        goods: &[LadderGood],
        lp: &LadderParams,
    ) -> Holdings {
        let k = self.keeper[i];
        let mut h = Holdings::default();
        add_holdings(&mut h, own, goods, lp, false);
        add_holdings(&mut h, home, goods, lp, true);
        h.size = self.size[k].max(1.0);
        h.room_clothes = self.clothes[k] / h.size;
        h
    }

    fn holdings(
        &self,
        store: &AgentStore,
        i: usize,
        goods: &[LadderGood],
        lp: &LadderParams,
    ) -> Holdings {
        self.holdings_with(i, &store.owned[i], &store.owned[self.keeper[i]], goods, lp)
    }
}

/// Fuel one hearth burns in a day, degree-days: `fire_per_c` per degree the
/// room falls short of comfort after the house and the clothes worn, and a
/// share more (`fire_per_extra_person`) for each further person at it.
pub fn fire_need(mean_c: f32, clothes_c: f32, size: f32, lp: &LadderParams) -> f32 {
    (lp.comfort_c - (mean_c + lp.house_c + clothes_c)).max(0.0)
        * lp.fire_per_c
        * (1.0 + lp.fire_per_extra_person * (size - 1.0).max(0.0))
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

/// Expected daily mean temperatures over the planning days: the seasons as
/// everyone knows them, with today's spell fading over about five days.
fn expected_temps(params: &Params, env: &crate::env::Environment, today: f32) -> Vec<f32> {
    let lp_days = match &params.market.consumption {
        crate::consumption::ConsumptionParams::Needs(lp) => lp.plan_days,
        _ => 30.0,
    };
    let dpy = params.life.days_per_year.max(1) as f32;
    let spell = today - crate::env::climate_c(&params.climate, env.season);
    (0..lp_days.max(1.0) as usize)
        .map(|d| {
            let d = d as f32;
            crate::env::climate_c(&params.climate, env.season + d / dpy) + spell * (-d / 5.0).exp()
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

/// A person's satisfaction, with `h` to draw on and `wealth` in hand; with
/// `plan`, warmth looks ahead over the planning days.
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
            .map(|&c| fire_need(c, h.room_clothes, h.size, lp))
            .sum();
        coverage(h.fuel, need, 1.0)
    } else {
        coverage(
            h.fuel,
            fire_need(view.mean_c, h.room_clothes, h.size, lp),
            1.0,
        )
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

/// Daily: clothes and goods wear, each hearth burns the fuel the cold asks
/// for, cold hurts, and happiness follows the ladder.
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
    // Wear and use-up over each good's life.
    for i in 0..store.len() {
        if !store.alive[i] {
            continue;
        }
        for e in store.owned[i].iter_mut() {
            if let Some(g) = goods.iter().find(|g| g.good == e.0 as usize) {
                if g.serves.iter().any(|s| s.by != Use::Burn) {
                    e.1 *= 1.0 - 1.0 / g.life_days;
                }
            }
        }
    }
    let homes = Homes::of(store, &goods, lp);
    // Each hearth burns what the day needs, fuel by fuel.
    let mut warm = vec![1.0f32; store.len()];
    for k in 0..store.len() {
        if !store.alive[k] || homes.keeper[k] != k {
            continue;
        }
        let view = &views[store.town[k] as usize];
        let size = homes.size[k].max(1.0);
        let need = fire_need(view.mean_c, homes.clothes[k] / size, size, lp);
        let mut left = need;
        for e in store.owned[k].iter_mut() {
            if left <= 0.0 {
                break;
            }
            let Some(heat) = goods
                .iter()
                .find(|g| g.good == e.0 as usize)
                .and_then(|g| {
                    g.serves
                        .iter()
                        .find(|s| s.by == Use::Burn && s.need == Need::Warmth)
                })
                .map(|s| s.amount.max(1e-3))
            else {
                continue;
            };
            let burn = e.1.min(left / heat);
            e.1 -= burn;
            left -= burn * heat;
        }
        warm[k] = if need > 0.0 { 1.0 - left / need } else { 1.0 };
    }
    for owned in store.owned.iter_mut() {
        owned.retain(|e| e.1 > 0.01);
    }
    for i in 0..store.len() {
        if !store.alive[i] || matches!(store.activity[i], Activity::Captive) {
            continue;
        }
        let view = &views[store.town[i] as usize];
        let w = warm[homes.keeper[i]];
        if w < 0.5 {
            towns.0[store.town[i] as usize].cold_days += 1;
        }
        let h = homes.holdings(store, i, &goods, lp);
        let short_c = (lp.comfort_c - (view.mean_c + lp.house_c + h.clothes)).max(0.0);
        let mut sat = assess(store, i, &h, store.wealth[i], view, lp, db, false);
        sat[idx(Need::Warmth)] = w;
        let target = happiness(&sat, lp);
        let st = &mut store.state[i];
        st[HEALTH] -= lp.cold_damage * (1.0 - w) * (short_c / 10.0).min(1.5);
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
    let homes = Homes::of(store, &goods, lp);
    (0..store.len())
        .map(|i| {
            store.alive[i].then(|| {
                let h = homes.holdings(store, i, &goods, lp);
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
/// satisfaction most per coin, while that beats keeping the money: clothes and
/// other personal goods for themself, and, if they keep the home, fuel and home
/// goods for the household (worth as much as it does for everyone at the
/// hearth).
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
    let homes = Homes::of(store, &goods, lp);
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
        let keeps_home = homes.keeper[i] == i;
        let size = homes.size[i].max(1.0);
        for _ in 0..16 {
            let h = homes.holdings(store, i, &goods, lp);
            let now = score(
                &assess(store, i, &h, store.wealth[i], view, lp, db, true),
                lp,
            );
            let town = &towns.0[t];
            // The good worth most per coin, counting the savings the money is worth.
            let mut best: Option<(usize, f32)> = None;
            for (k, g) in goods.iter().enumerate() {
                if g.home && !keeps_home {
                    continue;
                }
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
                let home: &[(u32, f32)] = if keeps_home {
                    &owned
                } else {
                    &store.owned[homes.keeper[i]]
                };
                let h2 = homes.holdings_with(i, &owned, home, &goods, lp);
                let after = score(
                    &assess(store, i, &h2, store.wealth[i] - price, view, lp, db, true),
                    lp,
                );
                // Fuel and home goods serve everyone at the hearth; the money
                // is only the buyer's.
                let saving_loss = score(
                    &assess(store, i, &h, store.wealth[i] - price, view, lp, db, true),
                    lp,
                ) - now;
                let goods_gain = after - now - saving_loss;
                let worth = if g.home { size } else { 1.0 };
                let gain = (goods_gain * worth + saving_loss) / price;
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
            fire_per_c: 0.5,
            fire_per_extra_person: 0.25,
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
    fn cold_days_need_fire_and_a_family_shares_it() {
        let lp = lp();
        assert_eq!(fire_need(15.0, 0.0, 1.0, &lp), 0.0);
        let bare = fire_need(-5.0, 0.0, 1.0, &lp);
        let dressed = fire_need(-5.0, 6.0, 1.0, &lp);
        assert!((bare - 15.0 * 0.5).abs() < 1e-5 && dressed < bare);
        // Four people at one fire burn less than four fires.
        let family = fire_need(-5.0, 0.0, 4.0, &lp);
        assert!(family > bare && family < 4.0 * bare);
        assert_eq!(coverage(0.0, bare, 7.0), 0.0);
        assert!((coverage(bare * 7.0, bare, 7.0) - 1.0).abs() < 1e-5);
    }
}
