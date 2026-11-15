//! Food audit: the bread chain as it runs, against the paper calculation.
//! For each town over year N: worker-days on farming, milling, baking and
//! woodcutting, what each made per worker-day, and bread eaten per head.
//! cargo run --release -p sim_cli --example food_audit -- [year] [seed]
use sim_core::goods::Flow;
use sim_core::{AgentStore, AssetPaths, Db, Simulation, Towns};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let year: u64 = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(3);
    let seed: Option<u64> = args.get(2).and_then(|v| v.parse().ok());
    let paths = AssetPaths::discover();
    let (db, params, mut scenario) = sim_core::load(&paths).unwrap();
    if let Some(s) = seed {
        scenario.seed = s;
    }
    let dpy = params.time.months_per_year as u64;
    let mut sim = Simulation::new(db, params, &scenario);
    for _ in 0..(year - 1) * dpy {
        sim.step();
    }
    let (nt, nr, np) = {
        let db = sim.world.resource::<Db>();
        (
            sim.world.resource::<Towns>().0.len(),
            db.recipes.len(),
            db.content.products.len(),
        )
    };
    let flows_at = |sim: &Simulation, f: Flow| -> Vec<Vec<f64>> {
        sim.world
            .resource::<Towns>()
            .0
            .iter()
            .map(|t| (0..np).map(|p| t.flows.get(f, p)).collect())
            .collect()
    };
    let made0 = flows_at(&sim, Flow::Made);
    let eaten0 = flows_at(&sim, Flow::Eaten);
    let mut days = vec![vec![0u32; nr]; nt];
    let mut heads = vec![0u64; nt];
    for _ in 0..dpy {
        sim.step();
        let store = sim.world.resource::<AgentStore>();
        for i in 0..store.len() {
            if !store.alive[i] {
                continue;
            }
            let t = store.town[i] as usize;
            heads[t] += 1;
            if let Some(r) = store.worked_today[i] {
                days[t][r as usize] += 1;
            }
        }
    }
    let made1 = flows_at(&sim, Flow::Made);
    let eaten1 = flows_at(&sim, Flow::Eaten);
    let db = sim.world.resource::<Db>();
    let towns = sim.world.resource::<Towns>();
    let recipe = |id: &str| db.content.recipes.iter().position(|r| r.id == id);
    let good = |id: &str| db.content.product(id).map(|p| p as usize);
    println!("year {year}: per worker-day made (paper: farm 4 grain, mill 3 flour, bake 4 bread)");
    for (t, town) in towns.0.iter().enumerate() {
        let pop = heads[t] as f64 / dpy as f64;
        println!("{} (mean pop {pop:.0})", town.name);
        for (rid, gid) in [
            ("farm_grain", "grain"),
            ("mill_flour", "flour"),
            ("bake_bread", "bread"),
            ("chop_wood", "wood"),
        ] {
            let (Some(r), Some(g)) = (recipe(rid), good(gid)) else {
                continue;
            };
            let d = days[t][r] as f64;
            let m = made1[t][g] - made0[t][g];
            println!(
                "  {:<12} {:>7.0} worker-days ({:>5.1} full-time)  made {:>8.0} {gid}  = {:>5.2} per worker-day",
                rid,
                d,
                d / dpy as f64,
                m,
                m / d.max(1.0)
            );
        }
        let food: f64 = db
            .foods
            .iter()
            .map(|&(f, v)| (eaten1[t][f as usize] - eaten0[t][f as usize]) * v as f64)
            .sum();
        println!(
            "  food eaten {:.1} per head (paper: {:.1})",
            food / pop.max(1.0),
            0.12 * dpy as f64
        );
    }
}
