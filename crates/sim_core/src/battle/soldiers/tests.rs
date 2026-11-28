//! The soldier model's own tests (design note, "Tests"). The squad model's
//! matchups run on both models in `battle/tests.rs`.

use super::*;
use crate::battle::tests::{losses_on, wins_on};

fn soldiers() -> BattleParams {
    BattleParams {
        model: BattleModel::Soldiers,
        ..BattleParams::default()
    }
}

/// The same armies with the sides swapped: the defenders, mirrored across
/// the field, attack, and listed first.
fn swapped(field: &Field, specs: Vec<SquadSpec>) -> Vec<SquadSpec> {
    let mut out: Vec<SquadSpec> = specs
        .into_iter()
        .map(|mut s| {
            s.side = 1 - s.side;
            s.pos.0 = field.width - s.pos.0;
            s
        })
        .collect();
    out.sort_by_key(|s| s.side);
    out
}

/// Side 0's wins over `seeds` battles of `lineup`, as listed and swapped.
fn side_0_wins(lineup: &str, seeds: u64) -> (usize, usize) {
    let won = |swap: bool| {
        (0..seeds)
            .into_par_iter()
            .filter(|&seed| {
                let (field, specs) = parse_lineup(lineup).unwrap();
                let specs = if swap { swapped(&field, specs) } else { specs };
                Battle::new(soldiers(), field, specs, seed).run().winner == Some(0)
            })
            .count()
    };
    (won(false), won(true))
}

/// Over 200 battles: at 100 a single lucky run of seeds can move one
/// count by 15 with no bias behind it.
fn assert_even(lineup: &str) {
    let (listed, swapped) = side_0_wins(lineup, 200);
    assert!(
        (70..=130).contains(&listed) && listed.abs_diff(swapped) <= 20,
        "{lineup}: side 0 won {listed} of 200, {swapped} with the sides swapped"
    );
}

/// Equal armies, mirror images on `Auto`, win about half the time each:
/// neither the side nor the order the soldiers move in decides the fight.
#[test]
fn equal_armies_win_about_half_the_time() {
    assert_even("swordsmen*200 vs swordsmen*200");
}

/// The same at 2,000 a side (400 battles of 4,000 soldiers: release only).
#[test]
#[ignore]
fn equal_armies_win_about_half_the_time_at_2000_a_side() {
    assert_even("swordsmen*2000 vs swordsmen*2000");
}

/// A flank beats a frontal attack: it wins at least 70 of 100, and at
/// least 25 more than frontally on the same seeds. (Over 200 seeds the flank
/// wins 84%, the frontal mirror about half.)
#[test]
fn a_flank_beats_a_frontal_attack() {
    let m = BattleModel::Soldiers;
    let frontal = "swordsmen*100,swordsmen*100 vs swordsmen*100,swordsmen*100";
    let flank = "swordsmen*100,swordsmen*100#flank vs swordsmen*100,swordsmen*100";
    let (front_won, _) = wins_on(m, frontal, 100);
    let (flank_won, _) = wins_on(m, flank, 100);
    let (_, front_lost) = losses_on(m, frontal, 100);
    let (_, flank_lost) = losses_on(m, flank, 100);
    assert!(
        flank_won >= 70 && flank_won >= front_won + 25 && flank_lost > front_lost,
        "side 0 won {front_won} of 100 frontally, {flank_won} with the flank; \
         the enemy lost {front_lost} and {flank_lost}"
    );
}

#[test]
fn rested_beats_exhausted() {
    let m = BattleModel::Soldiers;
    let (spent, _) = wins_on(m, "swordsmen*100 vs swordsmen*100%0.8", 50);
    let (tired, _) = wins_on(m, "swordsmen*100 vs swordsmen*100%0.4", 50);
    assert!(
        spent >= 40 && tired > 25,
        "the rested won {spent} of 50 against fatigue 0.8, {tired} against 0.4"
    );
}

const MIXED: &str = "swordsmen*60,archers*30 vs spearmen*60,cavalry*20";

#[test]
fn one_thread_and_eight_fight_the_same_battle() {
    let params = BattleParams {
        soldiers: SoldierParams {
            parallel_from: 0,
            ..SoldierParams::default()
        },
        ..soldiers()
    };
    let on = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            let (field, specs) = parse_lineup(MIXED).unwrap();
            Battle::new(params.clone(), field, specs, 5).run()
        })
    };
    assert_eq!(on(1), on(8));
}

#[test]
fn stepping_gives_the_run_battle() {
    let (field, specs) = parse_lineup(MIXED).unwrap();
    let run = Battle::new(soldiers(), field.clone(), specs.clone(), 9).run();
    // A live battle steps a second at a time and looks at the field between.
    let mut b = Battle::new(soldiers(), field, specs, 9);
    while !b.finished() {
        b.step();
        let _ = b.frame();
    }
    assert_eq!(b.finish(), run);
}

/// Blows that meet land by initiative: a man downed by a quicker blow does
/// not strike back, blows of the same initiative all land, and the order the
/// blows were booked in changes nothing.
#[test]
fn blows_land_by_initiative_in_any_order() {
    let (field, specs) = parse_lineup("swordsmen*2 vs swordsmen*2").unwrap();
    let mut b = SoldierBattle::new(soldiers(), field, specs, 1);
    let (a0, a1) = (b.squads[0].first, b.squads[0].first + 1);
    let (d0, d1) = (b.squads[1].first, b.squads[1].first + 1);
    let blow = |target: usize, init: f32| Blow {
        target: target as u32,
        damage: 100.0,
        hit: true,
        init,
        ..Blow::default()
    };
    // a0 is quicker than d0, who aims at him; a1 and d1 tie.
    let booked = [
        (a0 as u32, blow(d0, 15.0)),
        (d0 as u32, blow(a0, 9.0)),
        (a1 as u32, blow(d1, 12.0)),
        (d1 as u32, blow(a1, 12.0)),
    ];
    let mut outcomes = Vec::new();
    for order in [[0, 1, 2, 3], [3, 2, 1, 0], [1, 3, 0, 2]] {
        let blows: Vec<(u32, Blow)> = order.iter().map(|&k| booked[k]).collect();
        b.hurt.iter_mut().for_each(|h| *h = Hurt::default());
        b.land(&blows);
        let mut landed: Vec<u32> = blows
            .iter()
            .zip(&b.lands)
            .filter(|(_, &l)| l)
            .map(|((i, _), _)| *i)
            .collect();
        landed.sort_unstable();
        let hurt: Vec<f32> = b.hurt.iter().map(|h| h.damage).collect();
        outcomes.push((landed, hurt));
    }
    let mut want = vec![a0 as u32, a1 as u32, d1 as u32];
    want.sort_unstable();
    assert_eq!(outcomes[0].0, want, "d0 was downed before his blow landed");
    assert!(outcomes.iter().all(|o| *o == outcomes[0]), "{outcomes:?}");
}

#[test]
fn the_rear_refills_a_fallen_front_rank() {
    let (field, specs) = parse_lineup("swordsmen*100 vs swordsmen*100").unwrap();
    let mut b = SoldierBattle::new(soldiers(), field, specs, 1);
    let r = b.squads[0].first..b.squads[0].first + 100;
    let front = r.clone().find(|&i| b.soldier[i].rank == 0).unwrap();
    let file = b.soldier[front].file;
    assert!(
        r.clone()
            .any(|i| b.soldier[i].file == file && b.soldier[i].rank == 1),
        "the line has a second rank"
    );
    b.soldier[front].state = SoldierState::Down;
    b.health[front] = 0.0;
    b.squads[0].losses += 1;
    b.squads[0].left += 1;
    b.substep();
    assert!(
        r.clone().any(|i| i != front
            && b.soldier[i].state.in_line()
            && b.soldier[i].file == file
            && b.soldier[i].rank == 0),
        "nobody stepped into file {file} of the front rank"
    );
}

#[test]
fn a_braced_line_turns_to_face_riders() {
    let (field, specs) = parse_lineup("spearmen*60 vs cavalry*20").unwrap();
    let mut b = SoldierBattle::new(soldiers(), field, specs, 2);
    // The spearmen look up the field, with the riders off their right.
    let up = V2::new(0.0, 1.0);
    b.squads[0].facing = up;
    let r = b.squads[0].first..b.squads[0].first + 60;
    for i in r.clone() {
        b.soldier[i].facing = up;
    }
    for _ in 0..90 {
        b.step();
    }
    let s = &b.squads[0];
    let to_riders = (b.squads[1].centre - s.centre).norm_or(up);
    let faced = r
        .clone()
        .filter(|&i| b.soldier[i].facing.dot(to_riders) > 0.9)
        .count();
    assert!(
        s.formation == Formation::Braced && s.facing.dot(to_riders) > 0.95 && faced >= 50,
        "{:?} facing {:?}, riders toward {to_riders:?}, {faced} of 60 face them",
        s.formation,
        s.facing
    );
}

/// Going round: while one squad fights a standing enemy's front, another,
/// starting behind it and sent round the enemy's side (`LiveView::flank`:
/// moves along a lane off its side, then `Attack`), meets it more than 60
/// degrees off the front it fights on. Sent straight at it (`Attack`), it
/// meets it at that front.
#[test]
fn a_squad_sent_round_the_side_meets_the_enemy_from_the_side() {
    // The cosine of the angle, at the enemy, between the squad holding its
    // front and the second squad when the second squad first meets it; `None`
    // if the enemy broke before it got there.
    let meets = |flank: bool, seed: u64| {
        let (field, mut specs) =
            parse_lineup("swordsmen*100,swordsmen*30 vs swordsmen*100").unwrap();
        // Both squads straight in front of the enemy, the second behind the first.
        specs[0].pos.1 = specs[2].pos.1;
        specs[1].pos = (specs[0].pos.0 - 40.0, specs[2].pos.1);
        let mut b = Battle::new(soldiers(), field, specs, seed);
        b.set_orders(2, &[Order::Hold]);
        let plan = if flank {
            b.live_view(0, (0.0, 0.0), (1.0, 0.0), 0).flank(1, 2)
        } else {
            vec![Order::Attack(2)]
        };
        b.set_orders(1, &plan);
        while !b.finished() {
            b.step();
            let Battle::Soldiers(sb) = &b else {
                unreachable!()
            };
            let (front, s, q) = (&sb.squads[0], &sb.squads[1], &sb.squads[2]);
            if s.in_contact > 0 && s.fighting_with == Some(2) {
                let way = |p: V2| (p - q.centre).norm_or(V2::new(-1.0, 0.0));
                return Some(way(s.centre).dot(way(front.centre)));
            }
        }
        None
    };
    let mut flanked = 0;
    for seed in 1..=8 {
        let (flank, attack) = (meets(true, seed), meets(false, seed));
        assert!(
            flank.is_none_or(|c| c < 0.5) && attack.is_none_or(|c| c > 0.9),
            "seed {seed}: met the enemy at cos {flank:.2?} off its front going round, {attack:.2?} straight"
        );
        flanked += flank.is_some() as usize;
    }
    assert!(
        flanked >= 5,
        "the flank came round in time {flanked} of 8 battles"
    );
}

/// `MoveTo` with a width and facing: the squad re-forms that wide where it
/// stands. Sent on by two moves, it goes to the first, then the second,
/// takes the second's width and turns there as told.
#[test]
fn a_move_sets_the_front_and_gives_way_to_the_next_order() {
    let (field, specs) = parse_lineup("swordsmen*40 vs swordsmen*40").unwrap();
    let mut b = Battle::new(soldiers(), field, specs, 1);
    let line = b.live_view(0, (0.0, 0.0), (1.0, 0.0), 0).lines[0];
    assert_eq!(line.width, 40.0);
    let ahead = (line.anchor.0 + 20.0, line.anchor.1);
    // Narrower where it stands: there at once, so the move stays in force.
    b.set_orders(
        0,
        &[Order::MoveTo {
            anchor: line.anchor,
            facing: Some(line.facing),
            width: Some(4.0),
            run: false,
        }],
    );
    b.step();
    let v = b.live_view(0, (0.0, 0.0), (1.0, 0.0), 0);
    assert_eq!((v.lines[0].width, v.orders[0].len()), (4.0, 1));
    let then = (ahead.0, ahead.1 + 10.0);
    let last = Order::MoveTo {
        anchor: then,
        facing: Some((0.0, 1.0)),
        width: Some(10.0),
        run: false,
    };
    b.set_orders(0, &[Order::move_to(ahead), last]);
    for _ in 0..40 {
        b.step();
    }
    let v = b.live_view(0, (0.0, 0.0), (1.0, 0.0), 0);
    let l = v.lines[0];
    assert_eq!((v.orders[0].as_slice(), l.width), (&[last][..], 10.0));
    let off = (l.anchor.0 - then.0).hypot(l.anchor.1 - then.1);
    assert!(off < 2.0 && l.facing.1 > 0.95, "{l:?}");
}

/// A recorded battle on the soldier model has every soldier in each frame,
/// packed to six bytes, and played back between frames.
#[test]
fn replay_frames_carry_every_soldier() {
    assert_eq!(std::mem::size_of::<FrameSoldier>(), 6);
    let (field, specs) = parse_lineup("swordsmen*30 vs spearmen*20").unwrap();
    let mut b = Battle::new(soldiers(), field.clone(), specs, 1);
    let (_, squads, frames) = b.run_recorded(5.0);
    let replay = Replay {
        town: 0,
        at: (0.0, 0.0),
        facing: (1.0, 0.0),
        field: (field.width, field.height),
        squads,
        frames,
        winner: None,
    };
    for f in &replay.frames {
        let sizes: Vec<usize> = f.soldiers.iter().map(Vec::len).collect();
        assert_eq!(sizes, [30, 20]);
    }
    let last = replay.frames.last().unwrap();
    assert!(last
        .soldiers
        .iter()
        .flatten()
        .any(|m| m.state == FighterState::Down));
    // Halfway between two frames, a soldier stands halfway between.
    let (a, b) = (&replay.frames[4], &replay.frames[5]);
    let mid = replay.soldiers_at(0.5 * (a.t + b.t));
    let (pa, pb, pm) = (
        a.soldiers[0][0].pos(),
        b.soldiers[0][0].pos(),
        mid[0][0].pos(),
    );
    assert!((pm.0 - 0.5 * (pa.0 + pb.0)).abs() <= 0.1 && (pm.1 - 0.5 * (pa.1 + pb.1)).abs() <= 0.1);
}

#[test]
fn a_frame_soldier_keeps_a_tenth_of_a_metre_and_turns_the_short_way() {
    let m = FrameSoldier::new(123.456, -7.89, (0.0, 2.0), FighterState::Routing);
    let (x, y) = m.pos();
    assert!((x - 123.456).abs() <= 0.05 && (y + 7.89).abs() <= 0.05);
    let f = m.facing();
    assert!(f.0.abs() < 0.02 && (f.1 - 1.0).abs() < 1e-3);
    assert_eq!(m.state, FighterState::Routing);
    // From just below east to just above it: through east, not round west.
    let a = FrameSoldier::new(0.0, 0.0, (1.0, -0.1), FighterState::Standing);
    let b = FrameSoldier::new(10.0, 0.0, (1.0, 0.1), FighterState::Down);
    let h = FrameSoldier::between(a, b, 0.5);
    assert!(h.facing().0 > 0.99 && (h.pos().0 - 5.0).abs() < 1e-4);
    assert_eq!(h.state, FighterState::Down);
}

/// Debugging aid: `TRACE="cavalry*20 vs spearmen*30" cargo test trace -- --ignored --nocapture`.
#[test]
#[ignore]
fn trace() {
    let lineup = std::env::var("TRACE").unwrap_or("swordsmen*30 vs spearmen*30".into());
    let seed = std::env::var("SEED").map_or(1, |s| s.parse().unwrap());
    let every = std::env::var("EVERY").map_or(10, |s| s.parse().unwrap());
    let (field, specs) = parse_lineup(&lineup).unwrap();
    let mut b = SoldierBattle::new(soldiers(), field, specs, seed);
    let mut t = 0;
    while !finished(&b) {
        if t % every == 0 {
            for (c, s) in b.squads.iter().enumerate() {
                let r = s.first..s.first + s.spec.members.len();
                let count =
                    |st: SoldierState| r.clone().filter(|&i| b.soldier[i].state == st).count();
                let fat = r.clone().map(|i| b.soldier[i].fatigue).sum::<f32>() / r.len() as f32;
                eprintln!(
                    "t={t:>4} {c} {:?} {:?} centre ({:.0},{:.0}) face ({:.2},{:.2}) m {:.2} form {} eng {} wav {} rout {} down {} fled {} fat {:.2} tgt {:?} with {:?}",
                    s.state, s.formation, s.centre.x, s.centre.y, s.facing.x, s.facing.y, s.morale,
                    count(SoldierState::Formed), count(SoldierState::Engaged), count(SoldierState::Wavering),
                    count(SoldierState::Routing), count(SoldierState::Down), count(SoldierState::Fled), fat, s.target, s.fighting_with
                );
            }
        }
        b.step();
        t += 1;
    }
    for e in &b.events {
        eprintln!("{:.0} {:?}", e.t, e.kind);
    }
}

/// Measure: a soldier replay's frames and bytes in memory, for the design
/// note's table. `TRACE="swordsmen*200 vs swordsmen*200" cargo test
/// replay_size -- --ignored --nocapture`.
#[test]
#[ignore]
fn replay_size() {
    let lineup = std::env::var("TRACE").unwrap_or("swordsmen*30 vs spearmen*20".into());
    let (field, specs) = parse_lineup(&lineup).unwrap();
    let (report, _, frames) = Battle::new(soldiers(), field, specs, 1).run_recorded(5.0);
    let bytes: usize = frames
        .iter()
        .map(|f| {
            f.soldiers
                .iter()
                .map(|m| m.len() * std::mem::size_of::<FrameSoldier>() + 24)
                .sum::<usize>()
        })
        .sum();
    let n: usize = frames[0].soldiers.iter().map(Vec::len).sum();
    eprintln!(
        "{lineup}: {n} soldiers, {:.0} s, {} frames, {bytes} bytes of soldiers",
        report.seconds,
        frames.len()
    );
}

/// Debugging aid: how each of `SEEDS` battles of `TRACE` ended.
#[test]
#[ignore]
fn probe() {
    let lineup = std::env::var("TRACE").unwrap_or("swordsmen*200 vs swordsmen*200".into());
    let seeds = std::env::var("SEEDS").map_or(20, |s| s.parse().unwrap());
    for seed in 0..seeds {
        let (field, specs) = parse_lineup(&lineup).unwrap();
        let r = Battle::new(soldiers(), field, specs, seed).run();
        let routs: Vec<String> = r
            .events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::Routing(_) | EventKind::Wavering(_)))
            .map(|e| format!("{:.0}:{:?}", e.t, e.kind))
            .collect();
        eprintln!(
            "{seed} winner {:?} {:?} {}",
            r.winner,
            r.squads
                .iter()
                .map(|s| (s.standing, s.state))
                .collect::<Vec<_>>(),
            routs.join(" ")
        );
    }
}
