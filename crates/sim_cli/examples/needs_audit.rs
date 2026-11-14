//! Needs audit: how well each town meets each need on the ladder, month by
//! month, with the cold and the fuel. Diagnostic only (see `sim_core::ladder`).
//! cargo run --release -p sim_cli --example needs_audit -- [years] [seed]
//! Runs with `market.consumption: Needs` (the default settings below) unless
//! models.ron already sets it.
use sim_core::consumption::ConsumptionParams;
use sim_core::ladder::{all, LadderParams};
use sim_core::{AgentStore, AssetPaths, Db, Simulation, Towns};
use sim_data::Need;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let years: u64 = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(3);
    let seed: Option<u64> = args.get(2).and_then(|v| v.parse().ok());
    let paths = AssetPaths::discover();
    let (db, mut params, mut scenario) = sim_core::load(&paths).unwrap();
    if !matches!(params.market.consumption, ConsumptionParams::Needs(_)) {
        params.market.consumption = ConsumptionParams::Needs(LadderParams {
            interval: 7,
            gate_at: 0.7,
            gate_width: 0.15,
            comfort_c: 16.0,
            house_c: 6.0,
            max_clothes: 2.0,
            fire_per_c: 0.5,
            fire_per_extra_person: 0.25,
            savings_days: 60.0,
            larder_days: 0.0,
            cold_damage: 0.02,
            plan_days: 30.0,
            fixed_needs: Vec::new(),
        });
    }
    if let Some(s) = seed {
        scenario.seed = s;
    }
    let dpy = params.life.days_per_year as u64;
    let month = (dpy / 12).max(1);
    let mut sim = Simulation::new(db, params, &scenario);
    print!("{:>4} {:>3} {:>6}", "year", "mon", "temp");
    for n in Need::ALL {
        print!(" {:>9}", format!("{n:?}"));
    }
    println!(" {:>6} {:>8}", "cold%", "charcoal");
    let mut cold_before = 0u32;
    for y in 0..years {
        for m in 0..12 {
            for _ in 0..month {
                sim.step();
            }
            let w = &sim.world;
            let params = w.resource::<sim_core::world::Params>();
            let ConsumptionParams::Needs(lp) = &params.market.consumption else {
                unreachable!()
            };
            let towns = w.resource::<Towns>();
            let env = w.resource::<sim_core::env::Environment>();
            let db = w.resource::<Db>();
            let store = w.resource::<AgentStore>();
            let sats = all(store, towns, db, params, env, lp);
            let mut sum = [0.0f32; 9];
            let mut n = 0.0f32;
            for s in sats.iter().flatten() {
                for k in 0..9 {
                    sum[k] += s[k];
                }
                n += 1.0;
            }
            let temp =
                env.towns.iter().map(|t| t.day_mean_c).sum::<f32>() / env.towns.len().max(1) as f32;
            let cold: u32 = towns.0.iter().map(|t| t.cold_days).sum();
            let charcoal = db.content.product("charcoal").map_or(0.0, |p| {
                towns.0.iter().map(|t| t.price[p as usize]).sum::<f32>() / towns.0.len() as f32
            });
            print!("{:>4} {:>3} {:>6.1}", y + 1, m + 1, temp);
            for v in sum {
                print!(" {:>9.2}", v / n.max(1.0));
            }
            println!(
                " {:>6.1} {:>8.2}",
                100.0 * (cold - cold_before) as f32 / (n * month as f32).max(1.0),
                charcoal
            );
            cold_before = cold;
        }
    }
}
