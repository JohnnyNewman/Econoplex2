//! Decisions as linear algebra.
//!
//! Every option has a feature vector f = [value, skill, effort, novelty, social, need].
//! An agent's preference weights are w = bias + Wm·mind + Ws·state, so personality
//! is stable while hunger and fatigue shift priorities. Utility u = w·f, and the
//! choice is a softmax over a short candidate list (temperature = bounded rationality).

use super::streams;
use super::work::{physique_bonus, state_factor};
use crate::components::NaturalResource;
use crate::config::FEATURES;
use crate::db::Db;
use crate::store::{Activity, AgentStore};
use crate::world::{Decisions, Params, SimClock, SimSeed, Towns};
use bevy_ecs::prelude::*;
use rand::Rng;
use sim_data::dims::*;

/// How many more workers each recipe can absorb in `town` today, limited by the
/// nature left to harvest and the inputs in stock. Jobs fill up as agents choose
/// them, so the rest of the town spreads to other work instead of queueing.
pub fn job_capacity(
    db: &Db,
    town: &crate::world::Town,
    sites: &Query<&NaturalResource>,
) -> Vec<f32> {
    let mut nature_total = vec![0.0f32; db.content.nature.len()];
    for &e in &town.nature_sites {
        if let Ok(n) = sites.get(e) {
            nature_total[n.kind as usize] += n.amount;
        }
    }
    db.recipes
        .iter()
        .map(|m| {
            // One worker does about 1/duration runs per day.
            let mut cap = f32::INFINITY;
            if let Some(k) = m.nature {
                let out: f32 = m.outputs.iter().map(|o| o.1).sum();
                cap = cap.min(nature_total[k as usize] / out * m.duration);
            }
            for &(p, q) in &m.inputs {
                cap = cap.min(town.stock[p as usize] / q * m.duration);
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
    let out: f32 = m
        .outputs
        .iter()
        .map(|&(p, q)| town.price[p as usize] * q)
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

#[allow(clippy::too_many_arguments)]
pub fn decide(
    mut store: ResMut<AgentStore>,
    towns: Res<Towns>,
    db: Res<Db>,
    params: Res<Params>,
    clock: Res<SimClock>,
    seed: Res<SimSeed>,
    sites: Query<&NaturalResource>,
    mut decisions: ResMut<Decisions>,
    mut log: ResMut<crate::world::WorkLog>,
) {
    decisions.jobs.clear();
    let mut rng = seed.rng(clock.tick, streams::DECIDE);
    let d = &params.decision;
    let mastery = params.mastery();

    // Per-town context, computed once.
    let mut capacity: Vec<Vec<f32>> = towns
        .0
        .iter()
        .map(|t| job_capacity(&db, t, &sites))
        .collect();
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

    let mut options: Vec<(Option<u32>, f32)> = Vec::with_capacity(d.candidates + 2);
    let mut pool: Vec<u32> = Vec::new();
    let store = &mut *store;
    for i in 0..store.len() {
        if !store.alive[i] || store.activity[i] != Activity::Idle {
            continue;
        }
        let t = store.town[i] as usize;
        let st = store.state[i];
        let w = preference_weights(&params, &store.mind[i], &st);
        let cap = store.effective_cap(i);
        let sf = state_factor(&st, &params.needs);

        // Candidate list: a random sample of feasible jobs plus the agent's habitual job.
        pool.clear();
        pool.extend(
            feasible[t]
                .iter()
                .filter(|&&r| capacity[t][r as usize] >= 1.0),
        );
        let k = d.candidates.min(pool.len());
        for j in 0..k {
            let swap = rng.random_range(j..pool.len());
            pool.swap(j, swap);
        }
        pool.truncate(k);
        if let Some(last) = store.last_recipe[i] {
            if capacity[t][last as usize] >= 1.0
                && feasible[t].contains(&last)
                && !pool.contains(&last)
            {
                pool.push(last);
            }
        }

        options.clear();
        let fatigue = st[state::FATIGUE];
        let rest = [0.0, 0.0, 1.5 * fatigue, 0.0, 0.0, fatigue];
        options.push((None, dot(&w, &rest)));
        for &r in &pool {
            let m = &db.recipes[r as usize];
            let prof = dot(&cap, &m.skill);
            let eff = (prof + physique_bonus(&store.phys[i], m, params.physique.weight)) * sf;
            let margin = eff - m.difficulty;
            let in_guild = store.guild[i].is_some_and(|g| g.domain == m.domain);
            let f = [
                ((profit[t][r as usize] - mean_profit[t]) / d.value_scale).tanh()
                    * crate::models::sigmoid(6.0 * margin),
                // Centered success chance: rewards fit, not trivially easy work.
                crate::models::sigmoid(6.0 * margin) - 0.5,
                -(0.3 + m.phys_total) * (0.5 + fatigue),
                (1.0 - (prof / mastery).min(1.0)) * if margin > -0.3 { 1.0 } else { 0.2 },
                if in_guild { 1.0 } else { 0.0 },
                m.food_out * 0.25 * scarcity[t] * (0.5 + st[state::HUNGER]),
            ];
            options.push((Some(r), dot(&w, &f)));
        }

        // Softmax sample.
        let max = options.iter().map(|o| o.1).fold(f32::MIN, f32::max);
        let temp = d.temperature.max(1e-4);
        let total: f32 = options.iter().map(|o| ((o.1 - max) / temp).exp()).sum();
        let mut x = rng.random::<f32>() * total;
        let mut choice = options[0].0;
        for o in &options {
            x -= ((o.1 - max) / temp).exp();
            if x <= 0.0 {
                choice = o.0;
                break;
            }
        }
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
                log.chosen[r as usize] += 1;
                decisions.jobs.push((i as u32, r));
            }
        }
    }
}
