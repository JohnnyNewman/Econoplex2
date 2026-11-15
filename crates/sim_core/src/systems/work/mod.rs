//! Production: team formation, task progress, success and quality, learning.

mod learning;
mod pay;
mod teams;

pub use learning::*;
pub use pay::*;
pub use teams::*;

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

fn jitter(p: (f32, f32), r: f32, rng: &mut impl Rng) -> (f32, f32) {
    (p.0 + rng.random_range(-r..r), p.1 + rng.random_range(-r..r))
}

/// Takes `need` units of good `p` for a run, with firms on: from the market's
/// stock first, then from firms' stock in order. The buyer (a firm, or the
/// treasury for public work: `None`) pays each seller at the town price; a firm
/// using its own stock, or the treasury the market's, pays nothing.
fn take_input(town: &mut crate::world::Town, p: usize, need: f32, buyer: Option<usize>) {
    use crate::firms::FirmFlow;
    use crate::ledger::Flow;
    let price = town.price[p];
    let mut need = need;
    if let Some(f) = buyer {
        // What the firm uses, for its stock target; its own store first.
        let firm = &mut town.firms[f];
        let np = firm.stock.len().max(p + 1);
        firm.input_used.resize(np, 0.0);
        firm.input_used[p] += need;
        if let Some(own) = firm.inputs.get_mut(p) {
            let q = own.min(need).max(0.0);
            *own -= q;
            need -= q;
            town.flows.add(crate::goods::Flow::Input, p, q);
        }
    }
    let from_market = need.min(town.stock[p].max(0.0));
    town.stock[p] -= from_market;
    town.flows.add(crate::goods::Flow::Input, p, from_market);
    if let Some(f) = buyer {
        let pay = from_market * price;
        town.firms[f].book(FirmFlow::Inputs, pay);
        firm_pays(town, f, crate::firms::TREASURY, pay);
    }
    let mut left = need - from_market;
    // Own stock first, then the others.
    let mut order: Vec<usize> = (0..town.firms.len()).collect();
    if let Some(f) = buyer {
        order.retain(|&k| k != f);
        order.insert(0, f);
    }
    for k in order {
        if left <= 1e-6 {
            break;
        }
        let have = town.firms[k].stock.get(p).copied().unwrap_or(0.0);
        let q = have.min(left);
        if q <= 0.0 {
            continue;
        }
        town.firms[k].stock[p] -= q;
        town.flows.add(crate::goods::Flow::Input, p, q);
        left -= q;
        if buyer == Some(k) {
            continue;
        }
        let pay = q * price;
        town.firms[k].book(FirmFlow::Sales, pay);
        match buyer {
            Some(f) => {
                town.firms[f].book(FirmFlow::Inputs, pay);
                firm_pays(town, f, k as u32, pay);
            }
            None => {
                town.treasury -= crate::coin::Coin::from_coins(pay);
                town.ledger.add(Flow::MarketPurchases, pay);
                town.firms[k].cash += crate::coin::Coin::from_coins(pay);
            }
        }
    }
}

/// Firm `f` pays `amount` to a creditor (a firm, or `firms::TREASURY`) out of its
/// cash; what it can't pay it owes on account (trade credit).
pub fn firm_pays(town: &mut crate::world::Town, f: usize, creditor: u32, amount: f32) {
    let pay = amount.min(town.firms[f].cash.coins().max(0.0));
    town.firms[f].cash -= crate::coin::Coin::from_coins(pay);
    receive(town, creditor, pay);
    let rest = amount - pay;
    if rest > 1e-6 {
        town.firms[f].payables.push((creditor, rest));
    }
}

/// A creditor receives a payment (no sale is booked: it was booked at delivery).
pub fn receive(town: &mut crate::world::Town, creditor: u32, amount: f32) {
    if creditor == crate::firms::TREASURY {
        town.treasury += crate::coin::Coin::from_coins(amount);
        town.ledger.add(crate::ledger::Flow::InputSales, amount);
    } else if let Some(c) = town.firms.get_mut(creditor as usize) {
        c.cash += crate::coin::Coin::from_coins(amount);
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
    env: Res<crate::env::Environment>,
) {
    let mut rng = seed.rng(clock.tick, streams::PRODUCE);
    // Outdoor jobs (nature, buildings, new land) only work while it is light.
    let outdoor_share = crate::env::outdoor_work_share(&env, &params.time);
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
        let day = (workers.len() as f32).powf(params.work.work_exponent);
        task.progress += if db.outdoor[task.recipe as usize] {
            day * outdoor_share
        } else {
            day
        };
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
        {
            // Diagnostics: labour spent, and labour spent on goods already
            // glutted (over target, at their floor price).
            let town = &mut towns.0[task.town as usize];
            let days = n as f32 * m.duration.max(1.0);
            town.work_days[0] += days;
            let glut = m.builds.is_none()
                && !m.outputs.is_empty()
                && m.outputs.iter().all(|&(p, _)| {
                    let pi = p as usize;
                    super::decide::over_target(&db, town, &params, pi, 1.0)
                        && super::decide::at_floor(&db, town, &params, pi)
                });
            if glut {
                town.glut_work_days[0] += days;
            }
        }
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
            // The firm this work belongs to, unless the treasury runs it.
            let firm = (params.market.firms.on() && !crate::firms::is_public(&db, m))
                .then(|| db.recipe_firm[r as usize]);
            for &(p, q) in &m.outputs {
                let q = q * task.batches as f32;
                let pi = p as usize;
                town.produced_this_year[pi] += q;
                town.flows.add(crate::goods::Flow::Made, pi, q);
                if db.coin[pi] == 0.0
                    && db.category(p) != Category::Building
                    && super::decide::over_target(&db, town, &params, pi, 1.0)
                    && super::decide::at_floor(&db, town, &params, pi)
                {
                    town.glut_made_this_year
                        .resize(db.content.products.len(), 0.0);
                    town.glut_made_this_year[pi] += q;
                }
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
                // With workshops as firms, the goods are the firm's to sell; tools
                // and weapons the market buys at once.
                if let Some(fi) = firm {
                    let cat = db.category(p);
                    if cat.is_item() {
                        let pay = (town.price[pi] * q * (0.5 + quality))
                            .min(town.treasury.coins().max(0.0));
                        town.treasury -= crate::coin::Coin::from_coins(pay);
                        town.ledger.add(crate::ledger::Flow::MarketPurchases, pay);
                        town.firms[fi].cash += crate::coin::Coin::from_coins(pay);
                        town.firms[fi].book(crate::firms::FirmFlow::Sales, pay);
                    } else if db.coin[pi] <= 0.0
                        && db.opens[pi].is_none()
                        && cat != Category::Building
                    {
                        town.firms[fi].stock.resize(db.content.products.len(), 0.0);
                        town.firms[fi].stock[pi] += q;
                        continue;
                    }
                }
                if db.coin[pi] > 0.0 {
                    // Struck coin is new money in the treasury, not a good in stock.
                    town.flows.add(crate::goods::Flow::Minted, pi, q);
                    let minted = q * db.coin[pi];
                    let struck = crate::coin::Coin::from_coins(minted);
                    town.treasury += struck;
                    town.minted += struck;
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
                    town.flows.add(crate::goods::Flow::Procured, pi, delivered);
                    town.policy.procurement.retain(|o| o.1 > 1e-3);
                }
                if let Some(kind) = db.opens[pi] {
                    // New land: a fresh site that starts bare and grows in. Two crews
                    // can finish on the last free plot; only the first one gets it.
                    town.stock[pi] -= q;
                    town.flows.add(crate::goods::Flow::Opened, pi, q);
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
                    town.flows.add(crate::goods::Flow::Built, pi, q);
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
                                Item {
                                    durability,
                                    worn_hours: 0.0,
                                },
                                Stored,
                                TownId(task.town),
                                crate::goods::Owner(crate::goods::Holder::Armory(task.town)),
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
                firm,
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
                firm: (params.market.firms.on() && !crate::firms::is_public(&db, m))
                    .then(|| db.recipe_firm[r as usize]),
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
        params.market.firms,
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
    /// The firm that pays the wage; none: the treasury.
    firm: Option<usize>,
}
