//! Squads and raids.
//!
//! Each town keeps a squad sized to what its treasury can pay. Soldiers draw
//! longswords from the armory, drill every day (learning the Fighting skill),
//! and are paid from the treasury, so an army is a running cost the economy has
//! to carry. A town whose squad clearly outmatches a neighbor's defense may raid
//! it: the squad marches over, fights, and on a win carries off part of the
//! defender's treasury and stores, and takes some of its people captive. Captives
//! march home with the squad and then live and work in the captor's town, so the
//! skills they carry change hands with them. Fallen fighters die through the
//! normal lifecycle (their health drops to zero), so their skills are lost with them.
//!
//! A crushing win conquers the defender. A conquered town pays its ruler tribute
//! from its treasury, may keep only a small squad, and is defended by its ruler's
//! soldiers. Rulers don't raid their own tributaries, and tributaries don't raid
//! each other. A tributary is free again when it beats its ruler in battle, or with
//! a small chance every year as the ruler's grip loosens.

use super::migrate::relocate;
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

/// Fighting strength of a group of soldiers: their own power, raised when they
/// trust each other (`loyalty` is the extra share at full cohesion).
pub fn squad_power(store: &AgentStore, members: &[u32], fight: &CapVec, loyalty: f32) -> f32 {
    let sum: f32 = members
        .iter()
        .map(|&s| fighter_power(store, s as usize, fight))
        .sum();
    let mut sorted = members.to_vec();
    sorted.sort_unstable();
    sum * (1.0 + loyalty * store.cohesion(members, &sorted))
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
    terrain: Res<crate::terrain::Terrain>,
    weapons: Query<&Product, With<Stored>>,
    mut kit: Query<&mut crate::components::Item, Without<Stored>>,
    gear_defs: Query<&Product, Without<Stored>>,
    mut squads: ResMut<crate::goods::Squads>,
    env: Res<crate::env::Environment>,
    mut battle_log: Option<ResMut<crate::battle::BattleLog>>,
) {
    let mp = &params.military;
    let loyalty = params.trust.loyalty;
    let Some(fight_idx) = db.content.skill("fighting") else {
        return;
    };
    let fight = db.skill_vecs[fight_idx as usize].clone();
    let mut rng = seed.rng(clock.tick, streams::MILITARY);
    let store = &mut *store;
    let dpy = clock.days_per_year;
    let n_towns = towns.0.len();

    for town in towns.0.iter_mut() {
        town.squad.retain(|&s| store.alive[s as usize]);
    }

    // --- muster: recruit, release and arm
    if mp.muster_interval > 0 && clock.tick.is_multiple_of(mp.muster_interval as u64) {
        for town in towns.0.iter_mut() {
            let adults = town
                .residents
                .iter()
                .filter(|&&i| store.activity[i as usize] != Activity::Child)
                .count();
            // Keep enough in the treasury for a month of pay.
            let affordable = (town.treasury.max(0.0) / (mp.soldier_pay * 30.0).max(1e-6)) as usize;
            let share = if town.ruler.is_some() {
                mp.vassal_soldier_share
            } else {
                town.policy.army_share.unwrap_or(mp.soldier_share)
            };
            let want = ((adults as f32 * share) as usize).min(affordable);
            let at_home = town.raid.is_none();

            if town.squad.len() < want && at_home {
                let mut candidates: Vec<(f32, u32)> = town
                    .residents
                    .iter()
                    .copied()
                    .filter(|&i| {
                        let i = i as usize;
                        // Anyone not in the middle of a job can be called up; in a busy
                        // economy few adults are ever idle.
                        matches!(store.activity[i], Activity::Idle | Activity::Resting)
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

            // Soldiers fill each empty slot (weapon, armour, shield, mount) with
            // the best piece of kit the armory has for it.
            for slot in KIT_SLOTS {
                for k in 0..town.squad.len() {
                    let i = town.squad[k] as usize;
                    if crate::goods::gear(store, i, slot).is_some() {
                        continue;
                    }
                    let best = town
                        .armory
                        .iter()
                        .enumerate()
                        .filter_map(|(at, &e)| {
                            let p = weapons.get(e).ok()?;
                            let def = &db.content.products[p.def as usize];
                            (def.category == Category::Weapon
                                && crate::goods::Slot::of(def) == Some(slot))
                            .then(|| (at, kit_value(def) * (0.5 + p.quality)))
                        })
                        .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)));
                    let Some((at, power)) = best else { break };
                    let e = town.armory.swap_remove(at);
                    crate::goods::to_person(&mut commands, store, i, slot, e);
                    if slot == crate::goods::Slot::Weapon {
                        store.weapon_power[i] = mp.weapon_power * power;
                    }
                }
            }
        }
    }

    // --- tribute, uprisings and peaceful independence
    if mp.tribute_interval > 0 && clock.tick.is_multiple_of(mp.tribute_interval as u64) {
        let chance = mp.independence_rate * mp.tribute_interval as f32 / dpy as f32;
        // A ruler's whole squad holds all its tributaries at once.
        let hold: Vec<f32> = (0..n_towns)
            .map(|r| {
                let held = towns.0.iter().filter(|o| o.ruler == Some(r as u16)).count();
                let squad = squad_power(store, &towns.0[r].squad, &fight, loyalty);
                squad * mp.garrison_strength / held.max(1) as f32
            })
            .collect();
        for t in 0..n_towns {
            let Some(r) = towns.0[t].ruler else { continue };
            let r = r as usize;
            // A tributary whose soldiers can outfight the ruler's hold on it rises up.
            if home_power(store, &towns.0[t], &fight, loyalty) > hold[r] {
                towns.0[t].ruler = None;
                towns.0[t].war.revolts += 1;
                continue;
            }
            let pay = towns.0[t].treasury.max(0.0) * mp.tribute_share;
            towns.0[t].treasury -= pay;
            towns.0[t].war.tribute_paid += pay;
            towns.0[t].ledger.add(crate::ledger::Flow::TributePaid, pay);
            towns.0[r].treasury += pay;
            towns.0[r]
                .ledger
                .add(crate::ledger::Flow::TributeReceived, pay);
            towns.0[r].war.tribute_received += pay;
            if rng.random::<f32>() < chance {
                towns.0[t].ruler = None;
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
            town.ledger.add(crate::ledger::Flow::SoldierPay, pay);
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
        let home_pos = towns.0[t].pos;
        let now = clock.tick;
        let weather_march = mp.march == crate::config::MarchModel::Weather;
        if weather_march {
            march(&mut raid, t, &towns, &terrain, &env, mp);
            if let Some(id) = raid.squad {
                if let Some(s) = squads.list.get_mut(id as usize) {
                    s.members.retain(|m| raid.soldiers.contains(m));
                    s.place = crate::goods::Place::Road {
                        from: t as u16,
                        to: raid.target,
                        progress: raid.progress,
                    };
                }
            }
        }
        let arrived = if weather_march {
            raid.progress >= 1.0
        } else {
            now >= raid.arrive
        };
        if arrived && !raid.fought {
            raid.fought = true;
            raid.back = true;
            let fought = battle(
                &mut commands,
                &mut raid,
                t,
                &mut towns,
                store,
                &db,
                &models,
                &params,
                &fight,
                &gear_defs,
                &mut rng,
                battle_log.as_deref_mut(),
            );
            // A battle wears everyone's kit; what breaks is gone.
            wear_kit(&mut commands, store, &mut kit, &fought, mp.battle_hours);
        }
        let home = if weather_march {
            raid.back && raid.progress <= 0.0
        } else {
            now >= raid.home
        };
        if home || raid.soldiers.is_empty() {
            // What is left in the packs goes back to the town's market.
            if let Some(id) = raid.squad.take() {
                disband(id, t, &mut squads, &mut towns, store, &db);
            }
            let town = &mut towns.0[t];
            for (p, q) in raid.loot.iter().enumerate() {
                town.stock[p] += q;
            }
            town.raid_ready = now + mp.raid_cooldown as u64;
            for &c in &raid.captives {
                if store.alive[c as usize] {
                    store.activity[c as usize] = Activity::Idle;
                }
            }
            continue;
        }
        // Positions along the road: out until arrival, back afterwards.
        let f = if weather_march {
            raid.progress.clamp(0.0, 1.0)
        } else if now < raid.arrive {
            (now - raid.depart) as f32 / (raid.arrive - raid.depart).max(1) as f32
        } else {
            1.0 - (now - raid.arrive) as f32 / (raid.home - raid.arrive).max(1) as f32
        };
        let road = terrain.path(home_pos, raid.target as usize);
        let (x, y) = crate::terrain::along(&road, f);
        for &s in raid.soldiers.iter().chain(&raid.captives) {
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
        // A raid the town's government ordered goes ahead whatever the odds.
        let ordered = town
            .policy
            .raid_order
            .filter(|&o| (o as usize) < n_towns && towns.0[o as usize].ruler != Some(t as u16));
        if ordered.is_none() && town.policy.war == crate::policy::War::Defend {
            continue;
        }
        let attack = squad_power(store, &town.squad, &fight, loyalty);
        // The richest neighbor the squad can beat with a clear margin.
        let mut best: Option<(usize, f32)> = ordered.map(|o| (o as usize, 0.0));
        for (o, other) in towns.0.iter().enumerate() {
            // Rulers protect their tributaries; tributaries of one ruler keep the peace.
            let ruled = other.ruler == Some(t as u16)
                || (town.ruler.is_some() && other.ruler == town.ruler);
            // Nobody marches where no road leads.
            if o == t || ruled || ordered.is_some() || !terrain.road(t, o).is_finite() {
                continue;
            }
            let defense = total_defense(store, &towns, o, &fight, mp, loyalty);
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
        if ordered.is_none() && rng.random::<f32>() >= mp.raid_appetite * temper {
            continue;
        }
        // The squad marches along the road, around rivers and over hills.
        let dist = terrain.road(t, target);
        if !dist.is_finite() {
            towns.0[t].policy.raid_order = None;
            continue;
        }
        let days = (dist / mp.march_speed.max(1.0)).ceil().max(1.0) as u64;
        let soldiers = towns.0[t].squad.clone();
        let np = db.content.products.len();
        let town = &mut towns.0[t];
        town.policy.raid_order = None;
        town.war.raids += 1;
        let squad = (mp.march == crate::config::MarchModel::Weather).then(|| {
            let trip_days = 2.0 * dist / mp.march_speed.max(1.0);
            raise_squad(
                t,
                target as u16,
                &soldiers,
                trip_days * mp.ration_margin,
                &mut squads,
                &mut towns,
                store,
                &db,
                &params,
            )
        });
        let town = &mut towns.0[t];
        town.raid = Some(Raid {
            target: target as u16,
            soldiers,
            depart: clock.tick,
            arrive: clock.tick + days,
            home: clock.tick + 2 * days,
            fought: false,
            captives: Vec::new(),
            loot: vec![0.0; np],
            squad,
            progress: 0.0,
            back: false,
        });
    }
}

/// Puts a raid's soldiers in a squad (reusing a disbanded slot) and packs
/// rations for `days` from the town's granary, then its market.
#[allow(clippy::too_many_arguments)]
fn raise_squad(
    t: usize,
    target: u16,
    soldiers: &[u32],
    days: f32,
    squads: &mut crate::goods::Squads,
    towns: &mut Towns,
    store: &mut AgentStore,
    db: &Db,
    params: &Params,
) -> u32 {
    use crate::goods::{Holder, Place, Squad};
    let squad = Squad {
        home: t as u16,
        members: soldiers.to_vec(),
        place: Place::Road {
            from: t as u16,
            to: target,
            progress: 0.0,
        },
        items: Vec::new(),
        stacks: Vec::new(),
    };
    let id = match squads
        .list
        .iter()
        .position(|s| s.members.is_empty() && s.stacks.is_empty())
    {
        Some(k) => {
            squads.list[k] = squad;
            k
        }
        None => {
            squads.list.push(squad);
            squads.list.len() - 1
        }
    } as u32;
    // Food value the squad will eat: its hunger per day for every soldier.
    let mut need = params.needs.hunger_per_day * soldiers.len() as f32 * days;
    let mut foods = db.foods.clone();
    foods.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    for (food, value) in foods {
        for from in [Holder::Granary(t as u16), Holder::Market(t as u16)] {
            if need <= 0.0 || value <= 0.0 {
                break;
            }
            let moved = crate::goods::move_stack(
                from,
                Holder::Squad(id),
                food as usize,
                (need / value).ceil(),
                towns,
                store,
                squads,
                db,
            );
            need -= moved * value;
        }
    }
    id
}

/// Empties a squad's packs into its home market and frees its slot.
fn disband(
    id: u32,
    t: usize,
    squads: &mut crate::goods::Squads,
    towns: &mut Towns,
    store: &mut AgentStore,
    db: &Db,
) {
    use crate::goods::Holder;
    let stacks = squads
        .list
        .get(id as usize)
        .map(|s| s.stacks.clone())
        .unwrap_or_default();
    for (good, q) in stacks {
        crate::goods::move_stack(
            Holder::Squad(id),
            Holder::Market(t as u16),
            good as usize,
            q,
            towns,
            store,
            squads,
            db,
        );
    }
    if let Some(s) = squads.list.get_mut(id as usize) {
        s.members.clear();
        s.stacks.clear();
        s.items.clear();
        s.place = crate::goods::Place::Town(t as u16);
    }
}

/// One day of a raid's march (`march: Weather`; the military system runs once
/// a day): soldiers walk in daylight, `march_speed` map units on a 12-hour day,
/// more in long summer days and less in short winter ones, slowed by wet roads
/// and snow and stopped by storms; out to the target and, once they fought, home.
fn march(
    raid: &mut Raid,
    t: usize,
    towns: &Towns,
    terrain: &crate::terrain::Terrain,
    env: &crate::env::Environment,
    mp: &crate::config::MilitaryParams,
) {
    // The weather where the squad is: near home, or near the target.
    let here = if raid.progress < 0.5 {
        t
    } else {
        raid.target as usize
    };
    let local = env.at(here.min(towns.0.len().saturating_sub(1)));
    if local.weather.storm() {
        return;
    }
    let road = terrain.road(t, raid.target as usize).max(1.0);
    let daylight = (env.sunset - env.sunrise).clamp(0.0, 24.0) / 12.0;
    let pace = (1.0 - 0.5 * local.ground.wet) * (1.0 - 0.7 * local.ground.snow);
    let step = mp.march_speed * daylight * pace / road;
    if raid.back {
        raid.progress = (raid.progress - step).max(0.0);
    } else {
        raid.progress = (raid.progress + step).min(1.0);
    }
}

fn recruit_score(store: &AgentStore, i: usize) -> f32 {
    let m = &store.mind[i];
    let p = &store.phys[i];
    m[mind::AMBITION] + m[mind::RISK] + (p[0] + p[1] + p[2]) / 3.0
}

/// Pieces of a kit good a town keeps when its government sets no stockpile
/// for it (`military.kit_demand: Soldiers`): one per soldier it means to keep
/// plus one per adult of the levy reserve (`military.levy_reserve` of its
/// residents), shared among the goods that fill the same slot.
pub fn kit_reserve(
    db: &Db,
    town: &crate::world::Town,
    params: &crate::config::ModelParams,
    p: usize,
) -> f32 {
    let mp = &params.military;
    let def = &db.content.products[p];
    let share = town.policy.army_share.unwrap_or(mp.soldier_share);
    let people = town.residents.len() as f32;
    let soldiers = (share * people).max(town.squad.len() as f32);
    let slot = crate::goods::Slot::of(def);
    let rivals = db
        .content
        .products
        .iter()
        .filter(|q| q.category == Category::Weapon && crate::goods::Slot::of(q) == slot)
        .count()
        .max(1);
    (soldiers + mp.levy_reserve * people) / rivals as f32
}

/// The slots a soldier fills from the armory.
const KIT_SLOTS: [crate::goods::Slot; 4] = [
    crate::goods::Slot::Weapon,
    crate::goods::Slot::Armour,
    crate::goods::Slot::Shield,
    crate::goods::Slot::Mount,
];

/// How much a piece of kit is worth to a soldier, before quality: weapons by
/// their tier, armour and shields by the share they stop, a mount by itself.
fn kit_value(def: &sim_data::ProductDef) -> f32 {
    let Some(f) = &def.fight else {
        return def.tier.max(1) as f32;
    };
    match crate::goods::Slot::of(def) {
        Some(crate::goods::Slot::Weapon) | None => def.tier.max(1) as f32,
        _ => f.armour + f.shield + if f.mounted { 1.0 } else { 0.0 },
    }
}

/// Wears the kit of everyone who fought by `hours`; broken pieces are gone.
fn wear_kit(
    commands: &mut Commands,
    store: &mut AgentStore,
    kit: &mut Query<&mut crate::components::Item, Without<Stored>>,
    fought: &[u32],
    hours: f32,
) {
    for &s in fought {
        let i = s as usize;
        for slot in KIT_SLOTS {
            let Some(e) = crate::goods::gear(store, i, slot) else {
                continue;
            };
            let Ok(mut item) = kit.get_mut(e) else {
                continue;
            };
            if crate::goods::wear(&mut item, hours) {
                crate::goods::from_person(store, i, slot);
                commands.entity(e).despawn();
                if slot == crate::goods::Slot::Weapon {
                    store.weapon_power[i] = 0.0;
                }
            }
        }
    }
}

fn release(commands: &mut Commands, store: &mut AgentStore, armory: &mut Vec<Entity>, i: usize) {
    store.activity[i] = Activity::Idle;
    store.weapon_power[i] = 0.0;
    for slot in KIT_SLOTS {
        if let Some(e) = crate::goods::from_person(store, i, slot) {
            crate::goods::to_armory(commands, armory, store.town[i], e);
        }
    }
}

/// A town's own defense plus help from its ruler's soldiers at home.
fn total_defense(
    store: &AgentStore,
    towns: &Towns,
    t: usize,
    fight: &CapVec,
    mp: &crate::config::MilitaryParams,
    loyalty: f32,
) -> f32 {
    let own = defense_power(store, &towns.0[t], fight, mp.militia_power, loyalty);
    let aid = towns.0[t].ruler.map_or(0.0, |r| {
        home_power(store, &towns.0[r as usize], fight, loyalty) * mp.ruler_aid
    });
    own + aid
}

/// Fighting strength of a town's soldiers who are not away on a raid.
fn home_power(store: &AgentStore, town: &crate::world::Town, fight: &CapVec, loyalty: f32) -> f32 {
    squad_power(store, &at_home(town), fight, loyalty)
}

/// The squad minus those away on a raid.
fn at_home(town: &crate::world::Town) -> Vec<u32> {
    let away: &[u32] = town.raid.as_ref().map_or(&[], |r| &r.soldiers);
    town.squad
        .iter()
        .copied()
        .filter(|s| !away.contains(s))
        .collect()
}

/// Soldiers at home plus the militia of everyone else.
fn defense_power(
    store: &AgentStore,
    town: &crate::world::Town,
    fight: &CapVec,
    militia: f32,
    loyalty: f32,
) -> f32 {
    let squad = home_power(store, town, fight, loyalty);
    let civilians = town
        .residents
        .iter()
        .filter(|&&i| {
            let a = store.activity[i as usize];
            a != Activity::Child && a != Activity::Soldier && a != Activity::Captive
        })
        .count();
    squad + militia * civilians as f32
}

#[allow(clippy::too_many_arguments)]
fn battle(
    commands: &mut Commands,
    raid: &mut Raid,
    attacker: usize,
    towns: &mut Towns,
    store: &mut AgentStore,
    db: &Db,
    models: &Models,
    params: &Params,
    fight: &CapVec,
    gear_defs: &Query<&Product, Without<Stored>>,
    rng: &mut ChaCha8Rng,
    log: Option<&mut crate::battle::BattleLog>,
) -> Vec<u32> {
    let mp = &params.military;
    let loyalty = params.trust.loyalty;
    let d = raid.target as usize;
    let attack = squad_power(store, &raid.soldiers, fight, loyalty);
    let defense = total_defense(store, towns, d, fight, mp, loyalty);
    let away: Vec<u32> = towns.0[d]
        .raid
        .as_ref()
        .map_or(Vec::new(), |r| r.soldiers.clone());
    let mut defenders: Vec<u32> = towns.0[d]
        .squad
        .iter()
        .copied()
        .filter(|s| !away.contains(s))
        .collect();
    let (won, fallen_a, fallen_d) = match mp.battles {
        crate::config::BattleModel::Roll => {
            let k = mp.battle_steepness;
            let p_win = attack.powf(k) / (attack.powf(k) + defense.powf(k)).max(1e-6);
            let won = rng.random::<f32>() < p_win;
            let total = (attack + defense).max(1e-6);
            // Casualties: each side's fighters fall in proportion to the enemy's
            // strength, and the losing side takes heavier losses.
            let fall =
                |share: f32, lost: bool| mp.casualty_rate * share * if lost { 1.5 } else { 0.6 };
            let mut fallen_a = 0;
            for &s in &raid.soldiers {
                if rng.random::<f32>() < fall(defense / total, !won) {
                    store.state[s as usize][state::HEALTH] = 0.0;
                    fallen_a += 1;
                }
            }
            let mut fallen_d = 0;
            for &s in &defenders {
                if rng.random::<f32>() < fall(attack / total, won) {
                    store.state[s as usize][state::HEALTH] = 0.0;
                    fallen_d += 1;
                }
            }
            (won, fallen_a, fallen_d)
        }
        crate::config::BattleModel::Field => {
            // Townsfolk take up arms beside the soldiers at home.
            let militia = muster_militia(store, &towns.0[d], &defenders, mp);
            let seed = rng.random::<u64>();
            // The field lies outside the defenders' town, toward the attackers.
            let (home, there) = (towns.0[attacker].pos, towns.0[d].pos);
            let (dx, dy) = (there.0 - home.0, there.1 - home.1);
            let len = dx.hypot(dy).max(1e-3);
            let facing = (dx / len, dy / len);
            let at = (there.0 - facing.0 * 60.0, there.1 - facing.1 * 60.0);
            let out = field_battle(
                store,
                db,
                params,
                fight,
                gear_defs,
                &raid.soldiers,
                &defenders,
                &militia,
                seed,
                log.map(|log| Record {
                    log,
                    town: d as u16,
                    at,
                    facing,
                }),
            );
            defenders.extend(militia);
            out
        }
    };
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
        towns.0[d].ledger.add(crate::ledger::Flow::LootLost, money);
        towns.0[attacker].treasury += money;
        towns.0[attacker]
            .ledger
            .add(crate::ledger::Flow::LootTaken, money);
        if mp.battles == crate::config::BattleModel::Field {
            plunder(raid, d, towns, store, db);
        } else {
            for p in 0..db.content.products.len() {
                let cat = db.category(p as u32);
                if cat == Category::Building || cat.is_item() {
                    continue;
                }
                let q = (towns.0[d].stock[p] * mp.loot_share).floor();
                towns.0[d].stock[p] -= q;
                raid.loot[p] += q;
            }
        }
        towns.0[attacker].war.raids_won += 1;

        // Captives: idle adults of the defeated town, a few per surviving soldier.
        let survivors = raid
            .soldiers
            .iter()
            .filter(|&&s| store.state[s as usize][state::HEALTH] > 0.0)
            .count();
        let mut pool: Vec<(f32, u32)> = towns.0[d]
            .residents
            .iter()
            .copied()
            .filter(|&i| {
                let i = i as usize;
                store.alive[i]
                    && store.state[i][state::HEALTH] > 0.0
                    && matches!(store.activity[i], Activity::Idle | Activity::Resting)
            })
            .map(|i| (rng.random::<f32>(), i))
            .collect();
        let n = ((pool.len() as f32 * mp.captive_share) as usize)
            .min((survivors as f32 * mp.captives_per_soldier) as usize);
        pool.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let home = towns.0[attacker].pos;
        for &(_, c) in pool.iter().take(n) {
            let i = c as usize;
            store.activity[i] = Activity::Captive;
            if let Some(p) = store.partner[i].take() {
                store.partner[p as usize] = None;
            }
            let st = &mut store.state[i];
            st[state::HAPPINESS] = (st[state::HAPPINESS] - 0.3).max(0.0);
            let jitter = (
                rng.random_range(-40.0..40.0f32),
                rng.random_range(-40.0..40.0f32),
            );
            relocate(commands, store, i, attacker as u16, home, jitter);
            raid.captives.push(c);
        }
        towns.0[attacker].war.captives_taken += n as u32;
        towns.0[d].war.captives_lost += n as u32;

        // Beating its ruler frees a tributary; a crushing win over anyone else
        // conquers them, and their own tributaries go free.
        if towns.0[attacker].ruler == Some(d as u16) {
            towns.0[attacker].ruler = None;
            towns.0[attacker].war.revolts += 1;
        } else if attack >= mp.conquest_margin * defense {
            for town in towns.0.iter_mut() {
                if town.ruler == Some(d as u16) {
                    town.ruler = None;
                }
            }
            towns.0[d].ruler = Some(attacker as u16);
            towns.0[attacker].war.conquests += 1;
            towns.0[d].war.conquered += 1;
        }
    } else {
        towns.0[attacker].war.raids_lost += 1;
        towns.0[d].war.defended += 1;
    }
    towns.0[d].war.attacked += 1;
    towns.0[attacker].war.fallen += fallen_a;
    towns.0[d].war.fallen += fallen_d;
    raid.soldiers
        .iter()
        .chain(defenders.iter())
        .copied()
        .collect()
}

/// Civilians who take up arms when their town is attacked: `levy_reserve` of
/// its residents, the fittest adults not already soldiers, children or captives.
fn muster_militia(
    store: &AgentStore,
    town: &crate::world::Town,
    soldiers: &[u32],
    mp: &crate::config::MilitaryParams,
) -> Vec<u32> {
    let n = (mp.levy_reserve * town.residents.len() as f32).round() as usize;
    let mut pool: Vec<(f32, u32)> = town
        .residents
        .iter()
        .copied()
        .filter(|&i| {
            let k = i as usize;
            store.alive[k]
                && store.state[k][state::HEALTH] > 0.3
                && !soldiers.contains(&i)
                && !matches!(
                    store.activity[k],
                    Activity::Child | Activity::Soldier | Activity::Captive
                )
        })
        .map(|i| (recruit_score(store, i as usize), i))
        .collect();
    pool.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    pool.into_iter().take(n).map(|(_, i)| i).collect()
}

/// Where to record a field battle for the view: the log, the town fought
/// over, the field's centre on the map and the attackers' heading.
struct Record<'a> {
    log: &'a mut crate::battle::BattleLog,
    town: u16,
    at: (f32, f32),
    facing: (f32, f32),
}

/// A raid's fight as squads on a field (`battle.rs`): each side's people are
/// grouped by what they carry (weapon, mount), so spearmen fight as spearmen
/// and riders as riders; the militia is its own squad. Health afterwards goes
/// back to the people: the dead die, the wounded are badly hurt. Returns
/// whether the attackers won and how many died on each side.
#[allow(clippy::too_many_arguments)]
fn field_battle(
    store: &mut AgentStore,
    db: &Db,
    params: &Params,
    fight: &CapVec,
    gear_defs: &Query<&Product, Without<Stored>>,
    attackers: &[u32],
    defenders: &[u32],
    militia: &[u32],
    seed: u64,
    record: Option<Record<'_>>,
) -> (bool, u32, u32) {
    use crate::battle::{Battle, Field, Kit, Member, SquadSpec};
    use crate::goods::Slot;
    let bp = &params.battle;
    let field = Field::default();
    let fight_of = |i: usize, slot: Slot| -> Option<(u32, &sim_data::FightDef)> {
        let e = crate::goods::gear(store, i, slot)?;
        let p = gear_defs.get(e).ok()?;
        db.content.products[p.def as usize]
            .fight
            .as_ref()
            .map(|f| (p.def, f))
    };
    // Groups: (side, weapon good or none, mounted, militia) -> people.
    type Group = ((u8, u32, bool, bool), Vec<u32>);
    let mut groups: Vec<Group> = Vec::new();
    let sides = [
        (0u8, attackers, false),
        (1, defenders, false),
        (1, militia, true),
    ];
    for (side, people, levy) in sides {
        for &i in people {
            let k = i as usize;
            let weapon = fight_of(k, Slot::Weapon).map_or(u32::MAX, |w| w.0);
            let mounted = fight_of(k, Slot::Mount).is_some_and(|m| m.1.mounted);
            let key = (side, weapon, mounted, levy);
            match groups.iter_mut().find(|g| g.0 == key) {
                Some(g) => g.1.push(i),
                None => groups.push((key, vec![i])),
            }
        }
    }
    let per_side = |side: u8| groups.iter().filter(|g| g.0 .0 == side).count().max(1) as f32;
    let mut index = [0.0f32; 2];
    let specs: Vec<SquadSpec> = groups
        .iter()
        .map(|((side, weapon, mounted, levy), people)| {
            let gear: Vec<Vec<&sim_data::FightDef>> = people
                .iter()
                .map(|&i| {
                    [Slot::Weapon, Slot::Armour, Slot::Shield, Slot::Mount]
                        .into_iter()
                        .filter_map(|s| fight_of(i as usize, s).map(|f| f.1))
                        .collect()
                })
                .collect();
            let name = if *levy {
                "militia".to_string()
            } else if *weapon == u32::MAX {
                "unarmed".to_string()
            } else {
                let w = &db.content.products[*weapon as usize].name;
                if *mounted {
                    format!("{w}, mounted")
                } else {
                    w.clone()
                }
            };
            let members: Vec<Member> = people
                .iter()
                .map(|&i| {
                    let k = i as usize;
                    Member {
                        health: store.state[k][state::HEALTH].clamp(0.05, 1.0),
                        skill: dot(&store.cap[k], fight).clamp(0.0, 1.5),
                        experience: 0.0,
                    }
                })
                .collect();
            // Morale from their needs: fed and content soldiers stand firm.
            let morale = people
                .iter()
                .map(|&i| {
                    let st = &store.state[i as usize];
                    0.5 * (1.0 - st[state::HUNGER]).clamp(0.0, 1.0)
                        + 0.5 * st[state::HAPPINESS].clamp(0.0, 1.0)
                })
                .sum::<f32>()
                / people.len().max(1) as f32;
            let s = *side as usize;
            let y = field.height * (index[s] + 0.5) / per_side(*side);
            index[s] += 1.0;
            let x = if *side == 0 {
                field.width * 0.15
            } else {
                field.width * 0.75
            };
            SquadSpec {
                name,
                side: *side,
                kit: Kit::from_gear("", &gear),
                members,
                drill: if *levy { 0.0 } else { 0.5 },
                morale: if *levy { morale * 0.7 } else { morale }.clamp(0.1, 1.0),
                pos: (x, y),
                behind_wall: false,
            }
        })
        .collect();
    if !specs.iter().any(|s| s.side == 0) {
        return (false, 0, 0);
    }
    if !specs.iter().any(|s| s.side == 1) {
        return (true, 0, 0);
    }
    let size = (field.width, field.height);
    let mut b = Battle::new(bp.clone(), field, specs, seed);
    let report = match record {
        Some(Record {
            log,
            town,
            at,
            facing,
        }) => {
            let (report, squads, frames) = b.run_recorded(5.0);
            log.push(crate::battle::Replay {
                town,
                at,
                facing,
                field: size,
                squads,
                frames,
                winner: report.winner,
            });
            report
        }
        None => b.run(),
    };
    let mut fallen = [0u32; 2];
    for (g, (key, people)) in groups.iter().enumerate() {
        for (m, &i) in b.members(g).iter().zip(people) {
            let k = i as usize;
            let st = &mut store.state[k];
            if m.health <= bp.death_below {
                st[state::HEALTH] = 0.0;
                fallen[key.0 as usize] += 1;
            } else if m.health <= 0.0 {
                // Badly wounded, out of the fight.
                st[state::HEALTH] = st[state::HEALTH].min(0.1);
            } else {
                st[state::HEALTH] = st[state::HEALTH].min(m.health);
            }
        }
    }
    (report.winner == Some(0), fallen[0], fallen[1])
}

/// After a won fight the raiders carry off what they can: the goods worth most
/// for their weight first, up to what the survivors can carry
/// (`goods::KG_PER_MEMBER` each, less the rations in their packs).
fn plunder(raid: &mut Raid, d: usize, towns: &mut Towns, store: &AgentStore, db: &Db) {
    let survivors = raid
        .soldiers
        .iter()
        .filter(|&&s| store.state[s as usize][state::HEALTH] > 0.0)
        .count() as f32;
    let carried: f32 = raid
        .loot
        .iter()
        .enumerate()
        .map(|(p, q)| q * db.content.products[p].kg)
        .sum();
    let mut room = (survivors * crate::goods::KG_PER_MEMBER - carried).max(0.0);
    let town = &mut towns.0[d];
    let mut goods: Vec<usize> = (0..db.content.products.len())
        .filter(|&p| {
            let cat = db.category(p as u32);
            cat != Category::Building && !cat.is_item() && town.stock[p] >= 1.0
        })
        .collect();
    let worth = |p: usize| town.price[p] / db.content.products[p].kg.max(0.01);
    goods.sort_by(|&a, &b| worth(b).total_cmp(&worth(a)).then(a.cmp(&b)));
    for p in goods {
        if room <= 0.0 {
            break;
        }
        let kg = db.content.products[p].kg.max(0.01);
        let q = town.stock[p].floor().min((room / kg).floor());
        if q <= 0.0 {
            continue;
        }
        town.stock[p] -= q;
        raid.loot[p] += q;
        room -= q * kg;
    }
}
