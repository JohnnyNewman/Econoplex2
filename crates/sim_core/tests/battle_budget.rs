//! The soldier model's budget: a 4,000-soldier battle (50 squads of 40 a
//! side) costs at most 1 ms per battle second on one core.
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
    assert!(
        per <= 1.0,
        "{per:.3} ms per battle second ({ms:.0} ms for {seconds:.0} s)"
    );
}
