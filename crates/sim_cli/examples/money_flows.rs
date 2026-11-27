//! Money flows per town per year (ledger totals). Diagnostic only.
//! cargo run --release -p sim_cli --example money_flows -- [years] [seed]
use sim_core::{AssetPaths, Simulation, Towns};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let years: u64 = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(10);
    let seed: Option<u64> = args.get(2).and_then(|v| v.parse().ok());
    let paths = AssetPaths::discover();
    let (db, params, mut scenario) = sim_core::load(&paths).unwrap();
    if let Some(s) = seed {
        scenario.seed = s;
    }
    let dpy = params.time.months_per_year as u64;
    let mut sim = Simulation::new(db, params, &scenario);
    let flows = sim_core::ledger::Flow::ALL;
    let mut prev: Vec<Vec<f64>> = Vec::new();
    for y in 1..=years {
        for _ in 0..dpy {
            sim.step();
        }
        let towns = sim.world.resource::<Towns>();
        if prev.is_empty() {
            prev = vec![vec![0.0; flows.len()]; towns.0.len()];
        }
        println!("year {y}");
        for (t, town) in towns.0.iter().enumerate() {
            let held: Vec<String> = town
                .foreign
                .iter()
                .map(|c| format!("{:.0}", c.coins()))
                .collect();
            println!(
                "  {:<11} coin value {:.3}, treasury {:.0}, holds [{}]",
                town.name,
                town.coin_value,
                town.treasury.coins(),
                held.join(", ")
            );
            let mut line = format!("  {:<11}", town.name);
            for (k, f) in flows.iter().enumerate() {
                let v = town.ledger.total[*f as usize];
                let d = v - prev[t][k];
                prev[t][k] = v;
                if d.abs() > 50.0 {
                    line += &format!(" {f:?} {d:.0},");
                }
            }
            println!("{line}");
        }
    }
}
