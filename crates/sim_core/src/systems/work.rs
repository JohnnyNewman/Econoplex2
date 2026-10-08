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
                for &e in &town.nature_sites {
                    if let Ok((mut n, p)) = sites.get_mut(e) {
                        if n.kind == kind && n.amount >= out {
                            batches = batches.min((n.amount / out).floor() as u32);
                            n.amount -= out * batches as f32;
                            found = Some((p.x, p.y));
                            break;
                        }
                    }
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
            for &(p, q) in &m.inputs {
                town.stock[p as usize] -= q * batches as f32;
                input_quality = input_quality.min(town.quality[p as usize]);
            }
            pending.push(PendingTask {
                recipe: r,
                town: t,
                workers: team.clone(),
                apprentices: Vec::new(),
                input_quality,
                pos,
                batches,
            });
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
        });
    }
}

fn jitter(p: (f32, f32), r: f32, rng: &mut impl Rng) -> (f32, f32) {
    (p.0 + rng.random_range(-r..r), p.1 + rng.random_range(-r..r))
}

#[allow(clippy::too_many_arguments)]
pub fn produce(
    mut commands: Commands,
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
        if task.progress < task.required {
            continue;
        }

        // --- resolve the step
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
            + params.trust.team_bonus * store.cohesion(&workers, &sorted);
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
        let eff = (prof + pb) * sf;
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
            for &(p, q) in &m.outputs {
                let q = q * task.batches as f32;
                let pi = p as usize;
                value += town.price[pi] * q * (0.5 + quality);
                let old = town.stock[pi];
                town.quality[pi] = (old * town.quality[pi] + q * quality) / (old + q).max(1e-6);
                town.stock[pi] += q;
                town.produced_this_year[pi] += q;
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
                    let pos = town.next_building_pos();
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
            let pay =
                (params.work.wage_share * (value - input_cost)).clamp(0.0, town.treasury.max(0.0));
            town.treasury -= pay;
            let wage = pay / n as f32;
            for &w in &workers {
                store.wealth[w as usize] += wage;
                let st = &mut store.state[w as usize];
                st[state::HAPPINESS] = (st[state::HAPPINESS] + 0.03).min(1.0);
            }
        } else {
            for &w in &workers {
                let st = &mut store.state[w as usize];
                st[state::HAPPINESS] = (st[state::HAPPINESS] - 0.03).max(0.0);
            }
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
                        store.equip_vec[wi] = [0.0; CAP_DIM];
                    }
                }
            }
            store.activity[wi] = Activity::Idle;
            store.last_recipe[wi] = Some(r);
        }

        log.events.push(WorkEvent {
            recipe: r,
            workers,
            apprentices: parts.apprentices.clone(),
            success,
            quality,
        });
        commands.entity(e).despawn();
    }
}

/// Practice for every worker; diffusion from the team's best member to the others and to apprentices.
pub fn learn(
    mut store: ResMut<AgentStore>,
    db: Res<Db>,
    models: Res<Models>,
    params: Res<Params>,
    log: Res<WorkLog>,
) {
    let store = &mut *store;
    for ev in &log.events {
        let s = db.recipes[ev.recipe as usize].skill;
        for &w in &ev.workers {
            let i = w as usize;
            let mult = store.aptitude[i] * (0.5 + store.state[i][state::HAPPINESS]);
            models
                .learning
                .practice(&mut store.cap[i], &s, mult, ev.success);
        }
        let Some(&master) = ev.workers.iter().max_by(|&&a, &&b| {
            dot(&store.cap[a as usize], &s)
                .total_cmp(&dot(&store.cap[b as usize], &s))
                .then(b.cmp(&a))
        }) else {
            continue;
        };
        let master_cap = store.cap[master as usize];
        let mut gain = 0.0;
        for &o in ev.workers.iter().chain(ev.apprentices.iter()) {
            if o != master && store.alive[o as usize] {
                // A trusted master teaches more.
                let o = o as usize;
                let boost =
                    1.0 + params.trust.teaching_bonus * store.trust(o, master as usize).max(0.0);
                let before = store.cap[o];
                let g = models.diffusion.diffuse(&mut store.cap[o], &master_cap, &s);
                for (c, b) in store.cap[o].iter_mut().zip(before) {
                    *c = b + (*c - b) * boost;
                }
                gain += g * boost;
            }
        }
        let mc = &mut store.cap[master as usize];
        for k in 0..CAP_DIM {
            mc[k] += gain * s[k];
        }
    }
}
