//! Production: team formation, task progress, success and quality, learning.

use super::streams;
use crate::components::*;
use crate::config::NeedsParams;
use crate::db::{Db, RecipeMeta};
use crate::models::Models;
use crate::store::{Activity, AgentStore};
use crate::world::{Decisions, Params, SimClock, SimSeed, Towns, WorkEvent, WorkLog};
use bevy_ecs::prelude::*;
use rand::Rng;
use rayon::prelude::*;
use sim_data::dims::*;
use sim_data::Category;
use std::collections::BTreeMap;

/// Physique contribution to effective proficiency: demand-weighted deviation from average build.
pub fn physique_bonus(phys: &PhysVec, r: &RecipeMeta, weight: f32) -> f32 {
    weight
        * (0..PHYS_DIM)
            .map(|k| r.phys[k] * (phys[k] - 0.5))
            .sum::<f32>()
}

/// Fatigue and health scale how well an agent works today.
pub fn state_factor(st: &StateVec, p: &NeedsParams) -> f32 {
    (1.0 - p.fatigue_penalty * st[state::FATIGUE]).max(0.2)
        * (0.5 + 0.5 * st[state::HEALTH].clamp(0.0, 1.0))
}

struct PendingTask {
    recipe: u32,
    town: u16,
    workers: Vec<u32>,
    apprentices: Vec<u32>,
    trainees: Vec<u32>,
    input_quality: f32,
    pos: (f32, f32),
    batches: u32,
}

/// Split the people who chose the same job into teams. Each team forms around the
/// first person not yet placed, who brings the coworkers they trust most; the rest
/// fill up in the order they chose.
fn trusted_teams(
    store: &AgentStore,
    agents: &[u32],
    size: usize,
    place: &mut [u32],
) -> Vec<Vec<u32>> {
    const PLACED: u32 = u32::MAX - 1;
    for (k, &a) in agents.iter().enumerate() {
        place[a as usize] = k as u32;
    }
    let mut teams = Vec::new();
    let mut next = 0;
    for &lead in agents {
        if place[lead as usize] == PLACED {
            continue;
        }
        place[lead as usize] = PLACED;
        let mut team = vec![lead];
        let mut ties: Vec<_> = store.ties[lead as usize]
            .iter()
            .filter(|t| t.value > 0.0 && t.other != u32::MAX)
            .filter(|t| place[t.other as usize] < PLACED)
            .copied()
            .collect();
        ties.sort_by(|a, b| b.value.total_cmp(&a.value).then(a.other.cmp(&b.other)));
        for t in ties {
            if team.len() >= size {
                break;
            }
            place[t.other as usize] = PLACED;
            team.push(t.other);
        }
        while team.len() < size && next < agents.len() {
            let a = agents[next];
            next += 1;
            if place[a as usize] != PLACED {
                place[a as usize] = PLACED;
                team.push(a);
            }
        }
        teams.push(team);
    }
    for &a in agents {
        place[a as usize] = u32::MAX;
    }
    teams
}

#[allow(clippy::too_many_arguments)]
pub fn assign(
    mut commands: Commands,
    mut store: ResMut<AgentStore>,
    mut towns: ResMut<Towns>,
    db: Res<Db>,
    params: Res<Params>,
    decisions: Res<Decisions>,
    clock: Res<SimClock>,
    seed: Res<SimSeed>,
    mut sites: Query<(&mut NaturalResource, &Pos)>,
    mut log: ResMut<WorkLog>,
) {
    let mut rng = seed.rng(clock.tick, streams::ASSIGN);
    let store = &mut *store;

    // Group choices by (town, recipe), keeping agent order for determinism.
    let mut groups: BTreeMap<(u16, u32), Vec<u32>> = BTreeMap::new();
    for &(a, r) in &decisions.jobs {
        groups
            .entry((store.town[a as usize], r))
            .or_default()
            .push(a);
    }

    // Scarce inputs go to construction first (the town commissioned it), then to the
    // most profitable work, as if those teams outbid the others at the market.
    let mut groups: Vec<((u16, u32), Vec<u32>)> = groups.into_iter().collect();
    groups.sort_by(|((ta, ra), _), ((tb, rb), _)| {
        let pa = super::decide::profit_per_day(&db, &towns.0[*ta as usize], *ra as usize);
        let pb = super::decide::profit_per_day(&db, &towns.0[*tb as usize], *rb as usize);
        let build_a = db.recipes[*ra as usize].builds.is_some();
        let build_b = db.recipes[*rb as usize].builds.is_some();
        ta.cmp(tb)
            .then(build_b.cmp(&build_a))
            .then(pb.total_cmp(&pa))
            .then(ra.cmp(rb))
    });

    // Each town's nature sites by kind, in the town's order, with their amounts.
    // Amounts only go down while teams are placed, so a site too worn for one team
    // stays too worn for the next: a cursor per kind skips it for good instead of
    // scanning every site again (big towns have hundreds of fields).
    let nk = db.content.nature.len();
    // (site, amount left, position), per town and kind.
    type Site = (Entity, f32, (f32, f32));
    let mut nature: Vec<Vec<Vec<Site>>> = towns
        .0
        .iter()
        .map(|town| {
            let mut by_kind = vec![Vec::new(); nk];
            for &e in &town.nature_sites {
                if let Ok((n, p)) = sites.get(e) {
                    by_kind[n.kind as usize].push((e, n.amount, (p.x, p.y)));
                }
            }
            by_kind
        })
        .collect();
    let mut cursor = vec![vec![0usize; nk]; towns.0.len()];

    let mut pending: Vec<PendingTask> = Vec::new();
    let mut place = vec![u32::MAX; store.len()];
    for ((t, r), agents) in groups {
        let m = &db.recipes[r as usize];
        let town = &mut towns.0[t as usize];
        for team in &trusted_teams(store, &agents, m.max_team, &mut place) {
            // A team's daily work covers several runs of a short recipe.
            let rate = (team.len() as f32).powf(params.work.work_exponent);
            let mut batches = ((rate / m.duration).floor() as u32).max(1);
            for &(p, q) in &m.inputs {
                batches = batches.min((town.stock[p as usize] / q).floor() as u32);
            }
            if batches == 0 {
                continue;
            }
            let out: f32 = m.outputs.iter().map(|o| o.1).sum();
            let mut pos = town.pos;
            if let Some(kind) = m.nature {
                let mut found = None;
                let list = &mut nature[t as usize][kind as usize];
                let c = &mut cursor[t as usize][kind as usize];
                while let Some(site) = list.get_mut(*c) {
                    if site.1 >= out {
                        batches = batches.min((site.1 / out).floor() as u32);
                        site.1 -= out * batches as f32;
                        found = Some(site.2);
                        break;
                    }
                    *c += 1;
                }
                match found {
                    Some(p) => pos = p,
                    None => continue,
                }
            }
            if let Some(b) = m.building {
                match town.building(b) {
                    Some((_, p)) => pos = p,
                    None => continue,
                }
            }
            let mut input_quality: f32 = 1.0;
            town.input_today.resize(town.stock.len(), 0.0);
            for &(p, q) in &m.inputs {
                town.stock[p as usize] -= q * batches as f32;
                town.input_today[p as usize] += q * batches as f32;
                town.inputs_year.resize(town.stock.len(), 0.0);
                town.inputs_year[p as usize] += q * batches as f32;
                input_quality = input_quality.min(town.quality[p as usize]);
            }
            pending.push(PendingTask {
                recipe: r,
                town: t,
                workers: team.clone(),
                apprentices: Vec::new(),
                trainees: team
                    .iter()
                    .copied()
                    .filter(|w| decisions.apprentices.binary_search(w).is_ok())
                    .collect(),
                input_quality,
                pos,
                batches,
            });
        }
    }

    // Write the harvested amounts back.
    for (e, amount, _) in nature.into_iter().flatten().flatten() {
        if let Ok((mut n, _)) = sites.get_mut(e) {
            if n.amount != amount {
                n.amount = amount;
            }
        }
    }

    // Children old enough follow a working parent as apprentices.
    let mut task_of = vec![usize::MAX; store.len()];
    for (k, p) in pending.iter().enumerate() {
        for &w in &p.workers {
            task_of[w as usize] = k;
        }
    }
    let dpy = params.life.days_per_year;
    for i in 0..store.len() {
        if !store.alive[i] || store.activity[i] != Activity::Child {
            continue;
        }
        if store.age_years(i, dpy) < params.life.apprentice_age {
            continue;
        }
        if let Some((a, b)) = store.parents[i] {
            let k = if task_of[a as usize] != usize::MAX {
                task_of[a as usize]
            } else {
                task_of[b as usize]
            };
            if k != usize::MAX {
                pending[k].apprentices.push(i as u32);
                store.target[i] = jitter(pending[k].pos, 10.0, &mut rng);
            }
        }
    }

    for p in pending {
        let m = &db.recipes[p.recipe as usize];
        log.next_serial += 1;
        let e = commands
            .spawn((
                Task {
                    serial: log.next_serial,
                    recipe: p.recipe,
                    town: p.town,
                    progress: 0.0,
                    required: m.duration * p.batches as f32,
                    input_quality: p.input_quality,
                    batches: p.batches,
                },
                TownId(p.town),
                Pos {
                    x: p.pos.0,
                    y: p.pos.1,
                },
            ))
            .id();
        for &w in &p.workers {
            store.activity[w as usize] = Activity::Working(e);
            store.target[w as usize] = jitter(p.pos, 14.0, &mut rng);
        }
        commands.entity(e).insert(Participants {
            workers: p.workers,
            apprentices: p.apprentices,
            trainees: p.trainees,
        });
    }
}

fn jitter(p: (f32, f32), r: f32, rng: &mut impl Rng) -> (f32, f32) {
    (p.0 + rng.random_range(-r..r), p.1 + rng.random_range(-r..r))
}

/// Share of the way the public pay rate moves toward each realised step.
const PAY_RATE_SMOOTHING: f32 = 0.05;

/// What a worker-day at recipe `r` would pay in this town at today's prices: the
/// workers' share of the value added (plus subsidy) of one run, over the days one
/// worker needs for it, at a typical worker's success rate.
pub fn posted_pay(town: &crate::world::Town, db: &Db, params: &Params, r: usize) -> f32 {
    let m = &db.recipes[r];
    let out: f32 = m
        .outputs
        .iter()
        .map(|&(p, q)| {
            let p = p as usize;
            (town.price[p] + town.policy.subsidy[p] * db.base_price[p]) * q
        })
        .sum();
    let inputs: f32 = m
        .inputs
        .iter()
        .map(|&(p, q)| town.price[p as usize] * q)
        .sum();
    let base = town.policy.wage_share.unwrap_or(params.work.wage_share);
    let factor = town.wage_factor.get(r).copied().unwrap_or(1.0);
    let share = (base * factor).min(1.0);
    share * (out - inputs).max(0.0) * params.decision.valuation.typical_success
        / m.duration.max(1.0)
}

/// Every day each town's public pay rate drifts toward the posted pay at today's
/// prices, over `decision.valuation.posted_pay_days`. Realised pay pulls much
/// harder (each finished step), so for jobs people do the rate is what they
/// earn; a job nobody has tried yet still shows what it would pay, instead of
/// nothing, and so can be tried.
pub fn post_pay(towns: &mut crate::world::Towns, db: &Db, params: &Params) {
    let days = params.decision.valuation.posted_pay_days;
    if days <= 0.0 {
        return;
    }
    let typical = params.decision.valuation.typical_success.max(1e-3);
    for town in towns.0.iter_mut() {
        town.pay_rate.resize(db.recipes.len(), 0.0);
        town.success_pay.resize(db.recipes.len(), 0.0);
        for r in 0..db.recipes.len() {
            let posted = posted_pay(town, db, params, r);
            town.pay_rate[r] += (posted - town.pay_rate[r]) / days.max(1.0);
            // The same pay, for a step that succeeds.
            town.success_pay[r] += (posted / typical - town.success_pay[r]) / days.max(1.0);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn produce(
    mut commands: Commands,
    decision: Res<crate::decision::DecisionModels>,
    mut store: ResMut<AgentStore>,
    mut towns: ResMut<Towns>,
    db: Res<Db>,
    models: Res<Models>,
    params: Res<Params>,
    clock: Res<SimClock>,
    seed: Res<SimSeed>,
    mut tasks: Query<(Entity, &mut Task, &Participants)>,
    mut items: Query<&mut Item>,
    mut log: ResMut<WorkLog>,
) {
    let mut rng = seed.rng(clock.tick, streams::PRODUCE);
    let mut order: Vec<(u64, Entity)> = tasks.iter().map(|(e, t, _)| (t.serial, e)).collect();
    order.sort_unstable();
    let store = &mut *store;

    // Phase A: a day's work on every task; collect the ones that finish today.
    let mut done: Vec<(Entity, u32, Vec<u32>)> = Vec::new();
    for (_, e) in order {
        let Ok((_, mut task, parts)) = tasks.get_mut(e) else {
            continue;
        };
        let workers: Vec<u32> = parts
            .workers
            .iter()
            .copied()
            .filter(|&w| store.alive[w as usize])
            .collect();
        if workers.is_empty() {
            commands.entity(e).despawn();
            continue;
        }
        let r = task.recipe;
        let m = &db.recipes[r as usize];
        for &w in &workers {
            let st = &mut store.state[w as usize];
            st[state::FATIGUE] = (st[state::FATIGUE]
                + params.needs.fatigue_per_work_day * (0.5 + m.phys_total))
                .min(1.0);
            store.worked_today[w as usize] = Some(r);
        }
        task.progress += (workers.len() as f32).powf(params.work.work_exponent);
        if task.progress >= task.required {
            done.push((e, r, workers));
        }
    }

    // Phase B, in parallel: each finishing team's effective skill. It reads only
    // the team's own members, whom no other task touches, so it is the same as
    // working it out task by task.
    let effs: Vec<f32> = {
        let store = &*store;
        done.par_iter()
            .map(|(_, r, workers)| {
                let m = &db.recipes[*r as usize];
                let n = workers.len();
                let caps: Vec<CapVec> = workers
                    .iter()
                    .map(|&w| store.effective_cap(w as usize))
                    .collect();
                let team = models.pooling.pool(&caps);
                let mut sorted = workers.clone();
                sorted.sort_unstable();
                // People who trust each other coordinate better.
                let prof = dot(&team, &m.skill) - models.pooling.coordination_penalty(n)
                    + params.trust.team_bonus * store.cohesion(workers, &sorted);
                let pb: f32 = workers
                    .iter()
                    .map(|&w| physique_bonus(&store.phys[w as usize], m, params.physique.weight))
                    .sum::<f32>()
                    / n as f32;
                let sf: f32 = workers
                    .iter()
                    .map(|&w| state_factor(&store.state[w as usize], &params.needs))
                    .sum::<f32>()
                    / n as f32;
                (prof + pb) * sf
            })
            .collect()
    };

    let mut claims: Vec<Claim> = Vec::new();

    // Phase C, in task order: resolve each finished step.
    for ((e, r, workers), eff) in done.into_iter().zip(effs) {
        let Ok((_, task, parts)) = tasks.get_mut(e) else {
            continue;
        };
        let m = &db.recipes[r as usize];
        let n = workers.len();
        // A building always goes up once the work is done; skill shows in its quality.
        let success = m.builds.is_some()
            || rng.random::<f32>() < models.success.probability(eff, m.difficulty);
        let quality = models.quality.combine(
            models.quality.step_quality(eff, m.difficulty),
            task.input_quality,
        );

        log.attempts += 1;
        log.runs[r as usize] += task.batches as u64;
        let t = task.town as usize;
        if success {
            log.successes += 1;
            let town = &mut towns.0[t];
            let mut value = 0.0;
            let mut subsidy = 0.0;
            let keep = params.decision.subsistence.share();
            let larder_cap = params.decision.subsistence.larder_meals();
            for &(p, q) in &m.outputs {
                let q = q * task.batches as f32;
                let pi = p as usize;
                town.produced_this_year[pi] += q;
                // Households keep part of the food they make; the rest goes to market.
                let mut kept = 0.0;
                if keep > 0.0 && db.is_food[pi] && db.coin[pi] <= 0.0 {
                    let each = q * keep / workers.len() as f32;
                    for &w in &workers {
                        let back = crate::decision::subsistence::store(
                            &mut store.larder[w as usize],
                            p,
                            each,
                            larder_cap,
                        );
                        kept += each - back;
                    }
                }
                let q = q - kept;
                value += town.price[pi] * q * (0.5 + quality);
                subsidy += town.policy.subsidy[pi] * db.base_price[pi] * q;
                if db.coin[pi] > 0.0 {
                    // Struck coin is new money in the treasury, not a good in stock.
                    let minted = q * db.coin[pi];
                    town.treasury += minted;
                    town.ledger.add(crate::ledger::Flow::Minted, minted);
                    continue;
                }
                let old = town.stock[pi];
                town.quality[pi] = (old * town.quality[pi] + q * quality) / (old + q).max(1e-6);
                town.stock[pi] += q;
                // Deliveries for a government order go to public use.
                if let Some(o) = town
                    .policy
                    .procurement
                    .iter_mut()
                    .find(|o| o.0 as usize == pi)
                {
                    let delivered = q.min(o.1);
                    o.1 -= delivered;
                    town.stock[pi] -= delivered;
                    town.procured[pi] += delivered;
                    town.policy.procurement.retain(|o| o.1 > 1e-3);
                }
                if let Some(kind) = db.opens[pi] {
                    // New land: a fresh site that starts bare and grows in. Two crews
                    // can finish on the last free plot; only the first one gets it.
                    town.stock[pi] -= q;
                    if town.free_land == 0 {
                        continue;
                    }
                    let def = &db.content.nature[kind as usize];
                    let k = (town.nature_sites.len() + town.cleared as usize) as f32;
                    let a = k * 2.399_963 + task.town as f32;
                    let r = 270.0 + 6.0 * town.cleared as f32;
                    let e = commands
                        .spawn((
                            NaturalResource {
                                kind,
                                amount: 0.0,
                                capacity: def.capacity,
                                regrowth: def.regrowth,
                            },
                            TownId(task.town),
                            Pos {
                                x: town.pos.0 + r * a.cos(),
                                y: town.pos.1 + r * a.sin(),
                            },
                        ))
                        .id();
                    town.nature_sites.push(e);
                    town.free_land = town.free_land.saturating_sub(1);
                    town.cleared += 1;
                    town.price[pi] = 0.0;
                } else if db.category(p) == Category::Building {
                    // The finished building becomes a workplace (stock tracks nothing here).
                    town.stock[pi] -= q;
                    // An ordered building goes where the order put it.
                    let commission = town.policy.commissions.iter().position(|c| c.building == p);
                    let pos = match commission.map(|k| town.policy.commissions.remove(k)) {
                        Some(crate::policy::Commission { pos: Some(pos), .. }) => pos,
                        _ => town.next_building_pos(),
                    };
                    town.built += 1;
                    let e = commands
                        .spawn((
                            Product { def: p, quality },
                            Structure {
                                footprint: (24.0, 24.0),
                            },
                            TownId(task.town),
                            Pos { x: pos.0, y: pos.1 },
                        ))
                        .id();
                    town.buildings.push((p, e, pos));
                    // The new building's slots dilute the load.
                    let n = town.building_count(p) as f32;
                    town.building_load[pi] *= (n - 1.0) / n;
                    town.price[pi] = 0.0;
                }
                if db.category(p).is_item() {
                    let durability = db.content.products[pi]
                        .equip
                        .as_ref()
                        .map_or(1000, |e| e.durability);
                    for _ in 0..q as u32 {
                        let item = commands
                            .spawn((
                                Product { def: p, quality },
                                Item { durability },
                                Stored,
                                TownId(task.town),
                            ))
                            .id();
                        town.armory.push(item);
                    }
                }
            }
            // The town market buys the output and pays the team its share of the value
            // added, out of its treasury: money moves, it is never created.
            let input_cost: f32 = m
                .inputs
                .iter()
                .map(|&(p, q)| town.price[p as usize] * q * task.batches as f32)
                .sum();
            // The team's claim on the town market: its share of the value added
            // (the town's base share times this job's market factor), plus any
            // subsidy. Claims are paid after all of today's steps, at one ratio.
            let base = town.policy.wage_share.unwrap_or(params.work.wage_share);
            let factor = town.wage_factor.get(r as usize).copied().unwrap_or(1.0);
            let share = (base * factor).min(1.0);
            let days = (task.required / (n as f32).powf(params.work.work_exponent)).max(1.0);
            // Apprentices on the team cost the market less.
            let aw = params.decision.matching.apprentice_wage();
            let trainees: Vec<u32> = workers
                .iter()
                .copied()
                .filter(|w| parts.trainees.contains(w))
                .collect();
            let weight = (n as f32 - trainees.len() as f32 * (1.0 - aw)) / n as f32;
            claims.push(Claim {
                town: t,
                recipe: r,
                workers: workers.clone(),
                trainees,
                wage: (share * (value - input_cost)).max(0.0) * weight,
                subsidy,
                days,
                succeeded: true,
            });
            for &w in &workers {
                let st = &mut store.state[w as usize];
                st[state::HAPPINESS] = (st[state::HAPPINESS] + 0.03).min(1.0);
            }
        } else {
            for &w in &workers {
                let st = &mut store.state[w as usize];
                st[state::HAPPINESS] = (st[state::HAPPINESS] - 0.03).max(0.0);
            }
            // A failed step earns nothing.
            let days = (task.required / (n as f32).powf(params.work.work_exponent)).max(1.0);
            claims.push(Claim {
                town: t,
                recipe: r,
                workers: workers.clone(),
                trainees: Vec::new(),
                wage: 0.0,
                subsidy: 0.0,
                days,
                succeeded: false,
            });
        }

        // Tool wear.
        for &w in &workers {
            let wi = w as usize;
            if let Some(tool) = store.equipped[wi] {
                if let Ok(mut it) = items.get_mut(tool) {
                    it.durability = it.durability.saturating_sub(1);
                    if it.durability == 0 {
                        commands.entity(tool).despawn();
                        store.equipped[wi] = None;
                        store.equip_vec[wi].iter_mut().for_each(|x| *x = 0.0);
                    }
                }
            }
            store.activity[wi] = Activity::Idle;
            store.last_recipe[wi] = Some(r);
        }

        log.events.push(WorkEvent {
            recipe: r,
            town: task.town,
            workers,
            apprentices: parts.apprentices.clone(),
            success,
            quality,
        });
        commands.entity(e).despawn();
    }

    settle_wages(
        claims,
        &mut towns,
        store,
        &decision,
        params.decision.matching.apprentice_wage(),
    );
}

/// A finished step's claim on its town market.
struct Claim {
    town: usize,
    recipe: u32,
    workers: Vec<u32>,
    /// Workers paid as apprentices.
    trainees: Vec<u32>,
    wage: f32,
    subsidy: f32,
    /// Days the team worked on it.
    days: f32,
    /// Whether the step succeeded (failed steps pay nothing).
    succeeded: bool,
}

/// Pay the day's claims. When a town's treasury can't cover all wages, every claim
/// gets the same share of what it is owed, so pay still follows prices; subsidies are
/// paid the same way from what is left. Then income tax, the workers' pay, and the pay
/// expectations (public rate and each worker's own).
fn settle_wages(
    claims: Vec<Claim>,
    towns: &mut Towns,
    store: &mut AgentStore,
    decision: &crate::decision::DecisionModels,
    apprentice_wage: f32,
) {
    let n_towns = towns.0.len();
    let mut owed = vec![(0.0f32, 0.0f32); n_towns];
    for c in &claims {
        owed[c.town].0 += c.wage;
        owed[c.town].1 += c.subsidy;
    }
    let ratios: Vec<(f32, f32)> = towns
        .0
        .iter()
        .zip(&owed)
        .map(|(town, &(w, sub))| {
            let budget = town.treasury.max(0.0);
            let rw = if w > 0.0 { (budget / w).min(1.0) } else { 0.0 };
            let left = (budget - w * rw).max(0.0);
            let rs = if sub > 0.0 {
                (left / sub).min(1.0)
            } else {
                0.0
            };
            (rw, rs)
        })
        .collect();
    for c in claims {
        let town = &mut towns.0[c.town];
        let (rw, rs) = ratios[c.town];
        let wage = c.wage * rw;
        let subsidy = c.subsidy * rs;
        town.treasury -= wage + subsidy;
        town.ledger.add(crate::ledger::Flow::Wages, wage);
        town.ledger.add(crate::ledger::Flow::Subsidies, subsidy);
        let pay = wage + subsidy;
        // Income tax: the treasury keeps its share of every wage.
        let tax = pay * town.policy.income_tax;
        town.treasury += tax;
        town.ledger.add(crate::ledger::Flow::IncomeTax, tax);
        // Shares: 1 per regular worker, `apprentice_wage` per apprentice.
        let shares = c.workers.len() as f32 - c.trainees.len() as f32 * (1.0 - apprentice_wage);
        let each = (pay - tax) / shares.max(1e-6);
        let per_day = each / c.days;
        town.worker_days += shares * c.days;
        let rate = &mut town.pay_rate[c.recipe as usize];
        *rate += PAY_RATE_SMOOTHING * (per_day - *rate);
        if c.succeeded {
            town.success_pay.resize(town.pay_rate.len(), 0.0);
            let rate = &mut town.success_pay[c.recipe as usize];
            *rate += PAY_RATE_SMOOTHING * (per_day - *rate);
        }
        for &w in &c.workers {
            let wi = w as usize;
            let own = if c.trainees.contains(&w) {
                each * apprentice_wage
            } else {
                each
            };
            store.wealth[wi] += own;
            let curiosity = store.mind[wi][sim_data::dims::mind::CURIOSITY];
            crate::decision::expectations::remember(
                &mut store.pay_expect[wi],
                c.recipe,
                own / c.days,
                curiosity,
                &*decision.expectations,
            );
        }
    }
}

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
            models.learning.practice(&mut cap, s, mult, ev.success);
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
    for &o in ev.workers.iter().chain(ev.apprentices.iter()) {
        if o != master && store.alive[o as usize] {
            // A trusted master teaches more.
            let boost = charter
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
            let g = models.diffusion.diffuse(&mut caps[k].1, &master_cap, s);
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
    log: Res<WorkLog>,
) {
    let store = &mut *store;
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
            log.events.par_iter().map(|ev| teach(store, ev)).collect()
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

    // --- schools: children of school age practice the town's chosen domain and learn
    // from its best teacher, who is paid per pupil and day out of the treasury.
    let pp = &params.policy;
    let dpy = clock.days_per_year;
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
        let affordable = (town.treasury.max(0.0) / pp.school_fee.max(1e-6)) as u32;
        let seats = school.seats.min(affordable) as usize;
        let pupils: Vec<usize> = town
            .residents
            .iter()
            .map(|&i| i as usize)
            .filter(|&i| {
                let age = store.age_years(i, dpy);
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
            models.learning.practice(&mut store.cap[c], dir, mult, true);
            let before = store.cap[c].clone();
            models
                .diffusion
                .diffuse(&mut store.cap[c], &teacher_cap, dir);
            for (x, b) in store.cap[c].iter_mut().zip(before) {
                *x = b + (*x - b) * pp.school_rate;
            }
        }
        let fee = pupils.len() as f32 * pp.school_fee;
        town.treasury -= fee;
        town.ledger.add(crate::ledger::Flow::SchoolFees, fee);
        store.wealth[teacher] += fee;
    }
}
