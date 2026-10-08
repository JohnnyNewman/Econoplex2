//! Migration: households move to towns that suit them better.
//!
//! Every few weeks some idle adults weigh the towns they could live in: how much
//! food there is per head, what their own trade sells for there, how close the
//! town's people are to them in outlook, whether it was raided this year, and how
//! far the road is, and where the people they trust live. When another town is clearly better, the household (the agent,
//! an idle partner, and their young children) packs up and moves, carrying its
//! skills, wealth and tools. Skills therefore follow demand between towns instead
//! of staying where they were first learned.

use super::streams;
use crate::components::*;
use crate::db::Db;
use crate::store::{Activity, AgentStore};
use crate::world::{Params, SimClock, SimSeed, Towns};
use bevy_ecs::prelude::*;
use rand::Rng;
use sim_data::dims::*;

const IDEOLOGY: usize = MIND_DIM - mind::IDEOLOGY_START;

/// Move one agent to another town: new home, new guild, the walk over there.
pub fn relocate(
    commands: &mut Commands,
    store: &mut AgentStore,
    i: usize,
    to: u16,
    pos: (f32, f32),
    jitter: (f32, f32),
) {
    store.town[i] = to;
    commands.entity(store.entity[i]).insert(TownId(to));
    // Guilds are per town; organize enrolls the agent in the new town's guild.
    if let Some(g) = store.guild[i].take() {
        commands.entity(g.membership).despawn();
    }
    store.target[i] = (pos.0 + jitter.0, pos.1 + jitter.1);
}

#[allow(clippy::too_many_arguments)]
pub fn migrate(
    mut commands: Commands,
    mut store: ResMut<AgentStore>,
    mut towns: ResMut<Towns>,
    db: Res<Db>,
    params: Res<Params>,
    clock: Res<SimClock>,
    seed: Res<SimSeed>,
) {
    let mp = &params.migration;
    if mp.interval == 0 || !clock.tick.is_multiple_of(mp.interval as u64) || towns.0.len() < 2 {
        return;
    }
    let mut rng = seed.rng(clock.tick, streams::MIGRATE);
    let store = &mut *store;
    let dpy = clock.days_per_year;
    let n_towns = towns.0.len();

    // --- what each town offers everyone
    let base: Vec<f32> = towns
        .0
        .iter()
        .map(|t| {
            let food = t.food_stock(&db) / t.residents.len().max(1) as f32;
            let danger = if t.war.attacked > t.war.defended {
                1.0
            } else {
                0.0
            };
            mp.food_weight * (food / mp.food_reference.max(1e-6)).min(2.0)
                - mp.danger_weight * danger
        })
        .collect();
    let ideology: Vec<[f32; IDEOLOGY]> = towns
        .0
        .iter()
        .map(|t| {
            let mut m = [0.0; IDEOLOGY];
            for &i in &t.residents {
                for (k, v) in m.iter_mut().enumerate() {
                    *v += store.mind[i as usize][mind::IDEOLOGY_START + k];
                }
            }
            m.map(|v| v / t.residents.len().max(1) as f32)
        })
        .collect();
    let np = db.content.products.len();
    let mean_price: Vec<f32> = (0..np)
        .map(|p| towns.0.iter().map(|t| t.price[p]).sum::<f32>() / n_towns as f32)
        .collect();
    let dist = |a: usize, b: usize, towns: &Towns| {
        let (pa, pb) = (towns.0[a].pos, towns.0[b].pos);
        ((pa.0 - pb.0).powi(2) + (pa.1 - pb.1).powi(2)).sqrt()
    };

    let free = |s: &AgentStore, j: usize| {
        s.alive[j] && matches!(s.activity[j], Activity::Idle | Activity::Resting)
    };
    let mut moved = vec![false; store.len()];
    for i in 0..store.len() {
        if moved[i]
            || !free(store, i)
            || store.age_years(i, dpy) > mp.max_age
            || rng.random::<f32>() >= mp.consider
        {
            continue;
        }
        let home = store.town[i] as usize;
        // A partner who is busy, or lives elsewhere, keeps the household at home.
        let partner = store.partner[i].map(|p| p as usize);
        if let Some(p) = partner {
            if store.alive[p] && (!free(store, p) || store.town[p] as usize != home) {
                continue;
            }
        }

        let trade = store.last_recipe[i]
            .and_then(|r| db.recipes[r as usize].outputs.first())
            .map(|&(p, _)| p as usize);
        // People this household trusts, by the town they live in.
        let mut friends = vec![0.0f32; n_towns];
        for tie in &store.ties[i] {
            if tie.value > 0.0 && tie.other != u32::MAX && store.alive[tie.other as usize] {
                friends[store.town[tie.other as usize] as usize] += tie.value;
            }
        }
        let utility = |t: usize| {
            let mut d2 = 0.0;
            for (k, m) in ideology[t].iter().enumerate() {
                d2 += (store.mind[i][mind::IDEOLOGY_START + k] - m).powi(2);
            }
            let closeness = 1.0 - d2.sqrt() / (IDEOLOGY as f32).sqrt();
            let sells = trade.map_or(0.0, |p| {
                (towns.0[t].price[p] / mean_price[p].max(1e-6) - 1.0).clamp(-1.0, 2.0)
            });
            base[t]
                + mp.ideology_weight * closeness
                + mp.trade_weight * sells
                + params.trust.migration_weight * friends[t].min(1.0)
        };
        let stay = utility(home);
        let mut best: Option<(usize, f32)> = None;
        for t in 0..n_towns {
            if t == home {
                continue;
            }
            let u = utility(t) - mp.distance_cost * dist(home, t, &towns) / 1000.0;
            if best.is_none_or(|b| u > b.1) {
                best = Some((t, u));
            }
        }
        let Some((to, u)) = best else { continue };
        if u - stay < mp.min_gain {
            continue;
        }

        // --- the household moves together
        let mut household = vec![i];
        if let Some(p) = partner.filter(|&p| store.alive[p]) {
            household.push(p);
        }
        for c in 0..store.len() {
            if let (true, Activity::Child, Some((a, b))) =
                (store.alive[c], store.activity[c], store.parents[c])
            {
                if store.town[c] as usize == home
                    && (a as usize == i
                        || b as usize == i
                        || Some(a as usize) == partner
                        || Some(b as usize) == partner)
                {
                    household.push(c);
                }
            }
        }
        let pos = towns.0[to].pos;
        for &j in &household {
            let jitter = (
                rng.random_range(-40.0..40.0f32),
                rng.random_range(-40.0..40.0f32),
            );
            relocate(&mut commands, store, j, to as u16, pos, jitter);
            moved[j] = true;
        }
        let n = household.len() as u32;
        towns.0[home].moves.left += n;
        towns.0[to].moves.arrived += n;
    }
}
