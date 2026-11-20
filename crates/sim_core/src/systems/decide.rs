//! Job choice by the needs ladder.
//!
//! Each idle agent weighs resting against the jobs its town has open: an option
//! is worth what it changes in the agent's needs, each weighted by the slope of
//! its weighted satisfaction (`ladder::gradient`). The choice rule turns those
//! values into a pick (first-come places) or a ranking (deferred acceptance).

use super::streams;
use super::work::{physique_bonus, state_factor};
use crate::components::{NaturalResource, Participants, Task};
use crate::db::Db;
use crate::store::{Activity, AgentStore};
use crate::world::{Decisions, Params, SimClock, SimSeed, Towns};
use bevy_ecs::prelude::*;
use rand::Rng;
use sim_data::dims::*;
use sim_data::Category;

/// Whether a town already holds `glut_factor` times its target stock of a good
/// (or, with `stop_at_floor`, more than its target at the floor price), so
/// making more of it only piles it up.
pub fn glutted(db: &Db, town: &crate::world::Town, params: &Params, p: usize) -> bool {
    let mp = &params.market;
    over_target(db, town, params, p, mp.glut_factor)
        || (mp.stop_at_floor
            && at_floor(db, town, params, p)
            && over_target(db, town, params, p, 1.0))
}

/// The stock of a good a town's market aims to hold: per resident (more for
/// food), plus open government orders.
pub fn target_stock(db: &Db, town: &crate::world::Town, params: &Params, p: usize) -> f32 {
    let mp = &params.market;
    let def = &db.content.products[p];
    if def.category == sim_data::Category::Weapon {
        // Kit is what the government keeps in reserve: its stockpile target if
        // it set one, else enough for its soldiers and the levy it could call.
        let keep = match town.stockpile_target(p) {
            s if s > 0.0 => s,
            _ => super::military::kit_reserve(db, town, params, p),
        };
        return keep.max(1.0) + town.policy.ordered(p);
    }
    let per_capita = if db.is_food[p] {
        mp.food_target_per_capita
    } else {
        mp.target_per_capita
    };
    let base = (per_capita * town.residents.len() as f32).max(1.0) + town.policy.ordered(p);
    let stockpile = town.stockpile_target(p);
    if stockpile <= 0.0 {
        return base;
    }
    if def.category.is_item() {
        // Gear the town keeps: its armory is the market's stock of it.
        base.max(stockpile)
    } else {
        // What the town's store still lacks is wanted on top of the market's own.
        base + (stockpile - town.granary.get(p).copied().unwrap_or(0.0)).max(0.0)
    }
}

/// Whether a good's price in a town has fallen to its floor.
pub fn at_floor(db: &Db, town: &crate::world::Town, params: &Params, p: usize) -> bool {
    town.price[p] <= db.base_price[p] * params.market.min_price_factor * 1.05
}

/// Whether a town holds more than `factor` times its target stock of a good
/// (its firms' unsold goods included; a subsidized good's target is larger).
pub fn over_target(
    db: &Db,
    town: &crate::world::Town,
    params: &Params,
    p: usize,
    factor: f32,
) -> bool {
    let target = target_stock(db, town, params, p);
    // Goods the town's firms hold unsold count too.
    let unsold: f32 = town
        .firms
        .iter()
        .map(|f| f.stock.get(p).copied().unwrap_or(0.0))
        .sum();
    // A subsidized good is bought up to a larger stock.
    town.stock[p] + unsold > factor * target * (1.0 + town.policy.subsidy[p])
}

/// Whether a recipe feeds people: it makes more food than its inputs hold,
/// counting outputs that lead to food (grain to flour to bread).
pub fn makes_food(db: &Db, r: usize) -> bool {
    let m = &db.recipes[r];
    let in_food: f32 = m
        .inputs
        .iter()
        .map(|&(p, q)| db.content.products[p as usize].food * q)
        .sum();
    m.food_out > in_food
}

/// Inputs held back for `food_reserve_days` days of the food the town
/// needs, made along each food
/// chain (grain for the mills, flour and wood for the bakeries). Hungry people don't
/// eat the grain the mills need, and other trades don't take the wood the bakeries
/// need, but anything beyond that is free for other work. A recipe whose
/// output is already piling up reserves nothing.
pub fn food_reserve(db: &Db, town: &crate::world::Town, params: &Params) -> Vec<f32> {
    let need =
        town.residents.len() as f32 * params.needs.hunger_per_day * params.market.food_reserve_days;
    let mut r = vec![0.0; db.content.products.len()];
    for (k, m) in db.recipes.iter().enumerate() {
        if !makes_food(db, k)
            || m.outputs
                .iter()
                .all(|&(o, _)| glutted(db, town, params, o as usize))
        {
            continue;
        }
        let runs = need / m.food_out.max(1e-6);
        for &(p, q) in &m.inputs {
            r[p as usize] += q * runs;
        }
    }
    r
}

/// How many more workers each recipe can absorb in `town` now (each taking a
/// day's inputs and nature), limited by the
/// nature left to harvest and the inputs in stock. Jobs fill up as agents choose
/// them, so the rest of the town spreads to other work instead of queueing.
/// Nobody makes a good the town is glutted with, and trades that don't feed
/// anyone only use inputs beyond the food reserve.
pub fn job_capacity(
    db: &Db,
    town: &crate::world::Town,
    params: &Params,
    sites: &Query<&NaturalResource>,
    window_days: [f32; 2],
) -> Vec<f32> {
    let reserve = food_reserve(db, town, params);
    // Places count a day's inputs and nature per worker, as when the
    // economy ran once a simulated day: whoever takes the job draws on the
    // stock as the day's batches start, and the stock refills as others make
    // and sell (people choose once a simulated day).
    let mut nature_total = vec![0.0f32; db.content.nature.len()];
    for &e in &town.nature_sites {
        if let Ok(n) = sites.get(e) {
            nature_total[n.kind as usize] += n.amount;
        }
    }
    db.recipes
        .iter()
        .enumerate()
        .map(|(r, m)| {
            let goods = m
                .outputs
                .iter()
                .filter(|o| db.category(o.0) != Category::Building);
            let mut goods = goods.peekable();
            if goods.peek().is_some() && goods.all(|o| glutted(db, town, params, o.0 as usize)) {
                return 0.0;
            }
            if m.outputs.iter().any(|o| db.coin[o.0 as usize] > 0.0)
                && !town.mint_wanted(&params.market)
            {
                return 0.0;
            }
            let food = makes_food(db, r);
            // One worker does about 1/duration runs per day.
            let mut cap = f32::INFINITY;
            if let Some(k) = m.nature {
                let out: f32 = m.outputs.iter().map(|o| o.1).sum();
                cap = cap.min(nature_total[k as usize] / out * m.duration);
            }
            for &(p, q) in &m.inputs {
                let held = if food || !town.policy.food_reserve {
                    0.0
                } else {
                    reserve[p as usize]
                };
                // With firms, their unsold stock is on offer too.
                let have = town.stock[p as usize]
                    + if params.market.firms.on() {
                        town.firm_stock(p as usize)
                    } else {
                        0.0
                    };
                cap = cap.min((have - held).max(0.0) / q * m.duration);
            }
            // With workshops as firms, a firm offers only the places whose inputs
            // it can pay for (so nobody waits at a workshop that can't start).
            if params.market.firms.on() && !crate::firms::is_public(db, m) {
                let f = &town.firms[db.recipe_firm[r]];
                if f.closed {
                    return 0.0;
                }
                let per_batch: f32 = m
                    .inputs
                    .iter()
                    .map(|&(p, q)| town.price[p as usize] * q)
                    .sum();
                if per_batch > 0.0 {
                    let funds = f.cash.coins().max(0.0) + params.market.firms.credit_room(f);
                    cap = cap.min(funds / per_batch * m.duration);
                }
            }
            // A day of time without working hours (indoors or out, as the
            // job is: `window_days`) has no places.
            if window_days[usize::from(db.outdoor[r])] > 0.0 {
                cap
            } else {
                0.0
            }
        })
        .collect()
}

/// How many days of work one worker does over the day of time that starts at
/// this step, indoors and outdoors.
pub fn window_work_days(
    clock: &SimClock,
    env: &crate::env::Environment,
    time: &crate::env::TimeParams,
) -> [f32; 2] {
    let cal = clock.calendar();
    let k = clock.steps_per_day_of_time();
    let last = (clock.step + k).min(clock.steps_per_day);
    [false, true].map(|outdoor| {
        let (start, end) = crate::env::work_hours(env, time, outdoor);
        (clock.step..last)
            .map(|s| cal.work_share(s, start, end, time.day_length.equinox_hours))
            .sum::<f32>()
            * cal.month_days()
    })
}

/// Whether no earlier step of the simulated day had working hours (indoors
/// or out): this day of time holds the day's first choices.
fn first_choice_of_day(
    clock: &SimClock,
    env: &crate::env::Environment,
    time: &crate::env::TimeParams,
) -> bool {
    let cal = clock.calendar();
    [false, true].iter().all(|&outdoor| {
        let (start, end) = crate::env::work_hours(env, time, outdoor);
        (0..clock.step).all(|s| cal.work_share(s, start, end, time.day_length.equinox_hours) <= 0.0)
    })
}

/// Recipes a town can start right now (building, nature and inputs available).
pub fn feasible_recipes(
    db: &Db,
    town: &crate::world::Town,
    sites: &Query<&NaturalResource>,
) -> Vec<u32> {
    let mut nature_avail = vec![0.0f32; db.content.nature.len()];
    for &e in &town.nature_sites {
        if let Ok(n) = sites.get(e) {
            nature_avail[n.kind as usize] = nature_avail[n.kind as usize].max(n.amount);
        }
    }
    (0..db.recipes.len() as u32)
        .filter(|&r| {
            let m = &db.recipes[r as usize];
            let building_ok = m.building.is_none_or(|b| town.building(b).is_some());
            let out: f32 = m.outputs.iter().map(|o| o.1).sum();
            let nature_ok = m.nature.is_none_or(|k| nature_avail[k as usize] >= out);
            let inputs_ok = m.inputs.iter().all(|&(p, q)| town.stock[p as usize] >= q);
            building_ok && nature_ok && inputs_ok
        })
        .collect()
}

/// Expected profit per person-day of running recipe `r` in `town` at current prices.
pub fn profit_per_day(db: &Db, town: &crate::world::Town, r: usize) -> f32 {
    let m = &db.recipes[r];
    // Subsidies count as part of the value for whoever chooses the work.
    let out: f32 = m
        .outputs
        .iter()
        .map(|&(p, q)| {
            let p = p as usize;
            (town.price[p] + town.policy.subsidy[p] * db.base_price[p]) * q
        })
        .sum();
    let inp: f32 = m
        .inputs
        .iter()
        .map(|&(p, q)| town.price[p as usize] * q)
        .sum();
    (out - inp) / m.duration
}

/// What every agent's job choice reads; shared by the parallel pass.
struct Choice<'a> {
    db: &'a Db,
    params: &'a Params,
    feasible: &'a [Vec<u32>],
    decision: &'a crate::decision::DecisionModels,
    models: &'a crate::models::Models,
    towns: &'a Towns,
    /// Simulated days (months) a year, for ages.
    per_year: u32,
    /// Everyone's satisfaction per need, and the ladder's parameters.
    sats: &'a [Option<crate::ladder::Sat>],
    /// Per town: food, month and hunger costs for the savings slope.
    savings_costs: &'a [[f32; 3]],
    ladder: &'a crate::ladder::LadderParams,
}

impl Choice<'_> {
    /// How much agent `i` would like resting (first) and each job in `jobs`, when
    /// it would get `pay_factor` of a worker's pay (an apprentice place pays
    /// less). A score depends only on the agent and the job, so every agent's
    /// scores can be worked out in parallel before anyone picks.
    ///
    /// An option is worth what it changes in the person's needs, each weighted
    /// by the slope of their weighted satisfaction (`ladder::gradient`). A day's
    /// work brings its expected pay (and what the practice will earn later),
    /// worth what a coin buys at the margin: a meal for the hungry, savings for
    /// the fed; it costs rest. Resting gives rest.
    fn scores(
        &self,
        store: &AgentStore,
        i: usize,
        jobs: &[u32],
        pay_factor: f32,
    ) -> (f32, Vec<f32>) {
        use crate::decision::expectations;
        use sim_data::Need;
        let (db, params) = (self.db, self.params);
        let d = &params.decision;
        let t = store.town[i] as usize;
        let town = &self.towns.0[t];
        let st = store.state[i];
        let sat = self.sats.get(i).copied().flatten().unwrap_or([0.5; 9]);
        let lp = self.ladder;
        let g = crate::ladder::gradient(&sat, lp);
        let at = |n: Need| Need::ALL.iter().position(|&m| m == n).unwrap();
        // What a coin is worth: the best of a meal's worth of food (hunger is
        // `1 - food`) and savings (days of food covered).
        let per_hunger = db
            .foods
            .iter()
            .filter(|(p, _)| town.stock[*p as usize] >= 1.0)
            .map(|&(p, v)| town.price[p as usize] / v.max(1e-3))
            .fold(f32::INFINITY, f32::min);
        let per_hunger = if per_hunger.is_finite() {
            per_hunger
        } else {
            1.0
        };
        let coin_food = g[at(Need::Food)] / per_hunger.max(1e-3);
        let costs = self
            .savings_costs
            .get(t)
            .copied()
            .unwrap_or([1.0, 30.0, 0.12]);
        let coin_save =
            g[at(Need::Savings)] * crate::ladder::savings_slope(store, i, costs, db, lp);
        let coin = coin_food.max(coin_save).max(0.0);
        let rest_slope = g[at(Need::Rest)].max(0.0);
        let rest = rest_slope * params.needs.fatigue_rest_recovery.min(st[state::FATIGUE]);
        let sf = state_factor(&st, &params.needs);
        let job_scores = jobs
            .iter()
            .map(|&r| {
                let m = &db.recipes[r as usize];
                let prof = dot(&store.cap[i], &m.skill) + dot(&store.equip_vec[i], &m.skill);
                let eff = (prof + physique_bonus(&store.phys[i], m, params.physique.weight)) * sf;
                let success = self.models.success.probability(eff, m.difficulty);
                let pay = pay_factor
                    * expectations::expected_pay(
                        &store.pay_expect[i],
                        r,
                        town.pay_rate[r as usize],
                    );
                let crate::config::LearningParams::Diminishing {
                    rate,
                    mastery,
                    failure_factor,
                } = params.learning;
                let room = (1.0 - prof / mastery).max(0.0);
                let mult = store.aptitude[i] * (0.5 + st[state::HAPPINESS]);
                let practice = rate * mult * room * (success + (1.0 - success) * failure_factor);
                let after = self
                    .models
                    .success
                    .probability(eff + practice * sf, m.difficulty);
                let learned =
                    self.decision
                        .learning
                        .value(&crate::decision::learning::LearnInputs {
                            success_now: success,
                            success_after: after,
                            pay_per_day: pay,
                            typical_success: d.valuation.typical_success,
                            age_years: store.age_years(i, self.per_year),
                        });
                let tired = params.needs.fatigue_per_work_day * (0.5 + m.phys_total);
                d.ladder_scale * (coin * (pay + learned) - rest_slope * tired)
            })
            .collect();
        (d.ladder_scale * rest, job_scores)
    }

    /// Agent `i` weighs a random sample of the jobs still open (plus its habitual
    /// job, if still open) against resting, and picks one by softmax. `None` is
    /// rest. `score(r)` looks up the agent's score for job `r`.
    #[allow(clippy::too_many_arguments)]
    fn pick(
        &self,
        store: &AgentStore,
        i: usize,
        open_jobs: &[u32],
        open: impl Fn(u32) -> bool,
        rest: f32,
        score: impl Fn(u32) -> f32,
        pool: &mut Vec<u32>,
        rng: &mut impl Rng,
    ) -> Option<u32> {
        let d = &self.params.decision;
        let t = store.town[i] as usize;
        pool.clear();
        pool.extend_from_slice(open_jobs);
        let k = d.candidates.min(pool.len());
        for j in 0..k {
            let swap = rng.random_range(j..pool.len());
            pool.swap(j, swap);
        }
        pool.truncate(k);
        if let Some(last) = store.last_recipe[i] {
            if open(last) && self.feasible[t].contains(&last) && !pool.contains(&last) {
                pool.push(last);
            }
        }
        // The choice rule picks among rest and the candidates.
        let options: Vec<(u32, f32)> = pool.iter().map(|&r| (r, score(r))).collect();
        let rng: &mut dyn rand::RngCore = rng;
        self.decision
            .choice
            .pick(rest, &options, &store.mind[i], rng)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn decide(
    mut store: ResMut<AgentStore>,
    mut towns: ResMut<Towns>,
    db: Res<Db>,
    params: Res<Params>,
    clock: Res<SimClock>,
    seed: Res<SimSeed>,
    sites: Query<&NaturalResource>,
    tasks: Query<(&Task, &Participants)>,
    mut decisions: ResMut<Decisions>,
    mut log: ResMut<crate::world::WorkLog>,
    models: (
        Res<crate::decision::DecisionModels>,
        Res<crate::models::Models>,
        Res<crate::env::Environment>,
    ),
) {
    let (decision, models, env) = models;
    decisions.jobs.clear();
    decisions.apprentices.clear();
    let mastery = params.mastery();
    // People choose their work once a day of time, at its first step, as
    // when a step was a day: capacity, pay and places are judged for the
    // day's work, by the day's calendar. In its other steps the idle who
    // chose a job today go back to it, and whoever chose to rest rests until
    // the next day's choices.
    let step_cal = params.calendar();
    if !clock.work_day_start() {
        for i in 0..store.len() {
            if store.alive[i] && store.activity[i] == Activity::Idle {
                if let Some((r, apprentice)) = store.job_today[i] {
                    // Not into a glut: once what the job makes piles up,
                    // the rest of the day is off.
                    let town = &towns.0[store.town[i] as usize];
                    let mut goods = db.recipes[r as usize]
                        .outputs
                        .iter()
                        .filter(|o| db.category(o.0) != Category::Building)
                        .peekable();
                    if goods.peek().is_some()
                        && goods.all(|o| glutted(&db, town, &params, o.0 as usize))
                    {
                        store.job_today[i] = None;
                        continue;
                    }
                    decisions.jobs.push((i as u32, r));
                    if apprentice {
                        decisions.apprentices.push(i as u32);
                    }
                }
            }
        }
        return;
    }
    let k = clock.steps_per_day_of_time();
    let cal = step_cal.every(k);
    store.job_today.iter_mut().for_each(|j| *j = None);
    // A day of time in the night hours has no work to choose (nor a rest
    // to take): people stay as they are until the next.
    let window_days = window_work_days(&clock, &env, &params.time);
    if window_days.iter().all(|&d| d <= 0.0) {
        return;
    }
    // A rest lasts until the next day's choices: it restores for each day of
    // work it skips, from this step on.
    let rest_until = clock.steps() + k as u64;
    let rest_now = crate::env::work_days_this_step(&clock, &env, &params.time, false)
        * params.needs.fatigue_rest_recovery;
    // The people choosing now: those free of a rest, and those at work, who
    // finish the batch under way and then go to what they chose.
    let idle: Vec<usize> = (0..store.len())
        .filter(|&i| {
            store.alive[i] && matches!(store.activity[i], Activity::Idle | Activity::Working(_))
        })
        .collect();

    // Per-town context, computed once.
    let mut capacity: Vec<Vec<f32>> = towns
        .0
        .iter()
        .map(|t| job_capacity(&db, t, &params, &sites, window_days))
        .collect();
    // Building slots: a building holds `slots` workers, including those on
    // multi-day tasks. Only one crew at a time builds each building type.
    let np = db.content.products.len();
    let mut slots_total = vec![vec![f32::INFINITY; np]; towns.0.len()];
    for (t, town) in towns.0.iter().enumerate() {
        for (p, total) in slots_total[t].iter_mut().enumerate() {
            let slots = db.content.products[p].slots;
            if db.category(p as u32) == sim_data::Category::Building && slots > 0 {
                *total = (slots as usize * town.building_count(p as u32)) as f32;
            }
        }
    }
    // Every place is open: the people at work choose today too (they take
    // their place back by choosing it again).
    let mut slots_left = slots_total.clone();
    for (task, _) in tasks.iter() {
        let m = &db.recipes[task.recipe as usize];
        let t = task.town as usize;
        if m.builds.is_some() {
            capacity[t][task.recipe as usize] = 0.0;
        }
    }
    for (t, town) in towns.0.iter().enumerate() {
        for (r, m) in db.recipes.iter().enumerate() {
            if let Some(b) = m.builds {
                capacity[t][r] = if town.price[b as usize] > 0.0 {
                    capacity[t][r].min(m.max_team as f32)
                } else {
                    0.0
                };
            }
        }
    }
    let open = |slots_left: &[Vec<f32>], capacity: &[Vec<f32>], t: usize, r: u32| {
        capacity[t][r as usize] >= 1.0
            && db.recipes[r as usize]
                .building
                .is_none_or(|b| slots_left[t][b as usize] >= 1.0)
    };

    let feasible: Vec<Vec<u32>> = towns
        .0
        .iter()
        .map(|t| feasible_recipes(&db, t, &sites))
        .collect();
    // Jobs each town can still take people for, in `feasible` order; shrinks as
    // choices fill capacity and building slots, instead of re-filtering per agent.
    let mut open_jobs: Vec<Vec<u32>> = feasible
        .iter()
        .enumerate()
        .map(|(t, f)| {
            f.iter()
                .copied()
                .filter(|&r| open(&slots_left, &capacity, t, r))
                .collect()
        })
        .collect();

    // Everyone's satisfaction per need, once, on the ladder households buy by
    // (its defaults under another consumption model).
    let default_ladder;
    let ladder_params = match &params.market.consumption {
        crate::consumption::ConsumptionParams::Needs(lp) => lp,
        _ => {
            default_ladder = crate::ladder::LadderParams::default();
            &default_ladder
        }
    };
    // Only the people choosing now need their satisfaction.
    let sats = crate::ladder::of_people(&store, &towns, &db, &params, &env, ladder_params, &idle);
    let savings_costs = crate::ladder::savings_costs(&towns, &db, &params, &env);
    let ctx = Choice {
        db: &db,
        params: &params,
        feasible: &feasible,
        decision: &decision,
        models: &models,
        towns: &towns,
        per_year: clock.months_per_year,
        sats: &sats,
        savings_costs: &savings_costs,
        ladder: ladder_params,
    };

    // Places each job had at the start of the step, for the wage factors.
    let places_start: Vec<Vec<f32>> = (0..towns.0.len())
        .map(|t| {
            (0..db.recipes.len())
                .map(|r| {
                    let mut p = capacity[t][r].max(0.0);
                    if let Some(b) = db.recipes[r].building {
                        p = p.min(slots_left[t][b as usize].max(0.0));
                    }
                    if p.is_finite() {
                        p
                    } else {
                        0.0
                    }
                })
                .collect()
        })
        .collect();

    // Phase 1, in parallel: every idle agent scores the jobs its town has open at
    // the start of the step. Jobs only close during the step, so these scores
    // cover everything an agent can still pick.
    let store = &mut *store;
    // Position of each recipe in its town's opening list.
    let slot_of: Vec<Vec<u32>> = open_jobs
        .iter()
        .map(|jobs| {
            let mut v = vec![u32::MAX; db.recipes.len()];
            for (k, &r) in jobs.iter().enumerate() {
                v[r as usize] = k as u32;
            }
            v
        })
        .collect();
    let scores: Vec<(f32, Vec<f32>)> = {
        let store = &*store;
        let open_jobs = &open_jobs;
        super::par_map(&idle, |&i| {
            ctx.scores(store, i, &open_jobs[store.town[i] as usize], 1.0)
        })
    };

    // How many wanted each job today (for the wage factors).
    let mut demand = vec![vec![0.0f32; db.recipes.len()]; towns.0.len()];
    let matching = &params.decision.matching;
    if let crate::decision::matching::MatchingParams::DeferredAcceptance {
        apprentice_wage,
        share: share_params,
    } = matching
    {
        use crate::decision::matching::{deferred_acceptance, Opt, Places};
        let n_towns = towns.0.len();
        // Places per job at the start of the day, split into regular and apprentice.
        let places: Vec<Vec<Places>> = (0..n_towns)
            .map(|t| {
                open_jobs[t]
                    .iter()
                    .map(|&r| {
                        let total = places_start[t][r as usize].floor().max(0.0) as u32;
                        let share = towns.0[t]
                            .apprentice_share
                            .get(r as usize)
                            .copied()
                            .unwrap_or(0.0);
                        let app = (total as f32 * share).floor() as u32;
                        Places {
                            regular: total - app,
                            apprentice: app,
                        }
                    })
                    .collect()
            })
            .collect();
        // Every idle agent ranks its options (regular and apprentice places above
        // resting), in parallel, with its own random stream.
        let lists: Vec<Vec<Opt>> = {
            let store = &*store;
            let (open_jobs, places) = (&open_jobs, &places);
            let pairs: Vec<(usize, &(f32, Vec<f32>))> = idle.iter().copied().zip(&scores).collect();
            super::par_map(&pairs, |&(i, (rest, job_scores))| {
                let t = store.town[i] as usize;
                let jobs = &open_jobs[t];
                let app_scores = ctx.scores(store, i, jobs, *apprentice_wage).1;
                let mut options = Vec::with_capacity(jobs.len() * 2);
                for (k, &v) in job_scores.iter().enumerate() {
                    options.push((2 * k as u32, v));
                    if places[t][k].apprentice > 0 {
                        options.push((2 * k as u32 + 1, app_scores[k]));
                    }
                }
                let mut rng = seed.agent_rng(clock.steps(), streams::DECIDE, i);
                let rng: &mut dyn rand::RngCore = &mut rng;
                let trade = store.guild[i].map(|g| g.domain);
                let domain = |id: u32| db.recipes[jobs[(id / 2) as usize] as usize].domain;
                params
                    .decision
                    .attendance
                    .rank(trade, options, domain, |o| {
                        decision.choice.rank(*rest, o, &store.mind[i], rng)
                    })
                    .into_iter()
                    .map(|id| Opt {
                        job: id / 2,
                        apprentice: id % 2 == 1,
                    })
                    .collect()
            })
        };
        // Stable matching per town: regular places go to the best hands, apprentice
        // places to those with the most to learn.
        let mut matched: Vec<Option<Opt>> = vec![None; idle.len()];
        for t in 0..n_towns {
            let members: Vec<usize> = (0..idle.len())
                .filter(|&k| store.town[idle[k]] as usize == t)
                .collect();
            if members.is_empty() {
                continue;
            }
            let town_lists: Vec<Vec<Opt>> = members.iter().map(|&k| lists[k].clone()).collect();
            for l in &town_lists {
                if let Some(first) = l.first() {
                    demand[t][open_jobs[t][first.job as usize] as usize] += 1.0;
                }
            }
            let store_ref = &*store;
            let jobs = &open_jobs[t];
            let rank = |a: usize, opt: Opt| {
                let i = idle[members[a]];
                let m = &db.recipes[jobs[opt.job as usize] as usize];
                let prof =
                    dot(&store_ref.cap[i], &m.skill) + dot(&store_ref.equip_vec[i], &m.skill);
                if opt.apprentice {
                    store_ref.aptitude[i] * (1.0 - prof / mastery).max(0.0)
                } else {
                    prof - m.difficulty
                }
            };
            let result = deferred_acceptance(&town_lists, &places[t], rank);
            for (a, m) in result.into_iter().enumerate() {
                matched[members[a]] = m;
            }
        }
        // Building slots are shared by the jobs inside: keep the first matches.
        let mut used: Vec<Vec<f32>> = slots_left.iter().map(|v| vec![0.0; v.len()]).collect();
        for (k, m) in matched.iter_mut().enumerate() {
            let Some(opt) = *m else { continue };
            let t = store.town[idle[k]] as usize;
            let r = open_jobs[t][opt.job as usize];
            if let Some(b) = db.recipes[r as usize].building {
                if used[t][b as usize] + 1.0 > slots_left[t][b as usize] {
                    *m = None;
                    continue;
                }
                used[t][b as usize] += 1.0;
            }
        }
        // Take the jobs; whoever has no match rests.
        for (k, m) in matched.into_iter().enumerate() {
            let i = idle[k];
            let t = store.town[i] as usize;
            match m {
                Some(opt) => {
                    let r = open_jobs[t][opt.job as usize];
                    capacity[t][r as usize] -= 1.0;
                    if let Some(b) = db.recipes[r as usize].building {
                        slots_left[t][b as usize] -= 1.0;
                    }
                    log.chosen[r as usize] += 1;
                    store.job_today[i] = Some((r, opt.apprentice));
                    if store.activity[i] == Activity::Idle {
                        decisions.jobs.push((i as u32, r));
                        if opt.apprentice {
                            decisions.apprentices.push(i as u32);
                        }
                    }
                }
                None if store.activity[i] != Activity::Idle => {}
                None => {
                    let mut rng = seed.agent_rng(clock.steps(), streams::DECIDE + 100, i);
                    store.activity[i] = Activity::Resting;
                    store.rest_until[i] = rest_until;
                    // This step's rest (needs::sense has run already).
                    let st = &mut store.state[i];
                    st[state::FATIGUE] = (st[state::FATIGUE] - rest_now).max(0.0);
                    let c = towns.0[t].pos;
                    store.target[i] = (
                        c.0 + rng.random_range(-45.0..45.0f32),
                        c.1 + rng.random_range(-45.0..45.0f32),
                    );
                }
            }
        }
        decisions.apprentices.sort_unstable();
        // Days outside one's trade: a trigger to change it (systems::organize).
        // A day of rest by choice does not count; a day of other work, or of
        // wanting work and finding none, does.
        for (k, &i) in idle.iter().enumerate() {
            let Some(g) = store.guild[i] else { continue };
            let off = match store.job_today[i] {
                Some((r, _)) => db.recipes[r as usize].domain != g.domain,
                None => !lists[k].is_empty(),
            };
            store.off_trade[i] = if off {
                store.off_trade[i].saturating_add(1)
            } else {
                0
            };
        }

        // Every 30 days every workplace reconsiders its share of apprentice
        // places, at the day's first choices (there are none at night).
        let months = (30 / clock.days_per_month.max(1)).max(1) as u64;
        if clock.tick.is_multiple_of(months) && first_choice_of_day(&clock, &env, &params.time) {
            for town in towns.0.iter_mut() {
                town.apprentice_share
                    .resize(db.recipes.len(), share_params.min);
                let base = town.policy.wage_share.unwrap_or(params.work.wage_share);
                for r in 0..db.recipes.len() {
                    let factor = town.wage_factor.get(r).copied().unwrap_or(1.0);
                    let gain = share_params.gain(
                        (base * factor).min(1.0),
                        *apprentice_wage,
                        (factor - 1.0).max(0.0),
                    );
                    town.apprentice_share[r] = share_params.adjust(town.apprentice_share[r], gain);
                }
            }
        }
    } else {
        // Phase 2, in agent order: pick among the jobs still open, each agent with a
        // random stream of its own, and take the job.
        let mut pool: Vec<u32> = Vec::new();
        for (&i, (rest, job_scores)) in idle.iter().zip(&scores) {
            let t = store.town[i] as usize;
            let mut rng = seed.agent_rng(clock.steps(), streams::DECIDE, i);
            // Under trade attendance, a person with a trade weighs its open
            // jobs only, and any job when it has none open (day labour).
            let trade = store.guild[i]
                .map(|g| g.domain)
                .filter(|_| params.decision.attendance == crate::config::Attendance::Trade);
            let own: Vec<u32> = trade.map_or_else(Vec::new, |d| {
                open_jobs[t]
                    .iter()
                    .copied()
                    .filter(|&r| db.recipes[r as usize].domain == d)
                    .collect()
            });
            let jobs: &[u32] = if own.is_empty() { &open_jobs[t] } else { &own };
            let choice = ctx.pick(
                store,
                i,
                jobs,
                |r| open(&slots_left, &capacity, t, r),
                *rest,
                |r| job_scores[slot_of[t][r as usize] as usize],
                &mut pool,
                &mut rng,
            );
            match choice {
                None if store.activity[i] != Activity::Idle => {}
                None => {
                    store.activity[i] = Activity::Resting;
                    store.rest_until[i] = rest_until;
                    // This step's rest (needs::sense has run already).
                    let st = &mut store.state[i];
                    st[state::FATIGUE] = (st[state::FATIGUE] - rest_now).max(0.0);
                    let c = towns.0[t].pos;
                    store.target[i] = (
                        c.0 + rng.random_range(-45.0..45.0f32),
                        c.1 + rng.random_range(-45.0..45.0f32),
                    );
                }
                Some(r) => {
                    capacity[t][r as usize] -= 1.0;
                    let building = db.recipes[r as usize].building;
                    if let Some(b) = building {
                        slots_left[t][b as usize] -= 1.0;
                    }
                    if capacity[t][r as usize] < 1.0
                        || building.is_some_and(|b| slots_left[t][b as usize] < 1.0)
                    {
                        open_jobs[t].retain(|&q| open(&slots_left, &capacity, t, q));
                    }
                    log.chosen[r as usize] += 1;
                    store.job_today[i] = Some((r, false));
                    if store.activity[i] == Activity::Idle {
                        decisions.jobs.push((i as u32, r));
                    }
                }
            }
        }
        for &i in &idle {
            if let Some((r, _)) = store.job_today[i] {
                demand[store.town[i] as usize][r as usize] += 1.0;
            }
            // Days of work outside one's trade (systems::organize).
            if let Some(g) = store.guild[i] {
                store.off_trade[i] = match store.job_today[i] {
                    Some((r, _)) if db.recipes[r as usize].domain != g.domain => {
                        store.off_trade[i].saturating_add(1)
                    }
                    _ => 0,
                };
            }
        }
    }

    // Places each trade left unfilled today, averaged over about a month.
    let nd = db.domain_dirs.len();
    for (t, town) in towns.0.iter_mut().enumerate() {
        let mut open = vec![0.0f32; nd];
        for (r, m) in db.recipes.iter().enumerate() {
            if m.builds.is_none() {
                open[m.domain as usize] += capacity[t][r].clamp(0.0, places_start[t][r]);
            }
        }
        town.vacancies.resize(nd, 0.0);
        for (v, o) in town.vacancies.iter_mut().zip(open) {
            *v += (o - *v) * cal.smoothing(30.0);
        }
    }

    // Each job's pay moves with how many took it against its places.
    let takers = demand;
    for (t, town) in towns.0.iter_mut().enumerate() {
        town.wage_factor.resize(db.recipes.len(), 1.0);
        for r in 0..db.recipes.len() {
            let places = places_start[t][r];
            if places >= 1.0 || takers[t][r] > 0.0 {
                let old = town.wage_factor[r];
                let mut new = decision
                    .wages
                    .adjust(old, takers[t][r], places.max(takers[t][r]));
                // Labour's price class: wages are sticky, especially downward.
                if let Some(classes) = &params.market.price_classes {
                    let c = &classes.labour;
                    if old > 0.0 && new > 0.0 && new != old {
                        let mut d = (new / old).ln();
                        if let crate::pricing::Inertia::Sticky { up, down } = c.inertia {
                            if d < 0.0 && up > 0.0 {
                                d *= down / up;
                            }
                        }
                        let m = cal.flow(c.max_daily).max(0.0);
                        new = (old * d.clamp(-m, m).exp()).clamp(c.floor, c.ceiling.max(c.floor));
                    }
                }
                if old > 0.0 && new > 0.0 && new != old {
                    town.price_vol.resize(3, Default::default());
                    town.price_vol[2].add((new / old).ln().abs() / cal.days());
                }
                town.wage_factor[r] = new;
            }
        }
    }

    // How full each building type has been lately (a month-long moving average):
    // the signal for construction, smoothed so a busy week doesn't start a mill.
    for (t, town) in towns.0.iter_mut().enumerate() {
        for p in 0..np {
            let total = slots_total[t][p];
            let today = if total.is_finite() && total > 0.0 {
                1.0 - slots_left[t][p] / total
            } else {
                0.0
            };
            town.building_load[p] += (today - town.building_load[p]) * cal.smoothing(30.0);
        }
    }
}
