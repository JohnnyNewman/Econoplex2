//! Obstacles on the field: nobody ends inside one, squads find their way
//! round them on both models, and the ghost line does too.

use super::soldiers::V2;
use super::*;
use rayon::prelude::*;

fn params(model: BattleModel) -> BattleParams {
    BattleParams {
        model,
        ..BattleParams::default()
    }
}

/// A field with one house (20 by 30 m) square across the way at x = 300.
fn one_house() -> Field {
    Field {
        obstacles: vec![Obstacle::rect(7, (300.0, 200.0), (10.0, 15.0), (1.0, 0.0))],
        ..Field::default()
    }
}

fn swordsmen(side: u8, n: usize, pos: (f32, f32)) -> SquadSpec {
    SquadSpec::uniform(
        "swords",
        side,
        Kit::preset("swordsmen").unwrap(),
        n,
        0.5,
        0.0,
        0.8,
        pos,
    )
}

/// Whether any soldier standing on the field stands inside an obstacle
/// (a hair inside its wall counts as touching, not inside).
fn anyone_inside(b: &Battle) -> Option<(usize, (f32, f32))> {
    let nav = b.nav()?;
    let frame = b.frame();
    for (q, squad) in frame.soldiers.iter().enumerate() {
        for m in squad {
            if matches!(m.state, FighterState::Fled) {
                continue;
            }
            let p = m.pos();
            let side = b.side(q).unwrap_or(0);
            // FrameSoldier keeps a tenth of a metre.
            if nav.inside(V2::new(p.0, p.1), -0.15, side) {
                return Some((q, p));
            }
        }
    }
    None
}

#[test]
fn a_way_round_a_house_keeps_clear_of_it() {
    let nav = Nav::new(&one_house(), 0).unwrap();
    let (a, b) = (V2::new(250.0, 200.0), V2::new(350.0, 200.0));
    assert!(!nav.clear_line(a, b, 0));
    let way = nav.path(a, b, 0).unwrap();
    assert!(way.len() >= 2, "{way:?}");
    assert_eq!(*way.last().unwrap(), b);
    // Every step along it is clear, and it is longer than straight through.
    let mut at = a;
    for &p in &way {
        assert!(nav.clear_line(at, p, 0), "{at:?} to {p:?}");
        at = p;
    }
    let l = Nav::length(a, &way);
    assert!(l > 100.0 && l < 140.0, "{l}");
}

#[test]
fn an_order_onto_a_house_goes_to_the_nearest_free_spot() {
    let (field, specs) = (
        one_house(),
        vec![
            swordsmen(0, 20, (100.0, 200.0)),
            swordsmen(1, 20, (500.0, 200.0)),
        ],
    );
    for model in [BattleModel::Soldiers, BattleModel::Squads] {
        let mut b = Battle::new(params(model), field.clone(), specs.clone(), 1);
        b.set_orders(0, &[Order::move_to((302.0, 203.0))]);
        let Order::MoveTo { anchor, .. } = b.model().orders(0)[0] else {
            panic!()
        };
        // Out of the house, a little past its nearer long wall (x = 310).
        assert!(
            (anchor.0 - 311.5).abs() < 1.0 && (anchor.1 - 203.0).abs() < 1.0,
            "{anchor:?}"
        );
    }
}

#[test]
fn a_gate_lets_one_side_through_and_a_change_plans_again() {
    // A wall across the field at x = 300 with a gate in the middle.
    let field = Field {
        obstacles: vec![
            Obstacle {
                id: 1,
                shape: Shape::Segment {
                    a: (300.0, 0.0),
                    b: (300.0, 195.0),
                    half_width: 0.5,
                },
                passage: Passage::Blocked,
            },
            Obstacle {
                id: 2,
                shape: Shape::Segment {
                    a: (300.0, 195.0),
                    b: (300.0, 205.0),
                    half_width: 0.5,
                },
                passage: Passage::Gate(1),
            },
            Obstacle {
                id: 3,
                shape: Shape::Segment {
                    a: (300.0, 205.0),
                    b: (300.0, 400.0),
                    half_width: 0.5,
                },
                passage: Passage::Blocked,
            },
        ],
        ..Field::default()
    };
    let nav = Nav::new(&field, 0).unwrap();
    let (a, b) = (V2::new(250.0, 200.0), V2::new(350.0, 200.0));
    assert!(nav
        .path(a, b, 1)
        .is_some_and(|w| Nav::length(a, &w) < 101.0));
    assert!(nav.path(a, b, 0).is_none(), "the gate is shut to side 0");
    // Opened: side 0 is through it too, on the next plan.
    let specs = vec![
        swordsmen(0, 20, (250.0, 200.0)),
        swordsmen(1, 20, (550.0, 380.0)),
    ];
    let mut battle = Battle::new(params(BattleModel::Soldiers), field, specs, 3);
    battle.set_orders(0, &[Order::move_to((340.0, 200.0)), Order::Hold]);
    for _ in 0..20 {
        battle.step();
    }
    assert!(battle.position(0).0 < 300.0, "{:?}", battle.position(0));
    let v = battle.nav().unwrap().version();
    assert!(battle.set_passage(2, Passage::Open));
    assert_eq!(battle.nav().unwrap().version(), v + 1);
    for _ in 0..80 {
        battle.step();
    }
    assert!(battle.position(0).0 > 320.0, "{:?}", battle.position(0));
    assert!(battle.remove_obstacle(1) && !battle.remove_obstacle(1));
}

/// A squad sent to the far side of a house goes round it and stands there,
/// nobody inside it on the way: on both models.
#[test]
fn a_squad_ordered_past_a_house_reaches_the_far_side() {
    for model in [BattleModel::Soldiers, BattleModel::Squads] {
        let specs = vec![
            swordsmen(0, 60, (240.0, 200.0)),
            swordsmen(1, 20, (580.0, 20.0)),
        ];
        let mut b = Battle::new(params(model), one_house(), specs, 5);
        let to = (345.0, 200.0);
        b.set_orders(0, &[Order::move_to(to), Order::Hold]);
        b.set_orders(1, &[Order::Hold]);
        for _ in 0..240 {
            b.step();
            assert_eq!(anyone_inside(&b), None, "{model:?} at {}", b.seconds());
        }
        let at = b.position(0);
        assert!(at.0 > 320.0, "{model:?}: {at:?}");
        if model == BattleModel::Soldiers {
            // Everyone made it round, not just the centre.
            let behind = b.frame().soldiers[0]
                .iter()
                .filter(|m| m.pos().0 > 312.0)
                .count();
            assert!(behind >= 55, "{behind} of 60 behind the house");
        }
    }
}

/// In a town, over whole battles, nobody ever stands inside a house.
#[test]
fn a_soldier_never_ends_inside_a_footprint() {
    (0..4u64).into_par_iter().for_each(|seed| {
        let (field, specs) =
            parse_lineup("swordsmen*60,cavalry*20,archers*30 vs town spearmen*60,swordsmen*40")
                .unwrap();
        assert!(field.obstacles.len() > 40);
        let mut b = Battle::new(params(BattleModel::Soldiers), field, specs, seed);
        // Nobody is deployed inside.
        assert_eq!(anyone_inside(&b), None);
        while !b.finished() {
            b.step();
            assert_eq!(anyone_inside(&b), None, "seed {seed} at {}", b.seconds());
        }
    });
}

#[test]
fn the_ghost_line_goes_round_a_house() {
    for model in [BattleModel::Soldiers, BattleModel::Squads] {
        let specs = vec![
            swordsmen(0, 40, (240.0, 200.0)),
            swordsmen(1, 20, (580.0, 20.0)),
        ];
        let b = Battle::new(params(model), one_house(), specs, 1);
        let v = b.live_view(0, (0.0, 0.0), (1.0, 0.0), 0);
        let orders = [Order::move_to((360.0, 200.0))];
        let p = v.preview(0, &orders).unwrap();
        let nav = b.nav().unwrap();
        // Its way keeps out of the house, and so do its slots.
        for w in p.path.windows(2) {
            let (a, c) = (V2::new(w[0].0, w[0].1), V2::new(w[1].0, w[1].1));
            for k in 0..=20 {
                let q = a + (c - a) * (k as f32 / 20.0);
                assert!(!nav.inside(q, 0.0, 0), "{model:?}: {q:?} on the way");
            }
        }
        for &s in &p.slots {
            assert!(
                !nav.inside(V2::new(s.0, s.1), 0.0, 0),
                "{model:?}: slot {s:?}"
            );
        }
        // The time counts the way round, not straight through.
        let line = v.lines[0];
        let straight = (360.0 - line.anchor.0).hypot(200.0 - line.anchor.1);
        let way = Nav::length(
            V2::new(line.anchor.0, line.anchor.1),
            &p.path[1..]
                .iter()
                .map(|q| V2::new(q.0, q.1))
                .collect::<Vec<_>>(),
        );
        assert!(way > straight + 3.0, "{model:?}: {way} vs {straight}");
        assert!(
            (p.eta - way / v.paces[0].0).abs() < 0.5,
            "{model:?}: {}",
            p.eta
        );
    }
}

/// A squad narrows to pass between two houses, and widens again past them.
#[test]
fn a_line_narrows_to_pass_an_alley() {
    let field = Field {
        obstacles: vec![
            Obstacle::rect(1, (300.0, 186.0), (10.0, 10.0), (1.0, 0.0)),
            Obstacle::rect(2, (300.0, 214.0), (10.0, 10.0), (1.0, 0.0)),
        ],
        ..Field::default()
    };
    let specs = vec![
        swordsmen(0, 60, (240.0, 200.0)),
        swordsmen(1, 20, (580.0, 20.0)),
    ];
    let mut b = Battle::new(params(BattleModel::Soldiers), field, specs, 2);
    b.set_orders(
        0,
        &[
            Order::MoveTo {
                anchor: (360.0, 200.0),
                facing: Some((1.0, 0.0)),
                width: Some(30.0),
                run: false,
            },
            Order::Hold,
        ],
    );
    b.set_orders(1, &[Order::Hold]);
    let mut narrowest = f32::INFINITY;
    for _ in 0..200 {
        b.step();
        let line = b.live_view(0, (0.0, 0.0), (1.0, 0.0), 0).lines[0];
        if (line.anchor.0 - 300.0).abs() < 8.0 {
            narrowest = narrowest.min(line.width);
        }
        assert_eq!(anyone_inside(&b), None, "at {}", b.seconds());
    }
    let line = b.live_view(0, (0.0, 0.0), (1.0, 0.0), 0).lines[0];
    assert!(narrowest <= 8.0, "{narrowest} wide in an 8 m alley");
    assert!(line.anchor.0 > 355.0 && line.width >= 29.0, "{line:?}");
}

/// The squad model fights fewer at once in a built-up town than in the open.
#[test]
fn the_squad_model_fronts_narrow_in_a_town() {
    let (open, specs) = parse_lineup("swordsmen*60 vs swordsmen*60").unwrap();
    let (town, _) = parse_lineup("swordsmen*60 vs town swordsmen*60").unwrap();
    let longer = |field: Field| {
        (0..20u64)
            .map(|seed| {
                Battle::new(
                    params(BattleModel::Squads),
                    field.clone(),
                    specs.clone(),
                    seed,
                )
                .run()
                .seconds
            })
            .sum::<f32>()
    };
    let (o, t) = (longer(open), longer(town));
    assert!(t > o * 1.1, "town {t} s, open {o} s");
}

#[test]
#[ignore = "a look at a town battle as it goes"]
fn watch_a_town_battle() {
    let s = vec!["swordsmen*40"; 50].join(",");
    let (field, specs) = parse_lineup(&format!("{s} vs town {s}")).unwrap();
    let mut b = Battle::new(params(BattleModel::Soldiers), field, specs, 1);
    while !b.finished() {
        if (b.seconds() as u32).is_multiple_of(120) {
            let mut n = std::collections::BTreeMap::new();
            for q in 0..100 {
                *n.entry((b.side(q).unwrap(), format!("{:?}", b.state(q))))
                    .or_insert(0) += 1;
            }
            let routers: Vec<String> = (0..100)
                .filter(|&q| b.state(q) == SquadState::Routing)
                .map(|q| format!("{:.0},{:.0}", b.position(q).0, b.position(q).1))
                .take(6)
                .collect();
            println!("{:.0} {:?} {:?}", b.seconds(), n, routers);
        }
        b.step();
    }
    println!("{:?}", b.report().winner);
}

#[test]
#[ignore = "where a town battle's time goes"]
fn profile_a_town_battle() {
    let s = vec!["swordsmen*40"; 50].join(",");
    for l in [format!("{s} vs {s}"), format!("{s} vs town {s}")] {
        let (field, specs) = parse_lineup(&l).unwrap();
        let mut b = Battle::new(params(BattleModel::Soldiers), field, specs, 1);
        b.enable_profile();
        let t = std::time::Instant::now();
        while b.seconds() < 300.0 && !b.finished() {
            b.step();
        }
        let p = b.profile().unwrap();
        let s = b.seconds() as f64;
        println!(
            "{:.3} ms/s; {:?}",
            t.elapsed().as_secs_f64() * 1e3 / s,
            p.phases.map(|x| (x * 1e3 / s * 1000.0).round() / 1000.0)
        );
    }
}

/// Runners among the houses find their way off the field, so a town
/// battle ends: on these seeds some were once caught swinging between two
/// cells of the flow field, with an enemy who let them go beside them.
#[test]
fn runners_leave_a_town_and_the_battle_ends() {
    let s = vec!["swordsmen*40"; 50].join(",");
    let (field, specs) = parse_lineup(&format!("{s} vs town {s}")).unwrap();
    for seed in [1, 3, 5, 6, 11] {
        let mut b = Battle::new(
            params(BattleModel::Soldiers),
            field.clone(),
            specs.clone(),
            seed,
        );
        while !b.finished() && b.seconds() < 3600.0 {
            b.step();
        }
        assert!(b.finished(), "seed {seed}: still going after an hour");
    }
}

#[test]
fn a_field_without_obstacles_has_no_nav() {
    let (field, specs) = parse_lineup("swordsmen*20 vs swordsmen*20").unwrap();
    for model in [BattleModel::Soldiers, BattleModel::Squads] {
        assert!(Battle::new(params(model), field.clone(), specs.clone(), 1)
            .nav()
            .is_none());
    }
}
