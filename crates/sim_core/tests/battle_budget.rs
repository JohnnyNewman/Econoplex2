//! The soldier model's budget: a 4,000-soldier battle (50 squads of 40 a
//! side) costs at most 1 ms per battle second on one core over the whole
//! battle, and at most 1.5 ms over its engaged phase, the first 400 battle
//! seconds. Targets not met yet: 1 ms on four threads, and a town at most
//! twice the open battle.
//!
//! It runs on release builds only (`cargo test --release`). It lives here,
//! not beside the model, so that it times the library as it ships rather
//! than the build of it that carries the unit tests.

use sim_core::battle::{parse_lineup, Battle, BattleModel, BattleParams};
use std::time::Instant;

#[test]
#[cfg_attr(debug_assertions, ignore = "a timing: release builds only")]
fn four_thousand_soldiers_within_budget() {
    let side = vec!["swordsmen*40"; 50].join(",");
    let lineup = format!("{side} vs {side}");
    let params = BattleParams {
        model: BattleModel::Soldiers,
        ..BattleParams::default()
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    // The best of three: a shared machine's other work only ever adds time.
    let (ms, seconds) = (0..3)
        .map(|_| {
            pool.install(|| {
                let (field, specs) = parse_lineup(&lineup).unwrap();
                let mut b = Battle::new(params.clone(), field, specs, 1);
                let start = Instant::now();
                let r = b.run();
                (start.elapsed().as_secs_f64() * 1e3, r.seconds)
            })
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap();
    let per = ms / seconds as f64;
    eprintln!("{per:.3} ms per battle second ({ms:.0} ms for {seconds:.0} s)");
    assert!(
        per <= 1.0,
        "{per:.3} ms per battle second ({ms:.0} ms for {seconds:.0} s)"
    );
}

/// The engaged phase: the yardstick's first 400 battle seconds, while
/// every line is on the field, the armies meet and the first squads break.
/// It must cost at most 1.5 ms per battle second on one core. The same
/// armies with the defenders in a town (`town`: sixty houses and eight
/// workplaces) cost more, timed in the same run so that a busy machine
/// slows both alike: here at most three times the open battle, until the
/// target of twice (`engaged_phase_targets`) is met. A town costs more than
/// open ground: the obstacle queries, and the fighting they make, soldiers
/// pressed out of their slots and squads jammed in the streets.
///
/// A walled town (`walled town`: the same town in a palisade with two
/// gates) gets five times: its four thousand soldiers spend the battle
/// pressed along the wall, where every step asks the obstacles where it
/// may go and every blow whether the wall stands between.
///
/// It also prints the dearest ten battle seconds, the height of the melee
/// (at 140 to 180 s, about three times the phase's average): no budget
/// holds it yet, see "The engaged phase" in the design doc.
#[test]
#[cfg_attr(debug_assertions, ignore = "a timing: release builds only")]
fn engaged_phase_within_budget() {
    let side = vec!["swordsmen*40"; 50].join(",");
    let open = engaged(&format!("{side} vs {side}"), 1);
    let town = engaged(&format!("{side} vs town {side}"), 1);
    let walled = engaged(&format!("{side} vs walled town {side}"), 1);
    println!(
        "first 400 s on one core: open {:.3} (dearest 10 s {:.3}), town {:.3}, \
         walled town {:.3} ms per battle second",
        open.0, open.1, town.0, walled.0
    );
    assert!(
        open.0 <= 1.5,
        "open {:.3} ms per battle second on one core, more than 1.5",
        open.0
    );
    assert!(
        town.0 <= 3.0 * open.0,
        "town {:.3} ms per battle second, more than three times open {:.3}",
        town.0,
        open.0
    );
    assert!(
        walled.0 <= 5.0 * open.0,
        "walled town {:.3} ms per battle second, more than five times open {:.3}",
        walled.0,
        open.0
    );
}

/// The engaged phase's targets not met yet: at most 1 ms per battle second
/// on four threads, for live battles on a machine with the cores to spare,
/// and a town at most twice the open battle on one core. On the shared
/// 4-core machine this was written on, four threads cost about what one
/// does and a town about 2.1 times open (see "The engaged phase" in the
/// design doc).
#[test]
#[ignore = "targets not met yet: run with --ignored on a quiet machine with 4 cores"]
fn engaged_phase_targets() {
    let side = vec!["swordsmen*40"; 50].join(",");
    let one = engaged(&format!("{side} vs {side}"), 1);
    let four = engaged(&format!("{side} vs {side}"), 4);
    let town = engaged(&format!("{side} vs town {side}"), 1);
    println!(
        "first 400 s: {:.3} ms per battle second on one thread, {:.3} on four \
         (dearest 10 s {:.3} and {:.3}), town {:.3} on one",
        one.0, four.0, one.1, four.1, town.0
    );
    assert!(
        four.0 <= 1.0,
        "{:.3} ms per battle second on four threads, more than 1",
        four.0
    );
    assert!(
        town.0 <= 2.0 * one.0,
        "town {:.3} ms per battle second, more than twice open {:.3}",
        town.0,
        one.0
    );
}

/// The yardstick's first 400 battle seconds of `lineup` on `threads`,
/// best of three: ms per battle second, and the dearest ten battle
/// seconds' ms per battle second.
fn engaged(lineup: &str, threads: usize) -> (f64, f64) {
    let params = BattleParams {
        model: BattleModel::Soldiers,
        ..BattleParams::default()
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    let runs: Vec<(f64, f64)> = (0..3)
        .map(|_| {
            pool.install(|| {
                let (field, specs) = parse_lineup(lineup).unwrap();
                let mut b = Battle::new(params.clone(), field, specs, 1);
                let start = Instant::now();
                let mut dearest = 0.0f64;
                while b.seconds() < 400.0 && !b.finished() {
                    let (t0, s0) = (Instant::now(), b.seconds());
                    while b.seconds() < s0 + 10.0 && !b.finished() {
                        b.step();
                    }
                    let ds = (b.seconds() - s0).max(1.0) as f64;
                    dearest = dearest.max(t0.elapsed().as_secs_f64() * 1e3 / ds);
                }
                let per = start.elapsed().as_secs_f64() * 1e3 / b.seconds() as f64;
                (per, dearest)
            })
        })
        .collect();
    let best = |f: fn(&(f64, f64)) -> f64| runs.iter().map(f).fold(f64::INFINITY, f64::min);
    (best(|r| r.0), best(|r| r.1))
}
