//! Squads and raids.
//!
//! Each town keeps a squad sized to what its treasury can pay. Soldiers draw
//! longswords from the armory, drill every day (learning the Fighting skill),
//! and are paid from the treasury, so an army is a running cost the economy has
//! to carry. A town whose squad clearly outmatches a neighbor's defense may raid
//! it: the squad marches over, fights, and on a win carries off part of the
//! defender's treasury and stores. Fallen fighters die through the normal
//! lifecycle (their health drops to zero), so their skills are lost with them.

use super::streams;
use crate::components::*;
use crate::db::Db;
use crate::models::Models;
use crate::store::{Activity, AgentStore};
use crate::world::{Params, Raid, SimClock, SimSeed, Towns};
use bevy_ecs::prelude::*;
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use sim_data::dims::*;
use sim_data::Category;

/// Fighting strength of one agent.
pub fn fighter_power(store: &AgentStore, i: usize, fight: &CapVec) -> f32 {
    let skill = dot(&store.cap[i], fight).max(0.0);
    let p = &store.phys[i];
    let build = (p[0] + p[1] + p[2]) / 3.0;
    let health = store.state[i][state::HEALTH].clamp(0.0, 1.0);
    (0.3 + skill) * (0.5 + build) * (1.0 + store.weapon_power[i]) * (0.5 + 0.5 * health)
}

#[allow(clippy::too_many_arguments)]
pub fn military(
    mut commands: Commands,
    mut store: ResMut<AgentStore>,
    mut towns: ResMut<Towns>,
    db: Res<Db>,
    models: Res<Models>,
    params: Res<Params>,
    clock: Res<SimClock>,
    seed: Res<SimSeed>,
    weapons: Query<&Product, With<Stored>>,
) {
    let mp = &params.military;
    let Some(fight_idx) = db.content.skill("fighting") else {
        return;
    };
    let fight = db.skill_vecs[fight_idx as usize];
    let mut rng = seed.rng(clock.tick, streams::MILITARY);
    let store = &mut *store;
    let dpy = clock.days_per_year;
    let n_towns = towns.0.len();

    for town in towns.0.iter_mut() {
        town.squad.retain(|&s| store.alive[s as usize]);
    }

    // --- muster: recruit, release and arm
    if mp.muster_interval > 0 && clock.tick.is_multiple_of(mp.muster_interval as u64) {
        for t in 0..n_towns {
            let town = &mut towns.0[t];
            let adults = town
                .residents
                .iter()
                .filter(|&&i| store.activity[i as usize] != Activity::Child)
                .count();
            // Keep enough in the treasury for a month of pay.
            let affordable = (town.treasury.max(0.0) / (mp.soldier_pay * 30.0).max(1e-6)) as usize;
            let want = ((adults as f32 * mp.soldier_share) as usize).min(affordable);
            let at_home = town.raid.is_none();

            if town.squad.len() < want && at_home {
                let mut candidates: Vec<(f32, u32)> = town
                    .residents
                    .iter()
                    .copied()
                    .filter(|&i| {
                        let i = i as usize;
                        store.activity[i] == Activity::Idle
                            && store.age_years(i, dpy) <= mp.max_recruit_age
                            && store.state[i][state::HEALTH] > 0.6
                    })
                    .map(|i| (recruit_score(store, i as usize), i))
                    .collect();
                candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
                for &(_, i) in candidates.iter().take(want - town.squad.len()) {
                    store.activity[i as usize] = Activity::Soldier;
                    town.squad.push(i);
                }
            } else if town.squad.len() > want && at_home {
                // Release the most recent recruits first.
                while town.squad.len() > want {
                    let i = town.squad.pop().unwrap() as usize;
                    release(&mut commands, store, &mut town.armory, i);
                }
            }

            // Soldiers without a weapon take the best longsword in the armory.
            for k in 0..town.squad.len() {
                let i = town.squad[k] as usize;
                if store.weapon[i].is_some() {
                    continue;
                }
                let best = town
                    .armory
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, &e)| {
                        let p = weapons.get(e).ok()?;
                        let def = &db.content.products[p.def as usize];
                        (def.category == Category::Weapon)
                            .then(|| (slot, def.tier.max(1) as f32 * (0.5 + p.quality)))
                    })
                    .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)));
                let Some((slot, power)) = best else { break };
                let e = town.armory.swap_remove(slot);
                store.weapon[i] = Some(e);
                store.weapon_power[i] = mp.weapon_power * power;
                commands
                    .entity(e)
                    .remove::<Stored>()
                    .insert(EquippedBy(i as u32));
            }
        }
    }

    // --- daily pay and drill for soldiers at home
    for town in towns.0.iter_mut() {
        let marching: &[u32] = town.raid.as_ref().map_or(&[], |r| &r.soldiers);
        for &s in &town.squad {
            let i = s as usize;
            let pay = mp.soldier_pay.min(town.treasury.max(0.0));
            town.treasury -= pay;
            store.wealth[i] += pay;
            if !marching.contains(&s) {
                let mult = mp.drill_rate * store.aptitude[i];
                models
                    .learning
                    .practice(&mut store.cap[i], &fight, mult, true);
                let st = &mut store.state[i];
                st[state::FATIGUE] = (st[state::FATIGUE] + 0.05).min(1.0);
                store.target[i] = (
                    town.pos.0 - 30.0 + rng.random_range(-12.0..12.0f32),
                    town.pos.1 - 30.0 + rng.random_range(-12.0..12.0f32),
                );
            }
        }
    }

    // --- raids in progress: march, fight, return
    for t in 0..n_towns {
        let Some(mut raid) = towns.0[t].raid.take() else {
            continue;
        };
        raid.soldiers.retain(|&s| store.alive[s as usize]);
        let (home_pos, target_pos) = (towns.0[t].pos, towns.0[raid.target as usize].pos);
        let now = clock.tick;
        if now >= raid.arrive && !raid.fought {
            raid.fought = true;
            battle(
                &mut raid, t, &mut towns, store, &db, &models, &params, &fight, &mut rng,
            );
        }
        if now >= raid.home || raid.soldiers.is_empty() {
            let town = &mut towns.0[t];
            for (p, q) in raid.loot.iter().enumerate() {
                town.stock[p] += q;
            }
            town.raid_ready = now + mp.raid_cooldown as u64;
            continue;
        }
        // Positions along the road: out until arrival, back afterwards.
        let f = if now < raid.arrive {
            (now - raid.depart) as f32 / (raid.arrive - raid.depart).max(1) as f32
        } else {
            1.0 - (now - raid.arrive) as f32 / (raid.home - raid.arrive).max(1) as f32
        };
        let x = home_pos.0 + (target_pos.0 - home_pos.0) * f;
        let y = home_pos.1 + (target_pos.1 - home_pos.1) * f;
        for &s in &raid.soldiers {
            store.target[s as usize] = (
                x + rng.random_range(-15.0..15.0f32),
                y + rng.random_range(-15.0..15.0f32),
            );
        }
        towns.0[t].raid = Some(raid);
    }

    // --- new raids
    for t in 0..n_towns {
        let town = &towns.0[t];
        if town.raid.is_some()
            || clock.tick < town.raid_ready
            || town.squad.len() < mp.min_raid_squad
        {
            continue;
        }
        let attack: f32 = town
            .squad
            .iter()
            .map(|&s| fighter_power(store, s as usize, &fight))
            .sum();
        // The richest neighbor the squad can beat with a clear margin.
        let mut best: Option<(usize, f32)> = None;
        for (o, other) in towns.0.iter().enumerate() {
            if o == t {
                continue;
            }
            let defense = defense_power(store, other, &fight, mp.militia_power);
            if attack < mp.raid_margin * defense {
                continue;
            }
            let prize = other.treasury.max(0.0)
                + db.foods
                    .iter()
                    .map(|&(p, _)| other.stock[p as usize] * other.price[p as usize])
                    .sum::<f32>();
            if best.is_none_or(|b| prize > b.1) {
                best = Some((o, prize));
            }
        }
        let Some((target, _)) = best else { continue };
        // Ambitious, risk-loving soldiers push for war.
        let temper: f32 = town
            .squad
            .iter()
            .map(|&s| {
                let m = &store.mind[s as usize];
                m[mind::AMBITION] + m[mind::RISK]
            })
            .sum::<f32>()
            / town.squad.len() as f32;
        if rng.random::<f32>() >= mp.raid_appetite * temper {
            continue;
        }
        let tp = towns.0[target].pos;
        let dist = ((tp.0 - town.pos.0).powi(2) + (tp.1 - town.pos.1).powi(2)).sqrt();
        let days = (dist / mp.march_speed.max(1.0)).ceil().max(1.0) as u64;
        let soldiers = town.squad.clone();
        let np = db.content.products.len();
        let town = &mut towns.0[t];
        town.war.raids += 1;
        town.raid = Some(Raid {
            target: target as u16,
            soldiers,
            depart: clock.tick,
            arrive: clock.tick + days,
            home: clock.tick + 2 * days,
            fought: false,
            loot: vec![0.0; np],
        });
    }
}

fn recruit_score(store: &AgentStore, i: usize) -> f32 {
    let m = &store.mind[i];
    let p = &store.phys[i];
    m[mind::AMBITION] + m[mind::RISK] + (p[0] + p[1] + p[2]) / 3.0
}

fn release(commands: &mut Commands, store: &mut AgentStore, armory: &mut Vec<Entity>, i: usize) {
    store.activity[i] = Activity::Idle;
    store.weapon_power[i] = 0.0;
    if let Some(e) = store.weapon[i].take() {
        commands.entity(e).remove::<EquippedBy>().insert(Stored);
        armory.push(e);
    }
}

/// Soldiers at home plus the militia of everyone else.
fn defense_power(
    store: &AgentStore,
    town: &crate::world::Town,
    fight: &CapVec,
    militia: f32,
) -> f32 {
    let away: &[u32] = town.raid.as_ref().map_or(&[], |r| &r.soldiers);
    let squad: f32 = town
        .squad
        .iter()
        .filter(|s| !away.contains(s))
        .map(|&s| fighter_power(store, s as usize, fight))
        .sum();
    let civilians = town
        .residents
        .iter()
        .filter(|&&i| {
            let a = store.activity[i as usize];
            a != Activity::Child && a != Activity::Soldier
        })
        .count();
    squad + militia * civilians as f32
}

#[allow(clippy::too_many_arguments)]
fn battle(
    raid: &mut Raid,
    attacker: usize,
    towns: &mut Towns,
    store: &mut AgentStore,
    db: &Db,
    models: &Models,
    params: &Params,
    fight: &CapVec,
    rng: &mut ChaCha8Rng,
) {
    let mp = &params.military;
    let d = raid.target as usize;
    let attack: f32 = raid
        .soldiers
        .iter()
        .map(|&s| fighter_power(store, s as usize, fight))
        .sum();
    let defense = defense_power(store, &towns.0[d], fight, mp.militia_power);
    let k = mp.battle_steepness;
    let p_win = attack.powf(k) / (attack.powf(k) + defense.powf(k)).max(1e-6);
    let won = rng.random::<f32>() < p_win;
    let total = (attack + defense).max(1e-6);

    // Casualties: each side's fighters fall in proportion to the enemy's strength,
    // and the losing side takes heavier losses.
    let fall = |share: f32, lost: bool| mp.casualty_rate * share * if lost { 1.5 } else { 0.6 };
    let mut fallen_a = 0;
    for &s in &raid.soldiers {
        if rng.random::<f32>() < fall(defense / total, !won) {
            store.state[s as usize][state::HEALTH] = 0.0;
            fallen_a += 1;
        }
    }
    let away: Vec<u32> = towns.0[d]
        .raid
        .as_ref()
        .map_or(Vec::new(), |r| r.soldiers.clone());
    let defenders: Vec<u32> = towns.0[d]
        .squad
        .iter()
        .copied()
        .filter(|s| !away.contains(s))
        .collect();
    let mut fallen_d = 0;
    for &s in &defenders {
        if rng.random::<f32>() < fall(attack / total, won) {
            store.state[s as usize][state::HEALTH] = 0.0;
            fallen_d += 1;
        }
    }
    // Civilians are hurt when the defense breaks.
    if won {
        for &i in &towns.0[d].residents {
            let i = i as usize;
            if store.activity[i] != Activity::Soldier
                && rng.random::<f32>() < 0.1 * mp.casualty_rate
            {
                store.state[i][state::HEALTH] -= 0.5;
            }
            store.state[i][state::HAPPINESS] = (store.state[i][state::HAPPINESS] - 0.15).max(0.0);
        }
    }

    // Survivors learn from the fight.
    for &s in raid.soldiers.iter().chain(defenders.iter()) {
        let i = s as usize;
        if store.state[i][state::HEALTH] > 0.0 {
            let mult = mp.battle_learning * store.aptitude[i];
            let side_won = won == raid.soldiers.contains(&s);
            models
                .learning
                .practice(&mut store.cap[i], fight, mult, side_won);
        }
    }

    // Plunder: money changes hands now, goods travel home with the squad.
    if won {
        let money = towns.0[d].treasury.max(0.0) * mp.loot_share;
        towns.0[d].treasury -= money;
        towns.0[attacker].treasury += money;
        for p in 0..db.content.products.len() {
            let cat = db.category(p as u32);
            if cat == Category::Building || cat.is_item() {
                continue;
            }
            let q = (towns.0[d].stock[p] * mp.loot_share).floor();
            towns.0[d].stock[p] -= q;
            raid.loot[p] += q;
        }
        towns.0[attacker].war.raids_won += 1;
    } else {
        towns.0[attacker].war.raids_lost += 1;
        towns.0[d].war.defended += 1;
    }
    towns.0[d].war.attacked += 1;
    towns.0[attacker].war.fallen += fallen_a;
    towns.0[d].war.fallen += fallen_d;
}
