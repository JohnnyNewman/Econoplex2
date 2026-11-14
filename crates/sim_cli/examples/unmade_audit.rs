//! Unmade goods: for each town, the goods it did not make last year and why
//! (no building, no nature site, inputs missing, nobody took the work).
//! cargo run --release -p sim_cli --example unmade_audit -- [years] [seed]
use sim_core::components::NaturalResource;
use sim_core::{AssetPaths, Db, Simulation, Towns};

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
    for _ in 0..years * dpy {
        sim.step();
    }
    let mut q = sim.world.query::<&NaturalResource>();
    let kinds: Vec<Vec<u32>> = sim
        .world
        .resource::<Towns>()
        .0
        .iter()
        .map(|t| {
            t.nature_sites
                .iter()
                .filter_map(|&e| q.get(&sim.world, e).ok().map(|n| n.kind))
                .collect()
        })
        .collect();
    let db = sim.world.resource::<Db>();
    let towns = sim.world.resource::<Towns>();
    let mut tally: std::collections::BTreeMap<&str, u32> = Default::default();
    for (t, town) in towns.0.iter().enumerate() {
        println!("{}", town.name);
        for (r, m) in db.recipes.iter().enumerate() {
            let made: f32 = m
                .outputs
                .iter()
                .map(|o| {
                    town.produced_last_year
                        .get(o.0 as usize)
                        .copied()
                        .unwrap_or(0.0)
                })
                .sum();
            if made > 0.0 {
                continue;
            }
            let why = if m.nature.is_some_and(|k| !kinds[t].contains(&k)) {
                "no nature site".to_string()
            } else if m.building.is_some_and(|b| town.building(b).is_none()) {
                "no building".to_string()
            } else if m.inputs.iter().any(|&(p, q)| town.stock[p as usize] < q) {
                let missing: Vec<&str> = m
                    .inputs
                    .iter()
                    .filter(|&&(p, q)| town.stock[p as usize] < q)
                    .map(|&(p, _)| db.content.products[p as usize].name.as_str())
                    .collect();
                format!("inputs missing: {}", missing.join(", "))
            } else {
                format!("nobody worked it (pay {:.2})", town.pay_rate[r])
            };
            let key = why
                .split(':')
                .next()
                .unwrap_or("")
                .split(" (")
                .next()
                .unwrap_or("");
            *tally
                .entry(match key {
                    "no nature site" => "no nature site",
                    "no building" => "no building",
                    "inputs missing" => "inputs missing",
                    _ => "nobody worked it",
                })
                .or_default() += 1;
            println!("  {:<24} {}", db.content.recipes[r].name, why);
        }
    }
    println!(
        "
unmade recipes over all towns, by reason: {tally:?}"
    );
}
