//! A town's wall in battle: attackers batter a way in, defenders hold it.

use super::*;

fn params(model: BattleModel) -> BattleParams {
    BattleParams {
        model,
        ..BattleParams::default()
    }
}

#[test]
fn a_walled_town_has_its_wall_on_the_field() {
    let (field, specs) = parse_lineup("swordsmen*40 vs walled town swordsmen*40").unwrap();
    assert!(field.wall_closed());
    assert!(field.walls.iter().any(|w| w.gate));
    // The defenders stand inside it.
    let ring = walls::Ring::of(&field).unwrap();
    for s in specs.iter().filter(|s| s.side == 1) {
        assert!(ring.inside(V2::new(s.pos.0, s.pos.1)), "{:?}", s.pos);
    }
    for s in specs.iter().filter(|s| s.side == 0) {
        assert!(!ring.inside(V2::new(s.pos.0, s.pos.1)), "{:?}", s.pos);
    }
}

/// Attackers at a closed wall batter a breach and the defenders' gate
/// opens to everyone there; till then no defender stands outside it.
#[test]
fn attackers_batter_a_breach_and_defenders_keep_inside_till_then() {
    for model in [BattleModel::Squads, BattleModel::Soldiers] {
        let lineup = "swordsmen*40,swordsmen*40 vs walled town swordsmen*40,swordsmen*40";
        let (field, specs) = parse_lineup(lineup).unwrap();
        let ring = walls::Ring::of(&field).unwrap();
        let mut b = Battle::new(params(model), field, specs, 1);
        while !b.finished() && b.walls().iter().all(|w| w.hp > 0.0) {
            for q in 0..b.report().squads.len() {
                if b.report().squads[q].side == 1 && b.state(q) == SquadState::Fighting {
                    let p = b.position(q);
                    assert!(ring.inside(V2::new(p.0, p.1)), "{model:?}: {q} at {p:?}");
                }
            }
            b.step();
        }
        let k = b
            .walls()
            .iter()
            .position(|w| w.hp <= 0.0)
            .unwrap_or_else(|| panic!("{model:?}: no breach"));
        let id = b.walls()[k].id;
        let o = b.field().obstacles.iter().find(|o| o.id == id).unwrap();
        assert_eq!(o.passage, Passage::Open, "{model:?}");
        assert!(b
            .report()
            .events
            .iter()
            .any(|e| e.kind == EventKind::Breach(k)));
        // Others were battered too, or not at all; none healed.
        assert!(b.walls().iter().all(|w| w.hp <= w.max));
    }
}

/// On the squad model, as on the soldier model, the wall's two sides trade
/// blows over it before the breach: attackers below reach some of the
/// defenders on it, and the defenders share out the attackers below, so
/// that no attacking squad takes every blow and breaks before the breach.
#[test]
fn squads_trade_blows_over_the_wall_before_the_breach() {
    let lineup = "swordsmen*40,swordsmen*40 vs walled town swordsmen*40,swordsmen*40";
    let (mut defenders, mut broke) = (0, 0);
    for seed in 1..=4 {
        let (field, specs) = parse_lineup(lineup).unwrap();
        let mut b = Battle::new(params(BattleModel::Squads), field, specs, seed);
        while !b.finished() && b.walls().iter().all(|w| w.hp > 0.0) {
            b.step();
        }
        let r = b.report();
        defenders += r
            .squads
            .iter()
            .filter(|s| s.side == 1)
            .map(|s| s.wounded + s.dead)
            .sum::<usize>();
        broke += (0..r.squads.len())
            .filter(|&q| r.squads[q].side == 0 && b.state(q) != SquadState::Fighting)
            .count();
    }
    assert!(defenders > 0, "no defender struck before a breach");
    assert_eq!(broke, 0, "attacking squads broke before the breach");
}

/// A blunt weapon batters a wall faster than a spear.
#[test]
fn maces_batter_faster_than_spears() {
    let p = BattleParams::default();
    let blunt = p.wall_taken[2];
    assert!(blunt > p.wall_taken[1] && p.wall_taken[1] > p.wall_taken[0]);
}

/// A walled town holds against as many attackers far more often than an
/// open one, and falls to half again as many (both models).
#[test]
fn a_wall_is_worth_men() {
    let n = 8;
    let wins = |model: BattleModel, lineup: &str| {
        (0..n)
            .filter(|&seed| {
                let (field, specs) = parse_lineup(lineup).unwrap();
                Battle::new(params(model), field, specs, seed).run().winner == Some(0)
            })
            .count()
    };
    let two = "swordsmen*40,swordsmen*40";
    let three = "swordsmen*40,swordsmen*40,swordsmen*40";
    for model in [BattleModel::Squads, BattleModel::Soldiers] {
        let open = wins(model, &format!("{two} vs town {two}"));
        let walled = wins(model, &format!("{two} vs walled town {two}"));
        let more = wins(model, &format!("{three} vs walled town {two}"));
        assert!(
            walled < open.max(1),
            "{model:?}: walled {walled} vs open {open}"
        );
        assert!(more > walled, "{model:?}: {more} with more vs {walled}");
    }
}

#[test]
#[ignore = "a look at a battle at a wall"]
fn watch_a_walled_town() {
    let lineup = std::env::var("LINEUP").unwrap_or_else(|_| {
        "swordsmen*40,swordsmen*40 vs walled town swordsmen*40,swordsmen*40".into()
    });
    let model = if std::env::var("MODEL").as_deref() == Ok("squads") {
        BattleModel::Squads
    } else {
        BattleModel::Soldiers
    };
    let seed: u64 = std::env::var("SEED").map_or(1, |s| s.parse().unwrap());
    let (field, specs) = parse_lineup(&lineup).unwrap();
    let mut b = Battle::new(params(model), field, specs, seed);
    let mut next = 0.0;
    while !b.finished() && b.seconds() < 3.0 * 3600.0 {
        if b.seconds() >= next {
            next += std::env::var("EVERY").map_or(15.0, |s| s.parse().unwrap());
            let mut line = format!("t{:5.0}", b.seconds());
            for q in 0..b.report().squads.len() {
                let r = &b.report().squads[q];
                let p = b.position(q);
                line += &format!(
                    " | s{} {:?} ({:.0},{:.0}) {} m{:.2}",
                    r.side,
                    b.state(q),
                    p.0,
                    p.1,
                    r.standing,
                    r.morale
                );
            }
            let down: Vec<String> = b
                .walls()
                .iter()
                .filter(|w| w.hp < w.max)
                .map(|w| format!("{:.0}/{:.0}", w.hp, w.max))
                .collect();
            println!("{line} || walls {}", down.join(" "));
        }
        b.step();
    }
    println!("end t{:.0} {:?}", b.seconds(), b.report().winner);
}

/// The way in through the one gap in the wall is found, the long way round;
/// with the gap shut there is none (`Nav::regions`).
#[test]
fn the_long_way_round_to_a_gap_is_found() {
    let (mut field, _) = parse_lineup("swordsmen*40 vs walled town swordsmen*40").unwrap();
    let ring = walls::Ring::of(&field).unwrap();
    // The piece on the field nearest its top edge is a gap: the way in from
    // the attackers' side goes round to it.
    let far = ring
        .pieces
        .iter()
        .filter(|p| p.mid.x > 0.0 && p.mid.x < field.width && p.mid.y > 5.0)
        .min_by(|a, b| a.mid.y.total_cmp(&b.mid.y))
        .unwrap();
    let id = field.walls[far.k].id;
    field.walls[far.k].hp = 0.0;
    for o in &mut field.obstacles {
        if o.id == id {
            o.passage = nav::Passage::Open;
        }
    }
    let nav = nav::Nav::new(&field, 0).unwrap();
    let from = V2::new(60.0, ring.centre.y);
    let way = nav.path(from, ring.centre, 0).expect("a way in");
    let length = nav::Nav::length(from, &way);
    assert!(length > (ring.centre - from).len() + 20.0, "{length}");
    // With the gap shut there is none, and that is known at once.
    field.walls[far.k].hp = 1.0;
    for o in &mut field.obstacles {
        if o.id == id {
            o.passage = nav::Passage::Blocked;
        }
    }
    let nav = nav::Nav::new(&field, 1).unwrap();
    assert!(nav.path(from, ring.centre, 0).is_none());
}
