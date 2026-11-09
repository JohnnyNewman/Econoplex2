//! Decisions as linear algebra.
//!
//! Every option has a feature vector f = [value, skill, effort, novelty, social, need].
//! An agent's preference weights are w = bias + Wm·mind + Ws·state, so personality
//! is stable while hunger and fatigue shift priorities. Utility u = w·f, and the
//! choice is a softmax over a short candidate list (temperature = bounded rationality).

use super::streams;
use super::work::{physique_bonus, state_factor};
use crate::components::{NaturalResource, Participants, Task};
use crate::config::FEATURES;
use crate::db::Db;
use crate::store::{Activity, AgentStore};
use crate::world::{Decisions, Params, SimClock, SimSeed, Towns};
use bevy_ecs::prelude::*;
use rand::Rng;
use rayon::prelude::*;
use sim_data::dims::*;
use sim_data::Category;

/// Whether a town already holds `glut_factor` times its target stock of a good,
/// so making more of it only piles it up.
pub fn glutted(db: &Db, town: &crate::world::Town, params: &Params, p: usize) -> bool {
    let mp = &params.market;
    let per_capita = if db.is_food[p] {
        mp.food_target_per_capita
    } else {
        mp.target_per_capita
    };
    let target = (per_capita * town.residents.len() as f32).max(1.0) + town.policy.ordered(p);
    // Goods the town's firms hold unsold count too.
    let unsold: f32 = town
        .firms
        .iter()
        .map(|f| f.stock.get(p).copied().unwrap_or(0.0))
        .sum();
    // A subsidized good is bought up to a larger stock.
    town.stock[p] + unsold > mp.glut_factor * target * (1.0 + town.policy.subsidy[p])
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

/// Inputs held back for `food_reserve_days` of the food the town needs, made along each food
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

/// How many more workers each recipe can absorb in `town` today, limited by the
/// nature left to harvest and the inputs in stock. Jobs fill up as agents choose
/// them, so the rest of the town spreads to other work instead of queueing.
/// Nobody makes a good the town is glutted with, and trades that don't feed
/// anyone only use inputs beyond the food reserve.
pub fn job_capacity(
    db: &Db,
    town: &crate::world::Town,
    params: &Params,
    sites: &Query<&NaturalResource>,
) -> Vec<f32> {
    let reserve = food_reserve(db, town, params);
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
                let held = if food { 0.0 } else { reserve[p as usize] };
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
                    cap = cap.min(f.cash.max(0.0) / per_batch * m.duration);
                }
            }
            cap
        })
        .collect()
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

pub fn preference_weights(params: &Params, mind: &MindVec, state: &StateVec) -> [f32; FEATURES] {
    let d = &params.decision;
    std::array::from_fn(|f| {
        let m: f32 = d.mind_weights[f].iter().zip(mind).map(|(w, x)| w * x).sum();
        let s: f32 = d.state_weights[f]
            .iter()
            .zip(state)
            .map(|(w, x)| w * x)
            .sum();
        d.bias[f] + m + s
    })
}

/// What every agent's job choice reads; shared by the parallel pass.
struct Choice<'a> {
    db: &'a Db,
    params: &'a Params,
    feasible: &'a [Vec<u32>],
    profit: &'a [Vec<f32>],
    mean_profit: &'a [f32],
    scarcity: &'a [f32],
    charters: &'a [Vec<bool>],
    mastery: f32,
    decision: &'a crate::decision::DecisionModels,
    models: &'a crate::models::Models,
    towns: &'a Towns,
    /// Median resident wealth per town (for standing).
    median_wealth: &'a [f32],
    days_per_year: u32,
}

impl Choice<'_> {
    /// How much agent `i` would like resting (first) and each job in `jobs`.
    /// A score depends only on the agent and the job, so every agent's scores can
    /// be worked out in parallel before anyone picks.
    fn scores(&self, store: &AgentStore, i: usize, jobs: &[u32]) -> (f32, Vec<f32>) {
        if self.params.decision.model == crate::config::DecisionModel::Utility {
            return self.utility_scores(store, i, jobs);
        }
        let (db, params) = (self.db, self.params);
        let d = &params.decision;
        let t = store.town[i] as usize;
        let st = store.state[i];
        let w = preference_weights(params, &store.mind[i], &st);
        let sf = state_factor(&st, &params.needs);
        let fatigue = st[state::FATIGUE];
        let rest = [0.0, 0.0, 1.5 * fatigue, 0.0, 0.0, fatigue];
        let job_scores = jobs
            .iter()
            .map(|&r| {
                let m = &db.recipes[r as usize];
                // Effective capability (own plus tool) along the recipe's skill.
                let prof = dot(&store.cap[i], &m.skill) + dot(&store.equip_vec[i], &m.skill);
                let eff = (prof + physique_bonus(&store.phys[i], m, params.physique.weight)) * sf;
                let margin = eff - m.difficulty;
                let in_guild = store.guild[i].is_some_and(|g| g.domain == m.domain);
                // A chartered guild draws people into its trade, members or not.
                let guild_pull = if in_guild {
                    1.0
                } else if self.charters[t][m.domain as usize] {
                    params.policy.charter_pull
                } else {
                    0.0
                };
                let f = [
                    ((self.profit[t][r as usize] - self.mean_profit[t]) / d.value_scale).tanh()
                        * crate::models::sigmoid(6.0 * margin),
                    // Centered success chance: rewards fit, not trivially easy work.
                    crate::models::sigmoid(6.0 * margin) - 0.5,
                    -(0.3 + m.phys_total) * (0.5 + fatigue),
                    (1.0 - (prof / self.mastery).min(1.0)) * if margin > -0.3 { 1.0 } else { 0.2 },
                    guild_pull,
                    m.food_out * 0.25 * self.scarcity[t] * (0.5 + st[state::HUNGER]),
                ];
                dot(&w, &f)
            })
            .collect();
        (dot(&w, &rest), job_scores)
    }

    /// The utility model: each option's expected utility of the agent's needs over
    /// the coming weeks (see `decision::valuation`).
    fn utility_scores(&self, store: &AgentStore, i: usize, jobs: &[u32]) -> (f32, Vec<f32>) {
        self.utility_scores_paid(store, i, jobs, 1.0)
    }

    /// Utility scores when the agent would get `pay_factor` of a worker's pay (an
    /// apprentice place pays less).
    fn utility_scores_paid(
        &self,
        store: &AgentStore,
        i: usize,
        jobs: &[u32],
        pay_factor: f32,
    ) -> (f32, Vec<f32>) {
        use crate::decision::{expectations, utility, valuation};
        let (db, params) = (self.db, self.params);
        let d = &params.decision;
        let t = store.town[i] as usize;
        let st = store.state[i];
        let weights = utility::need_weights(&d.utility, &store.mind[i]);
        // Wealth after the rent of the coming weeks.
        let rent = self.towns.0[t].rent * d.valuation.horizon_weight();
        let situation = valuation::Situation {
            state: &st,
            wealth: (store.wealth[i] - rent).max(0.0),
            median_wealth: self.median_wealth[t],
            weights: &weights,
        };
        let u = &*self.decision.utility;
        let sf = state_factor(&st, &params.needs);
        let rest = valuation::rest_value(u, &situation, params.needs.fatigue_rest_recovery);
        let job_scores = jobs
            .iter()
            .map(|&r| {
                let m = &db.recipes[r as usize];
                let prof = dot(&store.cap[i], &m.skill) + dot(&store.equip_vec[i], &m.skill);
                let eff = (prof + physique_bonus(&store.phys[i], m, params.physique.weight)) * sf;
                let success = self.models.success.probability(eff, m.difficulty);
                let town = &self.towns.0[t];
                let pay = pay_factor
                    * match d.valuation.pay_basis {
                        valuation::PayBasis::Average => expectations::expected_pay(
                            &store.pay_expect[i],
                            r,
                            town.pay_rate[r as usize],
                        ),
                        basis => basis.typical_pay(
                            town.pay_rate[r as usize],
                            town.success_pay.get(r as usize).copied().unwrap_or(0.0),
                            d.valuation.typical_success,
                        ),
                    };
                let fatigue = params.needs.fatigue_per_work_day * (0.5 + m.phys_total);
                // A day of practice raises the agent's skill along the recipe by about
                // rate x aptitude x room left x (success, failures teaching less).
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
                            age_years: store.age_years(i, self.days_per_year),
                        });
                // Food the household would keep from a day of this job, in meals.
                let keep = d.subsistence.share();
                let cover = if keep > 0.0 {
                    let (meals, relief) = m
                        .outputs
                        .iter()
                        .filter(|o| db.is_food[o.0 as usize])
                        .fold((0.0, 0.0f32), |(n, r), &(p, q)| {
                            let v = db.foods.iter().find(|f| f.0 == p).map_or(0.0, |f| f.1);
                            (n + q, r.max(v))
                        });
                    crate::decision::subsistence::food_cover(
                        keep * meals / m.duration.max(1e-3),
                        relief,
                        params.needs.hunger_per_day,
                    )
                } else {
                    0.0
                };
                valuation::job_value_fed(
                    u,
                    &d.valuation,
                    &situation,
                    pay,
                    success,
                    fatigue,
                    learned,
                    cover,
                )
            })
            .collect();
        (rest, job_scores)
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
    ),
) {
    let (decision, models) = models;
    decisions.jobs.clear();
    decisions.apprentices.clear();
    // Median resident wealth per town, for the standing need.
    let median_wealth: Vec<f32> = towns
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
    let mastery = params.mastery();

    // Per-town context, computed once.
    let mut capacity: Vec<Vec<f32>> = towns
        .0
        .iter()
        .map(|t| job_capacity(&db, t, &params, &sites))
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
    let mut slots_left = slots_total.clone();
    for (task, parts) in tasks.iter() {
        let m = &db.recipes[task.recipe as usize];
        let t = task.town as usize;
        if let Some(b) = m.building {
            slots_left[t][b as usize] -= parts.workers.len() as f32;
        }
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
    let profit: Vec<Vec<f32>> = towns
        .0
        .iter()
        .map(|t| {
            (0..db.recipes.len())
                .map(|r| profit_per_day(&db, t, r))
                .collect()
        })
        .collect();
    // Opportunity cost: the average profit of the jobs a town can offer right now.
    let mean_profit: Vec<f32> = feasible
        .iter()
        .zip(&profit)
        .map(|(f, p)| {
            if f.is_empty() {
                0.0
            } else {
                f.iter().map(|&r| p[r as usize]).sum::<f32>() / f.len() as f32
            }
        })
        .collect();
    let scarcity: Vec<f32> = towns
        .0
        .iter()
        .map(|t| {
            let pc = t.food_stock(&db) / t.residents.len().max(1) as f32;
            (1.0 - pc / params.market.food_target_per_capita).clamp(0.0, 1.0)
        })
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

    let charters: Vec<Vec<bool>> = towns.0.iter().map(|t| t.policy.charters.clone()).collect();
    let ctx = Choice {
        db: &db,
        params: &params,
        feasible: &feasible,
        profit: &profit,
        mean_profit: &mean_profit,
        scarcity: &scarcity,
        charters: &charters,
        mastery,
        decision: &decision,
        models: &models,
        towns: &towns,
        median_wealth: &median_wealth,
        days_per_year: clock.days_per_year,
    };

    // Places each job had at the start of the day, for the wage factors.
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
    // the start of the day. Jobs only close during the day, so these scores cover
    // everything an agent can still pick.
    let store = &mut *store;
    let idle: Vec<usize> = (0..store.len())
        .filter(|&i| store.alive[i] && store.activity[i] == Activity::Idle)
        .collect();
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
        idle.par_iter()
            .map(|&i| ctx.scores(store, i, &open_jobs[store.town[i] as usize]))
            .collect()
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
        let utility_model = params.decision.model == crate::config::DecisionModel::Utility;
        let lists: Vec<Vec<Opt>> = {
            let store = &*store;
            let (open_jobs, places) = (&open_jobs, &places);
            idle.par_iter()
                .zip(&scores)
                .map(|(&i, (rest, job_scores))| {
                    let t = store.town[i] as usize;
                    let jobs = &open_jobs[t];
                    let app_scores: Vec<f32> = if utility_model {
                        ctx.utility_scores_paid(store, i, jobs, *apprentice_wage).1
                    } else {
                        // The feature model has no pay term: an apprentice place is
                        // just a little less attractive than a regular one.
                        job_scores
                            .iter()
                            .map(|v| v - 0.5 * params.decision.temperature)
                            .collect()
                    };
                    let mut options = Vec::with_capacity(jobs.len() * 2);
                    for (k, &v) in job_scores.iter().enumerate() {
                        options.push((2 * k as u32, v));
                        if places[t][k].apprentice > 0 {
                            options.push((2 * k as u32 + 1, app_scores[k]));
                        }
                    }
                    let mut rng = seed.agent_rng(clock.tick, streams::DECIDE, i);
                    let rng: &mut dyn rand::RngCore = &mut rng;
                    decision
                        .choice
                        .rank(*rest, &options, &store.mind[i], rng)
                        .into_iter()
                        .map(|id| Opt {
                            job: id / 2,
                            apprentice: id % 2 == 1,
                        })
                        .collect()
                })
                .collect()
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
                    decisions.jobs.push((i as u32, r));
                    if opt.apprentice {
                        decisions.apprentices.push(i as u32);
                    }
                }
                None => {
                    let mut rng = seed.agent_rng(clock.tick, streams::DECIDE + 100, i);
                    store.activity[i] = Activity::Resting;
                    let c = towns.0[t].pos;
                    store.target[i] = (
                        c.0 + rng.random_range(-45.0..45.0f32),
                        c.1 + rng.random_range(-45.0..45.0f32),
                    );
                }
            }
        }
        decisions.apprentices.sort_unstable();

        // Each month every workplace reconsiders its share of apprentice places.
        if clock.tick.is_multiple_of(30) {
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
            let mut rng = seed.agent_rng(clock.tick, streams::DECIDE, i);
            let choice = ctx.pick(
                store,
                i,
                &open_jobs[t],
                |r| open(&slots_left, &capacity, t, r),
                *rest,
                |r| job_scores[slot_of[t][r as usize] as usize],
                &mut pool,
                &mut rng,
            );
            match choice {
                None => {
                    store.activity[i] = Activity::Resting;
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
                    decisions.jobs.push((i as u32, r));
                }
            }
        }
        for &(i, r) in &decisions.jobs {
            demand[store.town[i as usize] as usize][r as usize] += 1.0;
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
                        let m = c.max_daily.max(0.0);
                        new = (old * d.clamp(-m, m).exp()).clamp(c.floor, c.ceiling.max(c.floor));
                    }
                }
                if old > 0.0 && new > 0.0 && new != old {
                    town.price_vol.resize(3, Default::default());
                    town.price_vol[2].add((new / old).ln().abs());
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
            town.building_load[p] += (today - town.building_load[p]) / 30.0;
        }
    }
}
