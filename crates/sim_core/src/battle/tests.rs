use super::*;

fn wins(lineup: &str, seeds: u64) -> (u64, u64) {
    let mut att = 0;
    let mut def = 0;
    for seed in 0..seeds {
        let (field, specs) = parse_lineup(lineup).unwrap();
        let r = Battle::new(BattleParams::default(), field, specs, seed).run();
        match r.winner {
            Some(0) => att += 1,
            Some(_) => def += 1,
            None => {}
        }
    }
    (att, def)
}

fn losses(lineup: &str, seeds: u64) -> (usize, usize) {
    let (mut a, mut d) = (0, 0);
    for seed in 0..seeds {
        let (field, specs) = parse_lineup(lineup).unwrap();
        let r = Battle::new(BattleParams::default(), field, specs, seed).run();
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
    let run = |seed| {
        let (field, specs) = parse_lineup("swordsmen*30,archers*20 vs spearmen*30").unwrap();
        Battle::new(BattleParams::default(), field, specs, seed).run()
    };
    assert_eq!(run(7), run(7));
}

#[test]
fn spear_wall_stops_a_charge() {
    let (att, def) = wins("cavalry*20 vs spearmen*30", 20);
    assert!(
        def >= 14,
        "spearmen won {def} of 20 against cavalry (cavalry {att})"
    );
}

#[test]
fn cavalry_rides_down_archers() {
    let (att, _) = wins("cavalry*20 vs archers*30", 20);
    assert!(att >= 16, "cavalry won {att} of 20 against archers");
}

#[test]
fn arrows_break_a_levy() {
    let (_, def) = wins("levy*40 vs archers*30", 20);
    assert!(def >= 14, "archers won {def} of 20 against a levy");
}

#[test]
fn swords_get_inside_spears() {
    let (att, _) = wins("swordsmen*30 vs spearmen*30", 20);
    assert!(att >= 14, "swordsmen won {att} of 20 against spearmen");
}

#[test]
fn veterans_beat_more_recruits() {
    let (att, _) = wins("swordsmen*25+20 vs swordsmen*30", 20);
    assert!(att >= 14, "25 veterans won {att} of 20 against 30 recruits");
}

#[test]
fn walls_hold_the_breach() {
    let (open_a, _) = wins("swordsmen*45 vs spearmen*30", 20);
    let (wall_a, _) = wins("swordsmen*45 vs walls spearmen*30", 20);
    assert!(
        wall_a + 6 <= open_a,
        "attackers won {open_a} of 20 in the open but {wall_a} against walls"
    );
    let (a, d) = losses("swordsmen*45 vs walls spearmen*30", 10);
    assert!(a > d, "attackers lost {a}, defenders on the wall {d}");
}

#[test]
fn hungry_squads_break_first() {
    let (fed, _) = wins("swordsmen*30!0.9 vs swordsmen*30!0.9", 20);
    let (hungry, _) = wins("swordsmen*30!0.3 vs swordsmen*30!0.9", 20);
    assert!(
        hungry + 5 <= fed,
        "fed won {fed}, hungry won {hungry} of 20"
    );
}

#[test]
fn routed_squads_leave_the_field() {
    let (field, specs) = parse_lineup("levy*20!0.3 vs swordsmen*30+5").unwrap();
    let r = Battle::new(BattleParams::default(), field, specs, 1).run();
    assert_eq!(r.winner, Some(1));
    assert!(
        matches!(r.squads[0].state, SquadState::Fled | SquadState::Destroyed),
        "{r}"
    );
    assert!(r
        .events
        .iter()
        .any(|e| matches!(e.kind, EventKind::Routing(0))));
}

#[test]
fn orders_change_the_battle() {
    // Archers ordered to hold and shoot never close; ordered to attack, they do.
    let (field, specs) = parse_lineup("archers*20 vs spearmen*20").unwrap();
    let mut b = Battle::new(BattleParams::default(), field, specs, 3);
    b.set_order(1, Order::Hold);
    b.set_order(0, Order::Shoot(1));
    for _ in 0..120 {
        b.step();
    }
    assert!(b.gap(0, 1) > 50.0, "archers closed in despite their order");
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
    assert!((kit.reach - preset.reach).abs() < 1e-4);
    assert!((kit.armour - preset.armour).abs() < 1e-6);
    assert!((kit.shield - preset.shield).abs() < 1e-6);
    assert!(!kit.mounted && kit.range == 0.0);

    let (lance, mail, horse, bow) = (fight("lance"), fight("mail"), fight("horse"), fight("bow"));
    let riders: Vec<Vec<&sim_data::FightDef>> =
        (0..10).map(|_| vec![&lance, &mail, &horse]).collect();
    let kit = Kit::from_gear("cavalry", &riders);
    assert!(kit.mounted && (kit.armour - 0.5).abs() < 1e-6);

    // Half the squad unarmed: fists pull the average down; bows still count.
    let mut mixed: Vec<Vec<&sim_data::FightDef>> = (0..5).map(|_| vec![&bow]).collect();
    mixed.extend((0..5).map(|_| Vec::new()));
    let kit = Kit::from_gear("archers", &mixed);
    assert_eq!(kit.range, 150.0);
    assert!(kit.damage < bow.damage);

    // Every weapon in the content can fight.
    for p in &db.content.products {
        if p.category == sim_data::Category::Weapon {
            assert!(p.fight.is_some(), "{} has no fight values", p.id);
        }
    }
}

#[test]
fn a_recorded_battle_ends_where_the_report_says() {
    let (field, specs) = parse_lineup("cavalry*20 vs archers*30").unwrap();
    let mut b = Battle::new(BattleParams::default(), field, specs, 5);
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
    let (field, specs) = parse_lineup("swordsmen*30 vs levy*20").unwrap();
    let mut b = Battle::new(BattleParams::default(), field, specs, 2);
    b.run();
    assert!(b
        .members(0)
        .iter()
        .any(|m| m.health > 0.0 && m.experience >= 1.0));
}
