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

mod fight;
mod kit;
mod live;
mod planner;

use fight::*;
pub use kit::*;
pub use live::*;
use planner::*;

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
    mut links: WarLinks,
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
        // The threat next door: the largest army a road leads from.
        let armies: Vec<f32> = towns.0.iter().map(|t| t.squad.len() as f32).collect();
        for (t, town) in towns.0.iter_mut().enumerate() {
            town.threat = (0..n_towns)
                .filter(|&o| o != t && terrain.road(t, o).is_finite())
                .map(|o| armies[o])
                .fold(0.0f32, f32::max);
        }
        for town in towns.0.iter_mut() {
            let adults = town
                .residents
                .iter()
                .filter(|&&i| store.activity[i as usize] != Activity::Child)
                .count();
            // Keep enough in the treasury for a month of pay.
            let affordable =
                (town.treasury.coins().max(0.0) / (mp.soldier_pay * 30.0).max(1e-6)) as usize;
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
            let pay = towns.0[t].treasury.coins().max(0.0) * mp.tribute_share;
            towns.0[t].treasury -= crate::coin::Coin::from_coins(pay);
            towns.0[t].war.tribute_paid += pay;
            towns.0[t].ledger.add(crate::ledger::Flow::TributePaid, pay);
            towns.0[r].treasury += crate::coin::Coin::from_coins(pay);
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
            let pay = mp.soldier_pay.min(town.treasury.coins().max(0.0));
            town.treasury -= crate::coin::Coin::from_coins(pay);
            town.ledger.add(crate::ledger::Flow::SoldierPay, pay);
            store.wealth[i] += crate::coin::Coin::from_coins(pay);
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
        let arrived = raid.progress >= 1.0;
        // A live battle the player is fighting: wait for it, then settle it.
        let pending_here = links
            .pending
            .as_ref()
            .and_then(|p| p.0.as_ref())
            .is_some_and(|b| b.attacker == t);
        if pending_here {
            let done = links
                .pending
                .as_ref()
                .and_then(|p| p.0.as_ref())
                .is_some_and(|b| b.battle.finished());
            if done {
                let live = links.pending.as_mut().and_then(|p| p.0.take()).unwrap();
                let (fallen_a, fallen_d) =
                    settle_field_battle(store, &params.battle, &live.battle, &live.groups);
                let won = live.battle.report().winner == Some(0);
                let fought = aftermath(
                    &mut commands,
                    &mut raid,
                    t,
                    &mut towns,
                    store,
                    &db,
                    &models,
                    &params,
                    &fight,
                    &mut rng,
                    Outcome {
                        won,
                        fallen_a,
                        fallen_d,
                        defenders: live.defenders,
                        attack: live.attack,
                        defense: live.defense,
                    },
                );
                wear_kit(&mut commands, store, &mut kit, &fought, mp.battle_hours);
                raid.back = true;
            } else {
                towns.0[t].raid = Some(raid);
                continue;
            }
        }
        if arrived && !raid.fought {
            raid.fought = true;
            raid.back = true;
            let live_ok = links.live.as_ref().is_some_and(|l| l.0)
                && links.pending.as_ref().is_some_and(|p| p.0.is_none())
                && links
                    .player
                    .as_ref()
                    .and_then(|p| p.0)
                    .is_some_and(|p| p as usize == t || p == raid.target);
            if live_ok {
                let player = links.player.as_ref().and_then(|p| p.0).unwrap();
                if let Some(live) = prepare_live_battle(
                    &raid, t, &towns, store, &db, &params, &fight, &gear_defs, &mut rng, player,
                ) {
                    links.pending.as_mut().unwrap().0 = Some(live);
                    raid.back = false;
                    towns.0[t].raid = Some(raid);
                    continue;
                }
            }
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
                links.battle_log.as_deref_mut(),
            );
            // A battle wears everyone's kit; what breaks is gone.
            wear_kit(&mut commands, store, &mut kit, &fought, mp.battle_hours);
        }
        let home = raid.back && raid.progress <= 0.0;
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
        let f = raid.progress.clamp(0.0, 1.0);
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
        let target = if let Some(o) = ordered {
            o as usize
        } else {
            // A plan once a muster, and never into deep snow or a storm.
            let here = env.at(t);
            if mp.muster_interval == 0
                || !clock.tick.is_multiple_of(mp.muster_interval as u64)
                || here.ground.snow > 0.3
                || here.weather.storm()
            {
                continue;
            }
            match plan_raid(
                t, &towns, store, &db, &params, &fight, &gear_defs, &terrain, &mut rng,
            ) {
                Some(o) => o,
                None => continue,
            }
        };
        // The squad marches along the road, around rivers and over hills.
        let dist = terrain.road(t, target);
        if !dist.is_finite() {
            towns.0[t].policy.raid_order = None;
            continue;
        }
        let soldiers = towns.0[t].squad.clone();
        let np = db.content.products.len();
        let town = &mut towns.0[t];
        town.policy.raid_order = None;
        town.war.raids += 1;
        let trip_days = 2.0 * dist / mp.march_speed.max(1.0);
        let squad = raise_squad(
            t,
            target as u16,
            &soldiers,
            trip_days * mp.ration_margin,
            &mut squads,
            &mut towns,
            store,
            &db,
            &params,
        );
        let town = &mut towns.0[t];
        town.raid = Some(Raid {
            target: target as u16,
            soldiers,
            fought: false,
            captives: Vec::new(),
            loot: vec![0.0; np],
            squad: Some(squad),
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

/// One day of a raid's march (the military system runs once
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

fn release(commands: &mut Commands, store: &mut AgentStore, armory: &mut Vec<Entity>, i: usize) {
    store.activity[i] = Activity::Idle;
    store.weapon_power[i] = 0.0;
    for slot in KIT_SLOTS {
        if let Some(e) = crate::goods::from_person(store, i, slot) {
            crate::goods::to_armory(commands, armory, store.town[i], e);
        }
    }
}

/// How a fight went, for its aftermath.
struct Outcome {
    won: bool,
    fallen_a: u32,
    fallen_d: u32,
    /// Everyone who fought for the defenders (soldiers and militia).
    defenders: Vec<u32>,
    /// Strength of each side before the fight (`squad_power`).
    attack: f32,
    defense: f32,
}

/// Whether battles that involve the player's town wait for the player
/// (`LiveBattles(true)`, set by the game) or resolve at once (the default).
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct LiveBattles(pub bool);

/// A battle waiting for the player: the simulation holds still while the game
/// steps it and gives orders, and settles it once it is over.
pub struct LiveBattle {
    pub battle: crate::battle::Battle,
    /// Each squad's side and people.
    groups: Vec<(u8, Vec<u32>)>,
    pub attacker: usize,
    pub target: usize,
    defenders: Vec<u32>,
    attack: f32,
    defense: f32,
    /// Where it is fought (map), the attackers' heading, and the player's side.
    pub at: (f32, f32),
    pub facing: (f32, f32),
    pub player_side: u8,
}

#[derive(Resource, Default)]
pub struct PendingBattle(pub Option<LiveBattle>);

/// The war system's links to the battle log, a pending live battle, and who
/// the player is.
#[derive(bevy_ecs::system::SystemParam)]
pub struct WarLinks<'w> {
    battle_log: Option<ResMut<'w, crate::battle::BattleLog>>,
    pending: Option<ResMut<'w, PendingBattle>>,
    live: Option<Res<'w, LiveBattles>>,
    player: Option<Res<'w, crate::policy::PlayerTown>>,
}
