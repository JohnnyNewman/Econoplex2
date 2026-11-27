//! Learning on the job: from success, failure and the team's best.

use super::*;
use crate::knowledge;

/// Practice for every worker; diffusion from the team's best member to the others and to apprentices.
/// One finished job's learning: every worker practices the recipe's skill, then the
/// team's best hand teaches the others (workers and apprentices) and gains a little
/// from teaching. Returns the new skill vectors of everyone it touched.
fn team_learning(
    store: &AgentStore,
    ev: &crate::world::WorkEvent,
    towns: &Towns,
    db: &Db,
    models: &Models,
    params: &Params,
) -> Vec<(u32, Vec<f32>)> {
    let s = &db.recipes[ev.recipe as usize].skill;
    // The run's days of practice and lessons (a month for a month's work).
    let days = ev.days;
    // Masters of a chartered guild teach faster.
    let charter = if towns.0[ev.town as usize].policy.charters
        [db.recipes[ev.recipe as usize].domain as usize]
    {
        1.0 + params.policy.charter_teaching
    } else {
        1.0
    };
    let mut caps: Vec<(u32, Vec<f32>)> = ev
        .workers
        .iter()
        .map(|&w| {
            let i = w as usize;
            let mut cap = store.cap[i].clone();
            let mult = store.aptitude[i] * (0.5 + store.state[i][state::HAPPINESS]);
            models
                .learning
                .practice(&mut cap, s, mult, ev.success, days);
            (w, cap)
        })
        .collect();
    let Some(mk) = (0..caps.len()).max_by(|&a, &b| {
        dot(&caps[a].1, s)
            .total_cmp(&dot(&caps[b].1, s))
            .then(caps[b].0.cmp(&caps[a].0))
    }) else {
        return caps;
    };
    let master = caps[mk].0;
    let master_cap = caps[mk].1.clone();
    let mut gain = 0.0;
    // A lesson is shared: the more learners beside a teacher, the less each gets.
    let learners = ev.workers.len() + ev.apprentices.len();
    let shared =
        1.0 / (1.0 + params.organize.teaching_crowding * (learners.saturating_sub(2)) as f32);
    for &o in ev.workers.iter().chain(ev.apprentices.iter()) {
        if o != master && store.alive[o as usize] {
            // A trusted master teaches more.
            let boost = shared
                * charter
                * (1.0
                    + params.trust.teaching_bonus
                        * store.trust(o as usize, master as usize).max(0.0));
            let k = match caps.iter().position(|c| c.0 == o) {
                Some(k) => k,
                None => {
                    caps.push((o, store.cap[o as usize].clone()));
                    caps.len() - 1
                }
            };
            let before = caps[k].1.clone();
            let g = models
                .diffusion
                .diffuse(&mut caps[k].1, &master_cap, s, days);
            for (c, b) in caps[k].1.iter_mut().zip(before) {
                *c = b + (*c - b) * boost;
            }
            gain += g * boost;
        }
    }
    for (m, x) in caps[mk].1.iter_mut().zip(s.iter()) {
        *m += gain * x;
    }
    caps
}

#[allow(clippy::too_many_arguments)]
pub fn learn(
    mut store: ResMut<AgentStore>,
    mut towns: ResMut<Towns>,
    db: Res<Db>,
    models: Res<Models>,
    params: Res<Params>,
    clock: Res<SimClock>,
    seed: Res<SimSeed>,
    log: Res<WorkLog>,
    mut history: ResMut<crate::history::History>,
) {
    let store = &mut *store;
    let kp = &params.knowledge;
    // Draws only with knowledge on, so runs without it are unchanged.
    let mut krng = kp
        .enabled
        .then(|| seed.rng(clock.steps(), streams::KNOWLEDGE));
    // Each finished job changes only its own team's skills. When no agent is in
    // two jobs (the normal case), every job's update is worked out in parallel from
    // the skills at the start of the step and written back afterwards, which gives
    // exactly the sequential result.
    let mut seen = vec![false; store.len()];
    let disjoint = log.events.iter().all(|ev| {
        ev.workers
            .iter()
            .chain(&ev.apprentices)
            .all(|&a| !std::mem::replace(&mut seen[a as usize], true))
    });
    let towns_ref = &*towns;
    let teach = |store: &AgentStore, ev: &crate::world::WorkEvent| {
        team_learning(store, ev, towns_ref, &db, &models, &params)
    };
    if disjoint {
        let updates: Vec<Vec<(u32, Vec<f32>)>> = {
            let store = &*store;
            crate::systems::par_map(&log.events, |ev| teach(store, ev))
        };
        for (a, cap) in updates.into_iter().flatten() {
            store.cap[a as usize] = cap;
        }
    } else {
        for ev in &log.events {
            for (a, cap) in teach(store, ev) {
                store.cap[a as usize] = cap;
            }
        }
    }

    // --- knowledge: whoever works beside someone who knows the recipe may learn it.
    if let Some(rng) = krng.as_mut() {
        for ev in &log.events {
            let r = ev.recipe as usize;
            let team = || {
                ev.workers
                    .iter()
                    .chain(&ev.apprentices)
                    .map(|&a| a as usize)
            };
            if !team().any(|a| store.alive[a] && knowledge::knows(kp, &db, store, a, r)) {
                continue;
            }
            let p = knowledge::chance(kp.learn_per_day, ev.days);
            for a in team() {
                if store.alive[a]
                    && !knowledge::knows(kp, &db, store, a, r)
                    && knowledge::can_learn(kp, &db, store, a, r)
                    && rng.random::<f32>() < p
                {
                    knowledge::learn(store, a, r);
                }
            }
        }
    }

    if kp.enabled && kp.invention.enabled && !log.events.is_empty() {
        invent(
            store,
            &towns,
            &db,
            &models,
            &params,
            &clock,
            &seed,
            &log,
            &mut history,
        );
    }

    // --- schools: children of school age practice the town's chosen domain and learn
    // from its best teacher, who is paid per pupil and day out of the treasury.
    let pp = &params.policy;
    let per_year = clock.months_per_year;
    let cal = clock.calendar();
    for town in towns.0.iter_mut() {
        let Some(school) = town.policy.school else {
            continue;
        };
        let dir = &db.domain_dirs[school.domain as usize];
        let Some(teacher) = town
            .residents
            .iter()
            .map(|&i| i as usize)
            .filter(|&i| store.alive[i] && store.activity[i] != Activity::Child)
            .max_by(|&a, &b| {
                dot(&store.cap[a], dir)
                    .total_cmp(&dot(&store.cap[b], dir))
                    .then(b.cmp(&a))
            })
        else {
            continue;
        };
        let affordable = (town.spendable(&params.market) / cal.flow(pp.school_fee).max(1e-6))
            .min(u32::MAX as f32) as u32;
        let seats = school.seats.min(affordable) as usize;
        let pupils: Vec<usize> = town
            .residents
            .iter()
            .map(|&i| i as usize)
            .filter(|&i| {
                let age = store.age_years(i, per_year);
                store.alive[i]
                    && store.activity[i] == Activity::Child
                    && age >= pp.school_age
                    && age < params.life.apprentice_age
            })
            .take(seats)
            .collect();
        let teacher_cap = store.cap[teacher].clone();
        for &c in &pupils {
            // Lessons are practice at a share of the pace of real work, plus what
            // the teacher passes on.
            let mult = store.aptitude[c] * pp.school_rate;
            models
                .learning
                .practice(&mut store.cap[c], dir, mult, true, cal.days());
            let before = store.cap[c].clone();
            models
                .diffusion
                .diffuse(&mut store.cap[c], &teacher_cap, dir, cal.days());
            for (x, b) in store.cap[c].iter_mut().zip(before) {
                *x = b + (*x - b) * pp.school_rate;
            }
            // Pupils also pick up the recipes the teacher knows in the domain.
            if let Some(rng) = krng.as_mut() {
                let p = knowledge::chance(kp.learn_per_day * kp.school_share, cal.days());
                for r in 0..db.recipes.len() {
                    if db.recipes[r].domain == school.domain
                        && !db.requires[r].is_empty()
                        && knowledge::knows(kp, &db, store, teacher, r)
                        && !knowledge::knows(kp, &db, store, c, r)
                        && knowledge::can_learn(kp, &db, store, c, r)
                        && rng.random::<f32>() < p
                    {
                        knowledge::learn(store, c, r);
                    }
                }
            }
        }
        let fee = cal.flow(pupils.len() as f32 * pp.school_fee);
        town.treasury -= crate::coin::Coin::from_coins(fee);
        town.ledger.add(crate::ledger::Flow::SchoolFees, fee);
        store.wealth[teacher] += crate::coin::Coin::from_coins(fee);
    }
}

/// After the day's work, workers may think of a recipe-skill close to it that
/// nobody in their town knows (`knowledge::InventionParams`).
#[allow(clippy::too_many_arguments)]
fn invent(
    store: &mut AgentStore,
    towns: &Towns,
    db: &Db,
    models: &Models,
    params: &Params,
    clock: &SimClock,
    seed: &SimSeed,
    log: &WorkLog,
    history: &mut crate::history::History,
) {
    let kp = &params.knowledge;
    let iv = &kp.invention;
    let mut rng = seed.rng(clock.steps(), streams::INVENT);
    // Per town, worked out when first needed: what its people know between
    // them, and its price level (mean price over base price, in its coin).
    let mut known: Vec<Option<Vec<u64>>> = vec![None; towns.0.len()];
    let mut level: Vec<Option<f32>> = vec![None; towns.0.len()];
    for ev in &log.events {
        let t = ev.town as usize;
        let town = &towns.0[t];
        let near: Vec<usize> = db.related[ev.recipe as usize]
            .iter()
            .take_while(|&&(_, c)| c >= iv.related)
            .map(|&(r, _)| r as usize)
            .collect();
        if near.is_empty() {
            continue;
        }
        for &w in &ev.workers {
            let i = w as usize;
            if !store.alive[i] {
                continue;
            }
            let town_bits =
                known[t].get_or_insert_with(|| knowledge::union(db, store, &town.residents));
            let checks: Vec<usize> = near
                .iter()
                .copied()
                .filter(|&r| {
                    !db.requires[r].is_empty()
                        && !knowledge::in_set(town_bits, r)
                        && knowledge::can_invent(kp, db, store, i, town_bits, r)
                })
                .take(iv.max_checks)
                .collect();
            if checks.is_empty() {
                continue;
            }
            let rest = (1.0 - store.state[i][state::FATIGUE]).clamp(0.0, 1.0);
            for r in checks {
                let m = &db.recipes[r];
                let skill = models
                    .success
                    .probability(dot(&store.cap[i], &m.skill), m.difficulty + iv.margin);
                let materials = if m.inputs.iter().all(|&(p, q)| town.stock[p as usize] >= q) {
                    1.0
                } else {
                    iv.no_materials
                };
                let lvl = *level[t].get_or_insert_with(|| price_level(db, &town.price));
                let dear = m
                    .outputs
                    .iter()
                    .map(|&(p, _)| relative_price(db, &town.price, p as usize))
                    .fold(0.0f32, f32::max);
                let need = if dear > 0.0 && lvl > 0.0 {
                    (dear / lvl).powf(iv.need_weight).clamp(0.5, 2.0)
                } else {
                    1.0
                };
                let p = iv.rate * skill * materials * need * rest;

                if rng.random::<f32>() < knowledge::chance(p, ev.days) {
                    knowledge::learn(store, i, r);
                    if let Some(b) = known[t].as_mut() {
                        knowledge::add(b, r);
                    }
                    history.events.push(crate::history::Event {
                        tick: clock.tick,
                        town: t as u16,
                        kind: crate::history::EventKind::Invented,
                        text: format!(
                            "{}: invented {} (person {i}, working at {})",
                            town.name, m.name, db.recipes[ev.recipe as usize].name
                        ),
                    });
                    // One idea a day is plenty.
                    break;
                }
            }
        }
    }
}

/// A product's price over its base price (0 when it has none).
fn relative_price(db: &Db, price: &[f32], p: usize) -> f32 {
    let base = db.base_price[p];
    if base > 0.0 && price[p] > 0.0 {
        price[p] / base
    } else {
        0.0
    }
}

/// The town's price level: the geometric mean of price over base price.
fn price_level(db: &Db, price: &[f32]) -> f32 {
    let (sum, n) = (0..price.len())
        .map(|p| relative_price(db, price, p))
        .filter(|&x| x > 0.0)
        .fold((0.0f32, 0u32), |(s, n), x| (s + x.ln(), n + 1));
    if n == 0 {
        1.0
    } else {
        (sum / n as f32).exp()
    }
}
