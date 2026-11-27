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
        b.set_order(1, Order::Hold);
        b.set_order(0, Order::Shoot(1));
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
    assert!((kit.reach - preset.reach).abs() < 1e-4);
    assert!(same(&kit.armour, &preset.armour), "{:?}", kit.armour);
    assert!(same(&kit.damage, &preset.damage), "{:?}", kit.damage);
    assert!((kit.shield - preset.shield).abs() < 1e-6);
    assert!(!kit.mounted && kit.range == 0.0);

    let (lance, mail, horse, bow) = (fight("lance"), fight("mail"), fight("horse"), fight("bow"));
    let sword = fight("longsword_2");
    assert_eq!(coat.armour.shares(), PADDED);
    assert_eq!(mail.armour.shares(), MAIL);
    let riders: Vec<Vec<&sim_data::FightDef>> =
        (0..10).map(|_| vec![&lance, &mail, &horse]).collect();
    let kit = Kit::from_gear("cavalry", &riders);
    assert!(kit.mounted && same(&kit.armour, &MAIL), "{:?}", kit.armour);
    let swords: Vec<Vec<&sim_data::FightDef>> = (0..10).map(|_| vec![&sword, &mail]).collect();
    let preset = Kit::preset("swordsmen").unwrap();
    let kit = Kit::from_gear("swordsmen", &swords);
    assert!(same(&kit.armour, &preset.armour) && same(&kit.damage, &preset.damage));
    // A mixed squad's damage splits by its weapons: half spears, half swords.
    let mut mixed: Vec<Vec<&sim_data::FightDef>> = (0..5).map(|_| vec![&spear]).collect();
    mixed.extend((0..5).map(|_| vec![&sword]));
    let kit = Kit::from_gear("mixed", &mixed);
    assert!(same(
        &kit.damage,
        &[spear.damage / 2.0, sword.damage / 2.0, 0.0]
    ));

    // Half the squad unarmed: fists pull the average down; bows still count.
    let mut mixed: Vec<Vec<&sim_data::FightDef>> = (0..5).map(|_| vec![&bow]).collect();
    mixed.extend((0..5).map(|_| Vec::new()));
    let kit = Kit::from_gear("archers", &mixed);
    assert_eq!(kit.range, 150.0);
    assert!(kit.melee_damage() < bow.damage);

    // Every weapon in the content can fight, and says how it hurts.
    for p in &db.content.products {
        if p.category == sim_data::Category::Weapon {
            let f = p
                .fight
                .as_ref()
                .unwrap_or_else(|| panic!("{} has no fight values", p.id));
            assert!(f.damage == 0.0 || f.damage_type.is_some(), "{}", p.id);
        }
    }
}

#[test]
fn a_recorded_battle_ends_where_the_report_says() {
    for model in MODELS {
        recorded(model);
    }
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
fn typed_squad(side: u8, damage: PerType, armour: PerType) -> SquadSpec {
    let kit = Kit {
        damage,
        armour,
        ..Kit::preset("swordsmen").unwrap()
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
        // The same blow by edge and by point: mail stops more of the edge.
        let sword = Kit {
            damage: typed(Slash, 0.6),
            ..Kit::preset("swordsmen").unwrap()
        };
        let point = Kit {
            damage: typed(Pierce, 0.6),
            ..sword.clone()
        };
        assert!(sword.blow(0.0, &MAIL) < point.blow(0.0, &MAIL));
        // In battle, mailed men facing edges lose fewer and win more.
        let mailed = typed_squad(1, typed(Slash, 0.6), MAIL);
        let (edge_won, edge_lost) =
            typed_battles(model, &typed_squad(0, typed(Slash, 0.6), MAIL), &mailed, 40);
        let (point_won, point_lost) = typed_battles(
            model,
            &typed_squad(0, typed(Pierce, 0.6), MAIL),
            &mailed,
            40,
        );
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
        // Plate (not in the content yet): turns edges and points, a weight less.
        let plate = [0.7, 0.75, 0.35];
        let plated = typed_squad(1, typed(Slash, 0.6), plate);
        let (slash_won, slash_lost) = typed_battles(
            model,
            &typed_squad(0, typed(Slash, 0.6), plate),
            &plated,
            40,
        );
        let (blunt_won, blunt_lost) = typed_battles(
            model,
            &typed_squad(0, typed(Blunt, 0.6), plate),
            &plated,
            40,
        );
        eprintln!("{model:?}: against plate: slash won {slash_won}, lost {slash_lost}; blunt won {blunt_won}, lost {blunt_lost}");
        assert!(
            blunt_won >= slash_won + 10 && blunt_lost > slash_lost,
            "{model:?}: against plate: slash won {slash_won} of 40 (plate lost {slash_lost}), \
             blunt won {blunt_won} (plate lost {blunt_lost})"
        );
    }
}
