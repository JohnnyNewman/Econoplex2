//! Batch mode: parameter sweeps over several seeds.
//!
//! Every combination of the swept values (a grid) runs on every seed. Runs are
//! independent and deterministic, so they go out to all cores and the results
//! do not depend on which thread ran them. Each run is reduced to one summary
//! row; rows for the same setting are averaged across seeds, with the spread.

use sim_core::config::Scenario;
use sim_core::metrics::Metrics;
use sim_core::{AssetPaths, Simulation};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

pub struct Plan {
    pub paths: AssetPaths,
    pub sets: Vec<(String, String)>,
    pub sweeps: Vec<(String, Vec<String>)>,
    pub seeds: u64,
    pub scenario: Scenario,
    pub years: u64,
    pub orders: Vec<(u64, String)>,
}

/// What one run comes to.
#[derive(Clone, Copy, Default)]
struct Summary {
    population: f64,
    /// Share of resident-days spent starving, over the whole run.
    starving: f64,
    food_per_capita: f64,
    /// Products made per town in the last year.
    diversity: f64,
    /// Largest minus smallest ECI between towns in the last year.
    eci_range: f64,
    specialization: f64,
    masters: f64,
    swords: f64,
    mean_wealth: f64,
    /// Gini coefficient of the towns' mean wealth (inequality between towns).
    wealth_gini: f64,
    raids: f64,
    conquests: f64,
    revolts: f64,
    /// Share of town-years spent paying tribute.
    ruled: f64,
    migrants: f64,
    ms_per_tick: f64,
}

const COLUMNS: [&str; 16] = [
    "pop", "starve%", "food/cap", "div", "ECI rng", "spec", "masters", "swords", "wealth", "gini",
    "raids", "conq", "revolts", "ruled%", "moves", "ms/tick",
];

impl Summary {
    fn values(&self) -> [f64; 16] {
        [
            self.population,
            self.starving * 100.0,
            self.food_per_capita,
            self.diversity,
            self.eci_range,
            self.specialization,
            self.masters,
            self.swords,
            self.mean_wealth,
            self.wealth_gini,
            self.raids,
            self.conquests,
            self.revolts,
            self.ruled * 100.0,
            self.migrants,
            self.ms_per_tick,
        ]
    }

    fn of(m: &Metrics, ms_per_tick: f64) -> Self {
        let mut s = Summary {
            ms_per_tick,
            ..Default::default()
        };
        let Some(last) = m.latest() else { return s };
        let towns = last.towns.len().max(1) as f64;
        s.population = last.population as f64;
        let pop = last
            .towns
            .iter()
            .map(|t| t.population)
            .sum::<usize>()
            .max(1) as f64;
        s.food_per_capita = last
            .towns
            .iter()
            .map(|t| t.food_per_capita as f64 * t.population as f64)
            .sum::<f64>()
            / pop;
        s.diversity = last.towns.iter().map(|t| t.diversity as f64).sum::<f64>() / towns;
        let eci = last.towns.iter().map(|t| t.eci as f64);
        s.eci_range = eci.clone().fold(f64::MIN, f64::max) - eci.fold(f64::MAX, f64::min);
        s.specialization = last
            .towns
            .iter()
            .map(|t| t.specialization as f64 * t.adults as f64)
            .sum::<f64>()
            / last.towns.iter().map(|t| t.adults).sum::<usize>().max(1) as f64;
        s.masters = last.towns.iter().map(|t| t.masters as f64).sum();
        s.swords = last
            .towns
            .iter()
            .map(|t| t.swords.iter().sum::<u32>() as f64)
            .sum();
        s.mean_wealth = last
            .towns
            .iter()
            .map(|t| t.mean_wealth as f64 * t.population as f64)
            .sum::<f64>()
            / pop;
        s.wealth_gini = gini(
            &last
                .towns
                .iter()
                .map(|t| t.mean_wealth as f64)
                .collect::<Vec<_>>(),
        );

        let (mut starve, mut people, mut town_years, mut ruled) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for y in &m.years {
            for t in &y.towns {
                starve += t.starving as f64 * t.population as f64;
                people += t.population as f64;
                town_years += 1.0;
                ruled += t.ruler.is_some() as u8 as f64;
                s.raids += t.war.raids as f64;
                s.conquests += t.war.conquests as f64;
                s.revolts += t.war.revolts as f64;
                s.migrants += t.moves.arrived as f64;
            }
        }
        s.starving = starve / people.max(1.0);
        s.ruled = ruled / town_years.max(1.0);
        s
    }
}

fn gini(x: &[f64]) -> f64 {
    let n = x.len() as f64;
    let mean = x.iter().sum::<f64>() / n.max(1.0);
    if mean <= 0.0 {
        return 0.0;
    }
    let mut d = 0.0;
    for a in x {
        for b in x {
            d += (a - b).abs();
        }
    }
    d / (2.0 * n * n * mean)
}

/// Every combination of the swept values, first sweep varying slowest.
fn grid(sweeps: &[(String, Vec<String>)]) -> Vec<Vec<(String, String)>> {
    let mut out = vec![Vec::new()];
    for (path, values) in sweeps {
        out = out
            .into_iter()
            .flat_map(|combo| {
                values.iter().map(move |v| {
                    let mut c = combo.clone();
                    c.push((path.clone(), v.clone()));
                    c
                })
            })
            .collect();
    }
    out
}

pub fn run(plan: &Plan, csv: Option<&std::path::Path>) -> Result<(), Box<dyn std::error::Error>> {
    let settings = grid(&plan.sweeps);
    // Fail on a bad path or value before starting any thread.
    for combo in &settings {
        let mut sets = plan.sets.clone();
        sets.extend(combo.iter().cloned());
        let (db, params, _) = sim_core::load_with(&plan.paths, &sets)?;
        let mut sim = Simulation::new(db, params, &plan.scenario);
        for (_, text) in &plan.orders {
            sim.order(text)?;
        }
    }
    let jobs: Vec<(usize, u64)> = (0..settings.len())
        .flat_map(|c| (0..plan.seeds).map(move |k| (c, plan.scenario.seed + k)))
        .collect();
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(jobs.len());
    println!(
        "Batch: {} settings x {} seeds = {} runs of {} years on {} threads",
        settings.len(),
        plan.seeds,
        jobs.len(),
        plan.years,
        threads
    );

    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Summary>>> = Mutex::new(vec![None; jobs.len()]);
    let start = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let j = next.fetch_add(1, Ordering::Relaxed);
                let Some(&(c, seed)) = jobs.get(j) else { break };
                let mut sets = plan.sets.clone();
                sets.extend(settings[c].iter().cloned());
                let (db, params, _) = sim_core::load_with(&plan.paths, &sets)
                    .expect("settings were checked before the threads started");
                let mut scenario = plan.scenario.clone();
                scenario.seed = seed;
                let ticks = plan.years * params.life.days_per_year as u64;
                let mut sim = Simulation::new(db, params, &scenario);
                let t0 = Instant::now();
                for tick in 0..ticks {
                    for (_, text) in plan.orders.iter().filter(|o| o.0 == tick) {
                        sim.order(text)
                            .expect("orders were checked before the threads started");
                    }
                    sim.step();
                }
                let ms = t0.elapsed().as_secs_f64() * 1000.0 / ticks.max(1) as f64;
                let s = Summary::of(sim.world.resource::<Metrics>(), ms);
                results.lock().unwrap()[j] = Some(s);
            });
        }
    });
    let results: Vec<Summary> = results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|s| s.expect("every job ran"))
        .collect();
    println!("done in {:.1} s\n", start.elapsed().as_secs_f64());

    // --- one line per setting: mean across seeds, and ± one standard deviation
    let label = |c: usize| {
        if settings[c].is_empty() {
            "(models.ron)".to_string()
        } else {
            settings[c]
                .iter()
                .map(|(p, v)| format!("{}={v}", p.rsplit('.').next().unwrap_or(p)))
                .collect::<Vec<_>>()
                .join(" ")
        }
    };
    let width = (0..settings.len())
        .map(|c| label(c).len())
        .max()
        .unwrap_or(0)
        .max(7);
    print!("{:<width$}", "setting");
    for c in COLUMNS {
        print!(" {c:>8}");
    }
    println!();
    for c in 0..settings.len() {
        let rows: Vec<[f64; 16]> = (0..jobs.len())
            .filter(|&j| jobs[j].0 == c)
            .map(|j| results[j].values())
            .collect();
        let n = rows.len() as f64;
        let mean: Vec<f64> = (0..16)
            .map(|k| rows.iter().map(|r| r[k]).sum::<f64>() / n)
            .collect();
        let sd: Vec<f64> = (0..16)
            .map(|k| (rows.iter().map(|r| (r[k] - mean[k]).powi(2)).sum::<f64>() / n).sqrt())
            .collect();
        print!("{:<width$}", label(c));
        for v in &mean {
            print!(" {:>8}", fmt(*v));
        }
        println!();
        if rows.len() > 1 {
            print!("{:<width$}", "  ±");
            for v in &sd {
                print!(" {:>8}", fmt(*v));
            }
            println!();
        }
    }

    if let Some(path) = csv {
        let mut f = std::fs::File::create(path)?;
        let keys: Vec<&str> = plan.sweeps.iter().map(|s| s.0.as_str()).collect();
        let header = [
            "population",
            "starving",
            "food_per_capita",
            "diversity",
            "eci_range",
            "specialization",
            "masters",
            "swords",
            "mean_wealth",
            "wealth_gini",
            "raids",
            "conquests",
            "revolts",
            "ruled",
            "migrants",
            "ms_per_tick",
        ];
        writeln!(
            f,
            "{}",
            keys.iter()
                .copied()
                .chain(["seed"])
                .chain(header)
                .collect::<Vec<_>>()
                .join(",")
        )?;
        for (j, &(c, seed)) in jobs.iter().enumerate() {
            let mut row: Vec<String> = settings[c].iter().map(|s| s.1.clone()).collect();
            row.push(seed.to_string());
            let mut v = results[j].values();
            v[1] /= 100.0;
            v[13] /= 100.0;
            row.extend(v.iter().map(|x| format!("{x:.4}")));
            writeln!(f, "{}", row.join(","))?;
        }
        println!("\nwrote {}", path.display());
    }
    Ok(())
}

fn fmt(v: f64) -> String {
    match v.abs() {
        a if a >= 1000.0 => format!("{v:.0}"),
        a if a >= 10.0 => format!("{v:.1}"),
        _ => format!("{v:.2}"),
    }
}
