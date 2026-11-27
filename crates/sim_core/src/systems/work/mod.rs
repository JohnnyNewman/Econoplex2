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
    mut sites: Query<&mut NaturalResource>,
) {
    let mut rng = seed.rng(clock.steps(), streams::PRODUCE);
    let cal = params.calendar();
    // The working day's last step: work under way is settled and the teams
    // break up for the next day's choices.
    let day_end = clock.work_day_end();
    let firms_on = params.market.firms.on();
    // A worker's work this step, in days: the step's slice of the working
    // hours on each day of the month. Outdoor jobs (nature, buildings, new
    // land) only work while it is light.
    let indoor_step = crate::env::work_days_this_step(&clock, &env, &params.time, false);
    let outdoor_step = crate::env::work_days_this_step(&clock, &env, &params.time, true);
    let mut order: Vec<(u64, Entity)> = tasks.iter().map(|(e, t, _)| (t.serial, e)).collect();
    order.sort_unstable();
    let store = &mut *store;
    // A long step is worked in slices of at most `production_hours`, each
    // delivering its finished runs before the next, so goods move along a
    // chain within the step as they would at a finer step.
    let slices = if indoor_step > 0.0 || outdoor_step > 0.0 {
        (cal.step_hours() / params.time.production_hours.max(1e-3))
            .ceil()
            .max(1.0) as u32
    } else {
        1
    };
    let indoor_days = indoor_step / slices as f32;
    let outdoor_days = outdoor_step / slices as f32;
    let mut claims: Vec<Claim> = Vec::new();
    // Building sites finished in an earlier slice (despawned at the step's end).
    let mut built: Vec<Entity> = Vec::new();
    for _ in 0..slices {
        // Phase A: this step's work on every task (a batch makes progress while
        // its team works and delivers when the work is done); collect the ones
        // that finish this step.
        let mut done: Vec<(Entity, u32, Vec<u32>)> = Vec::new();
        for &(_, e) in &order {
            if built.contains(&e) {
                continue;
            }
            let Ok((_, mut task, parts)) = tasks.get_mut(e) else {
                continue;
            };
            // Members called away (to a squad, a raid, another town) have left it.
            let workers: Vec<u32> = parts
                .workers
                .iter()
                .copied()
                .filter(|&w| {
                    store.alive[w as usize] && store.activity[w as usize] == Activity::Working(e)
                })
                .collect();
            if workers.is_empty() {
                commands.entity(e).despawn();
                continue;
            }
            let r = task.recipe;
            let m = &db.recipes[r as usize];
            let worked = if db.outdoor[r as usize] {
                outdoor_days
            } else {
                indoor_days
            };
            // On a task: busy (and training its muscles) until it is done.
            for &w in &workers {
                store.worked_today[w as usize] = Some(r);
            }
            let rate = team_rate(store, &db, &models, &params, r, &workers);
            if worked > 0.0 {
                for &w in &workers {
                    let st = &mut store.state[w as usize];
                    st[state::FATIGUE] = (st[state::FATIGUE]
                        + params.needs.fatigue_per_work_day * (0.5 + m.phys_total) * worked)
                        .min(1.0);
                }
                // Runs this step's work does, as a flow: inputs and nature go in
                // at the same rate (fractional amounts), at a fraction of the
                // rate when they run short.
                let full = rate * worked / m.duration.max(1e-6);
                let town = &mut towns.0[task.town as usize];
                let buyer = (firms_on && !crate::firms::is_public(&db, m))
                    .then(|| db.recipe_firm[r as usize]);
                let mut f = if buyer.is_some_and(|k| town.firms[k].closed) {
                    0.0f32
                } else {
                    1.0f32
                };
                for &(p, q) in &m.inputs {
                    let pi = p as usize;
                    let mut have = town.stock[pi].max(0.0);
                    if firms_on {
                        have += town.firm_stock(pi);
                        if let Some(k) = buyer {
                            have += town.firms[k].inputs.get(pi).copied().unwrap_or(0.0);
                        }
                    }
                    f = f.min(have / (q * full).max(1e-9));
                }
                let out: f32 = m.outputs.iter().map(|o| o.1).sum();
                if let Some(kind) = m.nature {
                    let have: f32 = town
                        .nature_sites
                        .iter()
                        .filter_map(|&s| sites.get(s).ok())
                        .filter(|n| n.kind == kind)
                        .map(|n| n.amount.max(0.0))
                        .sum();
                    f = f.min(have / (out * full).max(1e-9));
                }
                if firms_on {
                    let per_run: f32 = m
                        .inputs
                        .iter()
                        .map(|&(p, q)| town.price[p as usize] * q)
                        .sum();
                    if per_run > 0.0 {
                        let cash = match buyer {
                            Some(k) => {
                                town.firms[k].cash.coins().max(0.0)
                                    + params.market.firms.credit_room(&town.firms[k])
                            }
                            None => town.spendable(&params.market),
                        };
                        f = f.min(cash / (per_run * full).max(1e-9));
                    }
                }
                let runs = full * f.clamp(0.0, 1.0);
                if runs > 0.0 {
                    town.input_today.resize(town.stock.len(), 0.0);
                    town.index_year.inputs.resize(town.stock.len(), 0.0);
                    for &(p, q) in &m.inputs {
                        let pi = p as usize;
                        let need = q * runs;
                        if firms_on {
                            take_input(town, pi, need, buyer);
                        } else {
                            town.stock[pi] -= need;
                            town.flows.add(crate::goods::Flow::Input, pi, need);
                        }
                        town.input_today[pi] += need;
                        town.index_year.inputs[pi] += need;
                        task.input_quality = task.input_quality.min(town.quality[pi]);
                    }
                    if let Some(kind) = m.nature {
                        let mut need = out * runs;
                        for &s in &town.nature_sites {
                            if need <= 0.0 {
                                break;
                            }
                            let Ok(mut n) = sites.get_mut(s) else {
                                continue;
                            };
                            if n.kind != kind || n.amount <= 0.0 {
                                continue;
                            }
                            let q = n.amount.min(need);
                            n.amount -= q;
                            need -= q;
                        }
                    }
                    task.progress += runs;
                    task.worked_runs += runs;
                }
            }
            // Whole runs are delivered as they are done; the run under way goes on
            // with the next team on the recipe (`Town::wip`).
            let whole = task.progress.floor();
            let whole = if m.builds.is_some() {
                whole.min(1.0)
            } else {
                whole
            };
            if whole >= 1.0 {
                task.batches = whole as u32;
                task.progress = (task.progress - whole).max(0.0);
                done.push((e, r, workers));
            } else {
                task.batches = 0;
            }
        }

        // Phase B, in parallel: each finishing team's effective skill. It reads only
        // the team's own members, whom no other task touches, so it is the same as
        // working it out task by task.
        let effs: Vec<f32> = {
            let store = &*store;
            crate::systems::par_map(&done, |(_, r, workers)| {
                team_skill(store, &db, &models, &params, *r, workers)
            })
        };

        // Phase C, in task order: resolve each finished step.
        for ((e, r, workers), eff) in done.into_iter().zip(effs) {
            let Ok((_, mut task, parts)) = tasks.get_mut(e) else {
                continue;
            };
            let m = &db.recipes[r as usize];
            let n = workers.len();
            // Days the team worked on it (a run's success or failure cheers or
            // annoys once, as a day's work did).
            let rate_day = team_rate(store, &db, &models, &params, r, &workers);
            // Days each member worked for these runs.
            let team_days = task.batches as f32 * m.duration / rate_day;
            let uses = {
                let whole = team_days.floor();
                (whole + f32::from(rng.random::<f32>() < team_days - whole)) as u32
            };
            // Practice for the part of these runs this team did (the rest
            // was counted when the team before it broke up).
            let own = task.worked_runs.min(task.batches as f32);
            let practice_days = (own * practice_per_run(m.duration, rate_day)).max(1e-3);
            // A day's work cheers (or annoys) as it did when a simulated day was
            // one step: by the month's kick, for the share of the month worked.
            let mood = cal
                .every(clock.steps_per_day.max(1))
                .kick(0.03, params.needs.happiness_rate)
                * (team_days / cal.month_days()).min(1.0);
            {
                // Diagnostics: labour spent, and labour spent on goods already
                // glutted (over target, at their floor price).
                let town = &mut towns.0[task.town as usize];
                // Worker-days these runs took.
                let days = n as f32 * team_days;
                town.work_days[0] += days;
                town.recipe_days_this_year.resize(db.recipes.len(), 0.0);
                town.recipe_days_this_year[r as usize] += days;
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
                                .min(town.spendable(&params.market));
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
                        town.minted += struck;
                        if params.market.treasury == crate::config::Treasury::Issues {
                            // A town that issues its own coin keeps metal coin
                            // apart, to pay other towns with.
                            town.metal += struck;
                        } else {
                            town.treasury += struck;
                            town.ledger.add(crate::ledger::Flow::Minted, minted);
                        }
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
                    } else if db.house[pi].is_some() {
                        // A finished house adds to the town's houses.
                        town.stock[pi] -= q;
                        town.flows.add(crate::goods::Flow::Built, pi, q);
                        town.houses.resize(db.content.products.len(), 0);
                        town.houses[pi] += q.round().max(0.0) as u32;
                        town.built += 1;
                        town.price[pi] = 0.0;
                    } else if db.category(p) == Category::Building {
                        // The finished building becomes a workplace (stock tracks nothing here).
                        town.stock[pi] -= q;
                        town.flows.add(crate::goods::Flow::Built, pi, q);
                        // An ordered building goes where the order put it.
                        let commission =
                            town.policy.commissions.iter().position(|c| c.building == p);
                        let pos = match commission.map(|k| town.policy.commissions.remove(k)) {
                            Some(crate::policy::Commission { pos: Some(pos), .. }) => pos,
                            _ => town.next_building_pos(),
                        };
                        town.built += 1;
                        let e = commands
                            .spawn((
                                Product { def: p, quality },
                                Structure {
                                    footprint: db.content.products[pi].footprint.unwrap_or_default(),
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
                let days = team_days.max(1e-3);
                // Apprentices on the team cost the market less.
                let aw = params.decision.matching.apprentice_wage();
                let trainees: Vec<u32> = workers
                    .iter()
                    .copied()
                    .filter(|w| parts.trainees.contains(w))
                    .collect();
                let weight = (n as f32 - trainees.len() as f32 * (1.0 - aw)) / n as f32;
                // Each member's share of the pay: their contribution, an
                // apprentice's times `apprentice_wage`.
                let shares: Vec<f32> = contributions(store, &db, &models, &params, r, &workers)
                    .into_iter()
                    .zip(&workers)
                    .map(|(c, w)| if trainees.contains(w) { c * aw } else { c })
                    .collect();
                claims.push(Claim {
                    town: t,
                    recipe: r,
                    workers: workers.clone(),
                    shares,
                    wage: (share * (value - input_cost)).max(0.0) * weight,
                    subsidy,
                    days,
                    runs: task.batches,
                    firm,
                });
                for &w in &workers {
                    let st = &mut store.state[w as usize];
                    st[state::HAPPINESS] = (st[state::HAPPINESS] + mood).min(1.0);
                }
            } else {
                for &w in &workers {
                    let st = &mut store.state[w as usize];
                    st[state::HAPPINESS] = (st[state::HAPPINESS] - mood).max(0.0);
                }
                // A failed step earns nothing.
                let days = team_days.max(1e-3);
                claims.push(Claim {
                    town: t,
                    recipe: r,
                    workers: workers.clone(),
                    shares: vec![1.0; workers.len()],
                    wage: 0.0,
                    subsidy: 0.0,
                    days,
                    runs: task.batches,
                    firm: (params.market.firms.on() && !crate::firms::is_public(&db, m))
                        .then(|| db.recipe_firm[r as usize]),
                });
            }

            // Tool wear: a use for every day of the run.
            for &w in &workers {
                let wi = w as usize;
                if let Some(tool) = store.equipped[wi] {
                    if let Ok(mut it) = items.get_mut(tool) {
                        it.durability = it.durability.saturating_sub(uses);
                        if it.durability == 0 {
                            commands.entity(tool).despawn();
                            store.equipped[wi] = None;
                            store.equip_vec[wi].iter_mut().for_each(|x| *x = 0.0);
                        }
                    }
                }
                store.last_recipe[wi] = Some(r);
            }

            log.events.push(WorkEvent {
                recipe: r,
                town: task.town,
                workers,
                apprentices: parts.apprentices.clone(),
                success,
                quality,
                days: practice_days,
            });
            task.worked_runs -= own;
            if m.builds.is_some() {
                // A building site is done once it stands.
                for &w in &parts.workers {
                    if store.activity[w as usize] == Activity::Working(e) {
                        store.activity[w as usize] = Activity::Idle;
                    }
                }
                commands.entity(e).despawn();
                built.push(e);
            }
        }
    }
    // The day is over: the teams break up, and everyone chooses again; the
    // runs under way wait for the next team on the recipe.
    if day_end {
        for (e, task, parts) in tasks.iter() {
            if built.contains(&e) {
                continue; // just built, and gone
            }
            // The team learns from the work it leaves to the next one, as it
            // would from a run, judged by how well it was going.
            let workers: Vec<u32> = parts
                .workers
                .iter()
                .copied()
                .filter(|&w| {
                    store.alive[w as usize] && store.activity[w as usize] == Activity::Working(e)
                })
                .collect();
            if task.worked_runs > 1e-4 && !workers.is_empty() {
                let m = &db.recipes[task.recipe as usize];
                let eff = team_skill(store, &db, &models, &params, task.recipe, &workers);
                let success = m.builds.is_some()
                    || rng.random::<f32>() < models.success.probability(eff, m.difficulty);
                let quality = models.quality.combine(
                    models.quality.step_quality(eff, m.difficulty),
                    task.input_quality,
                );
                let rate = team_rate(store, &db, &models, &params, task.recipe, &workers);
                log.events.push(WorkEvent {
                    recipe: task.recipe,
                    town: task.town,
                    workers,
                    apprentices: parts.apprentices.clone(),
                    success,
                    quality,
                    days: task.worked_runs * practice_per_run(m.duration, rate),
                });
            }
            let town = &mut towns.0[task.town as usize];
            town.wip.resize(db.recipes.len(), 0.0);
            town.wip[task.recipe as usize] += task.progress;
            for &w in &parts.workers {
                if store.activity[w as usize] == Activity::Working(e) {
                    store.activity[w as usize] = Activity::Idle;
                }
            }
            commands.entity(e).despawn();
        }
    }

    settle_wages(claims, &mut towns, store, &decision, &params.market);
}

/// A finished step's claim on its town market.
struct Claim {
    town: usize,
    recipe: u32,
    workers: Vec<u32>,
    /// Each worker's share of the pay, in `workers` order.
    shares: Vec<f32>,
    wage: f32,
    subsidy: f32,
    /// Days the team worked on it.
    days: f32,
    /// Runs of the recipe in it.
    runs: u32,
    /// The firm that pays the wage; none: the treasury.
    firm: Option<usize>,
}

/// A team's effective skill at recipe `r`: its pooled skill less the cost of
/// coordinating, plus trust and physique, times how fit its members are.
fn team_skill(
    store: &AgentStore,
    db: &Db,
    models: &Models,
    params: &Params,
    r: u32,
    workers: &[u32],
) -> f32 {
    let m = &db.recipes[r as usize];
    let n = workers.len();
    let caps: Vec<CapVec> = workers
        .iter()
        .map(|&w| store.effective_cap(w as usize))
        .collect();
    let team = models.pooling.pool(&caps);
    let mut sorted = workers.to_vec();
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
}

/// How much each member of a team counts (`work.contribution`): under
/// SkillPay and Skill their own chance of doing recipe `r` right over the
/// team's best; under Heads one each.
pub(super) fn contributions(
    store: &AgentStore,
    db: &Db,
    models: &Models,
    params: &Params,
    r: u32,
    workers: &[u32],
) -> Vec<f32> {
    if params.work.contribution == crate::config::Contribution::Heads {
        return vec![1.0; workers.len()];
    }
    let m = &db.recipes[r as usize];
    let own: Vec<f32> = workers
        .iter()
        .map(|&w| {
            let i = w as usize;
            let cap = store.effective_cap(i);
            let eff = (dot(&cap, &m.skill)
                + physique_bonus(&store.phys[i], m, params.physique.weight))
                * state_factor(&store.state[i], &params.needs);
            models.success.probability(eff, m.difficulty)
        })
        .collect();
    let best = own.iter().copied().fold(0.0f32, f32::max);
    if best <= 1e-6 {
        return vec![1.0; workers.len()];
    }
    own.iter().map(|c| (c / best).max(0.05)).collect()
}

/// A team's work rate per day: its members, each counted by their
/// contribution, to the power `work_exponent`.
pub(super) fn team_rate(
    store: &AgentStore,
    db: &Db,
    models: &Models,
    params: &Params,
    r: u32,
    workers: &[u32],
) -> f32 {
    let heads: f32 = if params.work.contribution == crate::config::Contribution::Skill {
        contributions(store, db, models, params, r, workers)
            .iter()
            .sum()
    } else {
        workers.len() as f32
    };
    heads.max(1e-3).powf(params.work.work_exponent)
}

/// Days of practice one run gives its team (`rate` = n^work_exponent): its
/// days of work, counted as one lesson a day for runs shorter than a day.
fn practice_per_run(duration: f32, rate: f32) -> f32 {
    let days = duration / rate.max(1e-6);
    days / days.max(1.0)
}
