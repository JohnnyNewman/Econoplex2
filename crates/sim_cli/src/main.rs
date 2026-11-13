//! Headless Econoplex runner.
//!
//! ```text
//! econoplex-sim [--years N] [--seed S] [--scale F] [--assets DIR] [--csv FILE]
//!               [--set path=value]... [--sweep path=v1,v2,...]... [--seeds N]
//!               [--order DAY:ORDER]... [--save FILE] [--load FILE]
//!               [--generate N] [--map-svg FILE] [--events FILE] [--history FILE]
//!               [--weather FILE] [--battle LINEUP]
//!               [--embedding] [--hash] [--profile] [--quiet]
//! ```
//!
//! With `--sweep` or `--seeds` above 1 the runner switches to batch mode: every
//! combination of swept values is run on every seed, in parallel, and each run
//! is reduced to one summary row (see `sweep.rs`).
//!
//! `--load` continues a saved game for `--years` more years; its parameters and
//! scenario come from the save, so `--seed`, `--scale` and `--set` do not apply.

mod sweep;
mod techtree;

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
    tech_tree: bool,
    hash: bool,
    quiet: bool,
    profile: bool,
    /// Parameter overrides applied to models.ron, as (dotted.path, value).
    sets: Vec<(String, String)>,
    sweeps: Vec<(String, Vec<String>)>,
    seeds: u64,
    /// Player orders, by the day they are given.
    orders: Vec<(u64, String)>,
    save: Option<PathBuf>,
    load: Option<PathBuf>,
    /// Generate a world of this many towns from the seed.
    generate: Option<u32>,
    /// Write the map as an SVG picture and exit.
    map_svg: Option<PathBuf>,
    /// Write the event log / the monthly history as CSV after the run.
    events: Option<PathBuf>,
    /// Explain a town's problems at the end (the "why" view, as text).
    why: Option<String>,
    history: Option<PathBuf>,
    weather: Option<PathBuf>,
    /// Fight one squad battle with this line-up and exit.
    battle: Option<String>,
}

fn parse_args() -> Args {
    let mut a = Args {
        years: 20,
        seed: None,
        scale: 1.0,
        assets: None,
        csv: None,
        embedding: false,
        tech_tree: false,
        hash: false,
        quiet: false,
        profile: false,
        sets: Vec::new(),
        sweeps: Vec::new(),
        seeds: 1,
        save: None,
        load: None,
        orders: Vec::new(),
        generate: None,
        map_svg: None,
        events: None,
        why: None,
        history: None,
        weather: None,
        battle: None,
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
            "--events" => a.events = Some(PathBuf::from(val())),
            "--why" => a.why = Some(val()),
            "--history" => a.history = Some(PathBuf::from(val())),
            "--weather" => a.weather = Some(PathBuf::from(val())),
            "--battle" => a.battle = Some(val()),
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
            "--order" => {
                let v = val();
                let (day, text) = v.split_once(':').unwrap_or_else(|| {
                    die("--order expects DAY:ORDER, e.g. 0:subsidy Greenvale bread 0.3")
                });
                let day = day
                    .trim()
                    .parse()
                    .unwrap_or_else(|_| die("--order expects DAY:ORDER with a day number"));
                a.orders.push((day, text.trim().to_string()));
            }
            "--save" => a.save = Some(PathBuf::from(val())),
            "--load" => a.load = Some(PathBuf::from(val())),
            "--embedding" => a.embedding = true,
            "--tech-tree" => a.tech_tree = true,
            "--hash" => a.hash = true,
            "--generate" => {
                a.generate = Some(
                    val()
                        .parse()
                        .unwrap_or_else(|_| die("--generate expects a number of towns")),
                )
            }
            "--map-svg" => a.map_svg = Some(PathBuf::from(val())),
            "--quiet" => a.quiet = true,
            "--profile" => a.profile = true,
            "-h" | "--help" => {
                println!(
                    "econoplex-sim: run the Econoplex economy headless\n\n\
                     --years N      simulated years (default 20)\n\
                     --seed S       override the scenario seed\n\
                     --scale F      multiply every town's population and land (e.g. 10 for ~12,000 agents)\n\
                     --generate N   generate a world of N towns from the seed (instead of the scenario's)\n\
                     --map-svg FILE write the map (towns, land, rivers, hills) as an SVG picture and exit\n\
                     --events FILE  write the event log (raids, conquests, buildings, famines, orders) as CSV\n\
                     --history FILE write monthly samples per town as CSV\n\
                     --weather FILE write hour, season and weather per tick and town as CSV\n\
                     --battle L     fight one squad battle and exit (with --seeds N: win rates), e.g.\n\
                                    \"cavalry*20 vs spearmen*30\" or \"swordsmen*45,archers*20 vs walls spearmen*30+5!0.6\"\n\
                                    squad: KIT*COUNT[+BATTLES][@SKILL][!MORALE]; KIT: spearmen swordsmen\n\
                                    archers cavalry levy heavy horse_archers\n\
                     --assets DIR   assets folder (default: discovered)\n\
                     --csv FILE     write per-town yearly metrics as CSV (batch mode: one row per run)\n\
                     --set P=V      override a parameter in models.ron, e.g. military.tribute_share=0.3\n\
                     --sweep P=A,B  run once per value (repeat for a grid); implies batch mode\n\
                     --seeds N      run N consecutive seeds from the scenario seed; implies batch mode\n\
                     --embedding    print the skill embedding (product space) report and exit\n\
                     --tech-tree    print the technology tree as a Mermaid flowchart and exit\n\
                     --hash         print the final state hash (determinism check)\n\
                     --order D:O    give a player order on day D (repeatable), e.g. 0:school Greenvale smithing 20\n\
                                    orders: subsidy TOWN PRODUCT SHARE | school TOWN DOMAIN SEATS | school TOWN off\n\
                                    immigration TOWN open|closed|encouraged | charter TOWN DOMAIN on|off\n\
                                    army TOWN SHARE|auto | war TOWN auto|defend | raid TOWN TARGET\n\
                                    build TOWN BUILDING [X Y] | mint TOWN auto|on|off\n\
                                    tax TOWN wealth|income RATE|auto | wages TOWN SHARE|auto\n\
                                    spending TOWN RATE|auto | procure TOWN PRODUCT AMOUNT\n\
                     --save FILE    save the game at the end of the run (FILE.bin: compact binary; else RON text)
                     --load FILE    continue a saved game (days in --order count from the load)
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

/// Fight a squad battle from a line-up; with several seeds, print win rates instead.
fn run_battle(lineup: &str, params: &sim_core::battle::BattleParams, seed: u64, seeds: u64) {
    use sim_core::battle::{parse_lineup, Battle};
    let (mut att, mut def) = (0, 0);
    for s in seed..seed + seeds.max(1) {
        let (field, specs) = parse_lineup(lineup).unwrap_or_else(|e| die(&e));
        let report = Battle::new(params.clone(), field, specs, s).run();
        match report.winner {
            Some(0) => att += 1,
            Some(_) => def += 1,
            None => {}
        }
        if seeds <= 1 {
            print!("{report}");
        }
    }
    if seeds > 1 {
        println!("{lineup}: attackers won {att}, defenders {def} of {seeds}");
    }
}

fn die(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(2);
}

/// One CSV row per town for the current tick (`--weather`).
fn record_weather(out: &mut impl std::io::Write, sim: &Simulation) {
    let env = sim.world.resource::<sim_core::env::Environment>();
    let towns = &sim.world.resource::<sim_core::Towns>().0;
    for (i, town) in towns.iter().enumerate() {
        let l = env.at(i);
        let w = l.weather;
        writeln!(
            out,
            "{:.4},{:.2},{:.4},{:?},{},{:.2},{:.3},{:.3},{:.2},{:.3},{:.3},{:.3}",
            env.day,
            env.hour,
            env.season,
            env.phase,
            town.name,
            w.temperature,
            w.cloud,
            w.precip,
            w.wind_speed(),
            w.haze,
            l.ground.snow,
            l.ground.wet
        )
        .unwrap_or_else(|e| die(&format!("cannot write weather: {e}")));
    }
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
    if let Some(lineup) = &args.battle {
        run_battle(lineup, &params.battle, args.seed.unwrap_or(1), args.seeds);
        return;
    }
    if args.tech_tree {
        print!("{}", techtree::mermaid(&db));
        return;
    }
    if let Some(s) = args.seed {
        scenario.seed = s;
    }
    if let Some(n) = args.generate {
        scenario.generate = Some(sim_core::worldgen::GenSpec {
            towns: n.max(2),
            ..Default::default()
        });
    }
    if let Some(path) = &args.map_svg {
        let s = sim_core::worldgen::resolve(&scenario, &db);
        let terrain = sim_core::terrain::Terrain::build(&s);
        std::fs::write(path, sim_core::worldgen::map_svg(&s, &terrain))
            .unwrap_or_else(|e| die(&format!("cannot write {}: {e}", path.display())));
        println!("wrote {} ({} towns)", path.display(), s.towns.len());
        return;
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
            orders: args.orders,
        };
        sweep::run(&plan, args.csv.as_deref()).unwrap_or_else(|e| die(&e.to_string()));
        return;
    }

    let mut sim = match &args.load {
        Some(path) => {
            let sim = Simulation::load_save(path, &paths).unwrap_or_else(|e| die(&e.to_string()));
            println!(
                "Loaded {} at day {} (year {})",
                path.display(),
                sim.tick(),
                sim.world.resource::<sim_core::SimClock>().year()
            );
            sim
        }
        None => Simulation::new(db, params, &scenario),
    };
    let scenario = &sim.world.resource::<sim_core::world::ScenarioSpec>().0;
    let dpy = sim
        .world
        .resource::<sim_core::world::Params>()
        .life
        .days_per_year as u64;
    println!(
        "Econoplex: {} towns, {} agents, seed {}, {} days per year",
        scenario.towns.len(),
        sim.world.resource::<sim_core::AgentStore>().alive_count(),
        scenario.seed,
        dpy
    );

    let mut weather_out = args.weather.as_ref().map(|path| {
        let mut f = std::io::BufWriter::new(
            std::fs::File::create(path)
                .unwrap_or_else(|e| die(&format!("cannot write {}: {e}", path.display()))),
        );
        use std::io::Write;
        writeln!(
            f,
            "day,hour,season,phase,town,temperature,cloud,precip,wind,haze,snow,wet"
        )
        .unwrap_or_else(|e| die(&format!("cannot write weather: {e}")));
        f
    });
    let start = Instant::now();
    let mut shown = sim.world.resource::<Metrics>().years.len();
    for tick in 0..args.years * dpy {
        for (_, text) in args.orders.iter().filter(|o| o.0 == tick) {
            sim.order(text).unwrap_or_else(|e| die(&e));
        }
        match weather_out.as_mut() {
            Some(out) => loop {
                sim.step_tick();
                record_weather(out, &sim);
                if sim.world.resource::<sim_core::SimClock>().day_done() {
                    break;
                }
            },
            None => sim.step(),
        }
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
    print_match(&sim);
    print_eras(&sim);
    print_governors(&sim);
    if args.profile {
        print_profile(sim.world.resource::<sim_core::profile::Profile>(), ticks);
    }
    if args.hash {
        println!("state hash: {:016x}", sim.state_hash());
    }
    if let Some(path) = &args.save {
        sim.save(path).unwrap_or_else(|e| die(&e));
        println!("saved {} at day {}", path.display(), sim.tick());
    }
    if let Some(path) = args.csv {
        write_csv(&path, sim.world.resource::<Metrics>())
            .unwrap_or_else(|e| die(&format!("cannot write CSV: {e}")));
        println!("wrote {}", path.display());
    }
    if let (Some(path), Some(mut out)) = (&args.weather, weather_out) {
        use std::io::Write;
        out.flush()
            .unwrap_or_else(|e| die(&format!("cannot write weather: {e}")));
        println!("wrote {}", path.display());
    }
    if let Some(path) = &args.events {
        write_events(path, &sim).unwrap_or_else(|e| die(&format!("cannot write events: {e}")));
        println!("wrote {}", path.display());
    }
    if let Some(path) = &args.history {
        write_history(path, &sim).unwrap_or_else(|e| die(&format!("cannot write history: {e}")));
        println!("wrote {}", path.display());
    }
    if let Some(name) = &args.why {
        let towns = sim.world.resource::<sim_core::Towns>();
        let Some(t) = towns.0.iter().position(|t| &t.name == name) else {
            die(&format!("no town `{name}`"));
        };
        let found = sim_core::explain::explain(&sim.world, t as u16);
        println!("why, {name}:");
        if found.is_empty() {
            println!("  nothing stands out");
        }
        fn show(f: &[sim_core::explain::Finding], depth: usize) {
            for x in f {
                println!("{}- {}", "  ".repeat(depth + 1), x.text);
                show(&x.because, depth + 1);
            }
        }
        show(&found, 0);
    }
}

/// The event log as CSV: day, year, day of year, town, kind, text.
fn write_events(path: &std::path::Path, sim: &Simulation) -> std::io::Result<()> {
    let h = sim.world.resource::<sim_core::history::History>();
    let towns = sim.world.resource::<sim_core::Towns>();
    let dpy = sim.world.resource::<sim_core::SimClock>().days_per_year as u64;
    let mut f = std::fs::File::create(path)?;
    writeln!(f, "day,year,day_of_year,town,kind,text")?;
    for e in &h.events {
        writeln!(
            f,
            "{},{},{},{},{},\"{}\"",
            e.tick,
            e.tick / dpy,
            e.tick % dpy,
            towns
                .0
                .get(e.town as usize)
                .map_or("?", |t| t.name.as_str()),
            e.kind.name(),
            e.text.replace('"', "'")
        )?;
    }
    Ok(())
}

/// The monthly samples as CSV, one row per town and month.
fn write_history(path: &std::path::Path, sim: &Simulation) -> std::io::Result<()> {
    let h = sim.world.resource::<sim_core::history::History>();
    let towns = sim.world.resource::<sim_core::Towns>();
    let name = |t: u16| towns.0.get(t as usize).map_or("?", |t| t.name.as_str());
    let mut f = std::fs::File::create(path)?;
    writeln!(
        f,
        "day,town,population,food_per_capita,treasury,mean_wealth,soldiers,buildings,starving,bread_price,ruler"
    )?;
    for s in &h.samples {
        writeln!(
            f,
            "{},{},{},{:.3},{:.1},{:.2},{},{},{},{:.3},{}",
            s.tick,
            name(s.town),
            s.population,
            s.food_per_capita,
            s.treasury,
            s.mean_wealth,
            s.soldiers,
            s.buildings,
            s.starving,
            s.bread_price,
            s.ruler.map_or("", name)
        )?;
    }
    Ok(())
}

/// What the AI governments did: orders by kind, and each town's latest decisions.
fn print_governors(sim: &Simulation) {
    use sim_core::policy::Order;
    let queue = sim.world.resource::<sim_core::policy::OrderQueue>();
    let towns = sim.world.resource::<sim_core::Towns>();
    let govs = sim.world.resource::<sim_core::governor::Governors>();
    if queue.log.is_empty() {
        return;
    }
    let dpy = sim.world.resource::<sim_core::SimClock>().days_per_year as u64;
    println!(
        "
Government orders (all towns, player's and AI):"
    );
    for (t, town) in towns.0.iter().enumerate() {
        let mut kinds: std::collections::BTreeMap<&str, usize> = Default::default();
        for (_, o) in queue.log.iter().filter(|(_, o)| o.town() as usize == t) {
            let k = match o {
                Order::Subsidy { .. } => "subsidy",
                Order::Tax { .. } => "tax",
                Order::Spending { .. } => "spending",
                Order::Procure { .. } => "procure",
                Order::Loan { .. } => "loan",
                Order::Imports { .. } => "imports",
                Order::Mint { .. } => "mint",
                Order::Wages { .. } => "wages",
                _ => "other",
            };
            *kinds.entry(k).or_default() += 1;
        }
        let list: Vec<String> = kinds.iter().map(|(k, n)| format!("{k} {n}")).collect();
        println!("  {:<11} {}", town.name, list.join(", "));
        if let Some(g) = govs.towns.get(t) {
            for (tick, why) in g.decisions.iter().rev().take(2) {
                println!("      y{} d{:<3} {why}", tick / dpy, tick % dpy);
            }
        }
    }
}

/// The year each town first reached each technology era.
fn print_eras(sim: &Simulation) {
    let db = sim.world.resource::<sim_core::Db>();
    let m = sim.world.resource::<Metrics>();
    let Some(last) = m.latest() else { return };
    if db.content.eras.is_empty() {
        return;
    }
    println!("\nTechnology eras (year first reached):");
    for (t, ts) in last.towns.iter().enumerate() {
        let firsts: Vec<String> = (1..db.content.eras.len())
            .filter_map(|e| {
                m.years
                    .iter()
                    .find(|y| y.towns.get(t).is_some_and(|s| s.era as usize >= e))
                    .map(|y| format!("{} y{}", db.content.eras[e], y.year))
            })
            .collect();
        println!("  {:<11} {}", ts.name, firsts.join(", "));
    }
}

fn print_match(sim: &Simulation) {
    let Some(goal) = &sim.world.resource::<sim_core::world::ScenarioSpec>().0.goal else {
        return;
    };
    let state = sim.world.resource::<sim_core::goal::MatchState>();
    let towns = sim.world.resource::<sim_core::Towns>();
    println!(
        "
Match goal: {} x {} within {} years",
        goal.amount, goal.product, goal.years
    );
    for (t, town) in towns.0.iter().enumerate() {
        println!("  {:<11} {:>6.1}", town.name, state.progress(towns, t));
    }
    match state.result {
        Some(r) => println!(
            "  winner: {} in year {} ({})",
            towns.0[r.winner as usize].name,
            r.year,
            match r.reason {
                sim_core::goal::WinReason::Goal => "reached the goal",
                sim_core::goal::WinReason::TimeUp => "closest when time ran out",
            }
        ),
        None => println!("  no winner yet"),
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
        "  {:<11} {:>5} {:>6} {:>6} {:>4} {:>6} {:>6} {:>6} {:>7} {:>5}  {:>11} {:>8} {:>6} {:>4} {:>5} {:>9} {:>9} {:>9} {:>11}  ruler",
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
        "capt +/-",
        "trust/trade"
    );
    for t in &y.towns {
        println!(
            "  {:<11} {:>5} {:>6.1} {:>5.1}% {:>4} {:>6.2} {:>6.2} {:>6.2} {:>7} {:>5}  {:>3}/{:>3}/{:>3} {:>8.0} {:>6.1} {:>4} {:>5} {:>4}/{:>4} {:>4}/{:>4} {:>4}/{:>4} {:>5.2}/{:>4.0}%  {}",
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
            t.trust,
            t.trade_ties * 100.0,
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
        v.sort_by_key(|a| std::cmp::Reverse(a.1));
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
        "year,town,population,adults,food_per_capita,diversity,rca_products,eci,specialization,mean_top_skill,masters,guild_members,tools_in_use,swords_1,swords_2,swords_3,treasury,mean_wealth,buildings,tradition,faith,liberty,ideology_spread,starving,migrants_in,migrants_out,captives_taken,captives_lost,ruler,conquests,revolts,tribute_paid,trust,trade_ties,issued,taxed,spent,external_in,external_out,minted,era,vol_essential,vol_essential_p95,vol_commodity,vol_commodity_p95,vol_labour,vol_labour_p95,cpi_laspeyres,cpi_fisher,inflation,cpi_essentials,ppi_commodities,wage_index,purchasing_power,real_exchange_rate,money_stock,velocity,minted_share,empire_cpi,firm_cash,firm_sales,firm_direct_sales,firm_inputs,firm_wages,firm_dividends,firm_profit_tax,firm_grants,firm_owner_capital,firm_loans,firm_repayments,firm_interest,loans_outstanding,loan_defaults,trade_credit,credit_defaults,debt_owed,debt_lent"
    )?;
    for y in &m.years {
        for t in &y.towns {
            writeln!(
                f,
                "{},{},{},{},{:.3},{},{},{:.3},{:.4},{:.4},{},{},{},{},{},{},{:.1},{:.2},{},{:.4},{:.4},{:.4},{:.4},{:.4},{},{},{},{},{},{},{},{:.1},{:.4},{:.4},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5},{:.2},{:.2},{:.4},{:.2},{:.2},{:.2},{:.3},{:.4},{:.1},{:.3},{:.4},{:.2},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1}",
                y.year, t.name, t.population, t.adults, t.food_per_capita, t.diversity, t.rca_products, t.eci, t.specialization,
                t.mean_top_skill, t.masters, t.guild_members, t.tools_in_use, t.swords[0], t.swords[1], t.swords[2], t.treasury, t.mean_wealth, t.buildings,
                t.ideology[0], t.ideology[1], t.ideology[2], t.ideology_spread, t.starving,
                t.moves.arrived, t.moves.left, t.war.captives_taken, t.war.captives_lost,
                t.ruler.map_or("", |r| y.towns[r as usize].name.as_str()), t.war.conquests, t.war.revolts, t.war.tribute_paid, t.trust, t.trade_ties,
                t.issued, t.taxed, t.spent, t.external_in, t.external_out, t.minted, t.era,
                t.price_vol[0].0, t.price_vol[0].1, t.price_vol[1].0, t.price_vol[1].1, t.price_vol[2].0, t.price_vol[2].1,
                t.cpi.laspeyres, t.cpi.fisher, t.cpi.inflation, t.cpi.essentials, t.cpi.commodities, t.cpi.labour,
                t.purchasing_power, t.real_exchange_rate, t.money_stock, t.velocity, t.minted_share, t.empire_cpi,
                t.firm_cash, t.firm_flows[0], t.firm_flows[1], t.firm_flows[2], t.firm_flows[3], t.firm_flows[4], t.firm_flows[5], t.firm_flows[6], t.firm_flows[7],
                t.firm_flows[8], t.firm_flows[9], t.firm_flows[10], t.loans_outstanding, t.loan_defaults, t.trade_credit, t.credit_defaults, t.debt_owed, t.debt_lent
            )?;
        }
    }
    Ok(())
}
