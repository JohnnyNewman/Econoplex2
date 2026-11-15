//! Organizations: guild membership and roles from proficiency; tools from the armory.

use crate::components::*;
use crate::db::Db;
use crate::store::{Activity, AgentStore, GuildLink, Role};
use crate::world::{Params, SimClock, Towns};
use bevy_ecs::prelude::*;

pub fn organize(
    mut commands: Commands,
    mut store: ResMut<AgentStore>,
    mut towns: ResMut<Towns>,
    db: Res<Db>,
    params: Res<Params>,
    clock: Res<SimClock>,
    items: Query<&Product, With<Stored>>,
) {
    let op = &params.organize;
    if !clock.every_months(op.interval) {
        return;
    }
    let dpy = clock.months_per_year;
    let store = &mut *store;
    let nd = db.domain_dirs.len();

    // Tools in each town's armory, by domain. The armory also holds weapons and
    // goods nobody equips; scanning it per agent made this quadratic in big towns.
    let mut tools: Vec<Vec<Vec<(Entity, u32, f32)>>> = towns
        .0
        .iter()
        .map(|town| {
            let mut by_domain = vec![Vec::new(); nd];
            for &e in &town.armory {
                let Ok(prod) = items.get(e) else { continue };
                let def = &db.content.products[prod.def as usize];
                // Only tools: weapons, armour, shields and mounts are for soldiers.
                if crate::goods::Slot::of(def) != Some(crate::goods::Slot::Tool) {
                    continue;
                }
                let Some(eq) = &def.equip else {
                    continue;
                };
                if let Some(d) = db.content.domain(&eq.domain) {
                    by_domain[d as usize].push((e, prod.def, prod.quality));
                }
            }
            by_domain
        })
        .collect();

    // Apprentice places per town and trade: each master may take
    // `apprentices_per_master` (a hard limit), less those already taken.
    let mut places = vec![vec![0i64; nd]; towns.0.len()];
    for i in 0..store.len() {
        if let (true, Some(g)) = (store.alive[i], store.guild[i]) {
            let t = store.town[i] as usize;
            places[t][g.domain as usize] += match g.role {
                Role::Master => op.apprentices_per_master as i64,
                Role::Apprentice => -1,
                Role::Member => 0,
            };
        }
    }
    // What each trade pays in each town (mean over its jobs that pay).
    let domain_pay: Vec<Vec<f32>> = towns
        .0
        .iter()
        .map(|town| {
            let mut sum = vec![0.0f32; nd];
            let mut n = vec![0.0f32; nd];
            for (r, m) in db.recipes.iter().enumerate() {
                let p = town.pay_rate.get(r).copied().unwrap_or(0.0);
                if p > 0.0 {
                    sum[m.domain as usize] += p;
                    n[m.domain as usize] += 1.0;
                }
            }
            sum.iter().zip(&n).map(|(s, n)| s / n.max(1.0)).collect()
        })
        .collect();
    for i in 0..store.len() {
        if !store.alive[i] {
            continue;
        }
        let t = store.town[i] as usize;
        let age = store.age_years(i, dpy);

        // --- trade and rank: a trade is kept; the young enter it as apprentices
        // into an open place; rank rises with years in it.
        let years_in = |since: u64| (clock.tick.saturating_sub(since)) as f32 / dpy.max(1) as f32;
        let rank = |since: u64| {
            if age < params.life.adult_age {
                Role::Apprentice
            } else if years_in(since) >= op.master_years {
                Role::Master
            } else {
                Role::Member
            }
        };
        let desired: Option<(u32, Role, u64)> = match store.guild[i] {
            Some(g) => Some((g.domain, rank(g.since), g.since)),
            None if age >= params.life.apprentice_age && age < params.life.adult_age => {
                // Open places in town: the parent's trade first, then the trade
                // that pays best.
                let parent = store.parents[i]
                    .and_then(|(a, b)| store.guild[a as usize].or(store.guild[b as usize]))
                    .map(|g| g.domain);
                let open = |d: usize| places[t][d] > 0;
                let pick = parent.filter(|&d| open(d as usize)).or_else(|| {
                    (0..nd)
                        .filter(|&d| open(d))
                        .max_by(|&a, &b| domain_pay[t][a].total_cmp(&domain_pay[t][b]))
                        .map(|d| d as u32)
                });
                pick.map(|d| {
                    places[t][d as usize] -= 1;
                    (d, Role::Apprentice, clock.tick)
                })
            }
            None if age >= params.life.adult_age => {
                // An adult who never apprenticed joins the guild of their best
                // skill once good enough. At the start of the game the grown
                // population has been in its trade since it came of age.
                let (best, prof) = (0..nd)
                    .map(|d| (d as u32, db.domain_proficiency(&store.cap[i], d)))
                    .fold((0, f32::MIN), |acc, x| if x.1 > acc.1 { x } else { acc });
                (prof >= op.guild_threshold).then(|| {
                    let since = if clock.tick < dpy as u64 {
                        let adult_days =
                            ((age - params.life.adult_age).max(0.0) * dpy as f32) as u64;
                        clock.tick.saturating_sub(adult_days)
                    } else {
                        clock.tick
                    };
                    (best, rank(since), since)
                })
            }
            None => None,
        };
        let current = store.guild[i].map(|g| (g.domain, g.role, g.since));
        if current != desired {
            if let Some(old) = store.guild[i].take() {
                commands.entity(old.membership).despawn();
            }
            if let Some((domain, role, since)) = desired {
                let org = towns.0[t].guilds[domain as usize];
                let membership = commands
                    .spawn(Membership {
                        agent: i as u32,
                        org,
                        role,
                        since_day: since,
                    })
                    .id();
                store.guild[i] = Some(GuildLink {
                    org,
                    membership,
                    domain,
                    role,
                    since,
                });
            }
        }

        // --- tools: an idle guild member without a tool takes the best matching one
        if store.equipped[i].is_some() || store.activity[i] != Activity::Idle {
            continue;
        }
        let Some(g) = store.guild[i] else { continue };
        let town = &mut towns.0[t];
        let shelf = &mut tools[t][g.domain as usize];
        let mut best: Option<(usize, f32)> = None;
        for (k, &(_, def, quality)) in shelf.iter().enumerate() {
            let cost = town.price[def as usize] * (0.5 + quality);
            if cost <= store.wealth[i].coins() + params.market.tool_credit
                && best.is_none_or(|b| quality > b.1)
            {
                best = Some((k, quality));
            }
        }
        if let Some((k, _)) = best {
            let (e, _, _) = shelf.remove(k);
            crate::goods::from_armory(&mut town.armory, e);
            let prod = items.get(e).unwrap();
            // Tools are bought from the town, on credit if need be; wages repay it.
            let cost = town.price[prod.def as usize] * (0.5 + prod.quality);
            store.wealth[i] -= crate::coin::Coin::from_coins(cost);
            town.treasury += crate::coin::Coin::from_coins(cost);
            town.ledger.add(crate::ledger::Flow::ToolSales, cost);
            town.index_year.consumed.resize(town.stock.len(), 0.0);
            town.index_year.consumed[prod.def as usize] += 1.0;
            let bonus = db.content.products[prod.def as usize]
                .equip
                .as_ref()
                .unwrap()
                .bonus
                * (0.5 + prod.quality);
            let before = town.stock[prod.def as usize];
            town.stock[prod.def as usize] = (before - 1.0).max(0.0);
            town.flows.add(
                crate::goods::Flow::Equipped,
                prod.def as usize,
                before - town.stock[prod.def as usize],
            );
            crate::goods::to_person(&mut commands, store, i, crate::goods::Slot::Tool, e);
            let dir = &db.domain_dirs[g.domain as usize];
            store.equip_vec[i] = dir.iter().map(|d| d * bonus).collect();
        }
    }
}
