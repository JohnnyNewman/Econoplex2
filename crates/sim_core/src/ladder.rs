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
//! Warmth is physical. Each day (a simulated day burns a month of them) a person loses heat by the degrees the day's
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
    /// Days of time between a household's shopping trips (staggered by
    /// person over the steps).
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
    /// Stock target for food at home: days of food a household keeps in its
    /// larder against bad times, bought ahead when it can (0: none; food is
    /// then bought day by day). Half of savings is then food in the larder.
    #[serde(default)]
    pub larder_days: f32,
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

/// The ladder job choice reads when households do not buy by it (another
/// `market.consumption`): the values models.ron sets.
impl Default for LadderParams {
    fn default() -> Self {
        LadderParams {
            interval: 7,
            gate_at: 0.7,
            gate_width: 0.15,
            comfort_c: 16.0,
            house_c: 6.0,
            max_clothes: 2.0,
            fire_per_c: 0.5,
            fire_per_extra_person: 0.25,
            larder_days: 14.0,
            cold_damage: 0.02,
            plan_days: plan_days(),
        }
    }
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

/// How much the weighted satisfaction rises per unit of each need: the
/// derivative of [`score`]. A need's own rung counts with its gate, and raising
/// it also opens the rungs above a little, so their satisfaction counts too.
/// This is what decides what is worth doing next (job choice).
pub fn gradient(sat: &Sat, lp: &LadderParams) -> Sat {
    let base = score(sat, lp);
    let h = 1e-3;
    let mut g = [0.0f32; N];
    for k in 0..N {
        let mut s = *sat;
        // Step down when the need is full, so the slope is still measured.
        let step = if s[k] + h <= 1.0 { h } else { -h };
        s[k] += step;
        g[k] = (score(&s, lp) - base) / step;
    }
    g
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
    /// Food value a planned purchase would add to the larder.
    extra_food: f32,
    /// What a planned food purchase would add to the protein, fat and
    /// vitamin stores.
    extra_nutrients: [f32; 3],
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

/// Per person: who keeps their home. A couple keeps one home (kept by the
/// partner with the lower index); children live with a parent in their town;
/// everyone else keeps their own. Views use it to put families in one house.
impl Homes {
    /// People at the hearth person `i` keeps (1 if they keep none).
    pub fn size_of(&self, i: usize) -> f32 {
        self.size.get(i).copied().unwrap_or(1.0).max(1.0)
    }
}

pub fn keepers(store: &AgentStore) -> Vec<usize> {
    let n = store.len();
    let alive = |k: usize| k < n && store.alive[k];
    let adult_home = |i: usize| match store.partner[i] {
        Some(p) if alive(p as usize) && (p as usize) < i => p as usize,
        _ => i,
    };
    (0..n)
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
        .collect()
}

impl Homes {
    pub fn of(store: &AgentStore, goods: &[LadderGood], lp: &LadderParams) -> Homes {
        Self::build(store, goods, lp, None)
    }

    /// Like [`Homes::of`], with the clothes worked out only for the
    /// households of `people` (the others' are left at zero).
    pub fn for_people(
        store: &AgentStore,
        goods: &[LadderGood],
        lp: &LadderParams,
        people: &[usize],
    ) -> Homes {
        Self::build(store, goods, lp, Some(people))
    }

    fn build(
        store: &AgentStore,
        goods: &[LadderGood],
        lp: &LadderParams,
        people: Option<&[usize]>,
    ) -> Homes {
        let n = store.len();
        let keeper = keepers(store);
        let wanted: Option<Vec<bool>> = people.map(|people| {
            let mut w = vec![false; n];
            for &i in people {
                w[keeper[i]] = true;
            }
            w
        });
        let mut size = vec![0.0f32; n];
        let mut clothes = vec![0.0f32; n];
        for i in 0..n {
            if !store.alive[i] {
                continue;
            }
            size[keeper[i]] += 1.0;
            if wanted.as_ref().is_some_and(|w| !w[keeper[i]]) {
                continue;
            }
            let mut h = Holdings::default();
            add_holdings(&mut h, &store.owned[i], goods, lp, false);
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
    /// What a person's lower needs cost a day at today's prices: food, their
    /// share of the fire, the wear of their clothes, rent.
    day_cost: f32,
    /// Price of a degree-day of heat in the cheapest fuel.
    heat_price: f32,
    /// Days in a month.
    month_days: f32,
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
    let dpy = params.calendar().days_per_year();
    let spell = today - crate::env::climate_c(&params.climate, env.season);
    (0..lp_days.max(1.0) as usize)
        .map(|d| {
            let d = d as f32;
            crate::env::climate_c(&params.climate, env.season + d / dpy) + spell * (-d / 5.0).exp()
        })
        .collect()
}

/// Savings as months of security: ln(1 + months) / ln(1 + 12), so one month
/// scores 0.27, three 0.54, six 0.76 and twelve 1.
pub fn months_score(months: f32) -> f32 {
    ((1.0 + months.max(0.0)).ln() / 13f32.ln()).min(1.0)
}

/// How much savings satisfaction one more coin buys at `months` of security
/// costing `month_cost` a month (the slope of [`months_score`]).
pub fn months_slope(months: f32, month_cost: f32) -> f32 {
    if months >= 12.0 {
        return 0.0;
    }
    1.0 / (13f32.ln() * (1.0 + months.max(0.0)) * month_cost.max(1e-3))
}

fn lp_of(params: &Params) -> Option<&LadderParams> {
    match &params.market.consumption {
        crate::consumption::ConsumptionParams::Needs(lp) => Some(lp),
        _ => None,
    }
}

/// Price of a degree-day of heat in the town's cheapest fuel, and the daily
/// wear of the clothes a person wears, at today's prices.
fn upkeep_prices(town: &crate::world::Town, db: &Db, lp: Option<&LadderParams>) -> (f32, f32) {
    let goods = goods(db);
    let heat = goods
        .iter()
        .filter_map(|g| {
            g.serves
                .iter()
                .find(|s| s.by == Use::Burn && s.need == Need::Warmth)
                .map(|s| town.price[g.good] / s.amount.max(1e-3))
        })
        .fold(f32::INFINITY, f32::min);
    let clothes = goods
        .iter()
        .find(|g| g.serves.iter().any(|s| s.by == Use::Wear))
        .map_or(0.0, |g| {
            lp.map_or(0.0, |lp| lp.max_clothes) * town.price[g.good] / g.life_days
        });
    (if heat.is_finite() { heat } else { 0.0 }, clothes)
}

/// Per town: (cost of a day's food, cost of a month of the lower needs, hunger
/// per day), for [`savings_slope`].
pub fn savings_costs(
    towns: &Towns,
    db: &Db,
    params: &Params,
    env: &crate::env::Environment,
) -> Vec<[f32; 3]> {
    town_views(towns, db, params, env)
        .iter()
        .map(|v| [v.day_food_cost, v.day_cost * v.month_days, v.hunger_per_day])
        .collect()
}

/// The savings slope per coin for person `i` (for choosing work by the
/// ladder's derivative): what one more coin adds to their months of security.
pub fn savings_slope(
    store: &AgentStore,
    i: usize,
    costs: [f32; 3],
    db: &Db,
    lp: &LadderParams,
) -> f32 {
    let [day_food_cost, month_cost, hunger] = costs;
    let larder: f32 = store.larder[i]
        .iter()
        .map(|&(f, q)| q * db.foods.iter().find(|x| x.0 == f).map_or(0.0, |x| x.1))
        .sum();
    let stores = larder / hunger.max(1e-3) * day_food_cost;
    let months = (store.wealth[i].coins().max(0.0) + stores) / month_cost.max(1e-3);
    let share = if lp.larder_days > 0.0 { 0.5 } else { 1.0 };
    share * months_slope(months, month_cost)
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
            let housed = if db.house.iter().any(Option::is_some) {
                town.shelter(db)
            } else {
                (housing / town.residents.len().max(1) as f32).min(1.0)
            };
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
            let (heat_price, clothes_day) = upkeep_prices(town, db, lp_of(params));
            let lp = lp_of(params);
            let fire = lp.map_or(0.0, |lp| {
                fire_need(env.at(t).day_mean_c, lp.max_clothes * 3.0, 3.0, lp) / 3.0
            });
            let day_cost = day_food_cost.max(1e-3) + fire * heat_price + clothes_day + town.rent;
            TownView {
                housed,
                day_food_cost: day_food_cost.max(1e-3),
                day_cost,
                heat_price,
                month_days: params.calendar().month_days(),
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
    // Health: the body's, and the protein, fat and vitamins in store (when
    // foods carry them): a store half full counts as enough.
    let nourished = if db.nutrients.iter().any(|n| n.iter().any(|&x| x > 0.0)) {
        let nu = store.nutrition.get(i).copied().unwrap_or([1.0; 3]);
        (0..3)
            .map(|k| ((nu[k] + h.extra_nutrients[k]) / 0.5).min(1.0))
            .sum::<f32>()
            / 3.0
    } else {
        1.0
    };
    s[idx(Need::Health)] = st[HEALTH].clamp(0.0, 1.0) * (0.6 + 0.4 * nourished);
    let larder: f32 = store.larder[i]
        .iter()
        .map(|&(f, q)| q * db.foods.iter().find(|x| x.0 == f).map_or(0.0, |x| x.1))
        .sum();
    let larder_days = (larder + h.extra_food) / view.hunger_per_day;
    // Months of the lower needs the household could pay for from its coin and
    // stores, scored so the first month counts most (plan 2.3).
    let stores = larder_days * view.day_food_cost + h.fuel * view.heat_price;
    let months = (wealth.max(0.0) + stores) / (view.day_cost * view.month_days).max(1e-3);
    let reserve = months_score(months);
    s[idx(Need::Savings)] = if lp.larder_days > 0.0 {
        // Food at home is safer than coin when prices spike or the market is bare.
        0.5 * (larder_days / lp.larder_days).min(1.0) + 0.5 * reserve
    } else {
        reserve
    };
    let ties = &store.ties[i];
    let bonds = ties.iter().map(|t| t.value.max(0.0)).sum::<f32>() / ties.len().max(1) as f32;
    s[idx(Need::Belonging)] =
        (0.6 * bonds.min(1.0) + 0.4 * held(h.points[idx(Need::Belonging)])).min(1.0);
    s[idx(Need::Esteem)] = held(h.points[idx(Need::Esteem)]);
    // Nothing serves growth yet (learning and faith come later): neutral.
    s[idx(Need::Growth)] = 0.5;
    s
}

/// Over the days of `cal` (a day of time): clothes and goods wear, each
/// hearth burns the fuel the cold asks for, cold hurts, and happiness follows
/// the ladder. With `count`, people left cold add a month of person-days to
/// their town's `cold_days` (once a simulated day).
#[allow(clippy::too_many_arguments)]
pub fn daily(
    store: &mut AgentStore,
    towns: &mut Towns,
    db: &Db,
    params: &Params,
    env: &crate::env::Environment,
    lp: &LadderParams,
    cal: crate::calendar::Calendar,
    count: bool,
) {
    use sim_data::dims::state::*;
    let goods = goods(db);
    let views = town_views(towns, db, params, env);
    let rate = cal.share(params.needs.happiness_rate);
    // Wear and use-up over each good's life.
    for i in 0..store.len() {
        if !store.alive[i] {
            continue;
        }
        for e in store.owned[i].iter_mut() {
            if let Some(g) = goods.iter().find(|g| g.good == e.0 as usize) {
                if g.serves.iter().any(|s| s.by != Use::Burn) {
                    e.1 *= cal.keep(1.0 - 1.0 / g.life_days);
                }
            }
        }
    }
    let homes = Homes::of(store, &goods, lp);
    // Each hearth burns what the step's share of the days needs, fuel by fuel.
    let mut warm = vec![1.0f32; store.len()];
    for k in 0..store.len() {
        if !store.alive[k] || homes.keeper[k] != k {
            continue;
        }
        let view = &views[store.town[k] as usize];
        let size = homes.size[k].max(1.0);
        let need = cal.flow(fire_need(view.mean_c, homes.clothes[k] / size, size, lp));
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
        if w < 0.5 && count {
            towns.0[store.town[i] as usize].cold_days += cal.month_days().round() as u32;
        }
        let h = homes.holdings(store, i, &goods, lp);
        let short_c = (lp.comfort_c - (view.mean_c + lp.house_c + h.clothes)).max(0.0);
        let mut sat = assess(store, i, &h, store.wealth[i].coins(), view, lp, db, false);
        sat[idx(Need::Warmth)] = w;
        let target = happiness(&sat, lp);
        let st = &mut store.state[i];
        st[HEALTH] -= cal.flow(lp.cold_damage) * (1.0 - w) * (short_c / 10.0).min(1.5);
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
    let people: Vec<usize> = (0..store.len()).filter(|&i| store.alive[i]).collect();
    of_people(store, towns, db, params, env, lp, &people)
}

/// The satisfaction per need of `people` (by index, in a list as long as the
/// store; everyone else `None`).
pub fn of_people(
    store: &AgentStore,
    towns: &Towns,
    db: &Db,
    params: &Params,
    env: &crate::env::Environment,
    lp: &LadderParams,
    people: &[usize],
) -> Vec<Option<Sat>> {
    let mut out = vec![None; store.len()];
    if people.is_empty() {
        return out;
    }
    let goods = goods(db);
    let views = town_views(towns, db, params, env);
    let homes = Homes::for_people(store, &goods, lp, people);
    for &i in people {
        if !store.alive[i] {
            continue;
        }
        let h = homes.holdings(store, i, &goods, lp);
        out[i] = Some(assess(
            store,
            i,
            &h,
            store.wealth[i].coins(),
            &views[store.town[i] as usize],
            lp,
            db,
            false,
        ));
    }
    out
}

/// Every `interval` days (staggered over the steps), each adult buys what raises its weighted
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
    step: u64,
) {
    let goods = goods(db);
    if goods.is_empty() {
        return;
    }
    let cal = params.calendar();
    // A trip every `interval` days: each person on their own step of it.
    let iv = cal.steps(lp.interval.max(1) as f32).round().max(1.0) as u64;
    let shoppers: Vec<usize> = (0..store.len())
        .filter(|&i| {
            store.alive[i]
                && !matches!(store.activity[i], Activity::Child | Activity::Captive)
                && (step + i as u64).is_multiple_of(iv)
        })
        .collect();
    if shoppers.is_empty() {
        return;
    }
    let views = town_views(towns, db, params, env);
    let homes = Homes::for_people(store, &goods, lp, &shoppers);
    // Units bought one by one on a trip: up to 16 for each week the trip
    // covers.
    let trip_days = (iv as f32 * cal.days()).max(lp.interval.max(1) as f32);
    let max_units = (16.0 * trip_days / 7.0).ceil() as usize;
    let firms_on = params.market.firms.on();
    for i in shoppers {
        let t = store.town[i] as usize;
        let view = &views[t];
        let keeps_home = homes.keeper[i] == i;
        let size = homes.size[i].max(1.0);
        for _ in 0..max_units {
            let h = homes.holdings(store, i, &goods, lp);
            let now = score(
                &assess(store, i, &h, store.wealth[i].coins(), view, lp, db, true),
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
                if price <= 0.0 || price > store.wealth[i].coins() {
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
                    &assess(
                        store,
                        i,
                        &h2,
                        store.wealth[i].coins() - price,
                        view,
                        lp,
                        db,
                        true,
                    ),
                    lp,
                );
                // Fuel and home goods serve everyone at the hearth; the money
                // is only the buyer's.
                let saving_loss = score(
                    &assess(
                        store,
                        i,
                        &h,
                        store.wealth[i].coins() - price,
                        view,
                        lp,
                        db,
                        true,
                    ),
                    lp,
                ) - now;
                let goods_gain = after - now - saving_loss;
                let worth = if g.home { size } else { 1.0 };
                let gain = (goods_gain * worth + saving_loss) / price;
                if gain > 1e-6 && best.is_none_or(|b| gain > b.1) {
                    best = Some((k, gain));
                }
            }
            // Food for the larder, while the household is short of its target.
            let mut food_best: Option<(usize, f32)> = None;
            if lp.larder_days > 0.0 {
                for &(f, value) in &db.foods {
                    let f = f as usize;
                    let price = town.price[f];
                    if price <= 0.0 || price > store.wealth[i].coins() || town.stock[f] < 1.0 {
                        continue;
                    }
                    let mut h2 = h;
                    h2.extra_food = value;
                    // What it would add to the stores: its days of calories
                    // times what it carries of each, over each store's span.
                    let np = &params.needs.nutrition;
                    let days = value / params.needs.hunger_per_day.max(1e-6);
                    for k in 0..3 {
                        h2.extra_nutrients[k] =
                            days * db.nutrients[f][k] / np.drain_days[k].max(1.0);
                    }
                    let after = score(
                        &assess(
                            store,
                            i,
                            &h2,
                            store.wealth[i].coins() - price,
                            view,
                            lp,
                            db,
                            true,
                        ),
                        lp,
                    );
                    let gain = (after - now) / price;
                    if gain > 1e-6
                        && best.is_none_or(|b| gain > b.1)
                        && food_best.is_none_or(|b| gain > b.1)
                    {
                        food_best = Some((f, gain));
                    }
                }
            }
            if let Some((f, _)) = food_best {
                let town = &mut towns.0[t];
                let price = town.price[f];
                town.stock[f] -= 1.0;
                town.treasury += crate::coin::Coin::from_coins(price);
                town.ledger.add(crate::ledger::Flow::FoodSales, price);
                store.wealth[i] -= crate::coin::Coin::from_coins(price);
                crate::decision::subsistence::store(
                    &mut store.larder[i],
                    f as u32,
                    1.0,
                    f32::INFINITY,
                );
                continue;
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
                    town.firms[f].cash += crate::coin::Coin::from_coins(price);
                    town.firms[f].book(crate::firms::FirmFlow::DirectSales, price);
                }
                None => {
                    town.stock[p] -= 1.0;
                    town.treasury += crate::coin::Coin::from_coins(price);
                    town.ledger.add(crate::ledger::Flow::HouseholdGoods, price);
                }
            }
            town.flows.add(crate::goods::Flow::Bought, p, 1.0);
            let np = db.content.products.len();
            town.goods_today.resize(np, 0.0);
            town.index_year.consumed.resize(np, 0.0);
            town.goods_today[p] += 1.0;
            town.index_year.consumed[p] += 1.0;
            store.wealth[i] -= crate::coin::Coin::from_coins(price);
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
            larder_days: 0.0,
            cold_damage: 0.02,
            plan_days: 30.0,
        }
    }

    #[test]
    fn savings_count_months_and_the_first_counts_most() {
        let near = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert!(near(months_score(1.0), 0.27));
        assert!(near(months_score(3.0), 0.54));
        assert!(near(months_score(6.0), 0.76));
        assert!(near(months_score(12.0), 1.0) && months_score(24.0) == 1.0);
        // A coin is worth more to someone with one month than with six.
        assert!(months_slope(1.0, 10.0) > 2.0 * months_slope(6.0, 10.0));
        assert_eq!(months_slope(12.0, 10.0), 0.0);
    }

    #[test]
    fn the_slope_points_at_the_lowest_unmet_rung() {
        let lp = lp();
        // Hungry and cold: food and warmth are worth most.
        let mut sat = [0.9; N];
        sat[idx(Need::Food)] = 0.2;
        sat[idx(Need::Warmth)] = 0.3;
        let g = gradient(&sat, &lp);
        assert!(
            g[idx(Need::Food)] > 5.0 * g[idx(Need::Esteem)].abs().max(1e-3),
            "{g:?}"
        );
        // Fed and warm: the upper rungs count, and food still counts through
        // the gates it holds open.
        let full = gradient(&[0.9; N], &lp);
        assert!(full[idx(Need::Esteem)] > 0.5, "{full:?}");
        assert!(full[idx(Need::Food)] >= full[idx(Need::Esteem)], "{full:?}");
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
