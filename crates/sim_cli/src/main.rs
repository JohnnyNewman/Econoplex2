//! Headless Econoplex runner.
//!
//! ```text
//! econoplex-sim [--years N] [--seed S] [--scale F] [--assets DIR] [--csv FILE]
//!               [--set path=value]... [--sweep path=v1,v2,...]... [--seeds N]
//!               [--embedding] [--hash] [--profile] [--quiet]
//! ```
//!
//! With `--sweep` or `--seeds` above 1 the runner switches to batch mode: every
//! combination of swept values is run on every seed, in parallel, and each run
//! is reduced to one summary row (see `sweep.rs`).

mod sweep;

use sim_core::metrics::Metrics;
use sim_core::{AssetPaths, Simulation};
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

struct Args {
    years: u64,
    seed: Option<u64>,
    scale: f32,
    assets: Option<PathBuf>,
    csv: Option<PathBuf>,
    embedding: bool,
    hash: bool,
    quiet: bool,
    profile: bool,
    /// Parameter overrides applied to models.ron, as (dotted.path, value).
    sets: Vec<(String, String)>,
    sweeps: Vec<(String, Vec<String>)>,
    seeds: u64,
}

fn parse_args() -> Args {
    let mut a = Args {
        years: 20,
        seed: None,
        scale: 1.0,
        assets: None,
        csv: None,
        embedding: false,
        hash: false,
        quiet: false,
        profile: false,
        sets: Vec::new(),
        sweeps: Vec::new(),
        seeds: 1,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = || {
            it.next()
                .unwrap_or_else(|| die(&format!("{arg} needs a value")))
        };
        match arg.as_str() {
            "--years" => {
                a.years = val()
                    .parse()
                    .unwrap_or_else(|_| die("--years expects a number"))
            }
            "--seed" => {
                a.seed = Some(
                    val()
                        .parse()
                        .unwrap_or_else(|_| die("--seed expects a number")),
                )
            }
            "--scale" => {
                a.scale = val()
                    .parse()
                    .unwrap_or_else(|_| die("--scale expects a number"))
            }
            "--assets" => a.assets = Some(PathBuf::from(val())),
            "--csv" => a.csv = Some(PathBuf::from(val())),
            "--set" => {
                let v = val();
                let (k, x) = v
                    .split_once('=')
                    .unwrap_or_else(|| die("--set expects path=value"));
                a.sets.push((k.to_string(), x.to_string()));
            }
            "--sweep" => {
                let v = val();
                let (k, x) = v
                    .split_once('=')
                    .unwrap_or_else(|| die("--sweep expects path=v1,v2,..."));
                a.sweeps
                    .push((k.to_string(), x.split(',').map(str::to_string).collect()));
            }
            "--seeds" => {
                a.seeds = val()
                    .parse()
                    .unwrap_or_else(|_| die("--seeds expects a number"))
            }
            "--embedding" => a.embedding = true,
            "--hash" => a.hash = true,
            "--quiet" => a.quiet = true,
            "--profile" => a.profile = true,
            "-h" | "--help" => {
                println!(
                    "econoplex-sim: run the Econoplex economy headless\n\n\
                     --years N      simulated years (default 20)\n\
                     --seed S       override the scenario seed\n\
                     --scale F      multiply every town's population and land (e.g. 10 for ~12,000 agents)\n\
                     --assets DIR   assets folder (default: discovered)\n\
                     --csv FILE     write per-town yearly metrics as CSV (batch mode: one row per run)\n\
                     --set P=V      override a parameter in models.ron, e.g. military.tribute_share=0.3\n\
                     --sweep P=A,B  run once per value (repeat for a grid); implies batch mode\n\
                     --seeds N      run N consecutive seeds from the scenario seed; implies batch mode\n\
                     --embedding    print the skill embedding (product space) report and exit\n\
                     --hash         print the final state hash (determinism check)\n\
                     --profile      print wall-clock time per system\n\
                     --quiet        only print the summary"
                );
                std::process::exit(0);
            }
            other => die(&format!("unknown argument `{other}` (try --help)")),
        }
    }
    a
}

fn die(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(2);
}

fn main() {
    let args = parse_args();
    let paths = args
        .assets
        .as_deref()
        .map(AssetPaths::from_root)
        .unwrap_or_else(AssetPaths::discover);
    let (db, params, mut scenario) =
        sim_core::load_with(&paths, &args.sets).unwrap_or_else(|e| die(&e.to_string()));

    if args.embedding {
        print_embedding(&db);
        return;
    }
    if let Some(s) = args.seed {
        scenario.seed = s;
    }
    for t in &mut scenario.towns {
        t.population = (t.population as f32 * args.scale).round() as u32;
        for s in &mut t.stock {
            s.1 *= args.scale;
        }
        // Land scales with people, otherwise a scaled town simply starves.
        for n in &mut t.nature {
            n.1 = ((n.1 as f32 * args.scale).round() as u32).max(1);
        }
    }

    if !args.sweeps.is_empty() || args.seeds > 1 {
        let plan = sweep::Plan {
            paths,
            sets: args.sets,
            sweeps: args.sweeps,
            seeds: args.seeds,
            scenario,
            years: args.years,
        };
        sweep::run(&plan, args.csv.as_deref()).unwrap_or_else(|e| die(&e.to_string()));
        return;
    }

    let dpy = params.life.days_per_year as u64;
    let mut sim = Simulation::new(db, params, &scenario);
    println!(
        "Econoplex: {} towns, {} agents, seed {}, {} days per year",
        scenario.towns.len(),
        sim.world.resource::<sim_core::AgentStore>().alive_count(),
        scenario.seed,
        dpy
    );

    let start = Instant::now();
    let mut shown = 0;
    for _ in 0..args.years * dpy {
        sim.step();
        let metrics = sim.world.resource::<Metrics>();
        if !args.quiet && metrics.years.len() > shown {
            shown = metrics.years.len();
            print_year(metrics.latest().unwrap());
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    let ticks = args.years * dpy;
    let agents = sim.world.resource::<sim_core::AgentStore>().alive_count();
    println!(
        "\n{} ticks in {:.2} s ({:.2} ms per tick), {} agents alive",
        ticks,
        elapsed,
        elapsed * 1000.0 / ticks.max(1) as f64,
        agents
    );
    print_final(&sim);
    if args.profile {
        print_profile(sim.world.resource::<sim_core::profile::Profile>(), ticks);
    }
    if args.hash {
        println!("state hash: {:016x}", sim.state_hash());
    }
    if let Some(path) = args.csv {
        write_csv(&path, sim.world.resource::<Metrics>())
            .unwrap_or_else(|e| die(&format!("cannot write CSV: {e}")));
        println!("wrote {}", path.display());
    }
}

fn print_year(y: &sim_core::metrics::YearStats) {
    println!(
        "\nYear {:>3}  population {:>6}  step success {:>4.0}%  money {:>9.0}",
        y.year,
        y.population,
        y.success_rate * 100.0,
        y.total_money
    );
    println!(
        "  {:<11} {:>5} {:>6} {:>6} {:>4} {:>6} {:>6} {:>6} {:>7} {:>5}  {:>11} {:>8} {:>6} {:>4} {:>5} {:>9} {:>9} {:>9}  ruler",
        "town",
        "pop",
        "food",
        "starve",
        "div",
        "ECI",
        "spec",
        "top",
        "masters",
        "tools",
        "swords 1/2/3",
        "treasury",
        "wealth",
        "bldg",
        "army",
        "raids w/l",
        "in/out",
        "capt +/-"
    );
    for t in &y.towns {
        println!(
            "  {:<11} {:>5} {:>6.1} {:>5.1}% {:>4} {:>6.2} {:>6.2} {:>6.2} {:>7} {:>5}  {:>3}/{:>3}/{:>3} {:>8.0} {:>6.1} {:>4} {:>5} {:>4}/{:>4} {:>4}/{:>4} {:>4}/{:>4}  {}",
            t.name,
            t.population,
            t.food_per_capita,
            t.starving * 100.0,
            t.diversity,
            t.eci,
            t.specialization,
            t.mean_top_skill,
            t.masters,
            t.tools_in_use,
            t.swords[0],
            t.swords[1],
            t.swords[2],
            t.treasury,
            t.mean_wealth,
            t.buildings,
            t.soldiers,
            t.war.raids_won,
            t.war.raids_lost,
            t.moves.arrived,
            t.moves.left,
            t.war.captives_taken,
            t.war.captives_lost,
            t.ruler.map_or("-", |r| y.towns[r as usize].name.as_str())
        );
    }
}

fn print_final(sim: &Simulation) {
    let db = sim.world.resource::<sim_core::Db>();
    let store = sim.world.resource::<sim_core::AgentStore>();
    let towns = sim.world.resource::<sim_core::Towns>();
    let params = sim.world.resource::<sim_core::world::Params>();
    let dpy = params.life.days_per_year;

    println!("\nWhat each town's adults are best at (share of adults by top skill):");
    for town in &towns.0 {
        let mut counts = vec![0usize; db.content.skills.len()];
        let mut adults = 0;
        for &i in &town.residents {
            let i = i as usize;
            if store.age_years(i, dpy) < params.life.adult_age {
                continue;
            }
            adults += 1;
            let prof = db.skill_profile(&store.cap[i]);
            let best = (0..prof.len())
                .max_by(|&a, &b| prof[a].total_cmp(&prof[b]))
                .unwrap();
            counts[best] += 1;
        }
        let mut v: Vec<(usize, usize)> = counts
            .iter()
            .copied()
            .enumerate()
            .filter(|c| c.1 > 0)
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        let top: Vec<String> = v
            .iter()
            .take(5)
            .map(|(s, c)| {
                format!(
                    "{} {:.0}%",
                    db.content.skills[*s].name,
                    100.0 * *c as f32 / adults.max(1) as f32
                )
            })
            .collect();
        println!("  {:<11} {}", town.name, top.join(", "));
    }

    let log = sim.world.resource::<sim_core::world::WorkLog>();
    println!("\nBuildings (started with + built):");
    for town in &towns.0 {
        let mut kinds: Vec<(u32, usize)> = Vec::new();
        for b in &town.buildings {
            match kinds.iter_mut().find(|k| k.0 == b.0) {
                Some(k) => k.1 += 1,
                None => kinds.push((b.0, 1)),
            }
        }
        let list: Vec<String> = kinds
            .iter()
            .map(|(d, n)| format!("{} {n}", db.content.products[*d as usize].name))
            .collect();
        println!(
            "  {:<11} {} (built {})",
            town.name,
            list.join(", "),
            town.built
        );
    }

    println!("\nLand (sites; opened during the run, plots still free):");
    let mut q = sim
        .world
        .try_query::<&sim_core::components::NaturalResource>()
        .expect("nature sites are registered");
    for town in &towns.0 {
        let mut kinds = vec![0usize; db.content.nature.len()];
        for &e in &town.nature_sites {
            if let Ok(n) = q.get(&sim.world, e) {
                kinds[n.kind as usize] += 1;
            }
        }
        let list: Vec<String> = kinds
            .iter()
            .enumerate()
            .filter(|k| *k.1 > 0)
            .map(|(k, n)| format!("{} {n}", db.content.nature[k].name))
            .collect();
        println!(
            "  {:<11} {} (opened {}, free {})",
            town.name,
            list.join(", "),
            town.cleared,
            town.free_land
        );
    }

    println!("\nJobs over the whole run (times chosen / runs completed):");
    for (r, m) in db.recipes.iter().enumerate() {
        println!("  {:<26} {:>8} / {:>8}", m.name, log.chosen[r], log.runs[r]);
    }

    println!("\nTop producers last year (value at base prices):");
    for town in &towns.0 {
        let mut v: Vec<(usize, f32)> = town
            .produced_last_year
            .iter()
            .enumerate()
            .map(|(p, q)| (p, q * db.base_price[p]))
            .filter(|x| x.1 > 0.0)
            .collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        let top: Vec<String> = v
            .iter()
            .take(6)
            .map(|(p, _)| {
                format!(
                    "{} {:.0}",
                    db.content.products[*p].name, town.produced_last_year[*p]
                )
            })
            .collect();
        println!("  {:<11} {}", town.name, top.join(", "));
    }
}

fn print_profile(p: &sim_core::profile::Profile, ticks: u64) {
    let total: f64 = p.total.iter().map(|d| d.as_secs_f64()).sum();
    println!("\nTime per system (ms per tick, share):");
    for (name, d) in sim_core::profile::SYSTEMS.iter().zip(&p.total) {
        let s = d.as_secs_f64();
        println!(
            "  {:<10} {:>8.2} {:>5.1}%",
            name,
            s * 1000.0 / ticks.max(1) as f64,
            100.0 * s / total.max(1e-12)
        );
    }
}

fn print_embedding(db: &sim_core::Db) {
    let e = &db.embedding;
    println!(
        "Skill embedding: {} skills, {} dimensions, method {:?}, reconstruction error {:.3}\n",
        db.content.skills.len(),
        e.dim,
        e.method,
        e.error
    );
    println!("Nearest neighbors in the product space (cosine similarity):");
    for (i, s) in db.content.skills.iter().enumerate() {
        let nb: Vec<String> = e
            .neighbors(i, 3)
            .iter()
            .map(|(j, sim)| format!("{} {:.2}", db.content.skills[*j].name, sim))
            .collect();
        println!("  {:<16} [{:<11}] {}", s.name, s.domain, nb.join(", "));
    }
}

fn write_csv(path: &std::path::Path, m: &Metrics) -> std::io::Result<()> {
    let mut f = std::fs::File::create(path)?;
    writeln!(
        f,
        "year,town,population,adults,food_per_capita,diversity,rca_products,eci,specialization,mean_top_skill,masters,guild_members,tools_in_use,swords_1,swords_2,swords_3,treasury,mean_wealth,buildings,tradition,faith,liberty,ideology_spread,starving,migrants_in,migrants_out,captives_taken,captives_lost,ruler,conquests,revolts,tribute_paid"
    )?;
    for y in &m.years {
        for t in &y.towns {
            writeln!(
                f,
                "{},{},{},{},{:.3},{},{},{:.3},{:.4},{:.4},{},{},{},{},{},{},{:.1},{:.2},{},{:.4},{:.4},{:.4},{:.4},{:.4},{},{},{},{},{},{},{},{:.1}",
                y.year, t.name, t.population, t.adults, t.food_per_capita, t.diversity, t.rca_products, t.eci, t.specialization,
                t.mean_top_skill, t.masters, t.guild_members, t.tools_in_use, t.swords[0], t.swords[1], t.swords[2], t.treasury, t.mean_wealth, t.buildings,
                t.ideology[0], t.ideology[1], t.ideology[2], t.ideology_spread, t.starving,
                t.moves.arrived, t.moves.left, t.war.captives_taken, t.war.captives_lost,
                t.ruler.map_or("", |r| y.towns[r as usize].name.as_str()), t.war.conquests, t.war.revolts, t.war.tribute_paid
            )?;
        }
    }
    Ok(())
}
