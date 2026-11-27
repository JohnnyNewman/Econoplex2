//! A raid's fight: the defense at home, the battle on the field, and what follows.

use super::*;

/// A town's own defense plus help from its ruler's soldiers at home.
pub(super) fn total_defense(
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
pub(super) fn home_power(
    store: &AgentStore,
    town: &crate::world::Town,
    fight: &CapVec,
    loyalty: f32,
) -> f32 {
    squad_power(store, &at_home(town), fight, loyalty)
}

/// The squad minus those away on a raid.
pub(super) fn at_home(town: &crate::world::Town) -> Vec<u32> {
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
pub(super) fn battle(
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
    // Townsfolk take up arms beside the soldiers at home.
    let militia = muster_militia(store, &towns.0[d], &defenders, mp);
    let seed = rng.random::<u64>();
    // The field lies outside the defenders' town, toward the attackers.
    let (home, there) = (towns.0[attacker].pos, towns.0[d].pos);
    let (dx, dy) = (there.0 - home.0, there.1 - home.1);
    let len = dx.hypot(dy).max(1e-3);
    let facing = (dx / len, dy / len);
    let at = (there.0 - facing.0 * 60.0, there.1 - facing.1 * 60.0);
    let doctrines = [
        towns.0[attacker].policy.doctrine,
        towns.0[d].policy.doctrine,
    ];
    let (won, fallen_a, fallen_d, fought) = field_battle(
        store,
        db,
        params,
        fight,
        gear_defs,
        &raid.soldiers,
        &defenders,
        &militia,
        doctrines,
        seed,
        log.map(|log| Record {
            log,
            town: d as u16,
            at,
            facing,
        }),
    );
    // Runners taken prisoner go to the winners (a doctrine of capture).
    if let Some((b, groups)) = &fought {
        take_prisoners(commands, raid, attacker, towns, store, rng, b, groups);
    }
    defenders.extend(militia);
    aftermath(
        commands,
        raid,
        attacker,
        towns,
        store,
        db,
        models,
        params,
        fight,
        rng,
        Outcome {
            won,
            fallen_a,
            fallen_d,
            defenders,
            attack,
            defense,
        },
    )
}

/// What follows a raid's fight: civilians hurt, survivors' learning, plunder,
/// captives, conquest and the war tallies. Returns everyone who fought.
#[allow(clippy::too_many_arguments)]
pub(super) fn aftermath(
    commands: &mut Commands,
    raid: &mut Raid,
    attacker: usize,
    towns: &mut Towns,
    store: &mut AgentStore,
    db: &Db,
    models: &Models,
    params: &Params,
    fight: &CapVec,
    rng: &mut ChaCha8Rng,
    outcome: Outcome,
) -> Vec<u32> {
    let mp = &params.military;
    let d = raid.target as usize;
    let Outcome {
        won,
        fallen_a,
        fallen_d,
        defenders,
        attack,
        defense,
    } = outcome;
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
                .practice(&mut store.cap[i], fight, mult, side_won, 1.0);
        }
    }

    // Plunder: money changes hands now, goods travel home with the squad.
    if won {
        let money = towns.0[d].treasury.coins().max(0.0) * mp.loot_share;
        let taken = crate::coin::Coin::from_coins(money);
        towns.0[d].treasury -= taken;
        towns.0[d].ledger.add(crate::ledger::Flow::LootLost, money);
        if params.market.treasury == crate::config::Treasury::Issues {
            // The defender's coin is held abroad by the attacker; so is a
            // share of the foreign coin the defender held.
            towns.0[attacker].hold(d, taken);
            for c in 0..towns.0[d].foreign.len() {
                let part = crate::coin::Coin::from_coins(
                    towns.0[d].foreign[c].coins().max(0.0) * mp.loot_share,
                );
                towns.0[d].foreign[c] -= part;
                if c == attacker {
                    // Its own coin comes home.
                    towns.0[attacker].treasury += part;
                    towns.0[attacker]
                        .ledger
                        .add(crate::ledger::Flow::LootTaken, part.coins());
                } else {
                    towns.0[attacker].hold(c, part);
                }
            }
        } else {
            towns.0[attacker].treasury += taken;
            towns.0[attacker]
                .ledger
                .add(crate::ledger::Flow::LootTaken, money);
        }
        plunder(raid, d, towns, store, db);
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
                store.alive[i] && store.state[i][state::HEALTH] > 0.0 && store.can_be_called(i)
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
pub(super) fn muster_militia(
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
    doctrines: [crate::battle::Doctrine; 2],
    seed: u64,
    record: Option<Record<'_>>,
) -> FieldOutcome {
    let bp = &params.battle;
    let (mut b, groups) = match setup_field_battle(
        store, db, params, bp.model, fight, gear_defs, attackers, defenders, militia, doctrines,
        seed,
    ) {
        Setup::Decided(won) => return (won, 0, 0, None),
        Setup::Battle(b, groups) => (*b, groups),
    };
    let size = b.field_size();
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
    let (fa, fd) = settle_field_battle(store, bp, &b, &groups);
    (report.winner == Some(0), fa, fd, Some((b, groups)))
}

/// Whether the attackers won, each side's dead, and the battle fought with
/// each squad's side and people (`None`: one side had nobody).
type FieldOutcome = (
    bool,
    u32,
    u32,
    Option<(crate::battle::Battle, Vec<(u8, Vec<u32>)>)>,
);

/// A field battle ready to fight, or decided because one side has nobody.
pub(super) enum Setup {
    Decided(bool),
    /// The battle, and each squad's side and people (squad order).
    Battle(Box<crate::battle::Battle>, Vec<(u8, Vec<u32>)>),
}

/// Groups each side's people by what they carry (weapon, mount) into squads,
/// the militia as its own, and sets up the battle on `model`. Each member
/// keeps their own kit, and their fatigue, for the soldier model.
#[allow(clippy::too_many_arguments)]
pub(super) fn setup_field_battle(
    store: &AgentStore,
    db: &Db,
    params: &Params,
    model: crate::battle::BattleModel,
    fight: &CapVec,
    gear_defs: &Query<&Product, Without<Stored>>,
    attackers: &[u32],
    defenders: &[u32],
    militia: &[u32],
    doctrines: [crate::battle::Doctrine; 2],
    seed: u64,
) -> Setup {
    use crate::battle::{Battle, Field, Kit, Member, SquadSpec};
    use crate::goods::Slot;
    let bp = &params.battle;
    let field = Field::default();
    let gear_slots = [Slot::Weapon, Slot::Armour, Slot::Shield, Slot::Mount];
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
                    gear_slots
                        .into_iter()
                        .filter_map(|s| fight_of(i as usize, s).map(|f| f.1))
                        .collect()
                })
                .collect();
            // What each member carries, in kilograms.
            let weights: Vec<f32> = people
                .iter()
                .map(|&i| {
                    gear_slots
                        .into_iter()
                        .filter_map(|s| fight_of(i as usize, s))
                        .map(|f| db.content.products[f.0 as usize].kg)
                        .sum()
                })
                .collect();
            // Each member's own kit; kits that are the same are kept once.
            let mut kits: Vec<Kit> = Vec::new();
            let own: Vec<u16> = gear
                .iter()
                .zip(&weights)
                .map(|(g, &kg)| {
                    let mut kit = Kit::from_gear("", std::slice::from_ref(g));
                    kit.weight = kg;
                    let k = kits.iter().position(|k| *k == kit).unwrap_or_else(|| {
                        kits.push(kit);
                        kits.len() - 1
                    });
                    k as u16
                })
                .collect();
            let mut kit = Kit::from_gear("", &gear);
            kit.weight = weights.iter().sum::<f32>() / weights.len().max(1) as f32;
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
                .zip(own)
                .map(|(&i, kit)| {
                    let k = i as usize;
                    Member {
                        health: store.state[k][state::HEALTH].clamp(0.05, 1.0),
                        skill: dot(&store.cap[k], fight).clamp(0.0, 1.5),
                        experience: 0.0,
                        fatigue: store.state[k][state::FATIGUE].clamp(0.0, 1.0),
                        kit,
                        captured: false,
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
                kit,
                kits,
                members,
                drill: if *levy { 0.0 } else { 0.5 },
                morale: if *levy { morale * 0.7 } else { morale }.clamp(0.1, 1.0),
                pos: (x, y),
                behind_wall: false,
            }
        })
        .collect();
    if !specs.iter().any(|s| s.side == 0) {
        return Setup::Decided(false);
    }
    if !specs.iter().any(|s| s.side == 1) {
        return Setup::Decided(true);
    }
    let mut b = Battle::new(
        crate::battle::BattleParams {
            model,
            ..bp.clone()
        },
        field,
        specs,
        seed,
    );
    for (side, doctrine) in doctrines.into_iter().enumerate() {
        b.set_doctrine(side as u8, doctrine);
    }
    Setup::Battle(
        Box::new(b),
        groups.into_iter().map(|(k, p)| (k.0, p)).collect(),
    )
}

/// Health after a field battle goes back to the people: the dead die, the
/// badly wounded are hurt. Returns how many died on each side.
pub(super) fn settle_field_battle(
    store: &mut AgentStore,
    bp: &crate::battle::BattleParams,
    b: &crate::battle::Battle,
    groups: &[(u8, Vec<u32>)],
) -> (u32, u32) {
    let mut fallen = [0u32; 2];
    for (g, (side, people)) in groups.iter().enumerate() {
        for (m, &i) in b.members(g).iter().zip(people) {
            let k = i as usize;
            let st = &mut store.state[k];
            if m.health <= bp.death_below {
                st[state::HEALTH] = 0.0;
                fallen[(*side as usize).min(1)] += 1;
            } else if m.health <= 0.0 {
                // Badly wounded, out of the fight.
                st[state::HEALTH] = st[state::HEALTH].min(0.1);
            } else {
                st[state::HEALTH] = st[state::HEALTH].min(m.health);
            }
        }
    }
    (fallen[0], fallen[1])
}

/// Runners the winners took prisoner in a field battle (`Pursuit::Capture`)
/// go to the winning town, their kit to its armory:
///
/// - the raiders' prisoners march home with the raid as captives
///   (`Activity::Captive`, in `raid.captives`), as the civilians it carries
///   off do, and settle in the raiders' town when it gets home;
/// - the defenders' prisoners are already in the defenders' town: they leave
///   the raid and settle there at once.
///
/// Prisoners the losing side took go free. Returns how many were taken.
#[allow(clippy::too_many_arguments)]
pub(super) fn take_prisoners(
    commands: &mut Commands,
    raid: &mut Raid,
    attacker: usize,
    towns: &mut Towns,
    store: &mut AgentStore,
    rng: &mut ChaCha8Rng,
    b: &crate::battle::Battle,
    groups: &[(u8, Vec<u32>)],
) -> u32 {
    let Some(winner) = b.report().winner else {
        return 0;
    };
    let d = raid.target as usize;
    let (to, from) = if winner == 0 {
        (attacker, d)
    } else {
        (d, attacker)
    };
    let mut prisoners = prisoners(b, groups);
    prisoners.retain(|&i| store.alive[i as usize]);
    let home = towns.0[to].pos;
    for &c in &prisoners {
        let i = c as usize;
        for slot in KIT_SLOTS {
            if let Some(e) = crate::goods::from_person(store, i, slot) {
                crate::goods::to_armory(commands, &mut towns.0[to].armory, to as u16, e);
            }
        }
        store.weapon_power[i] = 0.0;
        towns.0[from].squad.retain(|&s| s != c);
        raid.soldiers.retain(|&s| s != c);
        if let Some(p) = store.partner[i].take() {
            store.partner[p as usize] = None;
        }
        let st = &mut store.state[i];
        st[state::HAPPINESS] = (st[state::HAPPINESS] - 0.3).max(0.0);
        let jitter = (
            rng.random_range(-40.0..40.0f32),
            rng.random_range(-40.0..40.0f32),
        );
        if winner == 0 {
            store.activity[i] = Activity::Captive;
            relocate(commands, store, i, to as u16, home, jitter);
            raid.captives.push(c);
        } else {
            store.activity[i] = Activity::Idle;
            relocate(commands, store, i, to as u16, home, jitter);
        }
    }
    let n = prisoners.len() as u32;
    towns.0[to].war.captives_taken += n;
    towns.0[to].war.prisoners_taken += n;
    towns.0[from].war.captives_lost += n;
    n
}

/// The people of the losing side the winners took prisoner.
pub(super) fn prisoners(b: &crate::battle::Battle, groups: &[(u8, Vec<u32>)]) -> Vec<u32> {
    let Some(winner) = b.report().winner else {
        return Vec::new();
    };
    groups
        .iter()
        .enumerate()
        .filter(|(_, (side, _))| *side != winner)
        .flat_map(|(g, (_, people))| {
            b.members(g)
                .iter()
                .zip(people)
                .filter(|(m, _)| m.captured)
                .map(|(_, &i)| i)
        })
        .collect()
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
