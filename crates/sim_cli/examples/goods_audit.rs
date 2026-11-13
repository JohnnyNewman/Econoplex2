//! Goods audit: for every good, is what is held equal to what was seeded and
//! made minus what left? Diagnostic only (see `sim_core::goods`).
//! cargo run --release -p sim_cli --example goods_audit -- [years] [seed]
use sim_core::goods::{audit, Flow};
use sim_core::{AgentStore, AssetPaths, Db, Simulation, Towns};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let years: u64 = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(10);
    let seed: Option<u64> = args.get(2).and_then(|v| v.parse().ok());
    let paths = AssetPaths::discover();
    let (db, params, mut scenario) = sim_core::load(&paths).unwrap();
    if let Some(s) = seed {
        scenario.seed = s;
    }
    let dpy = params.life.days_per_year as u64;
    let mut sim = Simulation::new(db, params, &scenario);
    for y in 1..=years {
        for _ in 0..dpy {
            sim.step();
        }
        let db = sim.world.resource::<Db>();
        let n = db.content.products.len();
        let bad = audit(
            sim.world.resource::<AgentStore>(),
            sim.world.resource::<Towns>(),
            n,
            1e-4,
        );
        println!("year {y}: {} goods out of balance", bad.len());
        for m in &bad {
            println!(
                "  {:<14} held {:>10.1}  expected {:>10.1}  gap {:>+9.1}",
                db.content.products[m.good].name,
                m.held,
                m.expected,
                m.gap()
            );
        }
    }
    // The year's flows over the whole world, for the goods that moved most.
    let db = sim.world.resource::<Db>();
    let towns = sim.world.resource::<Towns>();
    println!("\nflows since the start (all towns):");
    print!("{:<14}", "good");
    for f in Flow::ALL {
        print!(" {:>9}", f.name());
    }
    println!();
    for (p, prod) in db.content.products.iter().enumerate() {
        let row: Vec<f64> = Flow::ALL
            .iter()
            .map(|&f| towns.0.iter().map(|t| t.flows.get(f, p)).sum())
            .collect();
        if row.iter().all(|&v| v < 0.5) {
            continue;
        }
        print!("{:<14}", prod.name);
        for v in row {
            print!(" {v:>9.0}");
        }
        println!();
    }
}
