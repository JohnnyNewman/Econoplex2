//! The soldier model's own tests (design note, "Tests"). The squad model's
//! matchups run on both models in `battle/tests.rs`.

use super::*;
use crate::battle::tests::{losses_on, wins_on};
use std::time::Instant;

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

fn assert_even(lineup: &str) {
    let (listed, swapped) = side_0_wins(lineup, 100);
    assert!(
        (35..=65).contains(&listed) && listed.abs_diff(swapped) <= 10,
        "{lineup}: side 0 won {listed} of 100, {swapped} with the sides swapped"
    );
}

/// Equal armies, mirror images on `Auto`, win about half the time each:
/// neither the side nor the order the soldiers move in decides the fight.
#[test]
fn equal_armies_win_about_half_the_time() {
    assert_even("swordsmen*200 vs swordsmen*200");
}

/// The same at 2,000 a side (200 battles of 4,000 soldiers: release only).
#[test]
#[ignore]
fn equal_armies_win_about_half_the_time_at_2000_a_side() {
    assert_even("swordsmen*2000 vs swordsmen*2000");
}

/// A flank beats a frontal attack: it wins at least 40 of 50, and at least
/// 15 more than frontally on the same seeds. (The frontal mirror sits near
/// 25, so a margin of 25 would ask the flank to win every battle.)
#[test]
fn a_flank_beats_a_frontal_attack() {
    let m = BattleModel::Soldiers;
    let frontal = "swordsmen*100,swordsmen*100 vs swordsmen*100,swordsmen*100";
    let flank = "swordsmen*100,swordsmen*100#flank vs swordsmen*100,swordsmen*100";
    let (front_won, _) = wins_on(m, frontal, 50);
    let (flank_won, _) = wins_on(m, flank, 50);
    let (_, front_lost) = losses_on(m, frontal, 50);
    let (_, flank_lost) = losses_on(m, flank, 50);
    assert!(
        flank_won >= 40 && flank_won >= front_won + 15 && flank_lost > front_lost,
        "side 0 won {front_won} of 50 frontally, {flank_won} with the flank; \
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

#[test]
fn the_rear_refills_a_fallen_front_rank() {
    let (field, specs) = parse_lineup("swordsmen*100 vs swordsmen*100").unwrap();
    let mut b = SoldierBattle::new(soldiers(), field, specs, 1);
    let r = b.squads[0].first..b.squads[0].first + 100;
    let front = r.clone().find(|&i| b.rank[i] == 0).unwrap();
    let file = b.file[front];
    assert!(
        r.clone().any(|i| b.file[i] == file && b.rank[i] == 1),
        "the line has a second rank"
    );
    b.state[front] = SoldierState::Down;
    b.health[front] = 0.0;
    b.squads[0].losses += 1;
    b.squads[0].left += 1;
    b.substep();
    assert!(
        r.clone()
            .any(|i| i != front && b.state[i].in_line() && b.file[i] == file && b.rank[i] == 0),
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
        b.facing[i] = up;
    }
    for _ in 0..90 {
        b.step();
    }
    let s = &b.squads[0];
    let to_riders = (b.squads[1].centre - s.centre).norm_or(up);
    let faced = r
        .clone()
        .filter(|&i| b.facing[i].dot(to_riders) > 0.9)
        .count();
    assert!(
        s.formation == Formation::Braced && s.facing.dot(to_riders) > 0.95 && faced >= 50,
        "{:?} facing {:?}, riders toward {to_riders:?}, {faced} of 60 face them",
        s.formation,
        s.facing
    );
}

/// The budget: a 4,000-soldier battle (50 squads of 40 a side) costs at most
/// 1 ms per battle second on one core. Release only.
#[test]
#[ignore]
fn four_thousand_soldiers_within_budget() {
    let side = vec!["swordsmen*40"; 50].join(",");
    let lineup = format!("{side} vs {side}");
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let (ms, seconds) = pool.install(|| {
        let (field, specs) = parse_lineup(&lineup).unwrap();
        let mut b = Battle::new(soldiers(), field, specs, 1);
        let start = Instant::now();
        let r = b.run();
        (start.elapsed().as_secs_f64() * 1e3, r.seconds)
    });
    let per = ms / seconds as f64;
    assert!(
        per <= 1.0,
        "{per:.3} ms per battle second ({ms:.0} ms for {seconds:.0} s)"
    );
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
                let count = |st: SoldierState| r.clone().filter(|&i| b.state[i] == st).count();
                let fat = r.clone().map(|i| b.fatigue[i]).sum::<f32>() / r.len() as f32;
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
