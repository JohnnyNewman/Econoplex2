//! The squad model's tests. The matchups run on both models: the soldier
//! model must keep them.

use super::*;
use rayon::prelude::*;

const MODELS: [BattleModel; 2] = [BattleModel::Squads, BattleModel::Soldiers];

pub(super) fn params(model: BattleModel) -> BattleParams {
    BattleParams {
        model,
        ..BattleParams::default()
    }
}

/// One report per seed `0..seeds`, fought on all threads.
pub(super) fn reports(model: BattleModel, lineup: &str, seeds: u64) -> Vec<Report> {
    (0..seeds)
        .into_par_iter()
        .map(|seed| {
            let (field, specs) = parse_lineup(lineup).unwrap();
            Battle::new(params(model), field, specs, seed).run()
        })
        .collect()
}

pub(super) fn wins_on(model: BattleModel, lineup: &str, seeds: u64) -> (u64, u64) {
    let (mut att, mut def) = (0, 0);
    for r in reports(model, lineup, seeds) {
        match r.winner {
            Some(0) => att += 1,
            Some(_) => def += 1,
            None => {}
        }
    }
    (att, def)
}

pub(super) fn losses_on(model: BattleModel, lineup: &str, seeds: u64) -> (usize, usize) {
    let (mut a, mut d) = (0, 0);
    for r in reports(model, lineup, seeds) {
        for s in &r.squads {
            let lost = s.wounded + s.dead;
            if s.side == 0 {
                a += lost
            } else {
                d += lost
            }
        }
    }
    (a, d)
}

#[test]
fn same_seed_same_battle() {
    for model in MODELS {
        let run = |seed| {
            let (field, specs) = parse_lineup("swordsmen*30,archers*20 vs spearmen*30").unwrap();
            Battle::new(params(model), field, specs, seed).run()
        };
        assert_eq!(run(7), run(7), "{model:?}");
    }
}

/// Equal armies on an open field win about half the time each, at any size:
/// neither the side nor the order the squads move in decides the fight.
#[test]
fn equal_armies_win_about_half_the_time() {
    for lineup in [
        "swordsmen*30 vs swordsmen*30",
        "swordsmen*400 vs swordsmen*400",
        "spearmen*400 vs spearmen*400",
        "cavalry*200 vs cavalry*200",
        "swordsmen*40,swordsmen*40,swordsmen*40 vs swordsmen*40,swordsmen*40,swordsmen*40",
    ] {
        let (att, def) = wins_on(BattleModel::Squads, lineup, 100);
        assert!(
            (35..=65).contains(&att) && att + def >= 90,
            "{lineup}: attackers won {att}, defenders {def} of 100"
        );
    }
}

#[test]
fn spear_wall_stops_a_charge() {
    for model in MODELS {
        let (att, def) = wins_on(model, "cavalry*20 vs spearmen*30", 20);
        assert!(
            def >= 14,
            "{model:?}: spearmen won {def} of 20 against cavalry (cavalry {att})"
        );
    }
}

#[test]
fn cavalry_rides_down_archers() {
    for model in MODELS {
        let (att, _) = wins_on(model, "cavalry*20 vs archers*30", 20);
        assert!(
            att >= 16,
            "{model:?}: cavalry won {att} of 20 against archers"
        );
    }
}

#[test]
fn arrows_break_a_levy() {
    for model in MODELS {
        let (_, def) = wins_on(model, "levy*40 vs archers*30", 20);
        assert!(
            def >= 14,
            "{model:?}: archers won {def} of 20 against a levy"
        );
    }
}

#[test]
fn swords_get_inside_spears() {
    for model in MODELS {
        let (att, _) = wins_on(model, "swordsmen*30 vs spearmen*30", 20);
        assert!(
            att >= 14,
            "{model:?}: swordsmen won {att} of 20 against spearmen"
        );
    }
}

#[test]
fn veterans_beat_more_recruits() {
    for model in MODELS {
        let (att, _) = wins_on(model, "swordsmen*25+20 vs swordsmen*30", 20);
        assert!(
            att >= 14,
            "{model:?}: 25 veterans won {att} of 20 against 30 recruits"
        );
    }
}

#[test]
fn walls_hold_the_breach() {
    for model in MODELS {
        let (open_a, _) = wins_on(model, "swordsmen*45 vs spearmen*30", 20);
        let (wall_a, _) = wins_on(model, "swordsmen*45 vs walls spearmen*30", 20);
        assert!(
            wall_a + 6 <= open_a,
            "{model:?}: attackers won {open_a} of 20 in the open but {wall_a} against walls"
        );
        let (a, d) = losses_on(model, "swordsmen*45 vs walls spearmen*30", 10);
        assert!(
            a > d,
            "{model:?}: attackers lost {a}, defenders on the wall {d}"
        );
    }
}

#[test]
fn hungry_squads_break_first() {
    for model in MODELS {
        let (fed, _) = wins_on(model, "swordsmen*30!0.9 vs swordsmen*30!0.9", 20);
        let (hungry, _) = wins_on(model, "swordsmen*30!0.3 vs swordsmen*30!0.9", 20);
        assert!(
            hungry + 5 <= fed,
            "{model:?}: fed won {fed}, hungry won {hungry} of 20"
        );
    }
}

#[test]
fn routed_squads_leave_the_field() {
    for model in MODELS {
        for seed in 0..10 {
            let (field, specs) = parse_lineup("levy*20!0.3 vs swordsmen*30+5").unwrap();
            let r = Battle::new(params(model), field, specs, seed).run();
            assert_eq!(r.winner, Some(1), "{model:?} seed {seed}: {r}");
            assert!(
                matches!(r.squads[0].state, SquadState::Fled | SquadState::Destroyed),
                "{model:?} seed {seed}: {r}"
            );
            assert!(
                r.events
                    .iter()
                    .any(|e| matches!(e.kind, EventKind::Routing(0))),
                "{model:?} seed {seed}: {r}"
            );
        }
    }
}

#[test]
fn orders_change_the_battle() {
    // Archers ordered to hold and shoot never close.
    for model in MODELS {
        let (field, specs) = parse_lineup("archers*20 vs spearmen*20").unwrap();
        let mut b = Battle::new(params(model), field, specs, 3);
        b.set_orders(1, &[Order::Hold]);
        b.set_orders(0, &[Order::Shoot(1)]);
        for _ in 0..120 {
            b.step();
        }
        let (p, q) = (b.position(0), b.position(1));
        let apart = (p.0 - q.0).hypot(p.1 - q.1);
        assert!(
            apart > 60.0,
            "{model:?}: archers closed to {apart:.0} m despite their order"
        );
    }
}

#[test]
fn content_kit_matches_the_presets() {
    let paths = crate::AssetPaths::discover();
    let (db, _, _) = crate::load(&paths).unwrap();
    let fight = |id: &str| {
        let p = db.content.product(id).expect(id);
        db.content.products[p as usize].fight.clone().expect(id)
    };
    let (spear, shield, coat) = (fight("iron_spear"), fight("shield"), fight("padded_coat"));
    let squad: Vec<Vec<&sim_data::FightDef>> =
        (0..20).map(|_| vec![&spear, &shield, &coat]).collect();
    let kit = Kit::from_gear("spearmen", &squad);
    let preset = Kit::preset("spearmen").unwrap();
    let same = |a: &PerType, b: &PerType| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
    let same_armour = |a: &Kit, b: &Kit| {
        same(&a.taken, &b.taken) && (a.ac - b.ac).abs() < 1e-6 && (a.shield - b.shield).abs() < 1e-6
    };
    assert!((kit.reach - preset.reach).abs() < 1e-4);
    assert!(same_armour(&kit, &preset), "{kit:?}");
    assert!(same(&kit.damage, &preset.damage), "{:?}", kit.damage);
    assert_eq!(
        (kit.dice, kit.damage_type),
        (preset.dice, preset.damage_type)
    );
    assert!((kit.strike_rate - preset.strike_rate).abs() < 1e-6);
    assert!(!kit.mounted && kit.range == 0.0);

    let (lance, mail, horse, bow) = (fight("lance"), fight("mail"), fight("horse"), fight("bow"));
    let sword = fight("longsword_2");
    assert_eq!((coat.ac, coat.taken()), (1, PADDED));
    assert_eq!((mail.ac, mail.taken()), (5, MAIL));
    let riders: Vec<Vec<&sim_data::FightDef>> = (0..10)
        .map(|_| vec![&lance, &mail, &shield, &horse])
        .collect();
    let kit = Kit::from_gear("cavalry", &riders);
    let preset = Kit::preset("cavalry").unwrap();
    assert!(kit.mounted && same_armour(&kit, &preset), "{kit:?}");
    assert_eq!(
        (kit.dice, kit.charge_dice),
        (preset.dice, preset.charge_dice)
    );
    assert!(same(&kit.charge, &preset.charge));
    let swords: Vec<Vec<&sim_data::FightDef>> =
        (0..10).map(|_| vec![&sword, &mail, &shield]).collect();
    let preset = Kit::preset("swordsmen").unwrap();
    let kit = Kit::from_gear("swordsmen", &swords);
    assert!(same_armour(&kit, &preset) && same(&kit.damage, &preset.damage));
    assert_eq!(kit.dice.to_string(), "1d8+1");
    let archers: Vec<Vec<&sim_data::FightDef>> = (0..10).map(|_| vec![&bow, &coat]).collect();
    let preset = Kit::preset("archers").unwrap();
    let kit = Kit::from_gear("archers", &archers);
    assert!(same_armour(&kit, &preset) && same(&kit.shot_damage, &preset.shot_damage));
    assert_eq!(
        (kit.shot_dice, kit.arrows),
        (preset.shot_dice, preset.arrows)
    );
    // A mixed squad's damage splits by its weapons: half spears, half swords.
    let mut mixed: Vec<Vec<&sim_data::FightDef>> = (0..5).map(|_| vec![&spear]).collect();
    mixed.extend((0..5).map(|_| vec![&sword]));
    let kit = Kit::from_gear("mixed", &mixed);
    let mean = |f: &sim_data::FightDef| f.dice.unwrap().plus(f.quality).mean();
    assert!(same(
        &kit.damage,
        &[mean(&spear) / 2.0, mean(&sword) / 2.0, 0.0]
    ));
    // Layers of armour add their class and multiply what they let through.
    let both: Vec<Vec<&sim_data::FightDef>> = vec![vec![&sword, &coat, &mail]];
    let kit = Kit::from_gear("both", &both);
    assert!(
        kit.ac == 6.0 && same(&kit.taken, &[1.0, 0.5, 0.5]),
        "{kit:?}"
    );

    // Half the squad unarmed: fists pull the average down; bows still count.
    let mut mixed: Vec<Vec<&sim_data::FightDef>> = (0..5).map(|_| vec![&bow]).collect();
    mixed.extend((0..5).map(|_| Vec::new()));
    let kit = Kit::from_gear("archers", &mixed);
    assert_eq!(kit.range, 150.0);
    assert!(kit.melee_damage() < mean(&bow));

    // Every weapon in the content can fight, and says how it hurts.
    for p in &db.content.products {
        if p.category == sim_data::Category::Weapon {
            let f = p
                .fight
                .as_ref()
                .unwrap_or_else(|| panic!("{} has no fight values", p.id));
            assert!(f.dice.is_none() || f.damage_type.is_some(), "{}", p.id);
        }
    }
}

#[test]
fn a_recorded_battle_ends_where_the_report_says() {
    for model in MODELS {
        recorded(model);
    }
}

/// A recorded battle knows who it cost and when they fell: the lost are the
/// dead and the taken, they fell within the battle, and the replay's losses
/// grow to the battle's own.
fn recorded_losses(model: BattleModel) {
    let (field, specs) = parse_lineup("swordsmen*30 vs spearmen*20").unwrap();
    let mut b = Battle::new(params(model), field, specs, 3);
    let r = b.record(5.0);
    let people: Vec<Vec<Fighter>> = (0..2)
        .map(|q| {
            (0..b.members(q).len())
                .map(|k| Fighter {
                    id: k as u32,
                    name: format!("n{k}"),
                    trade: if k % 2 == 0 { "farmer" } else { "smith" }.into(),
                    battles: 1,
                    weekly: 10.0,
                })
                .collect()
        })
        .collect();
    let end = r.frames.last().unwrap().t;
    let mut dead = 0;
    for (q, lost) in r.lost.iter().enumerate() {
        assert_eq!(lost.len(), b.members(q).len());
        for (k, &at) in lost.iter().enumerate() {
            assert_eq!(at.is_some(), b.lost(q, k), "{q} {k}");
            if let Some(t) = at {
                assert!((0.0..=end).contains(&t));
                dead += 1;
            }
        }
    }
    assert!(dead > 0);
    let replay = Replay {
        town: 0,
        at: (0.0, 0.0),
        facing: (1.0, 0.0),
        field: b.field_size(),
        squads: r.squads,
        frames: r.frames,
        winner: r.report.winner,
        people: people.clone(),
        attacker: None,
        lost: r.lost,
        walls: Vec::new(),
    };
    assert_eq!(replay.losses_at(-1.0)[0].count(), 0);
    assert_eq!(replay.losses_at(end), b.losses(&people));
    let all: u32 = b.losses(&people).iter().map(Losses::count).sum();
    assert_eq!(all, dead);
}

#[test]
fn recorded_losses_squads() {
    recorded_losses(BattleModel::Squads);
}

#[test]
fn recorded_losses_soldiers() {
    recorded_losses(BattleModel::Soldiers);
}

fn recorded(model: BattleModel) {
    let (field, specs) = parse_lineup("cavalry*20 vs archers*30").unwrap();
    let mut b = Battle::new(params(model), field, specs, 5);
    let (report, squads, frames) = b.run_recorded(5.0);
    assert_eq!(squads.len(), 2);
    assert!(frames.len() > 2);
    let last = frames.last().unwrap();
    for (k, s) in report.squads.iter().enumerate() {
        assert_eq!(last.squads[k].2 as usize, s.standing);
        assert_eq!(last.squads[k].3, s.state);
    }
    let mut log = BattleLog::default();
    for _ in 0..BattleLog::KEEP + 3 {
        log.push(Replay {
            town: 0,
            at: (0.0, 0.0),
            facing: (1.0, 0.0),
            field: (600.0, 400.0),
            squads: squads.clone(),
            frames: Vec::new(),
            winner: report.winner,
            walls: Vec::new(),
            people: Vec::new(),
            attacker: None,
            lost: Vec::new(),
        });
    }
    assert_eq!(log.replays.len(), BattleLog::KEEP);
    assert_eq!(log.since(log.total - 2).len(), 2);
    assert_eq!(log.since(0).len(), BattleLog::KEEP);
}

#[test]
fn survivors_gain_experience() {
    for model in MODELS {
        let (field, specs) = parse_lineup("swordsmen*30 vs levy*20").unwrap();
        let mut b = Battle::new(params(model), field, specs, 2);
        b.run();
        assert!(
            b.members(0)
                .iter()
                .any(|m| m.health > 0.0 && m.experience >= 1.0),
            "{model:?}"
        );
    }
}

/// A squad of 30 with the given melee damage per type and armour per type,
/// otherwise swordsmen.
fn typed_squad(side: u8, t: sim_data::DamageType, taken: PerType) -> SquadSpec {
    let sword = Kit::preset("swordsmen").unwrap();
    let kit = Kit {
        damage: typed(t, sword.dice.mean()),
        charge: typed(t, sword.dice.mean()),
        damage_type: t,
        taken,
        ..sword
    };
    let x = if side == 0 { 90.0 } else { 450.0 };
    SquadSpec::uniform("typed", side, kit, 30, 0.5, 0.0, 0.8, (x, 200.0))
}

/// How many of `seeds` battles side 0 wins, and side 1's losses.
fn typed_battles(model: BattleModel, att: &SquadSpec, def: &SquadSpec, seeds: u64) -> (u64, usize) {
    let rs: Vec<Report> = (0..seeds)
        .into_par_iter()
        .map(|seed| {
            let specs = vec![att.clone(), def.clone()];
            Battle::new(params(model), Field::default(), specs, seed).run()
        })
        .collect();
    let won = rs.iter().filter(|r| r.winner == Some(0)).count() as u64;
    let lost = rs
        .iter()
        .map(|r| r.squads[1].wounded + r.squads[1].dead)
        .sum();
    (won, lost)
}

#[test]
fn mail_turns_swords_better_than_spears() {
    for model in MODELS {
        use sim_data::DamageType::{Pierce, Slash};
        // The same blow by edge and by point: mail resists the edge.
        let sword = typed_squad(0, Slash, MAIL).kit;
        let point = typed_squad(0, Pierce, MAIL).kit;
        assert!(sword.blow(false, 0.0, 0.0, &MAIL) < point.blow(false, 0.0, 0.0, &MAIL));
        // In battle, mailed men facing edges lose fewer and win more.
        let mailed = typed_squad(1, Slash, MAIL);
        let (edge_won, edge_lost) = typed_battles(model, &typed_squad(0, Slash, MAIL), &mailed, 40);
        let (point_won, point_lost) =
            typed_battles(model, &typed_squad(0, Pierce, MAIL), &mailed, 40);
        eprintln!("{model:?}: against mail: swords won {edge_won}, lost {edge_lost}; spears won {point_won}, lost {point_lost}");
        assert!(
            point_won >= edge_won + 10 && point_lost > edge_lost,
            "{model:?}: against mail: swords won {edge_won} of 40 (mail lost {edge_lost}), \
             spears won {point_won} (mail lost {point_lost})"
        );
    }
}

#[test]
fn blunt_beats_slash_against_plate() {
    for model in MODELS {
        use sim_data::DamageType::{Blunt, Slash};
        // Plate resists edges and points, not a weight.
        let plate = [0.5, 0.5, 1.0];
        let plated = typed_squad(1, Slash, plate);
        let (slash_won, slash_lost) =
            typed_battles(model, &typed_squad(0, Slash, plate), &plated, 40);
        let (blunt_won, blunt_lost) =
            typed_battles(model, &typed_squad(0, Blunt, plate), &plated, 40);
        eprintln!("{model:?}: against plate: slash won {slash_won}, lost {slash_lost}; blunt won {blunt_won}, lost {blunt_lost}");
        assert!(
            blunt_won >= slash_won + 10 && blunt_lost > slash_lost,
            "{model:?}: against plate: slash won {slash_won} of 40 (plate lost {slash_lost}), \
             blunt won {blunt_won} (plate lost {blunt_lost})"
        );
    }
}

/// The squad model takes the same orders: a move it has reached gives way
/// to the next order, the facing and width of a move change nothing, and its
/// frames carry no soldiers.
#[test]
fn the_squad_model_carries_out_orders_in_turn() {
    let fight = |orders: &[Order]| {
        let (field, specs) = parse_lineup("swordsmen*30,swordsmen*20 vs spearmen*40").unwrap();
        let mut b = Battle::new(params(BattleModel::Squads), field, specs, 4);
        b.set_orders(1, orders);
        assert!(b.frame().soldiers.is_empty());
        let v = b.live_view(0, (0.0, 0.0), (1.0, 0.0), 0);
        assert!(!v.on_soldiers() && v.lines[1].anchor == b.position(1));
        b.run()
    };
    let attack = fight(&[Order::Attack(2)]);
    let (field, specs) = parse_lineup("swordsmen*30,swordsmen*20 vs spearmen*40").unwrap();
    let here = Battle::new(params(BattleModel::Squads), field, specs, 4).position(1);
    let first_here = [
        Order::MoveTo {
            anchor: here,
            facing: Some((0.0, 1.0)),
            width: Some(3.0),
            run: false,
        },
        Order::Attack(2),
    ];
    assert_eq!(fight(&first_here), attack);
}

// Battle controls (module `control`), on both models where both have them.

/// A battle of `lineup` on `model`, seed 1.
fn battle(model: BattleModel, lineup: &str) -> Battle {
    let (field, specs) = parse_lineup(lineup).unwrap();
    Battle::new(params(model), field, specs, 1)
}

fn far((ax, ay): (f32, f32), (bx, by): (f32, f32)) -> f32 {
    (ax - bx).hypot(ay - by)
}

/// A move at a run gets further than one at a march.
#[test]
fn a_run_outpaces_a_march() {
    for model in MODELS {
        let gone = |run: bool| {
            let mut b = battle(model, "swordsmen*30 vs swordsmen*30");
            let from = b.position(0);
            let to = (from.0, from.1 + 150.0);
            b.set_orders(1, &[Order::Hold]);
            b.set_orders(
                0,
                &[Order::MoveTo {
                    anchor: to,
                    facing: None,
                    width: None,
                    run,
                }],
            );
            for _ in 0..20 {
                b.step();
            }
            far(b.position(0), from)
        };
        let (walk, run) = (gone(false), gone(true));
        assert!(
            run > 1.5 * walk && walk > 10.0,
            "{model:?}: {walk} walking, {run} running"
        );
    }
}

/// Archers holding fire shoot nobody; at will they thin the enemy.
#[test]
fn archers_holding_fire_shoot_nobody() {
    for model in MODELS {
        let hit = |fire: Fire| {
            let (field, mut specs) = parse_lineup("archers*30 vs swordsmen*30").unwrap();
            specs[0].pos.0 = specs[1].pos.0 - 100.0;
            let mut b = Battle::new(params(model), field, specs, 1);
            b.set_orders(1, &[Order::Hold]);
            b.set_orders(0, &[Order::Hold]);
            b.command(
                0,
                Command::Stance(Stance {
                    fire,
                    ..Stance::default()
                }),
            )
            .unwrap();
            assert_eq!(b.stance(0).fire, fire);
            for _ in 0..60 {
                b.step();
            }
            b.members(1).iter().filter(|m| m.health < 1.0).count()
        };
        assert_eq!(hit(Fire::Hold), 0, "{model:?}");
        assert!(hit(Fire::AtWill) > 0, "{model:?}");
    }
}

/// Skirmishers give ground to swordsmen who come at them; archers at will
/// stand.
#[test]
fn skirmishers_give_ground() {
    for model in MODELS {
        let back = |fire: Fire| {
            let (field, mut specs) = parse_lineup("archers*30 vs swordsmen*30").unwrap();
            specs[0].pos.0 = specs[1].pos.0 - 120.0;
            let mut b = Battle::new(params(model), field, specs, 1);
            b.set_stance(
                0,
                Stance {
                    fire,
                    ..Stance::default()
                },
            );
            b.set_orders(0, &[Order::Hold]);
            b.set_orders(1, &[Order::Attack(0)]);
            let from = b.position(0).0;
            for _ in 0..60 {
                b.step();
            }
            from - b.position(0).0
        };
        let (skirmish, stand) = (back(Fire::Skirmish), back(Fire::AtWill));
        assert!(
            skirmish > 20.0 && stand < 5.0,
            "{model:?}: gave {skirmish} m skirmishing, {stand} m standing"
        );
    }
}

/// A side that lets enemies who run go cuts fewer of them down than one
/// that chases them. (The squad model's pursuit is a disc's: its routers
/// mostly get away either way, so there it only must not kill more.)
#[test]
fn letting_them_go_kills_fewer() {
    for model in MODELS {
        let down = |prisoners: Pursuit| {
            (0..6)
                .map(|seed| {
                    let (field, specs) = parse_lineup("cavalry*40 vs archers*30!0.5").unwrap();
                    let mut b = Battle::new(params(model), field, specs, seed);
                    b.set_doctrine(
                        0,
                        Doctrine {
                            prisoners,
                            ..Doctrine::default()
                        },
                    );
                    assert_eq!(b.stance(0).pursuit, prisoners);
                    let r = b.run().squads[1].clone();
                    r.start - r.standing
                })
                .sum::<usize>()
        };
        let (kill, go) = (down(Pursuit::Kill), down(Pursuit::LetGo));
        let fewer = match model {
            BattleModel::Squads => go <= kill,
            BattleModel::Soldiers => (go as f32) < 0.8 * kill as f32,
        };
        assert!(fewer, "{model:?}: {go} down let go, {kill} cut down");
    }
}

/// A cautious side's squads hold their ground until the enemy comes close;
/// a balanced side's march out.
#[test]
fn a_cautious_side_waits_for_the_enemy() {
    for model in MODELS {
        let moved = |posture: Posture| {
            let mut b = battle(model, "swordsmen*30 vs swordsmen*30");
            b.set_orders(0, &[Order::Hold]);
            b.set_doctrine(
                1,
                Doctrine {
                    posture,
                    ..Doctrine::default()
                },
            );
            let from = b.position(1);
            for _ in 0..60 {
                b.step();
            }
            far(b.position(1), from)
        };
        let (cautious, balanced) = (moved(Posture::Cautious), moved(Posture::Balanced));
        assert!(
            cautious < 3.0 && balanced > 30.0,
            "{model:?}: {cautious} m cautious, {balanced} m balanced"
        );
    }
}

/// An aggressive side's squads break into a run from further out.
#[test]
fn an_aggressive_side_runs_in_sooner() {
    for model in MODELS {
        let covered = |posture: Posture| {
            let (field, mut specs) = parse_lineup("swordsmen*30 vs swordsmen*30").unwrap();
            specs[0].pos.0 = specs[1].pos.0 - 110.0;
            let mut b = Battle::new(params(model), field, specs, 1);
            b.set_orders(1, &[Order::Hold]);
            b.set_doctrine(
                0,
                Doctrine {
                    posture,
                    ..Doctrine::default()
                },
            );
            let from = b.position(0);
            for _ in 0..10 {
                b.step();
            }
            far(b.position(0), from)
        };
        let (eager, balanced) = (covered(Posture::Aggressive), covered(Posture::Balanced));
        assert!(
            eager > 1.5 * balanced,
            "{model:?}: {eager} m eager, {balanced} m balanced"
        );
    }
}

/// Before the battle a squad stands where it is deployed, in its own half;
/// outside it, or once the battle has started, it is refused.
#[test]
fn deployment_is_in_your_own_half_and_before_the_start() {
    for model in MODELS {
        let mut b = battle(model, "swordsmen*30 vs swordsmen*30");
        let zone = b.live_view(0, (0.0, 0.0), (1.0, 0.0), 0).deploy_zone(0);
        assert_eq!(zone, (0.0, 300.0, 0.0, 400.0));
        assert!(b.deploy(0, (400.0, 100.0), (1.0, 0.0), None).is_err());
        let to = (200.0, 100.0);
        b.command(
            0,
            Command::Deploy {
                anchor: to,
                facing: (0.0, 2.0),
                width: Some(10.0),
            },
        )
        .unwrap();
        let line = b.live_view(0, (0.0, 0.0), (1.0, 0.0), 0).lines[0];
        if model == BattleModel::Soldiers {
            assert!(
                far(line.anchor, to) < 0.01 && line.facing == (0.0, 1.0),
                "{line:?}"
            );
            assert_eq!(line.width, 10.0);
            let y = b.position(0).1;
            assert!(
                y < to.1 && y > to.1 - 3.0,
                "the ranks stand behind the front: {y}"
            );
        } else {
            assert_eq!(b.position(0), to);
        }
        b.step();
        assert!(b.deploy(0, to, (1.0, 0.0), None).is_err());
    }
    // Defenders behind a wall stay inside it.
    let b = battle(BattleModel::Squads, "swordsmen*30 vs walls swordsmen*30");
    let v = b.live_view(0, (0.0, 0.0), (1.0, 0.0), 1);
    assert_eq!(v.deploy_zone(1).0, 361.0);
}

/// Loose order stands soldiers `loose_spacing` apart, tight order archers
/// shoulder to shoulder.
#[test]
fn spacing_stances_set_the_files_apart() {
    let mut b = battle(
        BattleModel::Soldiers,
        "swordsmen*30,archers*30 vs swordsmen*30",
    );
    let spacing = |b: &Battle, q: usize| b.live_view(0, (0.0, 0.0), (1.0, 0.0), 0).lines[q].spacing;
    assert_eq!((spacing(&b, 0), spacing(&b, 1)), (1.0, 3.0));
    for (q, s) in [(0, Spacing::Loose), (1, Spacing::Tight)] {
        b.set_stance(
            q,
            Stance {
                spacing: s,
                ..Stance::default()
            },
        );
    }
    b.step();
    assert_eq!((spacing(&b, 0), spacing(&b, 1)), (3.0, 1.0));
}

/// The ghost line: every soldier's slot where the last move leaves the
/// squad, and the seconds to get there at a march or a run.
#[test]
fn a_preview_gives_the_slots_and_the_time() {
    let b = battle(BattleModel::Soldiers, "swordsmen*30 vs swordsmen*30");
    let v = b.live_view(0, (0.0, 0.0), (1.0, 0.0), 0);
    let line = v.lines[0];
    let to = (line.anchor.0 + 50.0, line.anchor.1 + 120.0);
    let go = |run: bool| Order::MoveTo {
        anchor: to,
        facing: Some((0.0, 1.0)),
        width: Some(10.0),
        run,
    };
    assert!(v.preview(0, &[Order::Hold]).is_none());
    let walk = v.preview(0, &[go(false)]).unwrap();
    let run = v.preview(0, &[go(true)]).unwrap();
    assert_eq!(walk.slots.len(), 30);
    assert!((walk.eta - 130.0 / 1.4).abs() < 1.0, "{}", walk.eta);
    assert!((run.eta - 130.0 / 3.0).abs() < 1.0, "{}", run.eta);
    // Ten files facing +y: the front rank on y = to.1, the third rank 2 m back.
    let front: Vec<&(f32, f32)> = walk
        .slots
        .iter()
        .filter(|s| (s.1 - to.1).abs() < 1e-3)
        .collect();
    assert_eq!(front.len(), 10);
    assert!(walk
        .slots
        .iter()
        .all(|s| s.1 <= to.1 + 1e-3 && s.1 >= to.1 - 2.0 - 1e-3));
    // The squad model draws the same ghost from its centre.
    let b = battle(BattleModel::Squads, "swordsmen*30 vs swordsmen*30");
    let v = b.live_view(0, (0.0, 0.0), (1.0, 0.0), 0);
    assert_eq!(v.preview(0, &[go(false)]).unwrap().slots.len(), 30);
}

/// One drag over several squads lines them up along it, each a share of
/// it by its front; one click moves them keeping their places in the group.
#[test]
fn group_orders_keep_a_formation() {
    let b = battle(
        BattleModel::Soldiers,
        "swordsmen*40,swordsmen*20 vs swordsmen*30",
    );
    let v = b.live_view(0, (0.0, 0.0), (1.0, 0.0), 0);
    let (w0, w1) = (v.lines[0].width, v.lines[1].width);
    let line = v.group_line(&[1, 0], (100.0, 50.0), (100.0, 151.0), true);
    let mv = |o: &Order| match *o {
        Order::MoveTo {
            anchor,
            facing,
            width,
            run,
        } => (anchor, facing.unwrap(), width.unwrap(), run),
        _ => panic!("{o:?}"),
    };
    // Squad 0 stands lower on the field, so it takes the start of the drag.
    assert_eq!(line.iter().map(|o| o.0).collect::<Vec<_>>(), [0, 1]);
    let (a, f, wa, run) = mv(&line[0].1);
    let (b1, _, wb, _) = mv(&line[1].1);
    assert!(run && f.0 < -0.99, "a drag up the field faces left: {f:?}");
    assert!((wa + wb + 1.0 - 101.0).abs() < 1e-3 && (wa / wb - w0 / w1).abs() < 1e-3);
    assert!((a.1 - (50.0 + 0.5 * wa)).abs() < 1e-3 && (b1.1 - (151.0 - 0.5 * wb)).abs() < 1e-3);
    let shift = v.group_shift(&[0, 1], (200.0, 200.0), false);
    let (a, _, _, _) = mv(&shift[0].1);
    let (c, _, _, _) = mv(&shift[1].1);
    let (l0, l1) = (v.lines[0].anchor, v.lines[1].anchor);
    assert!(far((a.0 - c.0, a.1 - c.1), (l0.0 - l1.0, l0.1 - l1.1)) < 1e-3);
    assert!(far(((a.0 + c.0) * 0.5, (a.1 + c.1) * 0.5), (200.0, 200.0)) < 1e-3);
}

/// The matchup hints: the attack roll a blow needs against armour class,
/// and blows and arrows per damage type after armour, which is the damage
/// the battle deals.
#[test]
fn matchups_follow_the_armour() {
    let b = battle(
        BattleModel::Squads,
        "spearmen*20,archers*20 vs swordsmen*20",
    );
    let v = b.live_view(0, (0.0, 0.0), (1.0, 0.0), 0);
    let spears = v.matchup(0, 2).unwrap();
    let (spear, sword) = (&v.kits[0], &v.kits[2]);
    // Spearmen of skill 0.5 (+2) with an iron spear (+1) against mail and
    // a shield (AC 17): 14 or more on the d20.
    assert_eq!((spears.needs, v.bonus[0].0), (14, 3.0));
    assert!((spears.hit - 0.35).abs() < 1e-6);
    let crit = 0.05 / 0.35;
    assert!(
        (spears.blow.iter().sum::<f32>() - spear.blow(false, 0.0, crit, &sword.taken)).abs() < 1e-5
    );
    assert!(spears.melee_per_second > 0.0 && spears.shots_per_second == 0.0);
    let arrows = v.matchup(1, 2).unwrap();
    let bare = Kit::preset("levy").unwrap();
    let (b, s) = v.bonus[1];
    assert!(arrows.arrow[0] > 0.0 && arrows.shot_hit < v.kits[1].against(&bare, b, s).shot_hit);
    assert!(v.matchup(0, 9).is_none());
    // Every soldier has a sheet: AC, attack bonus, hit points, dice.
    let sheet = v.sheets[2][0];
    assert_eq!(sheet.words(), "AC 17, +3 to hit, HP 8/8, 1d8+1 slashing");
}

/// The overlay: on the soldier model each soldier's morale and fatigue;
/// the squad model has neither per soldier.
#[test]
fn the_live_view_has_every_soldiers_condition() {
    let mut b = battle(BattleModel::Soldiers, "swordsmen*30 vs spearmen*20!0.6%0.3");
    b.step();
    let v = b.live_view(0, (0.0, 0.0), (1.0, 0.0), 0);
    assert_eq!(
        v.condition.iter().map(Vec::len).collect::<Vec<_>>(),
        [30, 20]
    );
    let c = v.condition[1][0];
    assert!(
        (c.morale() - 0.6).abs() < 0.02 && (c.fatigue() - 0.3).abs() < 0.02,
        "{c:?}"
    );
    assert!(v.paces[1].0 < 1.3 && v.paces[0].0 > 1.39);
    let v = battle(BattleModel::Squads, "swordsmen*30 vs spearmen*20").live_view(
        0,
        (0.0, 0.0),
        (1.0, 0.0),
        0,
    );
    assert!(v.condition.is_empty() && v.paces[0] == (1.4, 3.0));
}

/// Squads that take prisoners lay hands on runners instead of striking
/// them: the prisoners are out of the battle, and are not counted standing.
#[test]
fn prisoners_are_taken_not_struck() {
    let fight = |pursuit: Pursuit| {
        (0..4)
            .map(|seed| {
                let (field, specs) = parse_lineup("cavalry*40 vs archers*30!0.5").unwrap();
                let mut b = Battle::new(params(BattleModel::Soldiers), field, specs, seed);
                b.set_stance(
                    0,
                    Stance {
                        pursuit,
                        ..Stance::default()
                    },
                );
                let r = b.run();
                let taken = b.members(1).iter().filter(|m| m.captured).count();
                assert!(r.squads[1].standing + taken <= 30);
                (taken, r.squads[1].dead)
            })
            .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
    };
    let (taken, dead) = fight(Pursuit::Capture);
    let (none, killed) = fight(Pursuit::Kill);
    assert!(
        taken > 0 && none == 0,
        "{taken} taken capturing, {none} killing"
    );
    assert!(dead < killed, "{dead} dead capturing, {killed} killing");
}
