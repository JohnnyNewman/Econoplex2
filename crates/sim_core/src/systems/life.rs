//! Lifecycle: aging, death, partnering and births with genetic inheritance.
//!
//! Genetics sets each agent's physique *potential*, temperament and learning
//! aptitude; training fills the potential in. Skills are never inherited, only
//! taught: children of smiths become smiths by apprenticing with their parents.

use super::body::age_curve;
use super::streams;
use crate::components::*;
use crate::db::Db;
use crate::store::{Activity, AgentStore, NewAgent};
use crate::world::{Params, SimClock, SimSeed, Towns};
use bevy_ecs::prelude::*;
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use sim_data::dims::*;

/// Mate choice: ideological affinity plus physical fitness.
pub fn mate_score(store: &AgentStore, a: usize, b: usize) -> f32 {
    let mut d2 = 0.0;
    for k in mind::IDEOLOGY_START..MIND_DIM {
        d2 += (store.mind[a][k] - store.mind[b][k]).powi(2);
    }
    let affinity = 1.0 - d2.sqrt() / ((MIND_DIM - mind::IDEOLOGY_START) as f32).sqrt();
    let fitness = store.phys[b].iter().sum::<f32>() / PHYS_DIM as f32;
    affinity + 0.3 * fitness
}

/// Per gene: with probability `heritability` take one parent's value, otherwise
/// blend; then add mutation noise.
fn inherit<const N: usize>(
    a: &[f32; N],
    b: &[f32; N],
    h: f32,
    mutation: f32,
    rng: &mut ChaCha8Rng,
    lo: f32,
    hi: f32,
) -> [f32; N] {
    std::array::from_fn(|k| {
        let base = if rng.random::<f32>() < h {
            if rng.random_bool(0.5) {
                a[k]
            } else {
                b[k]
            }
        } else {
            0.5 * (a[k] + b[k])
        };
        (base + mutation * rng.random_range(-1.0..1.0f32)).clamp(lo, hi)
    })
}

/// Dead agents' wealth goes to the partner, else split among living children,
/// else to the town treasury, so money never leaves the economy.
fn bequeath(store: &mut AgentStore, towns: &mut Towns, dead: &[usize]) {
    let mut children: std::collections::BTreeMap<u32, Vec<usize>> = Default::default();
    if dead.iter().any(|&i| store.partner[i].is_none()) {
        for c in 0..store.len() {
            if let (true, Some((a, b))) = (store.alive[c], store.parents[c]) {
                children.entry(a).or_default().push(c);
                children.entry(b).or_default().push(c);
            }
        }
    }
    for &i in dead {
        let w = std::mem::take(&mut store.wealth[i]);
        if let Some(p) = store.partner[i].filter(|&p| store.alive[p as usize]) {
            store.wealth[p as usize] += w;
            continue;
        }
        match children.get(&(i as u32)) {
            Some(kids) if !kids.is_empty() => {
                let share = w / kids.len() as f32;
                for &c in kids {
                    store.wealth[c] += share;
                }
            }
            _ => towns.0[store.town[i] as usize].treasury += w,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn lifecycle(
    mut commands: Commands,
    mut store: ResMut<AgentStore>,
    mut towns: ResMut<Towns>,
    db: Res<Db>,
    params: Res<Params>,
    clock: Res<SimClock>,
    seed: Res<SimSeed>,
) {
    let mut rng = seed.rng(clock.tick, streams::LIFE);
    let lp = &params.life;
    let dpy = lp.days_per_year;
    let store = &mut *store;
    let n = store.len();

    // --- aging and death
    let mut dead = Vec::new();
    for i in 0..n {
        if !store.alive[i] {
            continue;
        }
        store.age_days[i] += 1;
        let age = store.age_years(i, dpy);
        if store.activity[i] == Activity::Child && age >= lp.adult_age {
            store.activity[i] = Activity::Idle;
        }
        let hazard = lp.mortality_a * (lp.mortality_b * age).exp() / dpy as f32;
        if store.state[i][state::HEALTH] <= 0.0 || rng.random::<f32>() < hazard {
            store.alive[i] = false;
            commands.entity(store.entity[i]).despawn();
            dead.push(i);
            if let Some(g) = store.guild[i].take() {
                commands.entity(g.membership).despawn();
            }
            if let Some(tool) = store.equipped[i].take() {
                commands.entity(tool).remove::<EquippedBy>().insert(Stored);
                towns.0[store.town[i] as usize].armory.push(tool);
            }
        }
    }
    // Bequests read partners, so they run before the partner links are cleared.
    bequeath(store, &mut towns, &dead);
    for &i in &dead {
        if let Some(p) = store.partner[i].take() {
            store.partner[p as usize] = None;
        }
    }

    // --- partnering
    if lp.partner_interval > 0 && clock.tick.is_multiple_of(lp.partner_interval as u64) {
        for town in &towns.0 {
            let single = |i: u32, female: bool, s: &AgentStore| {
                let i = i as usize;
                let age = s.age_years(i, dpy);
                s.alive[i]
                    && s.female[i] == female
                    && s.partner[i].is_none()
                    && age >= lp.min_birth_age
                    && age <= lp.max_birth_age + 5.0
            };
            let women: Vec<u32> = town
                .residents
                .iter()
                .copied()
                .filter(|&i| single(i, true, store))
                .collect();
            let mut men: Vec<u32> = town
                .residents
                .iter()
                .copied()
                .filter(|&i| single(i, false, store))
                .collect();
            for &w in &women {
                if men.is_empty() {
                    break;
                }
                let mut best: Option<(usize, f32)> = None;
                for _ in 0..5 {
                    let k = rng.random_range(0..men.len());
                    let s = mate_score(store, w as usize, men[k] as usize);
                    if best.is_none_or(|b| s > b.1) {
                        best = Some((k, s));
                    }
                }
                if let Some((k, s)) = best {
                    if s >= lp.partner_min_score {
                        let m = men.swap_remove(k);
                        store.partner[w as usize] = Some(m);
                        store.partner[m as usize] = Some(w);
                    }
                }
            }
        }
    }

    // --- births
    let food_factor: Vec<f32> = towns
        .0
        .iter()
        .map(|t| {
            (t.food_stock(&db) / t.residents.len().max(1) as f32 / lp.food_for_births)
                .clamp(0.0, 1.0)
        })
        .collect();
    let gp = &params.genetics;
    for mother in 0..n {
        if !store.alive[mother] || !store.female[mother] {
            continue;
        }
        let Some(father) = store.partner[mother] else {
            continue;
        };
        let father = father as usize;
        let age = store.age_years(mother, dpy);
        if age < lp.min_birth_age || age > lp.max_birth_age {
            continue;
        }
        let t = store.town[mother] as usize;
        if rng.random::<f32>() >= lp.births_per_year / dpy as f32 * food_factor[t] {
            continue;
        }
        let potential = inherit(
            &store.potential[mother],
            &store.potential[father],
            gp.heritability,
            gp.mutation,
            &mut rng,
            0.05,
            1.0,
        );
        let temperament = inherit(
            &store.temperament[mother],
            &store.temperament[father],
            gp.heritability,
            gp.mutation,
            &mut rng,
            0.0,
            1.0,
        );
        let aptitude = (0.5 * (store.aptitude[mother] + store.aptitude[father])
            + gp.mutation * rng.random_range(-2.0..2.0f32))
        .clamp(0.3, 2.0);
        // Personality from temperament; ideology from the parents' culture.
        let mut mind_v = temperament;
        for k in mind::IDEOLOGY_START..MIND_DIM {
            mind_v[k] = (0.5 * (store.mind[mother][k] + store.mind[father][k])
                + 0.05 * rng.random_range(-1.0..1.0f32))
            .clamp(0.0, 1.0);
        }
        let cap: CapVec =
            std::array::from_fn(|_| rng.random::<f32>() * crate::world::STARTING_SKILL_NOISE * 0.5);
        let c = age_curve(0.0, &params.physique) * params.physique.untrained_level;
        let phys: PhysVec = std::array::from_fn(|k| potential[k] * c);
        let entity = commands.spawn_empty().id();
        let idx = store.push(
            NewAgent {
                entity,
                town: t as u16,
                female: rng.random_bool(0.5),
                age_days: 0,
                cap,
                mind: mind_v,
                phys,
                potential,
                temperament,
                aptitude,
                parents: Some((mother as u32, father as u32)),
                target: store.target[mother],
                wealth: 0.0,
            },
            false,
        );
        commands
            .entity(entity)
            .insert((Agent { idx }, TownId(t as u16)));
    }

    // --- rebuild residents
    for town in towns.0.iter_mut() {
        town.residents.clear();
    }
    for i in 0..store.len() {
        if store.alive[i] {
            towns.0[store.town[i] as usize].residents.push(i as u32);
        }
    }
}
