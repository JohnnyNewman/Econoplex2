//! Jobs audit: what each job pays and how many take it, per town. Diagnostic.
//! cargo run --release -p sim_cli --example jobs_audit -- [years] [seed] [path=value ...]
use sim_core::{AgentStore, AssetPaths, Db, Simulation, Towns};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let years: u64 = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(3);
    let seed: Option<u64> = args.get(2).and_then(|v| v.parse().ok());
    let sets: Vec<(String, String)> = args
        .iter()
        .skip(3)
        .filter_map(|a| {
            a.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect();
    let paths = AssetPaths::discover();
    let (db, params, mut scenario) = sim_core::load_with(&paths, &sets).unwrap();
    if let Some(s) = seed {
        scenario.seed = s;
    }
    let dpy = params.life.days_per_year as u64;
    let mut sim = Simulation::new(db, params, &scenario);
    let mut counts: Vec<Vec<u32>> = Vec::new();
    for day in 0..years * dpy {
        sim.step();
        if day + 30 >= years * dpy {
            // Who worked what over the last month.
            let store = sim.world.resource::<AgentStore>();
            let towns = sim.world.resource::<Towns>();
            let nr = sim.world.resource::<Db>().recipes.len();
            counts.resize(towns.0.len(), vec![0; nr]);
            for i in 0..store.len() {
                if let (true, Some(r)) = (store.alive[i], store.worked_today[i]) {
                    counts[store.town[i] as usize][r as usize] += 1;
                }
            }
        }
    }
    let db = sim.world.resource::<Db>();
    let towns = sim.world.resource::<Towns>();
    for (t, town) in towns.0.iter().enumerate() {
        println!("{} (pop {})", town.name, town.residents.len());
        let mut rows: Vec<usize> = (0..db.recipes.len()).collect();
        rows.sort_by(|&a, &b| counts[t][b].cmp(&counts[t][a]));
        println!(
            "  {:<22} {:>9} {:>8} {:>10}",
            "job", "pay/day", "workers", "out price"
        );
        for r in rows.into_iter().take(14) {
            let m = &db.content.recipes[r];
            let out = db.recipes[r].outputs.first().map(|o| o.0 as usize);
            let pr = out.map_or(0.0, |p| town.price[p] / db.base_price[p].max(1e-6));
            println!(
                "  {:<22} {:>9.2} {:>8.1} {:>9.2}x",
                m.name,
                town.pay_rate[r],
                counts[t][r] as f32 / 30.0,
                pr
            );
        }
    }
}
