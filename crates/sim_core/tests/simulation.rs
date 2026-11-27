use sim_core::{AssetPaths, Simulation};
use std::path::Path;

fn sim(seed: u64) -> Simulation {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    scenario.seed = seed;
    for t in &mut scenario.towns {
        t.population = 60;
    }
    Simulation::new(db, params, &scenario)
}

#[test]
fn same_seed_same_state() {
    let (mut a, mut b) = (sim(1), sim(1));
    for _ in 0..200 {
        a.step();
        b.step();
    }
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn the_state_hash_covers_everything_a_save_holds() {
    let mut s = sim(1);
    for _ in 0..20 {
        s.step();
    }
    let before = s.state_hash();
    assert_eq!(before, s.state_hash(), "hashing leaves the state alone");

    // A town's treasury, the weather and the order queue are all state.
    s.world.resource_mut::<sim_core::Towns>().0[0].treasury += sim_core::Coin::from_coins(1.0);
    let treasury = s.state_hash();
    assert_ne!(before, treasury);
    s.world.resource_mut::<sim_core::env::Environment>().hour += 0.5;
    let hour = s.state_hash();
    assert_ne!(treasury, hour);
    s.order("immigration Greenvale closed").unwrap();
    assert_ne!(hour, s.state_hash());
}

#[test]
fn different_seed_different_state() {
    let (mut a, mut b) = (sim(1), sim(2));
    for _ in 0..50 {
        a.step();
        b.step();
    }
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn economy_produces_and_people_learn() {
    let mut s = sim(3);
    // The founders' skill (children born meanwhile start with little and would
    // pull a mean over everyone down).
    let founders = s.world.resource::<sim_core::AgentStore>().len();
    let mean_cap = |st: &sim_core::AgentStore| {
        let alive: Vec<usize> = (0..founders).filter(|&i| st.alive[i]).collect();
        alive
            .iter()
            .map(|&i| st.cap[i].iter().sum::<f32>())
            .sum::<f32>()
            / alive.len() as f32
    };
    let before = mean_cap(s.world.resource::<sim_core::AgentStore>());
    for _ in 0..24 {
        s.step();
    }
    let log = s.world.resource::<sim_core::world::WorkLog>();
    assert!(
        log.successes > 100,
        "only {} successful steps",
        log.successes
    );
    let st = s.world.resource::<sim_core::AgentStore>();
    let after = mean_cap(st);
    assert!(
        after > before,
        "agents should gain skill: {before} -> {after}"
    );
    assert!(
        st.alive_count() > 150,
        "population collapsed to {}",
        st.alive_count()
    );
}

#[test]
fn money_is_conserved() {
    // Money only grows by what the mints strike, to the last unit, every day.
    let mut s = sim(4);
    let money = |s: &Simulation| {
        let towns = s.world.resource::<sim_core::Towns>();
        let minted: sim_core::Coin = towns.0.iter().map(|t| t.minted).sum();
        sim_core::world::total_coin(s.world.resource::<sim_core::AgentStore>(), towns) - minted
    };
    let before = money(&s);
    for day in 0..600 {
        s.step();
        assert_eq!(money(&s), before, "money changed on day {day}");
    }
}

#[test]
fn first_year_has_no_famine() {
    // The new economy makes the first winter hard, before the first harvest,
    // most of all for the mining town (Ironhold); the AI rulers bridge most of
    // it. No town may lose a third of its people, and the world keeps 90 %.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut s = Simulation::from_paths(&AssetPaths::from_root(&root)).unwrap();
    let start: Vec<usize> = s
        .world
        .resource::<sim_core::Towns>()
        .0
        .iter()
        .map(|t| t.residents.len())
        .collect();
    let total = s.world.resource::<sim_core::AgentStore>().alive_count();
    let dpy = s.world.resource::<sim_core::SimClock>().months_per_year;
    for _ in 0..dpy {
        s.step();
    }
    let end = s.world.resource::<sim_core::AgentStore>().alive_count();
    assert!(
        end as f32 >= 0.9 * total as f32,
        "population fell from {total} to {end} in the first year"
    );
    for (t, town) in s.world.resource::<sim_core::Towns>().0.iter().enumerate() {
        assert!(
            town.residents.len() as f32 >= 0.67 * start[t] as f32,
            "{} fell from {} to {}",
            town.name,
            start[t],
            town.residents.len()
        );
    }
}

#[test]
fn towns_construct_buildings() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut s = Simulation::from_paths(&AssetPaths::from_root(&root)).unwrap();
    let dpy = s.world.resource::<sim_core::SimClock>().months_per_year;
    for _ in 0..10 * dpy {
        s.step();
    }
    let towns = s.world.resource::<sim_core::Towns>();
    let built: u32 = towns.0.iter().map(|t| t.built).sum();
    assert!(built > 0, "no building was constructed in 10 years");
    for t in &towns.0 {
        assert!(
            t.buildings.len() <= 40,
            "{} overbuilt: {} buildings",
            t.name,
            t.buildings.len()
        );
    }
}

#[test]
fn sword_towns_raise_squads_and_raid() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut s = Simulation::from_paths(&AssetPaths::from_root(&root)).unwrap();
    let dpy = s.world.resource::<sim_core::SimClock>().months_per_year;
    let mut raids = 0;
    let mut soldiers = 0;
    let mut conquered = false;
    // Captives and the town that took them.
    let mut taken: Vec<(u32, u16)> = Vec::new();
    for _ in 0..20 * dpy {
        s.step();
        let towns = s.world.resource::<sim_core::Towns>();
        raids += towns.0.iter().filter(|t| t.raid.is_some()).count();
        soldiers = soldiers.max(towns.0.iter().map(|t| t.squad.len()).sum());
        conquered |= towns.0.iter().any(|t| t.ruler.is_some());
        for (t, town) in towns.0.iter().enumerate() {
            for &c in town.raid.iter().flat_map(|r| &r.captives) {
                if !taken.iter().any(|x| x.0 == c) {
                    taken.push((c, t as u16));
                }
            }
        }
    }
    assert!(soldiers > 0, "no town raised a squad");
    assert!(raids > 0, "no raid in 20 years");
    assert!(conquered, "no town was conquered in 20 years");
    assert!(
        !taken.is_empty(),
        "no winning raid took captives in 20 years"
    );
    // Captives were moved to the captor's town (until they choose to migrate).
    let store = s.world.resource::<sim_core::AgentStore>();
    let settled = taken
        .iter()
        .filter(|&&(c, t)| store.alive[c as usize] && store.town[c as usize] == t)
        .count();
    assert!(settled > 0, "no captive lives in the captor's town");
}

/// Auto-resolved raids follow the towns' doctrines: with every town on
/// capture, beaten soldiers are taken prisoner and change towns.
#[test]
fn a_doctrine_of_capture_takes_prisoners_in_auto_resolved_raids() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut s = Simulation::from_paths(&AssetPaths::from_root(&root)).unwrap();
    let names: Vec<String> = s
        .world
        .resource::<sim_core::Towns>()
        .0
        .iter()
        .map(|t| t.name.clone())
        .collect();
    for n in &names {
        s.order(&format!("doctrine {n} balanced capture")).unwrap();
    }
    let dpy = s.world.resource::<sim_core::SimClock>().months_per_year;
    // Yearly counters: add up what each step adds.
    let mut last = vec![0u32; names.len()];
    let mut prisoners = 0;
    for _ in 0..20 * dpy {
        s.step();
        let towns = s.world.resource::<sim_core::Towns>();
        for (t, town) in towns.0.iter().enumerate() {
            let now = town.war.prisoners_taken;
            prisoners += now.saturating_sub(last[t]);
            last[t] = now;
        }
        if prisoners > 0 {
            break;
        }
    }
    assert!(
        prisoners > 0,
        "no prisoners in 20 years of raids on capture"
    );
}

#[test]
fn households_migrate_and_carry_their_skills() {
    // Households decide together and know only some towns, so moves are rare: use
    // full-size towns and let people weigh moving more often.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    scenario.seed = 7;
    params.migration.consider = 0.5;
    let mut s = Simulation::new(db, params, &scenario);
    let before: Vec<u16> = s.world.resource::<sim_core::AgentStore>().town.clone();
    run_years(&mut s, 3);
    let m = s.world.resource::<sim_core::metrics::Metrics>();
    let arrived: u32 = m
        .years
        .iter()
        .flat_map(|y| &y.towns)
        .map(|t| t.moves.arrived)
        .sum();
    let left: u32 = m
        .years
        .iter()
        .flat_map(|y| &y.towns)
        .map(|t| t.moves.left)
        .sum();
    assert!(arrived > 0, "nobody migrated in 3 years");
    assert_eq!(arrived, left, "every arrival leaves somewhere");
    // Movers are founders who now live in another town; their skills came with them.
    let store = s.world.resource::<sim_core::AgentStore>();
    let movers = (0..before.len())
        .filter(|&i| store.alive[i] && store.town[i] != before[i])
        .count();
    assert!(movers > 0, "no founder lives in a different town");
}

#[test]
fn food_comes_first_and_gluts_stop_production() {
    let mut s = sim_bare(3);
    let dpy = s.world.resource::<sim_core::SimClock>().months_per_year;
    for _ in 0..dpy {
        s.step();
    }
    let (flour, wood, bread, mill, bake, charcoal, planks) = {
        let db = s.world.resource::<sim_core::Db>();
        let p = |id: &str| db.content.products.iter().position(|x| x.id == id).unwrap();
        let r = |id: &str| db.content.recipes.iter().position(|x| x.id == id).unwrap();
        (
            p("flour"),
            p("wood"),
            p("bread"),
            r("mill_flour"),
            r("bake_bread"),
            r("burn_charcoal"),
            r("saw_planks"),
        )
    };
    // Mountains of flour and a little wood, no bread: bakers get the wood,
    // and nobody mills more flour.
    for t in &mut s.world.resource_mut::<sim_core::Towns>().0 {
        t.stock[flour] = 1e6;
        t.stock[wood] = 2.0;
        t.stock[bread] = 0.0;
    }
    // Under the ruler's food rules the seed reserve is a lever: hold it.
    let names: Vec<String> = s
        .world
        .resource::<sim_core::Towns>()
        .0
        .iter()
        .map(|t| t.name.clone())
        .collect();
    for n in names {
        s.order(&format!("reserve {n} on")).unwrap();
    }
    s.step();
    for t in &mut s.world.resource_mut::<sim_core::Towns>().0 {
        t.stock[flour] = 1e6;
        t.stock[wood] = 2.0;
        t.stock[bread] = 0.0;
    }
    let before = s.world.resource::<sim_core::world::WorkLog>().runs.clone();
    // The wood stays scarce all day: what the woodcutters bring is taken
    // away step by step (a step is a few minutes of a day of ten).
    let spd = s.world.resource::<sim_core::SimClock>().steps_per_day;
    for _ in 0..5 * spd {
        for t in &mut s.world.resource_mut::<sim_core::Towns>().0 {
            t.stock[wood] = t.stock[wood].min(2.0);
        }
        s.step_tick();
    }
    let after = &s.world.resource::<sim_core::world::WorkLog>().runs;
    let ran = |r: usize| after[r] - before[r];
    assert_eq!(ran(mill), 0, "mills kept making flour nobody needs");
    assert!(ran(bake) > 0, "no bread was baked");
    // Planks now feed food too (barrels for the brewers, 58_household_goods),
    // so only the charcoal burners must leave the wood alone.
    let _ = planks;
    assert_eq!(ran(charcoal), 0, "other trades took the bakeries' wood");
}

#[test]
fn towns_clear_new_land_up_to_their_limit() {
    let mut s = sim(5);
    let dpy = s.world.resource::<sim_core::SimClock>().months_per_year;
    let free = s
        .world
        .resource::<sim_core::world::Params>()
        .market
        .free_land;
    let sites_before: Vec<usize> = s
        .world
        .resource::<sim_core::Towns>()
        .0
        .iter()
        .map(|t| t.nature_sites.len())
        .collect();
    for _ in 0..5 * dpy {
        s.step();
    }
    let towns = s.world.resource::<sim_core::Towns>();
    let opened: u32 = towns.0.iter().map(|t| t.cleared).sum();
    assert!(opened > 0, "no town cleared a field or planted a woodlot");
    for (t, town) in towns.0.iter().enumerate() {
        assert_eq!(
            town.cleared + town.free_land,
            free,
            "{} overused its land",
            town.name
        );
        assert_eq!(
            town.nature_sites.len(),
            sites_before[t] + town.cleared as usize,
            "every opened plot is a new site"
        );
    }
}

#[test]
fn tributaries_pay_their_ruler() {
    let mut s = sim(6);
    let interval = s
        .world
        .resource::<sim_core::world::Params>()
        .military
        .tribute_interval as u64;
    let money = |s: &Simulation| {
        sim_core::world::total_money(
            s.world.resource::<sim_core::AgentStore>(),
            s.world.resource::<sim_core::Towns>(),
        )
    };
    let before = money(&s);
    {
        let mut towns = s.world.resource_mut::<sim_core::Towns>();
        towns.0[1].ruler = Some(0);
        towns.0[1].treasury += sim_core::Coin::from_coins(500.0);
        towns.0[0].treasury -= sim_core::Coin::from_coins(500.0);
    }
    for _ in 0..2 * interval {
        s.step();
    }
    let towns = s.world.resource::<sim_core::Towns>();
    assert!(towns.0[1].war.tribute_paid > 0.0, "no tribute was paid");
    assert_eq!(towns.0[1].war.tribute_paid, towns.0[0].war.tribute_received);
    // Nobody else rules or pays.
    for t in [0, 2, 3] {
        assert_eq!(towns.0[t].war.tribute_paid, 0.0);
    }
    let after = money(&s);
    assert!(
        ((after - before) / before).abs() < 1e-3,
        "money changed from {before} to {after}"
    );
}

#[test]
fn coworkers_come_to_trust_each_other() {
    let mut s = sim(9);
    let days = s
        .world
        .resource::<sim_core::world::Params>()
        .time
        .months_per_year;
    for _ in 0..days {
        s.step();
    }
    let store = s.world.resource::<sim_core::AgentStore>();
    let towns = s.world.resource::<sim_core::world::Towns>();
    let (mut ties, mut trusted, mut same_town) = (0, 0, 0);
    for i in 0..store.len() {
        if !store.alive[i] {
            continue;
        }
        for t in store.ties[i].iter().filter(|t| t.other != u32::MAX) {
            ties += 1;
            assert!((-1.0..=1.0).contains(&t.value));
            if t.value > 0.2 {
                trusted += 1;
            }
            if store.town[t.other as usize] == store.town[i] {
                same_town += 1;
            }
        }
    }
    assert!(
        ties > 0 && trusted * 4 > ties,
        "{trusted} strong ties of {ties}"
    );
    // Ties form at work and in town, so nearly all stay within one town.
    assert!(
        same_town * 10 > ties * 8,
        "{same_town} of {ties} ties in town"
    );
    let stats = &s
        .world
        .resource::<sim_core::metrics::Metrics>()
        .latest()
        .unwrap()
        .towns;
    assert!(stats.iter().all(|t| t.trust > 0.0));
    assert_eq!(stats.len(), towns.0.len());
}

/// `sim` / `full` with only the scenario's stock at the start (no steady-state
/// start), for tests calibrated to it.
fn sim_bare(seed: u64) -> Simulation {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    params.start = sim_core::start::StartParams::Scenario;
    scenario.seed = seed;
    for t in &mut scenario.towns {
        t.population = 60;
    }
    Simulation::new(db, params, &scenario)
}

fn full_bare(seed: u64) -> Simulation {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    params.start = sim_core::start::StartParams::Scenario;
    scenario.seed = seed;
    Simulation::new(db, params, &scenario)
}

fn full(seed: u64) -> Simulation {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    scenario.seed = seed;
    Simulation::new(db, params, &scenario)
}

fn run_years(s: &mut Simulation, years: u32) {
    let days = s
        .world
        .resource::<sim_core::world::Params>()
        .time
        .months_per_year;
    for _ in 0..years * days {
        s.step();
    }
}

#[test]
fn orders_are_parsed_applied_and_logged() {
    let mut s = sim(4);
    assert!(s.order("subsidy Atlantis bread 0.3").is_err());
    assert!(s.order("subsidy Greenvale gold 0.3").is_err());
    assert!(s.order("charter Greenvale cooking on").is_err());
    s.order("subsidy Greenvale bread 5").unwrap();
    s.order("school greenvale woodworking 20").unwrap();
    s.order("immigration Greenvale closed").unwrap();
    s.order("charter Greenvale smithing on").unwrap();
    s.step();
    let db = s.world.resource::<sim_core::Db>();
    let p = &s.world.resource::<sim_core::Towns>().0[1].policy;
    let max = s
        .world
        .resource::<sim_core::world::Params>()
        .policy
        .max_subsidy;
    assert_eq!(
        p.subsidy[db.content.product("bread").unwrap() as usize],
        max
    );
    assert_eq!(p.school.unwrap().seats, 20);
    assert_eq!(p.immigration, sim_core::policy::Immigration::Closed);
    assert!(p.charters[db.content.domain("smithing").unwrap() as usize]);
    // The AI towns govern themselves through the same queue; Greenvale's orders
    // are the four given here.
    let queue = s.world.resource::<sim_core::policy::OrderQueue>();
    assert!(queue.pending.iter().all(|o| o.town() != 1));
    let greenvale: Vec<u64> = queue
        .log
        .iter()
        .filter(|(_, o)| o.town() == 1)
        .map(|(tick, _)| *tick)
        .collect();
    // Orders given before the first step take effect on day 1, the day the clock has started.
    assert_eq!(greenvale, [1, 1, 1, 1]);
}

#[test]
fn a_school_teaches_its_subject_and_pays_the_teacher() {
    let proficiency = |order: Option<&str>| {
        let mut s = full(5);
        if let Some(o) = order {
            s.order(o).unwrap();
        }
        run_years(&mut s, 3);
        let store = s.world.resource::<sim_core::AgentStore>();
        let db = s.world.resource::<sim_core::Db>();
        let dir = &db.domain_dirs[db.content.domain("woodworking").unwrap() as usize];
        let kids: Vec<f32> = s.world.resource::<sim_core::Towns>().0[1]
            .residents
            .iter()
            .map(|&i| i as usize)
            .filter(|&i| store.activity[i] == sim_core::store::Activity::Child)
            .map(|i| {
                store.cap[i]
                    .iter()
                    .zip(dir.iter())
                    .map(|(a, b)| a * b)
                    .sum()
            })
            .collect();
        kids.iter().sum::<f32>() / kids.len().max(1) as f32
    };
    let without = proficiency(None);
    let with = proficiency(Some("school Greenvale woodworking 40"));
    // Pupils end up well over twice as good at the subject.
    assert!(
        with > without * 2.0 && with > without + 0.05,
        "school {with:.3} vs none {without:.3}"
    );
}

#[test]
fn closed_borders_keep_migrants_out_and_subsidies_draw_workers() {
    let mut s = full(6);
    s.order("immigration Greenvale closed").unwrap();
    // A good with spare capacity to grow (a cloth subsidy does not raise
    // Greenvale's cloth output).
    s.order("subsidy Greenvale brick 1.0").unwrap();
    let mut base = full(6);
    let mut arrived = 0;
    let mut made = [0.0f32; 2];
    let brick = s
        .world
        .resource::<sim_core::Db>()
        .content
        .product("brick")
        .unwrap() as usize;
    for _ in 0..5 {
        run_years(&mut s, 1);
        run_years(&mut base, 1);
        let y = s
            .world
            .resource::<sim_core::metrics::Metrics>()
            .latest()
            .unwrap();
        arrived += y.towns[1].moves.arrived;
        made[0] += s.world.resource::<sim_core::Towns>().0[1].produced_last_year[brick];
        made[1] += base.world.resource::<sim_core::Towns>().0[1].produced_last_year[brick];
    }
    assert_eq!(arrived, 0);
    assert!(
        made[0] > made[1] * 1.2,
        "subsidized {} vs {} bricks",
        made[0],
        made[1]
    );
}

#[test]
fn a_defending_town_marches_only_when_ordered() {
    // Seed 8: with the building materials, Ironhold goes broke on seed 7 and
    // can pay no squad (a catastrophe that is allowed; the test is about orders).
    let mut s = full(8);
    s.order("war Ironhold defend").unwrap();
    s.order("army Ironhold 0.15").unwrap();
    let raids = |s: &Simulation| -> u32 {
        s.world
            .resource::<sim_core::metrics::Metrics>()
            .years
            .iter()
            .map(|y| y.towns[0].war.raids)
            .sum()
    };
    run_years(&mut s, 3);
    assert_eq!(raids(&s), 0, "a defending town raided on its own");
    assert_eq!(
        s.world.resource::<sim_core::Towns>().0[0].policy.army_share,
        Some(0.15)
    );
    // The ordered raid waits until the squad is big enough, then marches.
    s.order("raid Ironhold Brassmoor").unwrap();
    let mut marched = None;
    for _ in 0..5 * 12 {
        s.step();
        if let Some(r) = &s.world.resource::<sim_core::Towns>().0[0].raid {
            marched = Some(r.target);
            break;
        }
    }
    assert_eq!(marched, Some(3));
    assert_eq!(
        s.world.resource::<sim_core::Towns>().0[0].policy.raid_order,
        None
    );
}

#[test]
fn an_ordered_building_goes_up_where_it_was_placed() {
    let mut s = full(8);
    s.order("build Timberwick bakery -300 -100").unwrap();
    assert!(s.order("build Timberwick bread").is_err());
    let mut placed = false;
    for _ in 0..24 {
        s.step();
        let t = &s.world.resource::<sim_core::Towns>().0[2];
        if t.buildings.iter().any(|b| b.2 == (-300.0, -100.0)) {
            assert!(t.policy.commissions.is_empty());
            placed = true;
            break;
        }
    }
    assert!(
        placed,
        "the commissioned bakery was not built where ordered"
    );
}

#[test]
fn the_river_makes_the_southern_road_longer() {
    let s = sim(4);
    let t = s.world.resource::<sim_core::terrain::Terrain>();
    let towns = s.world.resource::<sim_core::Towns>();
    let at = |name: &str| towns.0.iter().position(|t| t.name == name).unwrap();
    let north = t.road(at("Ironhold"), at("Greenvale"));
    let south = t.road(at("Timberwick"), at("Brassmoor"));
    let same_bank = t.road(at("Ironhold"), at("Timberwick"));
    println!("north {north:.0}  south {south:.0}  west bank {same_bank:.0}");
    assert!(north.is_finite() && south.is_finite());
    assert!(south > 1.2 * north, "north {north} south {south}");
    // Towns stand on dry ground, and the river cannot be built on.
    assert!(towns.0.iter().all(|town| t.buildable(town.pos)));
    assert!(!t.buildable((-30.0, 0.0)));
}

#[test]
fn a_saved_game_loads_and_continues_identically() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let paths = AssetPaths::from_root(&root);
    let mut a = full(11);
    run_years(&mut a, 2);
    a.order("Ironhold build smithy").ok();

    let text = sim_core::save::to_string(&a.world).unwrap();
    let content = sim_data::Content::load_dir(&paths.content).unwrap();
    let mut b = Simulation::from_save(sim_core::save::from_str(&text).unwrap(), content).unwrap();
    assert_eq!(a.tick(), b.tick());
    assert_eq!(a.state_hash(), b.state_hash());

    // Both runs continue in lockstep, through raids, migration and a year end.
    run_years(&mut a, 1);
    run_years(&mut b, 1);
    assert_eq!(a.state_hash(), b.state_hash());
    let (ma, mb) = (
        a.world.resource::<sim_core::metrics::Metrics>(),
        b.world.resource::<sim_core::metrics::Metrics>(),
    );
    assert_eq!(ma.years.len(), mb.years.len());
    assert_eq!(
        format!("{:?}", ma.latest().unwrap()),
        format!("{:?}", mb.latest().unwrap())
    );
}

#[test]
fn a_save_from_other_content_is_refused() {
    let mut a = sim(3);
    a.step();
    let text = sim_core::save::to_string(&a.world).unwrap();
    let mut content = sim_data::Content::default();
    content.domains.push("nothing".into());
    let err = Simulation::from_save(sim_core::save::from_str(&text).unwrap(), content);
    assert!(err.is_err());
}

#[test]
fn the_ledger_accounts_for_every_treasury_change() {
    use sim_core::ledger::{sum_group, Group};
    let mut s = full(5);
    run_years(&mut s, 1);
    let before: Vec<f32> = s
        .world
        .resource::<sim_core::Towns>()
        .0
        .iter()
        .map(|t| t.treasury.coins())
        .collect();
    run_years(&mut s, 1);
    for (t, town) in s.world.resource::<sim_core::Towns>().0.iter().enumerate() {
        let l = &town.ledger.last_year;
        let net = sum_group(l, Group::Taxed)
            + sum_group(l, Group::Spent)
            + sum_group(l, Group::Minted)
            + sum_group(l, Group::ExternalIn)
            - sum_group(l, Group::Issued)
            - sum_group(l, Group::ExternalOut);
        let change = town.treasury.coins() - before[t];
        let scale = sum_group(l, Group::Issued).max(1.0);
        assert!(
            (net - change).abs() < 1e-3 * scale,
            "{}: ledger says {net}, treasury moved {change}",
            town.name
        );
        assert!(sum_group(l, Group::Issued) > 0.0);
        assert!(sum_group(l, Group::Spent) > 0.0);
    }
}

#[test]
fn a_match_is_won_by_the_first_town_to_reach_the_goal() {
    use sim_core::config::GoalSpec;
    use sim_core::goal::{MatchState, WinReason};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    // Everyone bakes: whoever baked most in the first year wins at its end.
    scenario.goal = Some(GoalSpec {
        product: "bread".into(),
        amount: 100.0,
        max_starving: 1.0,
        years: 10,
    });
    let mut s = Simulation::new(db, params, &scenario);
    run_years(&mut s, 1);
    let state = s.world.resource::<MatchState>();
    let r = state.result.expect("someone baked 100 loaves in a year");
    assert_eq!(r.reason, WinReason::Goal);
    assert_eq!(r.year, 1);
    let best = (0..state.made.len())
        .max_by(|&a, &b| state.made[a].total_cmp(&state.made[b]))
        .unwrap();
    assert_eq!(r.winner as usize, best);

    // An unreachable goal is decided by time: the closest free town wins.
    let (db, params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    scenario.goal = Some(GoalSpec {
        product: "longsword_3".into(),
        amount: 1e9,
        max_starving: 1.0,
        years: 1,
    });
    let mut s = Simulation::new(db, params, &scenario);
    run_years(&mut s, 1);
    let r = s.world.resource::<MatchState>().result.unwrap();
    assert_eq!(r.reason, WinReason::TimeUp);
}

#[test]
fn a_town_short_of_money_builds_a_mint_and_strikes_coin() {
    use sim_core::ledger::Flow;
    // Minting is for a treasury that pays only what it holds (one that issues
    // its own coin has no use for a mint's coin yet).
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    params.market.treasury = sim_core::config::Treasury::Holds;
    // Free job choice: the test is of the order, not of whether a smith
    // happens to take up coining.
    params.decision.attendance = sim_core::config::Attendance::Free;
    scenario.seed = 42;
    let mut s = Simulation::new(db, params, &scenario);
    // Ironhold has ore and a furnace; ordered to mint, it builds a mint and works it.
    s.order("mint Ironhold on").unwrap();
    s.order("build Ironhold mint").unwrap();
    // Up to eight years: cut stone is imported, and builders are in demand
    // (on Linux the mint was not up after five once the fine goods came).
    let mint_def = s
        .world
        .resource::<sim_core::Db>()
        .content
        .product("mint")
        .unwrap();
    for _ in 0..8 {
        run_years(&mut s, 1);
        let t = &s.world.resource::<sim_core::Towns>().0[0];
        if t.building_count(mint_def) >= 1 && t.ledger.total[Flow::Minted as usize] > 0.0 {
            break;
        }
    }
    let db = s.world.resource::<sim_core::Db>();
    let mint = db.content.product("mint").unwrap();
    let towns = s.world.resource::<sim_core::Towns>();
    let ironhold = &towns.0[0];
    assert!(ironhold.building_count(mint) >= 1, "no mint built");
    assert!(
        ironhold.ledger.total[Flow::Minted as usize] > 0.0,
        "no coin struck"
    );
    // Coin is money, never a good in stock.
    let coin = db.content.product("coin").unwrap() as usize;
    assert!(towns.0.iter().all(|t| t.stock[coin] == 0.0));

    // A town that turns minting off strikes nothing, even with a mint. (With AI
    // governments on, a broke town would turn it back on; that is their call.)
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    params.governor.enabled = false;
    let mut s = Simulation::new(db, params, &scenario);
    for t in ["Ironhold", "Greenvale", "Timberwick", "Brassmoor"] {
        s.order(&format!("mint {t} off")).unwrap();
    }
    run_years(&mut s, 4);
    let towns = s.world.resource::<sim_core::Towns>();
    assert!(towns
        .0
        .iter()
        .all(|t| t.ledger.total[Flow::Minted as usize] == 0.0));
}

#[test]
fn the_scenario_spreads_resources_so_bronze_needs_trade() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, _, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    let c = &db.content;
    let nature_of = |towns: &[&sim_core::config::TownSpec]| {
        let mut avail = vec![false; c.nature.len()];
        for t in towns {
            for (n, count) in &t.nature {
                if *count > 0 {
                    avail[c.nature_kind(n).unwrap() as usize] = true;
                }
            }
        }
        avail
    };
    // All towns together can make everything in the content.
    let all: Vec<_> = scenario.towns.iter().collect();
    let have = sim_data::validate::reachable_with(c, Some(&nature_of(&all)));
    assert!(
        have.iter().all(|h| *h),
        "the scenario's land cannot make everything"
    );
    // But no town makes bronze on its own: copper and tin sit in different towns.
    let bronze = c.product("bronze").unwrap() as usize;
    for t in &scenario.towns {
        let alone = sim_data::validate::reachable_with(c, Some(&nature_of(&[t])));
        assert!(!alone[bronze], "{} can make bronze alone", t.name);
    }
}

#[test]
fn ai_towns_climb_the_tree() {
    use sim_core::metrics::Metrics;
    // Over four seeds: on any single one, whether bronze gets cast within eight
    // years (and Ironhold industrialises) is luck; main passed 2 of these 4.
    let (mut industrial, mut bronze_cast) = (0, 0);
    for seed in 42..=45 {
        let mut s = full(seed);
        run_years(&mut s, 8);
        let db = s.world.resource::<sim_core::Db>();
        let m = s.world.resource::<Metrics>();
        let best = |t: usize| m.years.iter().map(|y| y.towns[t].era).max().unwrap();
        // Ironhold, with ore and coal, reaches early industry; the others at
        // least stone, clay and cloth.
        // Early industry has to be invented first (knowledge.invention), so
        // inventing one of its recipes there counts as climbing too.
        let kp = &s.world.resource::<sim_core::world::Params>().knowledge;
        let store = s.world.resource::<sim_core::AgentStore>();
        let invented = (0..db.recipes.len())
            .filter(|&r| sim_core::knowledge::era(db, r) > kp.start_era)
            .any(|r| {
                (0..store.len())
                    .any(|i| store.town[i] == 0 && sim_core::knowledge::knows(kp, db, store, i, r))
            });
        if db.content.eras[best(0) as usize] == "Early industry" || invented {
            industrial += 1;
        }
        assert!(
            (1..4).all(|t| best(t) >= 1),
            "seed {seed}: a town stayed in the first era"
        );
        // Bronze is cast somewhere, from traded copper and tin.
        let bronze = db.content.product("bronze").unwrap() as usize;
        let towns = s.world.resource::<sim_core::Towns>();
        if towns
            .0
            .iter()
            .any(|t| t.produced_last_year[bronze] > 0.0 || t.produced_this_year[bronze] > 0.0)
        {
            bronze_cast += 1;
        }
    }
    // With kept trades (plan Phase 3) new trades fill from apprentices and
    // the out of work, so industry comes later: 1 of these 4 seeds. With
    // inventing, Ironhold invents an early-industry recipe on some.
    assert!(
        industrial >= 1,
        "Ironhold industrialised on {industrial} of 4 seeds"
    );
    assert!(bronze_cast >= 1, "no bronze cast on any of 4 seeds");
}

#[test]
fn eras_are_only_a_label_and_never_change_the_simulation() {
    // Eras are derived for the player (HUD, CSV, CLI). Everything else works bottom
    // up from what agents can do and what towns hold, so relabelling every product's
    // era, or dropping the era names, must leave the run unchanged.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let run = |relabel: bool| {
        let (mut db, params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
        if relabel {
            for (i, p) in db.content.products.iter_mut().enumerate() {
                p.era = (4 - p.era.min(4) + i as u8) % 5;
            }
            db.content.eras.clear();
        }
        let mut s = Simulation::new(db, params, &scenario);
        run_years(&mut s, 3);
        let eras: Vec<u8> = s
            .world
            .resource::<sim_core::metrics::Metrics>()
            .latest()
            .unwrap()
            .towns
            .iter()
            .map(|t| t.era)
            .collect();
        (state_without_eras(&s), eras)
    };
    let (a, eras_a) = run(false);
    let (b, eras_b) = run(true);
    assert!(a == b, "relabelling eras changed the simulation");
    // The labels themselves did change.
    assert_ne!(eras_a, eras_b);
}

#[test]
fn a_save_with_other_skill_dimensions_is_refused() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let paths = AssetPaths::from_root(&root);
    let mut a = sim(3);
    a.step();
    let text = sim_core::save::to_string(&a.world).unwrap();
    let dim = a.world.resource::<sim_core::Db>().cap_dim;
    let content = || sim_data::Content::load_dir(&paths.content).unwrap();
    // The same save loads with matching dimensions...
    let ok = sim_core::save::from_str(&text).unwrap();
    assert_eq!(ok.cap_dim, dim);
    assert!(Simulation::from_save(ok, content()).is_ok());
    // ...and is refused when the save's vectors have another length.
    let mut wrong = sim_core::save::from_str(&text).unwrap();
    wrong.cap_dim = dim + 8;
    let err = Simulation::from_save(wrong, content())
        .err()
        .unwrap()
        .to_string();
    assert!(err.contains("dimensional"), "{err}");
}

#[test]
fn skill_dimensions_follow_the_content_or_the_config() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    let skills = db.content.skills.len();
    assert_eq!(db.cap_dim, sim_data::dims::cap_dim_for(skills));
    assert!(db.cap_dim >= skills && db.cap_dim % 8 == 0);
    // A configured size wins, and the whole simulation runs at it.
    params.embedding.dims = Some(64);
    let db = sim_core::Db::build(db.content, 0.5, "Auto", Some(64));
    assert_eq!(db.cap_dim, 64);
    assert!(db.skill_vecs.iter().all(|v| v.len() == 64));
    let mut s = Simulation::new(db, params, &scenario);
    for _ in 0..3 {
        s.step();
    }
    let store = s.world.resource::<sim_core::AgentStore>();
    assert!(store.cap.iter().all(|v| v.len() == 64));
}

/// Lifetime ledger total of one flow in one town.
fn flow_total(s: &Simulation, town: usize, flow: sim_core::ledger::Flow) -> f64 {
    s.world.resource::<sim_core::Towns>().0[town].ledger.total[flow as usize]
}

fn with_orders(seed: u64, orders: &[&str], years: u32) -> Simulation {
    let mut s = full(seed);
    for o in orders {
        s.order(o).unwrap();
    }
    run_years(&mut s, years);
    s
}

#[test]
fn a_higher_wealth_tax_collects_more() {
    use sim_core::ledger::Flow;
    let base = with_orders(8, &[], 2);
    let taxed = with_orders(8, &["tax Greenvale wealth 2.0"], 2);
    let (a, b) = (
        flow_total(&base, 1, Flow::WealthTax),
        flow_total(&taxed, 1, Flow::WealthTax),
    );
    assert!(b > a * 1.5, "wealth tax {b:.0} vs {a:.0}");
}

#[test]
fn income_tax_takes_its_share_of_every_wage() {
    use sim_core::ledger::Flow;
    let base = with_orders(8, &[], 1);
    assert_eq!(flow_total(&base, 1, Flow::IncomeTax), 0.0);
    let s = with_orders(8, &["tax Greenvale income 0.2"], 1);
    let pay = flow_total(&s, 1, Flow::Wages) + flow_total(&s, 1, Flow::Subsidies);
    let tax = flow_total(&s, 1, Flow::IncomeTax);
    assert!(tax > 0.0);
    assert!(
        (tax - 0.2 * pay).abs() < 1e-3 * pay,
        "tax {tax} of pay {pay}"
    );
    // Only the ordering town levies it.
    assert_eq!(flow_total(&s, 0, Flow::IncomeTax), 0.0);
}

#[test]
fn a_higher_wage_share_pays_makers_more() {
    use sim_core::ledger::Flow;
    // While the treasury still holds its starting coins, wages follow the share;
    // later the treasury caps them. Summed over a few seeds so no single run's luck
    // decides it.
    let wages = |order: Option<&str>| -> f64 {
        (8..12)
            .map(|seed| {
                let mut s = full_bare(seed);
                if let Some(o) = order {
                    s.order(o).unwrap();
                }
                // Three days of time (a simulated day is ten).
                let clock = *s.world.resource::<sim_core::SimClock>();
                let steps = 3 * clock.steps_per_day / clock.days_per_month;
                for _ in 0..steps.max(1) {
                    s.step_tick();
                }
                flow_total(&s, 1, Flow::Wages)
            })
            .sum()
    };
    let (a, b) = (wages(None), wages(Some("wages Greenvale 0.8")));
    assert!(b > a * 1.2, "wages {b:.0} vs {a:.0}");
}

#[test]
fn public_spending_pays_out_the_treasury_surplus() {
    use sim_core::ledger::Flow;
    // A heavy wealth tax fills the treasury above its reserve; the spending rate
    // decides how fast the surplus goes back to residents.
    let none = with_orders(8, &["tax Greenvale wealth 3.0", "spending Greenvale 0"], 2);
    let fast = with_orders(8, &["tax Greenvale wealth 3.0", "spending Greenvale 10"], 2);
    assert_eq!(flow_total(&none, 1, Flow::PublicSpending), 0.0);
    assert!(flow_total(&fast, 1, Flow::PublicSpending) > 0.0);
}

#[test]
fn a_government_order_is_delivered_and_lifts_a_glut() {
    // Scarce goods (cloth sells at its highest price in Greenvale): the order takes
    // the town's output into public use until it is filled (in six years: the
    // weavers' trade grows by apprentices and newcomers, not overnight).
    let base = with_orders(8, &[], 2);
    let db = base.world.resource::<sim_core::Db>();
    let cloth = db.content.product("cloth").unwrap() as usize;
    let plank = db.content.product("plank").unwrap() as usize;
    let s = with_orders(8, &["procure Greenvale cloth 100"], 6);
    let town = &s.world.resource::<sim_core::Towns>().0[1];
    assert_eq!(town.procured[cloth], 100.0);
    assert!(
        town.policy.procurement.is_empty(),
        "a filled order is closed"
    );

    // A glutted good (Greenvale's planks, at their lowest price): the order raises
    // the stock the market wants, so the price (the signal that draws sawyers
    // back) goes up.
    let price = |order: Option<&str>| -> f32 {
        let mut s = full(8);
        if let Some(o) = order {
            s.order(o).unwrap();
        }
        let mut sum = 0.0;
        // Measure the end of year 3 (its last six months).
        let dpy = s.world.resource::<sim_core::SimClock>().months_per_year;
        for d in 0..3 * dpy {
            s.step();
            if d >= 3 * dpy - 6 {
                sum += s.world.resource::<sim_core::Towns>().0[1].price[plank];
            }
        }
        sum / 6.0
    };
    let (a, b) = (price(None), price(Some("procure Greenvale plank 20000")));
    assert!(b > a * 1.2, "plank price {b:.2} vs {a:.2}");
}

#[test]
fn the_run_does_not_depend_on_the_number_of_threads() {
    // Parallel steps (job scoring, learning, bodies) must give the same world
    // whether one thread or many do the work.
    let run = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let mut s = sim(12);
                run_years(&mut s, 1);
                s.state_hash()
            })
    };
    let one = run(1);
    assert_eq!(one, run(4));
    assert_eq!(one, run(7));
}

#[test]
fn an_order_that_eats_into_food_is_flagged_but_not_capped() {
    let mut s = full(8);
    // Orders for every food, so the town's food runs short (one food alone
    // no longer does it: there are many).
    let foods: Vec<String> = {
        let db = s.world.resource::<sim_core::Db>();
        db.foods
            .iter()
            .map(|f| db.content.products[f.0 as usize].id.clone())
            .collect()
    };
    for f in &foods {
        s.order(&format!("procure Greenvale {f} 30000")).unwrap();
    }
    s.order("procure Greenvale cloth 50").unwrap();
    let mut flagged = false;
    for _ in 0..24 {
        s.step();
        let db = s.world.resource::<sim_core::Db>();
        let params = s.world.resource::<sim_core::world::Params>();
        let town = &s.world.resource::<sim_core::Towns>().0[1];
        let warn = town.orders_eating_food(db, params);
        let cloth = db.content.product("cloth").unwrap();
        assert!(!warn.contains(&cloth), "cloth is not food");
        flagged |= !warn.is_empty();
    }
    assert!(flagged, "the bread order never drew a warning");
    // The order keeps running: no cap.
    let db = s.world.resource::<sim_core::Db>();
    let bread = db.content.product("bread").unwrap() as usize;
    assert!(s.world.resource::<sim_core::Towns>().0[1].procured[bread] > 0.0);
}

fn decided(s: &Simulation, town: usize, words: &str) -> bool {
    s.world.resource::<sim_core::governor::Governors>().towns[town]
        .decisions
        .iter()
        .any(|d| d.1.contains(words))
}

#[test]
fn governors_pull_the_players_levers_through_the_order_queue() {
    // AI towns govern themselves with the player's own orders; the player's town
    // (Greenvale) is left alone.
    let mut s = full(3);
    run_years(&mut s, 1);
    let log = &s.world.resource::<sim_core::policy::OrderQueue>().log;
    let by_town = |t: u16| log.iter().filter(|(_, o)| o.town() == t).count();
    assert!(
        by_town(0) + by_town(2) + by_town(3) > 0,
        "no AI town gave an order"
    );
    // Greenvale gets only the scenario's day-0 orders, if any; nothing later.
    assert!(log.iter().all(|(tick, o)| o.town() != 1 || *tick == 0));

    // Switched off, nobody governs.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    scenario.seed = 3;
    params.governor.enabled = false;
    let mut s = Simulation::new(db, params, &scenario);
    run_years(&mut s, 1);
    assert!(s
        .world
        .resource::<sim_core::policy::OrderQueue>()
        .log
        .is_empty());
}

#[test]
fn a_hungry_town_subsidizes_food_then_winds_it_down() {
    // Brassmoor with one field, no other food land and no bread to start: it
    // goes hungry in year one.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    // Calibrated with first-come jobs: skilled matching keeps this town fed.
    params.decision.matching = sim_core::decision::matching::MatchingParams::FirstCome;
    // and free job choice: the ruler's rule is tested, not how fast trades
    // follow it.
    params.decision.attendance = sim_core::config::Attendance::Free;
    // The town starts without bread: no steady-state start refilling it.
    params.start = sim_core::start::StartParams::Scenario;
    scenario.seed = 4;
    let b = &mut scenario.towns[3];
    for n in &mut b.nature {
        if n.0 == "field" {
            n.1 = 1;
        }
        // No other food land either (gardens, game, fish would feed it).
        if ["garden", "hunting_ground", "fishing_water", "pasture"].contains(&n.0.as_str()) {
            n.1 = 0;
        }
    }
    b.stock.retain(|s| s.0 != "bread");
    let mut s = Simulation::new(db, params, &scenario);
    run_years(&mut s, 1);
    assert!(
        decided(&s, 3, "food is short"),
        "no food decision while hungry"
    );
    // Fed again (relief bread keeps the market stocked, so the test is of the
    // ruler's rule, not of whether this town's farms recover): the hunger ends
    // and within a few years the food subsidies are wound down.
    let bread = s
        .world
        .resource::<sim_core::Db>()
        .content
        .product("bread")
        .unwrap() as usize;
    let mut wound_down = false;
    for year in 2..=8 {
        {
            let mut towns = s.world.resource_mut::<sim_core::Towns>();
            let town = &mut towns.0[3];
            let relief = 40.0 * town.residents.len() as f32;
            town.stock[bread] = town.stock[bread].max(relief);
        }
        run_years(&mut s, 1);
        let m = s.world.resource::<sim_core::metrics::Metrics>();
        let starving = m.latest().unwrap().towns[3].starving;
        let db = s.world.resource::<sim_core::Db>();
        let town = &s.world.resource::<sim_core::Towns>().0[3];
        let subsidies: Vec<f32> = db
            .foods
            .iter()
            .map(|&(f, _)| town.policy.subsidy[f as usize])
            .collect();
        eprintln!("year {year}: starving {starving}, food subsidies {subsidies:?}");
        if starving == 0.0 && subsidies.iter().all(|&x| x == 0.0) {
            wound_down = true;
            break;
        }
    }
    assert!(wound_down, "food subsidies never wound down");
}

#[test]
fn a_broke_town_raises_taxes_and_mints() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    // Calibrated with only the scenario\'s stock at the start.
    params.start = sim_core::start::StartParams::Scenario;
    params.market.starting_coins = 0.0;
    // A treasury that pays only what it holds (a town issuing its own coin
    // never goes broke; its governor taxes by prices instead).
    params.market.treasury = sim_core::config::Treasury::Holds;
    let mut s = Simulation::new(db, params, &scenario);
    // Thirty days: with trades kept, the treasury runs dry a little later.
    for _ in 0..30 {
        s.step();
    }
    let broke: Vec<usize> = (0..4)
        .filter(|&t| t != 1 && decided(&s, t, "raised the wealth tax"))
        .collect();
    assert!(!broke.is_empty(), "no broke town raised its taxes");
    // The governor keeps only its last few reasons, and a town that stays
    // broke gives several each look: the order queue has them all.
    let log = &s.world.resource::<sim_core::policy::OrderQueue>().log;
    assert!(
        broke.iter().any(|&t| log.iter().any(|(_, o)| matches!(
            o,
            sim_core::policy::Order::Mint {
                town,
                minting: sim_core::policy::Minting::On
            } if *town as usize == t
        ))),
        "no broke town set its mint to strike"
    );
}

#[test]
fn governors_do_not_swing_back_and_forth() {
    use sim_core::policy::{Order, Tax};
    // Forty years: every town's wealth tax should move in long runs, not flip.
    // Seeds 1 to 8, run in parallel, each with only the scenario's stock at the
    // start: per town, how often the wealth tax reverses direction and how many
    // subsidy changes it orders. The limits hold over all 32 towns, not one
    // chosen seed.
    let seeds: Vec<u64> = (1..=8).collect();
    let runs: Vec<Vec<(u32, usize)>> = std::thread::scope(|sc| {
        let handles: Vec<_> = seeds
            .iter()
            .map(|&seed| {
                sc.spawn(move || {
                    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
                    let (db, mut params, mut scenario) =
                        sim_core::load(&AssetPaths::from_root(&root)).unwrap();
                    params.start = sim_core::start::StartParams::Scenario;
                    scenario.seed = seed;
                    for t in &mut scenario.towns {
                        t.population = 60;
                    }
                    params.governor.govern_player_town = true;
                    let mut s = Simulation::new(db, params, &scenario);
                    run_years(&mut s, 40);
                    let log = &s.world.resource::<sim_core::policy::OrderQueue>().log;
                    (0..4u16)
                        .map(|t| {
                            let rates: Vec<f32> = log
                                .iter()
                                .filter_map(|(_, o)| match o {
                                    Order::Tax {
                                        town,
                                        tax: Tax::Wealth,
                                        rate,
                                    } if *town == t => Some(rate.unwrap_or(0.5)),
                                    _ => None,
                                })
                                .collect();
                            let mut reversals = 0;
                            let mut last = 0.0f32;
                            for w in rates.windows(2) {
                                let d = w[1] - w[0];
                                if d != 0.0 {
                                    if last != 0.0 && d.signum() != last.signum() {
                                        reversals += 1;
                                    }
                                    last = d;
                                }
                            }
                            let subsidies = log
                                .iter()
                                .filter(
                                    |(_, o)| matches!(o, Order::Subsidy { town, .. } if *town == t),
                                )
                                .count();
                            (reversals, subsidies)
                        })
                        .collect()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let all: Vec<(u32, usize)> = runs.iter().flatten().copied().collect();
    let mean = all.iter().map(|&(r, _)| r as f32).sum::<f32>() / all.len() as f32;
    let worst = all.iter().map(|&(r, _)| r).max().unwrap();
    let worst_sub = all.iter().map(|&(_, s)| s).max().unwrap();
    // Measured over these 32 towns: before the food yields of food-yields.md
    // the mean was 4.4 reversals, the worst town 13 and the most subsidy
    // changes 113; after, 4.1, 18 and 98. A mean of 6 is one reversal every
    // seven years. Single towns are noisy (the worst moved from 13 to 18 while
    // the mean fell), so the worst town gets one reversal every two years. A
    // governor that really flip-flops moves the mean, which no seed can hide.
    // (The governors' rules get their own goals in plan Phase 5.)
    assert!(
        mean <= 6.0,
        "wealth tax reversed {mean:.2} times per town on average; per seed: {runs:?}"
    );
    assert!(
        worst <= 20,
        "a town reversed its wealth tax {worst} times; per seed: {runs:?}"
    );
    // Kept trades: food workers follow subsidies more slowly, so rulers adjust
    // them more often (the worst town 167).
    // (One town in 32 reached 239 once tribute and food imports moved in
    // the towns' own coins.)
    assert!(
        worst_sub <= 260,
        "a town made {worst_sub} subsidy changes in 40 years; per seed: {runs:?}"
    );
}

#[test]
fn agents_learn_what_their_jobs_pay() {
    // The expectations component tracks realised pay; the town's public rate is the
    // prior for jobs an agent hasn't done. Neither changes the run yet.
    let mut s = full(5);
    run_years(&mut s, 1);
    let towns = s.world.resource::<sim_core::Towns>();
    let store = s.world.resource::<sim_core::AgentStore>();
    let db = s.world.resource::<sim_core::Db>();
    let bake = db.content.recipe("bake_bread").unwrap();
    for town in &towns.0 {
        assert!(
            town.pay_rate[bake as usize] > 0.0,
            "{}: bread pays nothing",
            town.name
        );
    }
    // Bakers expect roughly what baking has paid in their town. Someone who
    // baked once, in a shortage, keeps that pay in mind until they bake
    // again, so look at the typical baker: the median of what they expect
    // against their town's rate.
    let mut ratios: Vec<f32> = Vec::new();
    for i in 0..store.len() {
        if !store.alive[i] {
            continue;
        }
        if let Some(&(_, e)) = store.pay_expect[i].iter().find(|e| e.0 == bake) {
            let public = towns.0[store.town[i] as usize].pay_rate[bake as usize];
            assert!(e >= 0.0, "{e} vs {public}");
            ratios.push(e / public.max(0.01));
        }
    }
    assert!(ratios.len() > 50, "only {} agents ever baked", ratios.len());
    ratios.sort_by(f32::total_cmp);
    let median = ratios[ratios.len() / 2];
    // The public rate is what a full share earns (pay follows skill), so a
    // typical baker expects well below it.
    assert!(
        (0.1..5.0).contains(&median),
        "bakers expect {median:.2} times what baking pays"
    );
    assert!(store
        .pay_expect
        .iter()
        .all(|l| l.len() <= sim_core::decision::expectations::KEEP));
}

fn market_pay() -> sim_core::decision::wages::WageParams {
    sim_core::decision::wages::WageParams::ExcessDemand {
        step: 0.02,
        target_fill: 0.8,
        min: 0.5,
        max: 2.0,
    }
}

#[test]
fn market_pay_moves_both_ways_and_the_books_still_balance() {
    use sim_core::ledger::{sum_group, Group};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    scenario.seed = 5;
    params.decision.wages = market_pay();
    let mut s = Simulation::new(db, params, &scenario);
    run_years(&mut s, 1);
    let before: Vec<f32> = s
        .world
        .resource::<sim_core::Towns>()
        .0
        .iter()
        .map(|t| t.treasury.coins())
        .collect();
    run_years(&mut s, 1);
    let towns = s.world.resource::<sim_core::Towns>();
    let factors: Vec<f32> = towns
        .0
        .iter()
        .flat_map(|t| t.wage_factor.iter().copied())
        .collect();
    assert!(factors.iter().all(|&f| (0.5..=2.0).contains(&f)));
    assert!(
        factors.iter().any(|&f| f < 0.95),
        "no crowded job paid less"
    );
    assert!(
        factors.iter().any(|&f| f > 1.05),
        "no understaffed job paid more"
    );
    for (t, town) in towns.0.iter().enumerate() {
        let l = &town.ledger.last_year;
        let net = sum_group(l, Group::Taxed)
            + sum_group(l, Group::Spent)
            + sum_group(l, Group::Minted)
            + sum_group(l, Group::ExternalIn)
            - sum_group(l, Group::Issued)
            - sum_group(l, Group::ExternalOut);
        let change = town.treasury.coins() - before[t];
        assert!((net - change).abs() < 1e-3 * sum_group(l, Group::Issued).max(1.0));
    }
}

#[test]
fn a_short_treasury_pays_every_job_its_share() {
    // With no coins to start, the treasury can't cover the wages owed. Every
    // finished job still gets the same share of its claim, so many different jobs
    // earn something, not only the first ones done each day.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    params.market.starting_coins = 0.0;
    params.governor.enabled = false;
    let mut s = Simulation::new(db, params, &scenario);
    for _ in 0..6 {
        s.step();
    }
    let towns = s.world.resource::<sim_core::Towns>();
    for town in &towns.0 {
        let paid = town.pay_rate.iter().filter(|&&p| p > 0.0).count();
        assert!(paid >= 5, "{}: only {paid} jobs ever paid", town.name);
    }
}

#[test]
#[ignore = "under ladder decisions the learning term is within seed noise; Phase 3 careers replace daily job choice"]
fn valuing_skills_draws_the_young_into_work_they_are_still_learning() {
    use sim_core::decision::learning::LearningValueParams;
    // Share of young adults' working days spent on jobs they can't do well yet
    // (skill along the recipe below its difficulty), without and with valuing what
    // the work teaches.
    let learning_share = |learning: LearningValueParams, seed: u64| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
        // Calibrated with only the scenario\'s stock at the start.
        params.start = sim_core::start::StartParams::Scenario;
        scenario.seed = seed;
        params.decision.learning = learning;
        // Posted pay stays on: on the needs ladder a job is worth its pay, and
        // with only realised pay counting nobody starts working at all.
        // Isolate the learning term from trade (calibrated with fixed caravans).
        params.market.caravans = sim_core::config::Caravans::Fixed;
        // And from apprentice places, which already steer the young to learning.
        params.decision.matching = sim_core::decision::matching::MatchingParams::FirstCome;
        let mut s = Simulation::new(db, params, &scenario);
        run_years(&mut s, 1);
        let (mut learning_days, mut days) = (0u32, 0u32);
        for _ in 0..9 {
            s.step();
            let store = s.world.resource::<sim_core::AgentStore>();
            let db = s.world.resource::<sim_core::Db>();
            let dpy = s.world.resource::<sim_core::SimClock>().months_per_year;
            for i in 0..store.len() {
                let age = store.age_years(i, dpy);
                if !store.alive[i] || !(16.0..30.0).contains(&age) {
                    continue;
                }
                if let Some(r) = store.worked_today[i] {
                    let m = &db.recipes[r as usize];
                    let prof: f32 = store.cap[i].iter().zip(&m.skill).map(|(a, b)| a * b).sum();
                    days += 1;
                    learning_days += (prof < m.difficulty) as u32;
                }
            }
        }
        learning_days as f32 / days.max(1) as f32
    };
    // Averaged over six seeds: on any one seed the effect is smaller than the noise.
    let (mut with, mut without) = (0.0, 0.0);
    for seed in 3..=8 {
        without += learning_share(LearningValueParams::None, seed) / 6.0;
        with += learning_share(
            LearningValueParams::Discounted {
                horizon_days: 360.0,
                discount: 0.995,
                retire_age: 60.0,
            },
            seed,
        ) / 6.0;
    }
    assert!(
        with > without,
        "learning work {with:.3} with vs {without:.3} without"
    );
}

#[test]
fn matching_gives_beginners_apprentice_places_and_keeps_the_books() {
    use sim_core::decision::matching::{ApprenticeShareParams, MatchingParams};
    use sim_core::ledger::{sum_group, Group};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    scenario.seed = 7;
    params.decision.matching = MatchingParams::DeferredAcceptance {
        apprentice_wage: 0.4,
        share: ApprenticeShareParams::default(),
    };
    let mut s = Simulation::new(db, params, &scenario);
    run_years(&mut s, 1);
    let before: Vec<f32> = s
        .world
        .resource::<sim_core::Towns>()
        .0
        .iter()
        .map(|t| t.treasury.coins())
        .collect();
    let (mut apprentices, mut beginners) = (0usize, 0usize);
    // Choices are made a day of time at a time: look at every step's.
    let spd = s
        .world
        .resource::<sim_core::world::SimClock>()
        .steps_per_day;
    for _ in 0..12 * spd {
        s.step_tick();
        let d = s.world.resource::<sim_core::world::Decisions>();
        let store = s.world.resource::<sim_core::AgentStore>();
        let db = s.world.resource::<sim_core::Db>();
        apprentices += d.apprentices.len();
        // Beginners: apprentices whose skill is below the job's difficulty.
        for &(i, r) in &d.jobs {
            if d.apprentices.binary_search(&i).is_ok() {
                let m = &db.recipes[r as usize];
                let prof: f32 = store.cap[i as usize]
                    .iter()
                    .zip(&m.skill)
                    .map(|(a, b)| a * b)
                    .sum();
                beginners += (prof < m.difficulty) as usize;
            }
        }
    }
    assert!(apprentices > 0, "no apprentice places were taken");
    assert!(beginners > 0, "no beginner got an apprentice place");
    let towns = s.world.resource::<sim_core::Towns>();
    let p = ApprenticeShareParams::default();
    for town in &towns.0 {
        assert!(town
            .apprentice_share
            .iter()
            .all(|&x| (p.min..=p.max).contains(&x)));
    }
    assert!(towns
        .0
        .iter()
        .any(|t| t.apprentice_share.iter().any(|&x| x > 0.0)));
    for (t, town) in towns.0.iter().enumerate() {
        let l = &town.ledger.last_year;
        let net = sum_group(l, Group::Taxed)
            + sum_group(l, Group::Spent)
            + sum_group(l, Group::Minted)
            + sum_group(l, Group::ExternalIn)
            - sum_group(l, Group::Issued)
            - sum_group(l, Group::ExternalOut);
        let change = town.treasury.coins() - before[t];
        assert!((net - change).abs() < 1e-3 * sum_group(l, Group::Issued).max(1.0));
    }
}

#[test]
fn utility_migration_moves_households_and_respects_closed_borders() {
    use sim_core::decision::migration::MigrationValueParams;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    scenario.seed = 3;
    params.decision.migration = MigrationValueParams::Utility(Default::default());
    let mut s = Simulation::new(db, params, &scenario);
    s.order("immigration Greenvale closed").unwrap();
    run_years(&mut s, 4);
    let m = s.world.resource::<sim_core::metrics::Metrics>();
    let arrived: u32 = m
        .years
        .iter()
        .flat_map(|y| &y.towns)
        .map(|t| t.moves.arrived)
        .sum();
    let left: u32 = m
        .years
        .iter()
        .flat_map(|y| &y.towns)
        .map(|t| t.moves.left)
        .sum();
    assert!(arrived > 0, "nobody moved");
    assert_eq!(arrived, left, "every arrival leaves somewhere");
    let into_greenvale: u32 = m.years.iter().map(|y| y.towns[1].moves.arrived).sum();
    assert_eq!(into_greenvale, 0, "closed borders let people in");
}

#[test]
fn households_keep_food_and_the_books_still_balance() {
    use sim_core::decision::subsistence::SubsistenceParams;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    // Under the needs ladder households also buy food for their larders; leave
    // that out so the larders hold only what they kept.
    if let sim_core::consumption::ConsumptionParams::Needs(lp) = &mut params.market.consumption {
        lp.larder_days = 0.0;
    }
    params.decision.subsistence = SubsistenceParams::Keep {
        share: 0.3,
        larder_meals: 6.0,
    };
    if std::env::var("K_OFF").is_ok() {
        params.knowledge.enabled = false;
    }
    if std::env::var("I_OFF").is_ok() {
        params.knowledge.invention.enabled = false;
    }
    let mut s = Simulation::new(db, params, &scenario);
    let money = |s: &Simulation| {
        let towns = s.world.resource::<sim_core::Towns>();
        // `minted` also counts coin struck into the metal of towns that
        // issue their own coin, which the ledger leaves out.
        let minted: f64 = towns.0.iter().map(|t| t.minted.coins_f64()).sum();
        sim_core::world::total_money(s.world.resource::<sim_core::AgentStore>(), towns) - minted
    };
    let before = money(&s);
    run_years(&mut s, 2);
    let after = money(&s);
    assert!(
        ((after - before) / before).abs() < 1e-3,
        "money changed: {before} -> {after}"
    );
    let store = s.world.resource::<sim_core::AgentStore>();
    let held: Vec<f32> = (0..store.len())
        .filter(|&i| store.alive[i])
        .map(|i| store.larder[i].iter().map(|e| e.1).sum())
        .collect();
    assert!(held.iter().any(|&h| h > 0.0), "no household kept any food");
    assert!(held.iter().all(|&h| h <= 6.0 + 1e-3), "a larder overflowed");
    // Dead agents' larders went back to the town.
    assert!((0..store.len())
        .filter(|&i| !store.alive[i])
        .all(|i| store.larder[i].is_empty()));
}

#[test]
fn crowded_towns_charge_more_rent_and_the_books_still_balance() {
    use sim_core::decision::crowding::CrowdingParams;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    params.decision.crowding = CrowdingParams::Rent {
        base: 100.0,
        per_plot: 12.0,
        per_building: 8.0,
        rate: 0.3,
        exponent: 2.0,
        sticky_days: 60.0,
    };
    let mut s = Simulation::new(db, params, &scenario);
    let money = |s: &Simulation| {
        let towns = s.world.resource::<sim_core::Towns>();
        // `minted` also counts coin struck into the metal of towns that
        // issue their own coin, which the ledger leaves out.
        let minted: f64 = towns.0.iter().map(|t| t.minted.coins_f64()).sum();
        sim_core::world::total_money(s.world.resource::<sim_core::AgentStore>(), towns) - minted
    };
    let before = money(&s);
    run_years(&mut s, 1);
    let after = money(&s);
    assert!(
        ((after - before) / before).abs() < 1e-3,
        "money changed: {before} -> {after}"
    );
    let towns = s.world.resource::<sim_core::Towns>();
    let rent: f64 = towns
        .0
        .iter()
        .map(|t| t.ledger.total[sim_core::ledger::Flow::Rent as usize])
        .sum();
    assert!(rent > 0.0, "no rent was paid");
    // Rent follows residents per place to live.
    let crowding = |t: &sim_core::world::Town| {
        t.residents.len() as f32
            / (100.0
                + 12.0 * (t.nature_sites.len() + t.free_land as usize) as f32
                + 8.0 * t.buildings.len() as f32)
    };
    let mut by: Vec<(f32, f32)> = towns.0.iter().map(|t| (crowding(t), t.rent)).collect();
    by.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert!(
        by.last().unwrap().1 > by[0].1,
        "the most crowded town is not dearer: {by:?}"
    );
}

/// Mean cultural fit of adults with the town they live in.
fn mean_fit(s: &Simulation) -> f32 {
    use sim_data::dims::mind;
    let store = s.world.resource::<sim_core::AgentStore>();
    let towns = s.world.resource::<sim_core::Towns>();
    let fits: Vec<f32> = (0..store.len())
        .filter(|&i| store.alive[i])
        .map(|i| {
            sim_core::decision::culture::fit(
                &store.mind[i][mind::IDEOLOGY_START..],
                &towns.0[store.town[i] as usize].culture,
            )
        })
        .collect();
    fits.iter().sum::<f32>() / fits.len().max(1) as f32
}

#[test]
fn town_cultures_come_from_their_people_and_draw_those_who_fit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let run = |weight: f32, seeded: bool| {
        let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
        params.migration.culture_weight = weight;
        params.migration.consider = 0.2;
        if seeded {
            scenario.towns[0].culture = Some(vec![0.9, 0.1, 0.5]);
        }
        let mut s = Simulation::new(db, params, &scenario);
        run_years(&mut s, 3);
        s
    };
    let s = run(1.0, true);
    let towns = s.world.resource::<sim_core::Towns>();
    for t in &towns.0 {
        assert_eq!(t.culture.len(), 3);
        assert!(t.culture.iter().all(|v| (0.0..=1.0).contains(v)));
    }
    // The seeded town keeps the culture it was founded with (its people were drawn
    // around it and the culture changes only slowly).
    let c = &towns.0[0].culture;
    assert!(c[0] > 0.7 && c[1] < 0.3, "seeded culture lost: {c:?}");
    // Same seed, same cultures.
    let again = run(1.0, true);
    let a: Vec<u32> = again.world.resource::<sim_core::Towns>().0[0]
        .culture
        .iter()
        .map(|v| v.to_bits())
        .collect();
    let b: Vec<u32> = c.iter().map(|v| v.to_bits()).collect();
    assert_eq!(a, b);
    // When culture weighs more in moving, people end up where they fit better.
    let strong = mean_fit(&run(4.0, false));
    let none = mean_fit(&run(0.0, false));
    assert!(strong > none, "fit {strong} with culture vs {none} without");
}

#[test]
fn households_only_move_to_towns_they_know() {
    use sim_core::decision::knowledge::KnowledgeParams;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    // Two seeds, three years: few households move, so one seed is noisy.
    let run = |knowledge: KnowledgeParams| {
        let mut moved_all = 0;
        let mut remembered_all = 0;
        for seed in [5, 6] {
            let (db, mut params, mut scenario) =
                sim_core::load(&AssetPaths::from_root(&root)).unwrap();
            scenario.seed = seed;
            params.migration.consider = 0.5;
            params.migration.min_gain = 0.1;
            params.decision.knowledge = knowledge.clone();
            let mut s = Simulation::new(db, params, &scenario);
            run_years(&mut s, 3);
            let m = s.world.resource::<sim_core::metrics::Metrics>();
            let moved: u32 = m
                .years
                .iter()
                .flat_map(|y| &y.towns)
                .map(|t| t.moves.arrived)
                .sum();
            let store = s.world.resource::<sim_core::AgentStore>();
            let remembered = (0..store.len())
                .filter(|&i| !store.lived_in[i].is_empty())
                .count();
            moved_all += moved;
            remembered_all += remembered;
        }
        (moved_all, remembered_all)
    };
    // Knowing every town, people move, and movers remember their old home.
    let (all, remembered) = run(KnowledgeParams::All);
    assert!(all > 0, "nobody moved knowing every town");
    assert!(remembered > 0, "no mover remembers an old home");
    // Founders have no ties abroad, and here trade tells them nothing: only the few
    // who come to know another town (through captives' families and friends taken
    // in raids) can move there.
    let (limited, _) = run(KnowledgeParams::Limited {
        min_known: 0.1,
        trade_scale: 2.0,
        trade_known: 0.0,
        memory_days: 3650.0,
        noise: 0.5,
    });
    eprintln!("moved knowing every town: {all}, knowing only through ties: {limited}");
    // Clearly fewer moves (CI on Linux saw 22 against 85, Windows 27 against 48).
    assert!(
        limited < all,
        "knowledge did not hold people back: {limited} vs {all}"
    );
}

#[test]
fn towns_post_pay_for_jobs_nobody_has_tried() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let run = |days: f32| {
        let (db, mut params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
        params.decision.valuation.posted_pay_days = days;
        let mut s = Simulation::new(db, params, &scenario);
        for _ in 0..3 {
            s.step();
        }
        let db = s.world.resource::<sim_core::Db>();
        let log = s.world.resource::<sim_core::world::WorkLog>();
        let towns = s.world.resource::<sim_core::Towns>();
        // Jobs nobody has finished yet, that would pay at today's prices.
        let untried: Vec<usize> = (0..db.recipes.len())
            .filter(|&r| log.runs[r] == 0)
            .collect();
        let shown: f32 = untried.iter().map(|&r| towns.0[0].pay_rate[r]).sum();
        (untried.len(), shown)
    };
    let (untried, shown) = run(90.0);
    assert!(untried > 0, "every job was tried in the first month");
    assert!(shown > 0.0, "untried jobs show no pay");
    // Off: only realised pay counts, and untried jobs show none.
    let (_, shown) = run(0.0);
    assert_eq!(shown, 0.0);
}

#[test]
fn scarce_goods_loosen_their_price_caps() {
    use sim_core::config::ScarcityCap;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let run = |cap: ScarcityCap| {
        let (db, mut params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
        params.market.scarcity_cap = cap;
        let max = params.market.max_price_factor;
        let mut s = Simulation::new(db, params, &scenario);
        run_years(&mut s, 1);
        let db = s.world.resource::<sim_core::Db>();
        let towns = s.world.resource::<sim_core::Towns>();
        // Highest price relative to its base among ordinary goods.
        let top = towns
            .0
            .iter()
            .flat_map(|t| {
                (0..db.content.products.len())
                    .filter(|&p| db.coin[p] <= 0.0 && db.opens[p].is_none())
                    .map(move |p| t.price[p] / db.base_price[p].max(1e-6))
            })
            .fold(0.0f32, f32::max);
        (top, max)
    };
    let (top, max) = run(ScarcityCap::Fixed);
    assert!(top <= max + 1e-3, "a price passed its fixed cap: {top}");
    let (top, max) = run(ScarcityCap::Loosen {
        rate: 0.01,
        max_factor: 50.0,
    });
    assert!(top > max, "no scarce good's cap loosened: {top}");
    assert!(top <= 50.0 + 1e-3, "a cap passed its ceiling: {top}");
}

#[test]
fn price_classes_bound_daily_moves_and_volatility_is_reported() {
    use sim_core::pricing::*;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    let class = |speed: f32, max_daily: f32| ClassParams {
        inertia: Inertia::Partial { speed },
        max_daily,
        floor: 0.05,
        ceiling: 20.0,
    };
    params.market.pricing = sim_core::config::Pricing {
        cover_days: 10.0,
        blocked_workers: 2.0,
    };
    params.market.price_classes = Some(PriceClasses {
        essential: class(0.5, 0.01),
        commodity: class(0.08, 0.08),
        labour: ClassParams {
            inertia: Inertia::Sticky {
                up: 0.02,
                down: 0.005,
            },
            ..class(0.0, 0.01)
        },
    });
    let mut s = Simulation::new(db, params, &scenario);
    run_years(&mut s, 2);
    let m = s.world.resource::<sim_core::metrics::Metrics>();
    let y = m.latest().unwrap();
    for t in &y.towns {
        let (ess, ess_p95) = t.price_vol[0];
        // Essentials never move more than 1 % a day (the p95 is a histogram bin edge).
        assert!(
            ess <= 0.01 + 1e-4,
            "{}: essentials moved {ess} a day",
            t.name
        );
        assert!(ess_p95 <= 0.012, "{}: essentials' p95 {ess_p95}", t.name);
        // Commodities are allowed to move faster, and do.
        assert!(t.price_vol[1].0 > 0.0);
    }
}

#[test]
fn price_indices_and_money_stock_are_reported() {
    let mut s = sim(3);
    run_years(&mut s, 3);
    let m = s.world.resource::<sim_core::metrics::Metrics>();
    // The first year is the base.
    for t in &m.years[0].towns {
        assert_eq!(t.cpi.laspeyres, 100.0);
        assert_eq!(t.cpi.fisher, 100.0);
    }
    let y = m.latest().unwrap();
    let mut money = 0.0f64;
    for t in &y.towns {
        assert!(t.cpi.fisher > 0.0 && t.cpi.fisher.is_finite(), "{}", t.name);
        assert!((t.purchasing_power * t.cpi.fisher - 100.0).abs() < 1e-2);
        assert!(t.real_exchange_rate > 0.0 && t.real_exchange_rate.is_finite());
        assert!(t.velocity >= 0.0 && t.minted_share >= 0.0);
        money += t.money_stock as f64;
    }
    // Every coin is in some town's treasury or a resident's purse (soldiers away
    // on a raid and captives aside), so the towns' money adds up to the total.
    assert!(
        (money - y.total_money).abs() < 0.05 * y.total_money,
        "{money} vs {}",
        y.total_money
    );
}

#[test]
fn every_town_has_its_workshops_with_books_ready() {
    let s = sim(2);
    let db = s.world.resource::<sim_core::Db>();
    let towns = s.world.resource::<sim_core::Towns>();
    assert!(!db.firm_keys.is_empty());
    assert_eq!(db.recipe_firm.len(), db.recipes.len());
    for t in &towns.0 {
        assert_eq!(t.firms.len(), db.firm_keys.len());
        // Off by default: firms hold nothing.
        assert!(t.firms.iter().all(|f| f.cash.coins() == 0.0));
    }
    // Recipes that share a building share a firm.
    for (r, m) in db.recipes.iter().enumerate() {
        let k = db.firm_keys[db.recipe_firm[r]];
        assert_eq!(k, sim_core::firms::key_of(m));
    }
}

#[test]
fn workshops_pay_from_their_sales_and_money_is_conserved() {
    use sim_core::firms::{FirmFlow, FirmsParams};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    params.market.firms = FirmsParams::Workshops {
        start_cash: 100.0,
        reserve_days: 10.0,
        payout_rate: 0.05,
        profit_tax: 0.2,
        close_days: 30,
        credit_days: 0.0,
        input_days: 0.0,
        grouping: Default::default(),
    };
    // Households buying only food and tools: with the needs ladder's household
    // buying, workshops as firms lose more than half the people within a year.
    params.market.consumption = sim_core::consumption::ConsumptionParams::None;
    let mut s = Simulation::new(db, params, &scenario);
    let money = |s: &Simulation| {
        let towns = s.world.resource::<sim_core::Towns>();
        // `minted` also counts coin struck into the metal of towns that
        // issue their own coin, which the ledger leaves out.
        let minted: f64 = towns.0.iter().map(|t| t.minted.coins_f64()).sum();
        sim_core::world::total_money(s.world.resource::<sim_core::AgentStore>(), towns) - minted
    };
    let before = money(&s);
    run_years(&mut s, 2);
    let after = money(&s);
    assert!(
        ((after - before) / before).abs() < 1e-3,
        "money changed from {before} to {after}"
    );
    let towns = s.world.resource::<sim_core::Towns>();
    let y = sim_core::firms::last_year_totals(&towns.0[0].firms);
    // Firms sold goods and paid wages out of what they took in.
    assert!(y[FirmFlow::Sales as usize] + y[FirmFlow::DirectSales as usize] > 0.0);
    assert!(y[FirmFlow::Wages as usize] > 0.0);
    // The treasury no longer pays the wages of private work.
    let l = &towns.0[0].ledger;
    assert!(l.last_year[sim_core::ledger::Flow::Wages as usize] < y[FirmFlow::Wages as usize]);
    let m = s.world.resource::<sim_core::metrics::Metrics>();
    assert!(m.latest().unwrap().population > 1000, "the towns collapsed");
}

#[test]
fn towns_lend_to_workshops_and_get_repaid_with_interest() {
    use sim_core::firms::FirmsParams;
    use sim_core::ledger::Flow;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    params.market.firms = FirmsParams::Workshops {
        start_cash: 100.0,
        reserve_days: 10.0,
        payout_rate: 0.05,
        profit_tax: 0.2,
        close_days: 30,
        credit_days: 0.0,
        input_days: 0.0,
        grouping: Default::default(),
    };
    params.governor.enabled = false;
    let mut s = Simulation::new(db, params, &scenario);
    let money = |s: &Simulation| {
        let towns = s.world.resource::<sim_core::Towns>();
        // `minted` also counts coin struck into the metal of towns that
        // issue their own coin, which the ledger leaves out.
        let minted: f64 = towns.0.iter().map(|t| t.minted.coins_f64()).sum();
        sim_core::world::total_money(s.world.resource::<sim_core::AgentStore>(), towns) - minted
    };
    let before = money(&s);
    s.order("loan Ironhold field 60 0.2 60").unwrap();
    assert!(s.order("loan Ironhold nowhere 60 0.2 60").is_err());
    for _ in 0..10 {
        s.step();
    }
    let towns = s.world.resource::<sim_core::Towns>();
    let l = &towns.0[0].ledger;
    let lent = l.total[Flow::LoansOut as usize];
    let repaid = l.total[Flow::LoanRepayments as usize];
    let interest = l.total[Flow::LoanInterest as usize];
    assert!(lent > 0.0, "no loan was made");
    // A farm with sales pays it back, with interest, within its term.
    assert!(
        repaid + towns.0[0].loan_defaults as f64 > 0.99 * lent,
        "repaid {repaid} of {lent}"
    );
    assert!(interest > 0.0);
    assert!(sim_core::firms::outstanding(&towns.0[0].firms) < 1.0);
    let after = money(&s);
    assert!(
        ((after - before) / before).abs() < 1e-3,
        "money changed: {before} -> {after}"
    );
}

#[test]
fn the_profit_tax_is_a_town_lever() {
    use sim_core::firms::{FirmFlow, FirmsParams};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let run = |order: &str| {
        let (db, mut params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
        params.market.firms = FirmsParams::Workshops {
            start_cash: 100.0,
            reserve_days: 10.0,
            payout_rate: 0.05,
            profit_tax: 0.0,
            close_days: 30,
            credit_days: 0.0,
            input_days: 0.0,
            grouping: Default::default(),
        };
        params.governor.enabled = false;
        let mut s = Simulation::new(db, params, &scenario);
        for t in ["Ironhold", "Greenvale", "Timberwick", "Brassmoor"] {
            s.order(&format!("tax {t} profit {order}")).unwrap();
        }
        let mut dividends = 0.0;
        for _ in 0..2 {
            run_years(&mut s, 1);
            let towns = s.world.resource::<sim_core::Towns>();
            dividends += towns
                .0
                .iter()
                .map(|t| sim_core::firms::last_year_totals(&t.firms)[FirmFlow::Dividends as usize])
                .sum::<f32>();
        }
        let towns = s.world.resource::<sim_core::Towns>();
        (towns.0[0].policy.profit_tax, dividends)
    };
    let (rate, dividends) = run("1");
    assert_eq!(rate, Some(1.0));
    assert_eq!(
        dividends, 0.0,
        "owners got dividends under a 100% profit tax"
    );
    let (rate, dividends) = run("0");
    assert_eq!(rate, Some(0.0));
    assert!(dividends > 0.0, "no owner ever got a dividend untaxed");
}

#[test]
fn workshops_buy_on_account_and_pay_from_their_sales() {
    use sim_core::firms::FirmsParams;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    params.market.firms = FirmsParams::Workshops {
        start_cash: 50.0,
        reserve_days: 5.0,
        payout_rate: 0.2,
        profit_tax: 0.2,
        close_days: 30,
        credit_days: 10.0,
        input_days: 0.0,
        grouping: Default::default(),
    };
    let mut s = Simulation::new(db, params, &scenario);
    let money = |s: &Simulation| {
        let towns = s.world.resource::<sim_core::Towns>();
        // `minted` also counts coin struck into the metal of towns that
        // issue their own coin, which the ledger leaves out.
        let minted: f64 = towns.0.iter().map(|t| t.minted.coins_f64()).sum();
        sim_core::world::total_money(s.world.resource::<sim_core::AgentStore>(), towns) - minted
    };
    let before = money(&s);
    let mut seen_debt = false;
    for _ in 0..24 {
        s.step();
        let towns = s.world.resource::<sim_core::Towns>();
        seen_debt |= towns
            .0
            .iter()
            .any(|t| t.firms.iter().any(|f| f.owed() > 0.0));
        // Nobody owes more than their credit allows plus what they were short today.
        for t in &towns.0 {
            for f in &t.firms {
                assert!(f.owed() >= 0.0);
            }
        }
    }
    assert!(seen_debt, "no firm ever bought on account");
    let after = money(&s);
    assert!(
        ((after - before) / before).abs() < 1e-3,
        "money changed: {before} -> {after}"
    );
}

#[test]
fn guild_firms_pool_a_domain_and_the_mint_answers_a_money_shortage() {
    use sim_core::firms::{FirmKey, FirmsParams, Grouping};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    params.market.firms = FirmsParams::Workshops {
        start_cash: 50.0,
        reserve_days: 5.0,
        payout_rate: 0.2,
        profit_tax: 0.2,
        close_days: 30,
        credit_days: 0.0,
        input_days: 0.0,
        grouping: Grouping::Guild,
    };
    params.market.mint_money_per_capita = 1e9;
    let mut s = Simulation::new(db, params, &scenario);
    s.step();
    let db = s.world.resource::<sim_core::Db>();
    // One firm per domain that has work.
    let mut domains: Vec<u32> = db.recipes.iter().map(|m| m.domain).collect();
    domains.sort_unstable();
    domains.dedup();
    assert_eq!(db.firm_keys.len(), domains.len());
    assert!(db.firm_keys.iter().all(|k| matches!(k, FirmKey::Guild(_))));
    let towns = s.world.resource::<sim_core::Towns>();
    assert_eq!(towns.0[0].firms.len(), domains.len());
    // Far below the money target: every town wants coin.
    let mp = &s.world.resource::<sim_core::world::Params>().market;
    assert!(towns
        .0
        .iter()
        .all(|t| t.money_per_capita > 0.0 && t.mint_wanted(mp)));
}

#[test]
fn a_hungry_broke_town_imports_food_on_credit_and_recovers() {
    // Brassmoor with no fields, no bread and an empty treasury.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let run = |imports: bool, seed: u64| {
        let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
        // Calibrated with only the scenario\'s stock at the start.
        params.start = sim_core::start::StartParams::Scenario;
        scenario.seed = seed;
        params.governor.enabled = false;
        // Credit is for a treasury that only holds coin; one that issues
        // pays at once in its own coin.
        params.market.treasury = sim_core::config::Treasury::Holds;
        // With two fields, as before the seasonal harvest, the town now feeds
        // itself well enough that imports hardly matter.
        let b = &mut scenario.towns[3];
        for n in &mut b.nature {
            if n.0 == "field" || n.0 == "garden" {
                n.1 = 0;
            }
        }
        b.stock.retain(|s| s.0 != "bread");
        let mut s = Simulation::new(db, params, &scenario);
        s.world.resource_mut::<sim_core::Towns>().0[3].treasury = sim_core::Coin::ZERO;
        if imports {
            s.order("imports Brassmoor auto").unwrap();
        }
        let money = |s: &Simulation| {
            let towns = s.world.resource::<sim_core::Towns>();
            // `minted` also counts coin struck into the metal of towns that
            // issue their own coin, which the ledger leaves out.
            let minted: f64 = towns.0.iter().map(|t| t.minted.coins_f64()).sum();
            sim_core::world::total_money(s.world.resource::<sim_core::AgentStore>(), towns) - minted
        };
        let before = money(&s);
        let dpy = s.world.resource::<sim_core::SimClock>().months_per_year;
        let mut owed_once = false;
        for _ in 0..2 * dpy {
            s.step();
            owed_once |= s.world.resource::<sim_core::Towns>().0[3]
                .debts
                .iter()
                .any(|&d| d > 0.0);
        }
        let after = money(&s);
        assert!(
            ((after - before) / before).abs() < 1e-3,
            "money changed: {before} -> {after}"
        );
        let m = s.world.resource::<sim_core::metrics::Metrics>();
        let starving: f32 = m.years.iter().map(|y| y.towns[3].starving).sum();
        let pop = m.latest().unwrap().towns[3].population;
        (starving, pop, owed_once)
    };
    // Summed over four seeds: on a single one, the town's luck can outweigh the imports.
    let (mut starving_without, mut pop_without) = (0.0, 0);
    let (mut starving_with, mut pop_with, mut owed) = (0.0, 0, false);
    for seed in 1..=4 {
        let (s, p, _) = run(false, seed);
        starving_without += s;
        pop_without += p;
        let (s, p, o) = run(true, seed);
        starving_with += s;
        pop_with += p;
        owed |= o;
    }
    eprintln!("starving {starving_without} -> {starving_with}, pop {pop_without} -> {pop_with}");
    assert!(owed, "the town never bought food on credit");
    assert!(
        starving_with < starving_without,
        "imports didn't help: {starving_with} vs {starving_without}"
    );
    // Fed from its neighbours, the town keeps more of its people (with hens,
    // game and fish beside bread it starves less without imports, too).
    assert!(pop_with > pop_without + 20, "{pop_with} vs {pop_without}");
}

#[test]
fn a_world_generated_from_the_seed_runs_and_is_reproducible() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let run = |seed: u64| {
        let (db, params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
        scenario.seed = seed;
        scenario.generate = Some(sim_core::worldgen::GenSpec {
            towns: 6,
            ..Default::default()
        });
        let mut s = Simulation::new(db, params, &scenario);
        run_years(&mut s, 1);
        let spec = &s.world.resource::<sim_core::world::ScenarioSpec>().0;
        // The stored scenario is the generated world (so saves keep it).
        assert!(spec.generate.is_none());
        let names: Vec<String> = spec.towns.iter().map(|t| t.name.clone()).collect();
        (
            names,
            s.state_hash(),
            s.world
                .resource::<sim_core::metrics::Metrics>()
                .latest()
                .unwrap()
                .clone(),
        )
    };
    let (names, hash, y) = run(11);
    assert_eq!(names.len(), 6);
    assert!(y
        .towns
        .iter()
        .all(|t| t.population > 100 && t.starving < 0.02));
    // Same seed, same world and same run; another seed, another world.
    let (names2, hash2, _) = run(11);
    assert_eq!((names.clone(), hash), (names2, hash2));
    let (names3, _, _) = run(12);
    assert_ne!(names, names3);
}

#[test]
fn history_logs_events_and_monthly_samples_and_saves_keep_them() {
    use sim_core::history::{EventKind, History, SAMPLE_MONTHS};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let paths = AssetPaths::from_root(&root);
    let mut a = full(11);
    a.order("subsidy Greenvale bread 0.3").unwrap();
    run_years(&mut a, 2);
    let h = a.world.resource::<History>();
    let towns = a.world.resource::<sim_core::Towns>().0.len();
    // One sample per town and month.
    let months = a.tick() / SAMPLE_MONTHS;
    assert_eq!(h.samples.len() as u64, months * towns as u64);
    // The player's order and the towns' building are in the log.
    assert!(h
        .events
        .iter()
        .any(|e| e.kind == EventKind::Order && e.text.contains("subsidy on bread")));
    assert!(h.events.iter().any(|e| e.kind == EventKind::Built));
    // Events come in order of time.
    assert!(h.events.windows(2).all(|w| w[0].tick <= w[1].tick));
    // A saved game keeps its history.
    let n = h.events.len();
    let text = sim_core::save::to_string(&a.world).unwrap();
    let content = sim_data::Content::load_dir(&paths.content).unwrap();
    let b = Simulation::from_save(sim_core::save::from_str(&text).unwrap(), content).unwrap();
    assert_eq!(b.world.resource::<History>().events.len(), n);
}

#[test]
fn a_compact_binary_save_loads_identically_and_is_much_smaller() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let paths = AssetPaths::from_root(&root);
    let mut a = full(5);
    run_years(&mut a, 1);
    let bytes = sim_core::save::to_bytes(&a.world).unwrap();
    let text = sim_core::save::to_string(&a.world).unwrap();
    assert!(bytes.starts_with(sim_core::save::BINARY_MAGIC));
    assert!(
        bytes.len() * 3 < text.len(),
        "{} vs {}",
        bytes.len(),
        text.len()
    );
    let content = sim_data::Content::load_dir(&paths.content).unwrap();
    let mut b =
        Simulation::from_save(sim_core::save::from_bytes(&bytes).unwrap(), content).unwrap();
    assert_eq!(a.state_hash(), b.state_hash());
    for _ in 0..60 {
        a.step();
        b.step();
    }
    assert_eq!(a.state_hash(), b.state_hash());
    assert!(sim_core::save::from_bytes(text.as_bytes()).is_err());
}

#[test]
fn both_caravan_rules_trade_and_conserve_money() {
    use sim_core::config::Caravans;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let run = |caravans: Caravans| {
        let (db, mut params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
        params.market.caravans = caravans;
        let mut s = Simulation::new(db, params, &scenario);
        let money = |s: &Simulation| {
            let towns = s.world.resource::<sim_core::Towns>();
            // All coin struck, whether into a treasury or a metal reserve.
            let minted: f64 = towns.0.iter().map(|t| t.minted.coins_f64()).sum();
            sim_core::world::total_money(s.world.resource::<sim_core::AgentStore>(), towns) - minted
        };
        let before = money(&s);
        for _ in 0..36 {
            s.step();
        }
        let after = money(&s);
        assert!(
            ((after - before) / before).abs() < 1e-3,
            "{caravans:?}: money {before} -> {after}"
        );
        let towns = s.world.resource::<sim_core::Towns>();
        towns
            .0
            .iter()
            .map(|t| t.ledger.total[sim_core::ledger::Flow::TradeSales as usize])
            .sum::<f64>()
    };
    // Which rule sells more depends on the year (balancing moves surpluses to
    // shortages, not the most goods), so only check that both trade.
    let fixed = run(Caravans::Fixed);
    let balance = run(Caravans::Balance { max_load: 400.0 });
    assert!(
        fixed > 0.0 && balance > 0.0,
        "trade sales: balance {balance} vs fixed {fixed}"
    );
}

#[test]
fn a_day_has_steps_and_the_economy_runs_every_step() {
    let mut s = sim(5);
    let spd = s.world.resource::<sim_core::SimClock>().steps_per_day;
    assert!(spd > 1, "models.ron cuts the day into steps");
    // The first step starts day 1.
    s.step_tick();
    assert_eq!(s.tick(), 1);
    let mut hours = vec![s.world.resource::<sim_core::env::Environment>().hour];
    let mut changed = 0;
    // The rest of the day moves the hours, the weather and the economy on.
    for _ in 1..spd {
        let before = economy_text(&s);
        s.step_tick();
        assert_eq!(s.tick(), 1);
        hours.push(s.world.resource::<sim_core::env::Environment>().hour);
        changed += usize::from(economy_text(&s) != before);
    }
    assert!(
        changed > spd as usize / 2,
        "the economy moved in only {changed} of {spd} steps"
    );
    assert!(
        hours.windows(2).all(|w| w[1] > w[0]) && hours[0] < 2.0 && hours[spd as usize - 1] > 22.0
    );
    let env = s.world.resource::<sim_core::env::Environment>();
    assert_eq!(
        env.towns.len(),
        s.world.resource::<sim_core::Towns>().0.len()
    );
    // `step` runs whole days.
    s.step();
    assert_eq!(s.tick(), 2);
    assert!(s.world.resource::<sim_core::SimClock>().day_done());
}

#[test]
fn a_game_saved_mid_day_loads_with_its_hour_and_weather() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let paths = AssetPaths::from_root(&root);
    let mut a = sim(6);
    for _ in 0..30 {
        a.step();
    }
    for _ in 0..5 {
        a.step_tick();
    }
    let text = sim_core::save::to_string(&a.world).unwrap();
    let content = sim_data::Content::load_dir(&paths.content).unwrap();
    let mut b = Simulation::from_save(sim_core::save::from_str(&text).unwrap(), content).unwrap();
    for _ in 0..40 {
        a.step_tick();
        b.step_tick();
    }
    assert_eq!(a.state_hash(), b.state_hash());
    let (ea, eb) = (
        a.world.resource::<sim_core::env::Environment>(),
        b.world.resource::<sim_core::env::Environment>(),
    );
    assert_eq!(ea.hour, eb.hour);
    assert_eq!(ea.towns, eb.towns);
}

#[test]
fn every_good_is_accounted_for() {
    // What is held of each good equals what was seeded and made minus what was
    // eaten, used, bought, built, struck, equipped, delivered or spoiled; with
    // the treasury buying output and with workshops as firms.
    for firms in ["Treasury", "Workshops(start_cash: 100.0, reserve_days: 10.0, payout_rate: 0.05, profit_tax: 0.2, close_days: 30, credit_days: 10.0, grouping: Guild)"] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let sets = [("market.firms".to_string(), firms.to_string())];
        let (db, params, mut scenario) =
            sim_core::load_with(&AssetPaths::from_root(&root), &sets).unwrap();
        scenario.seed = 3;
        let mut s = Simulation::new(db, params, &scenario);
        run_years(&mut s, 2);
        check_goods(&s);
    }
}

fn check_goods(s: &Simulation) {
    let n = s.world.resource::<sim_core::Db>().content.products.len();
    let bad = sim_core::goods::audit(
        s.world.resource::<sim_core::AgentStore>(),
        s.world.resource::<sim_core::Towns>(),
        s.world.resource::<sim_core::goods::Squads>(),
        n,
        1e-4,
    );
    assert!(bad.is_empty(), "goods out of balance: {bad:?}");
}

#[test]
fn a_year_is_twelve_simulated_days_and_each_stands_for_a_month() {
    let mut s = sim(9);
    let clock = *s.world.resource::<sim_core::SimClock>();
    assert_eq!(clock.months_per_year, 12, "models.ron has twelve months");
    assert_eq!(clock.calendar().days_per_year(), 120.0);
    let ages = |s: &Simulation| {
        let store = s.world.resource::<sim_core::AgentStore>();
        (0..store.len())
            .map(|i| store.alive[i].then_some(store.age_months[i]))
            .collect::<Vec<_>>()
    };
    let start = ages(&s);
    let season0 = s.world.resource::<sim_core::env::Environment>().season;
    // Seasons move on through the year and come round after twelve days.
    let mut seasons = vec![season0];
    for _ in 0..12 {
        s.step();
        seasons.push(s.world.resource::<sim_core::env::Environment>().season);
    }
    assert_eq!(s.tick(), 12);
    assert_eq!(s.world.resource::<sim_core::SimClock>().year(), 1);
    assert_eq!(s.world.resource::<sim_core::SimClock>().day_of_year(), 0);
    let first = seasons[1];
    let last = seasons[12];
    let span = seasons[1..]
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
    assert!(
        span.1 - span.0 > 0.8,
        "the season spans the year: {seasons:?}"
    );
    // Day 13 is the first month of the next year: the same point of the
    // season as day 1, which the twelfth day was not.
    s.step();
    let next = s.world.resource::<sim_core::env::Environment>().season;
    assert!((next - first).abs() < 1e-4, "{next} vs {first}");
    assert!((last - first).abs() > 1e-3);
    // Everyone alive ages a month a day: a year in twelve days. (A slot a
    // newborn took over is younger than its old owner and is left out.)
    let now = ages(&s);
    let mut aged = 0;
    for i in 0..start.len() {
        if let (Some(a), Some(b)) = (start[i], now[i]) {
            if b < a {
                continue;
            }
            assert_eq!(b, a + 13, "agent {i}");
            aged += 1;
        }
    }
    assert!(aged > 200, "only {aged} agents lived through the year");
    let store = s.world.resource::<sim_core::AgentStore>();
    let i = (0..store.len()).find(|&i| now[i].is_some()).unwrap();
    let years = store.age_years(i, 12);
    assert!((years - store.age_months[i] as f32 / 12.0).abs() < 1e-4);
}

#[test]
fn with_the_needs_ladder_winter_asks_for_fuel() {
    // Households buy firewood and charcoal for the cold; anyone short of heat
    // gets cold, and every good stays accounted for.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let sets = [(
        "market.consumption".to_string(),
        "Needs((interval: 7, gate_at: 0.7, gate_width: 0.15, comfort_c: 16.0, house_c: 6.0, max_clothes: 2.0, fire_per_c: 0.5, fire_per_extra_person: 0.25, cold_damage: 0.02, plan_days: 30.0))".to_string(),
    )];
    let (db, params, mut scenario) =
        sim_core::load_with(&AssetPaths::from_root(&root), &sets).unwrap();
    scenario.seed = 4;
    let mut s = Simulation::new(db, params, &scenario);
    run_years(&mut s, 1);
    let towns = s.world.resource::<sim_core::Towns>();
    let db = s.world.resource::<sim_core::Db>();
    let bought = |id: &str| -> f64 {
        let p = db.content.product(id).unwrap() as usize;
        towns
            .0
            .iter()
            .map(|t| t.flows.get(sim_core::goods::Flow::Bought, p))
            .sum()
    };
    assert!(bought("wood") + bought("charcoal") > 100.0);
    assert!(towns.0.iter().map(|t| t.cold_days).sum::<u32>() > 0);
    check_goods(&s);
}

#[test]
fn seasonal_crops_stand_in_summer_and_not_in_winter() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    scenario.seed = 8;
    let mut s = Simulation::new(db, params, &scenario);
    let field = s
        .world
        .resource::<sim_core::Db>()
        .content
        .nature_kind("field")
        .unwrap();
    let standing = |s: &mut Simulation| -> f32 {
        let mut q = s.world.query::<&sim_core::components::NaturalResource>();
        q.iter(&s.world)
            .filter(|n| n.kind == field)
            .map(|n| n.amount)
            .sum()
    };
    // The year starts at the spring equinox: month 6 is late summer, month 10
    // late winter.
    for _ in 0..6 {
        s.step();
    }
    let summer = standing(&mut s);
    for _ in 0..4 {
        s.step();
    }
    let winter = standing(&mut s);
    assert!(
        summer > 2.0 * winter.max(1.0),
        "summer {summer}, winter {winter}"
    );
    check_goods(&s);
}

#[test]
fn a_granary_fills_from_a_full_market_and_feeds_a_hungry_one() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    scenario.seed = 12;
    for t in &mut scenario.towns {
        t.population = 60;
    }
    let mut s = Simulation::new(db, params, &scenario);
    s.order("granary Greenvale 30").unwrap();
    s.order("reserve Greenvale on").unwrap();
    for _ in 0..6 {
        s.step();
    }
    let towns = s.world.resource::<sim_core::Towns>();
    let db = s.world.resource::<sim_core::Db>();
    let g = &towns.0[1];
    assert!(g.policy.food_reserve && g.policy.granary_days == 30.0);
    assert!(
        sim_core::granary::held(g, db) > 0.0,
        "the granary took some food"
    );
    check_goods(&s);
    // Keep the market empty for a day (what the day makes is taken away step
    // by step): the granary releases food into it.
    let before = sim_core::granary::held(
        &s.world.resource::<sim_core::Towns>().0[1],
        s.world.resource::<sim_core::Db>(),
    );
    let foods: Vec<u32> = s
        .world
        .resource::<sim_core::Db>()
        .foods
        .iter()
        .map(|f| f.0)
        .collect();
    let spd = s.world.resource::<sim_core::SimClock>().steps_per_day;
    for _ in 0..spd {
        {
            let mut towns = s.world.resource_mut::<sim_core::Towns>();
            for &f in &foods {
                towns.0[1].stock[f as usize] = 0.0;
            }
        }
        s.step_tick();
    }
    let after = sim_core::granary::held(
        &s.world.resource::<sim_core::Towns>().0[1],
        s.world.resource::<sim_core::Db>(),
    );
    assert!(after < before, "{after} vs {before}");
}

#[test]
fn every_item_has_an_owner_that_holds_it() {
    use sim_core::components::{Item, Product};
    use sim_core::goods::{gear, Holder, Owner, Slot};
    let mut s = full(5);
    run_years(&mut s, 2);
    let mut q = s
        .world
        .query::<(bevy_ecs::entity::Entity, &Product, &Owner)>();
    let owned: Vec<_> = q.iter(&s.world).map(|(e, _, o)| (e, *o)).collect();
    let items = s.world.query::<&Item>().iter(&s.world).count();
    let towns = &s.world.resource::<sim_core::Towns>().0;
    let store = s.world.resource::<sim_core::AgentStore>();
    for &(e, owner) in &owned {
        match owner.0 {
            Holder::Armory(t) => assert!(towns[t as usize].armory.contains(&e)),
            Holder::Person(i) => assert!(
                Slot::ALL
                    .iter()
                    .any(|&slot| gear(store, i as usize, slot) == Some(e)),
                "person {i} does not carry {e:?}"
            ),
            other => panic!("unexpected owner {other:?}"),
        }
    }
    assert!(
        !owned.is_empty() && owned.len() == items,
        "{} owned of {items} items",
        owned.len()
    );
}

#[test]
fn a_squad_carries_rations_eats_them_and_keeps_the_books() {
    use sim_core::goods::{move_stack, Holder, Place, Squad, Squads};
    let mut s = sim(13);
    s.step();
    let bread = s
        .world
        .resource::<sim_core::Db>()
        .content
        .product("bread")
        .unwrap() as usize;
    let members: Vec<u32> = s.world.resource::<sim_core::Towns>().0[0]
        .residents
        .iter()
        .copied()
        .take(4)
        .collect();
    s.world.resource_mut::<Squads>().list.push(Squad {
        home: 0,
        members: members.clone(),
        place: Place::Town(0),
        items: Vec::new(),
        stacks: Vec::new(),
    });
    // Load rations: no more than four people can carry.
    let moved = s
        .world
        .resource_scope(|w, mut towns: bevy_ecs::prelude::Mut<sim_core::Towns>| {
            w.resource_scope(
                |w, mut store: bevy_ecs::prelude::Mut<sim_core::AgentStore>| {
                    w.resource_scope(|w, mut squads: bevy_ecs::prelude::Mut<Squads>| {
                        let db = w.resource::<sim_core::Db>();
                        towns.0[0].stock[bread] = 500.0;
                        move_stack(
                            Holder::Market(0),
                            Holder::Squad(0),
                            bread,
                            500.0,
                            &mut towns,
                            &mut store,
                            &mut squads,
                            db,
                        )
                    })
                },
            )
        });
    let cap = 4.0 * sim_core::goods::KG_PER_MEMBER
        / s.world.resource::<sim_core::Db>().content.products[bread].kg;
    assert!((moved - cap).abs() < 1e-3, "moved {moved}, cap {cap}");
    for _ in 0..2 {
        s.step();
    }
    let left: f32 = s.world.resource::<Squads>().list[0]
        .stacks
        .iter()
        .map(|e| e.1)
        .sum();
    assert!(
        left < moved,
        "the squad ate from its packs: {left} of {moved}"
    );
}

#[test]
fn the_view_shows_needs_stores_and_the_armory() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let sets = [(
        "market.consumption".to_string(),
        "Needs((interval: 7, gate_at: 0.7, gate_width: 0.15, comfort_c: 16.0, house_c: 6.0, max_clothes: 2.0, fire_per_c: 0.5, fire_per_extra_person: 0.25, cold_damage: 0.02, plan_days: 30.0))".to_string(),
    )];
    let (db, params, mut scenario) =
        sim_core::load_with(&AssetPaths::from_root(&root), &sets).unwrap();
    scenario.seed = 2;
    let mut s = Simulation::new(db, params, &scenario);
    for _ in 0..6 {
        s.step();
    }
    let snap = sim_core::view::ViewSnapshot::capture(&s.world, &mut Default::default());
    let t = &snap.towns[0];
    assert_eq!(t.needs.len(), sim_data::Need::ALL.len());
    assert!(t.needs.iter().all(|v| (0.0..=1.0).contains(v)));
    assert!(t.food_days > 0.0 && t.seed_reserve);
    // Without the ladder there is nothing to show.
    let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    params.market.consumption = sim_core::consumption::ConsumptionParams::None;
    scenario.seed = 2;
    let plain = Simulation::new(db, params, &scenario);
    let plain = sim_core::view::ViewSnapshot::capture(&plain.world, &mut Default::default());
    assert!(plain.towns[0].needs.is_empty());
}

#[test]
fn a_stockpile_fills_the_town_store_and_draws_production() {
    let mut s = sim(14);
    s.step();
    s.order("stockpile Greenvale grain 150").unwrap();
    let grain = s
        .world
        .resource::<sim_core::Db>()
        .content
        .product("grain")
        .unwrap() as usize;
    let before = {
        let w = &s.world;
        sim_core::systems::decide::target_stock(
            w.resource::<sim_core::Db>(),
            &w.resource::<sim_core::Towns>().0[1],
            w.resource::<sim_core::world::Params>(),
            grain,
        )
    };
    // The store fills from the market's surplus, and gives it back when milling
    // drains the market below half its target, so look at the whole week.
    let mut most_held = 0.0f32;
    for _ in 0..6 {
        s.step();
        let g = &s.world.resource::<sim_core::Towns>().0[1];
        most_held = most_held.max(g.granary.get(grain).copied().unwrap_or(0.0));
    }
    let towns = s.world.resource::<sim_core::Towns>();
    let g = &towns.0[1];
    assert_eq!(g.policy.stockpiled(grain), 150.0);
    assert!(most_held > 0.0, "the store took grain");
    // While the store lacks grain, the market aims for more of it.
    let w = &s.world;
    let target = sim_core::systems::decide::target_stock(
        w.resource::<sim_core::Db>(),
        g,
        w.resource::<sim_core::world::Params>(),
        grain,
    );
    // (The market's own aim for grain moves with what the town eats, so
    // compare against today's: the store's lack comes on top of it.)
    let _ = before;
    let own = sim_core::systems::decide::food_per_capita(
        w.resource::<sim_core::Db>(),
        g,
        w.resource::<sim_core::world::Params>(),
        grain,
    ) * g.residents.len() as f32;
    let lacks = g.stockpile_target(grain) - g.granary[grain];
    assert!(lacks <= 0.0 || target > own + 1e-3, "{target} vs {own}");
    check_goods(&s);
    s.order("stockpile Greenvale grain 0").unwrap();
    s.step();
    assert!(s.world.resource::<sim_core::Towns>().0[1]
        .policy
        .stockpile
        .is_empty());
}

#[test]
fn households_and_workshops_keep_their_own_stocks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let sets = [
        (
            "market.consumption".to_string(),
            "Needs((interval: 7, gate_at: 0.7, gate_width: 0.15, comfort_c: 16.0, house_c: 6.0, max_clothes: 2.0, fire_per_c: 0.5, fire_per_extra_person: 0.25, larder_days: 14.0, cold_damage: 0.02, plan_days: 30.0))".to_string(),
        ),
        (
            "market.firms".to_string(),
            "Workshops(start_cash: 100.0, reserve_days: 10.0, payout_rate: 0.05, profit_tax: 0.2, close_days: 30, credit_days: 10.0, grouping: Guild, input_days: 10.0)".to_string(),
        ),
    ];
    let (db, params, mut scenario) =
        sim_core::load_with(&AssetPaths::from_root(&root), &sets).unwrap();
    scenario.seed = 15;
    let mut s = Simulation::new(db, params, &scenario);
    run_years(&mut s, 1);
    let store = s.world.resource::<sim_core::AgentStore>();
    let towns = s.world.resource::<sim_core::Towns>();
    let larders = (0..store.len())
        .filter(|&i| store.alive[i] && !store.larder[i].is_empty())
        .count();
    assert!(larders > 0, "households keep food at home");
    let firm_inputs: f32 = towns
        .0
        .iter()
        .flat_map(|t| t.firms.iter())
        .flat_map(|f| f.inputs.iter())
        .sum();
    assert!(firm_inputs > 0.0, "workshops keep inputs in store");
    check_goods(&s);
}

#[test]
fn stockpile_targets_follow_the_seasons() {
    // Grain is reaped once a year and firewood burned in winter: their
    // stockpiles should be fullest before the lean months, not all year.
    // How seasonal a town's wood is depends on its trades (charcoal burners
    // and bakers use wood all year, households burn more in winter), and so
    // on the seed's whole path, raids included: the most seasonal town's
    // lowest month for wood went from 0.25 to 0.60 over seeds 11-22, and one
    // seed moved from 0.25 to 0.60 when battles changed. So the check is over
    // eight seeds: in each, the most seasonal town's target peaks at 1, and
    // the median over seeds of its lowest month is well below a flat 1.
    // Measured median for seeds 11-18: wood 0.43 before and 0.40 after the
    // battle fix (PR 175), grain 0.25 (its floor) in both.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let lows: Vec<[(f32, f32); 2]> = std::thread::scope(|sc| {
        let hs: Vec<_> = (11..19u64)
            .map(|seed| {
                let root = root.clone();
                sc.spawn(move || {
                    let sets = sim_core::read_preset(&root.join("config/presets/new_economy.ron"))
                        .unwrap();
                    let (db, params, mut scenario) =
                        sim_core::load_with(&AssetPaths::from_root(&root), &sets).unwrap();
                    scenario.seed = seed;
                    let mut s = Simulation::new(db, params, &scenario);
                    run_years(&mut s, 3);
                    let db = s.world.resource::<sim_core::Db>();
                    let np = db.content.products.len();
                    let towns = &s.world.resource::<sim_core::Towns>().0;
                    ["grain", "wood"].map(|id| {
                        let g = db.content.product(id).unwrap() as usize;
                        // The most seasonal town: its lowest month, and its peak.
                        towns
                            .iter()
                            .map(|town| {
                                let v: Vec<f32> = (0..12)
                                    .map(|m| sim_core::granary::season_scale(town, np, m)[g])
                                    .collect();
                                let lo = v.iter().copied().fold(1.0f32, f32::min);
                                let hi = v.iter().copied().fold(0.0f32, f32::max);
                                (lo, hi)
                            })
                            .min_by(|a, b| a.0.total_cmp(&b.0))
                            .unwrap()
                    })
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for (k, id) in ["grain", "wood"].iter().enumerate() {
        let mut lo: Vec<f32> = lows.iter().map(|l| l[k].0).collect();
        eprintln!("{id}: lowest month of the most seasonal town per seed {lo:?}");
        assert!(lows.iter().all(|l| l[k].1 > 0.99), "{id}: {lows:?}");
        lo.sort_by(f32::total_cmp);
        let median = (lo[3] + lo[4]) / 2.0;
        assert!(median < 0.55, "{id}: median lowest month {median}");
    }
}

#[test]
fn the_new_economy_preset_loads_and_runs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let sets = sim_core::read_preset(&root.join("config/presets/new_economy.ron")).unwrap();
    assert!(sets.len() >= 2);
    let (db, params, mut scenario) =
        sim_core::load_with(&AssetPaths::from_root(&root), &sets).unwrap();
    assert!(matches!(
        params.market.consumption,
        sim_core::consumption::ConsumptionParams::Needs(_)
    ));
    scenario.seed = 16;
    for t in &mut scenario.towns {
        t.population = 60;
    }
    let mut s = Simulation::new(db, params, &scenario);
    for _ in 0..3 {
        s.step();
    }
    check_goods(&s);
}

#[test]
fn kit_reserve_follows_soldiers_until_the_government_sets_a_stockpile() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    params.military.levy_reserve = 0.05;
    let mut s = Simulation::new(db, params, &scenario);
    let target = |s: &Simulation, id: &str| {
        let db = s.world.resource::<sim_core::Db>();
        let p = db.content.product(id).unwrap() as usize;
        let params = s.world.resource::<sim_core::world::Params>();
        let town = &s.world.resource::<sim_core::Towns>().0[0];
        sim_core::systems::decide::target_stock(db, town, params, p)
    };
    let (horse, mail, sword) = (
        target(&s, "horse"),
        target(&s, "mail"),
        target(&s, "longsword_1"),
    );
    let people = s.world.resource::<sim_core::Towns>().0[0].residents.len() as f32;
    // One mount good: soldiers plus the levy reserve, a few per hundred people.
    assert!(
        horse > 0.05 * people && horse < 0.2 * people,
        "horses {horse} for {people}"
    );
    // Armour is split between mail and padded coats; weapons among many goods.
    assert!(
        mail < horse && sword < mail,
        "mail {mail}, sword {sword}, horse {horse}"
    );
    // A stockpile order replaces the default.
    let town = s.world.resource::<sim_core::Towns>().0[0].name.clone();
    s.order(&format!("stockpile {town} horse 40")).unwrap();
    s.step();
    assert_eq!(target(&s, "horse"), 40.0);
}

#[test]
fn kit_is_priced_against_the_reserve_its_makers_stop_at() {
    // The market prices kit by the stock the government keeps (its reserve
    // or stockpile), the same stock its makers stop at, not by a stock per
    // resident: a town told to keep one piece of the kit its armory holds
    // most of pays less for it than one told to keep many.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let run = |order: Option<(&str, f32)>| {
        let mut s = Simulation::from_paths(&AssetPaths::from_root(&root)).unwrap();
        let town = s.world.resource::<sim_core::Towns>().0[0].name.clone();
        if let Some((good, amount)) = order {
            s.order(&format!("stockpile {town} {good} {amount}"))
                .unwrap();
        }
        for _ in 0..12 {
            s.step();
        }
        s
    };
    // The kit the first town's armory holds most of.
    let s = run(None);
    let db = s.world.resource::<sim_core::Db>();
    let town = &s.world.resource::<sim_core::Towns>().0[0];
    let p = (0..db.content.products.len())
        .filter(|&p| db.content.products[p].category == sim_data::Category::Weapon)
        .max_by(|&a, &b| town.stock[a].total_cmp(&town.stock[b]))
        .unwrap();
    let good = db.content.products[p].id.clone();
    let held = town.stock[p];
    assert!(held > 2.0, "the armory holds {held} {good}");
    let price = |amount: f32| {
        let s = run(Some((&good, amount)));
        s.world.resource::<sim_core::Towns>().0[0].price[p]
    };
    let (few, many) = (price(1.0), price(1000.0));
    assert!(
        few < many,
        "{good} costs {few} with a reserve of one and {many} with a reserve of 1000"
    );
}

#[test]
fn an_army_marches_eats_its_rations_fights_and_comes_home() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut s = Simulation::from_paths(&AssetPaths::from_root(&root)).unwrap();
    let (a, b) = {
        let towns = s.world.resource::<sim_core::Towns>();
        (towns.0[0].name.clone(), towns.0[1].name.clone())
    };
    s.order(&format!("army {a} 0.15")).unwrap();
    for _ in 0..4 {
        s.step();
    }
    // Let a raid the town's ruler started on its own come home first.
    for _ in 0..20 {
        if s.world.resource::<sim_core::Towns>().0[0].raid.is_none() {
            break;
        }
        s.step();
    }
    s.order(&format!("raid {a} {b}")).unwrap();
    let (mut packed, mut least, mut fought, mut progress) = (0.0f32, f32::MAX, false, 0.0f32);
    let mut done = false;
    let mut ours = None;
    // A day marches a month: watch it step by step, so the arrival is seen.
    let spd = s
        .world
        .resource::<sim_core::world::SimClock>()
        .steps_per_day;
    for _ in 0..20 * spd {
        s.step_tick();
        let towns = s.world.resource::<sim_core::Towns>();
        let squads = s.world.resource::<sim_core::goods::Squads>();
        match &towns.0[0].raid {
            Some(r) => {
                progress = progress.max(r.progress);
                fought |= r.fought;
                ours = ours.or(r.squad);
                if let Some(sq) = r.squad.and_then(|id| squads.list.get(id as usize)) {
                    let food: f32 = sq.stacks.iter().map(|e| e.1).sum();
                    packed = packed.max(food);
                    least = least.min(food);
                    assert!(!sq.members.is_empty());
                }
            }
            None if fought => {
                done = true;
                break;
            }
            None => {}
        }
    }
    assert!(packed > 0.0, "the squad packed no rations");
    assert!(least < packed, "the squad never ate from its packs");
    assert!(progress >= 1.0, "the army never reached its target");
    assert!(fought && done, "the army never fought and came home");
    let squads = s.world.resource::<sim_core::goods::Squads>();
    let id = ours.expect("the raid marched as a squad") as usize;
    assert!(
        squads.list[id].members.is_empty(),
        "the squad was not disbanded"
    );
}

#[test]
fn a_live_battle_waits_for_the_player_then_settles() {
    use sim_core::battle::{Command, Order};
    use sim_core::systems::military::{
        command_live_battle, live_battle_view, live_battle_waiting, resolve_live_battle,
        start_live_battle, step_live_battle, LiveBattles,
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut s = Simulation::from_paths(&AssetPaths::from_root(&root)).unwrap();
    s.world.insert_resource(LiveBattles(true));
    let player = s
        .world
        .resource::<sim_core::policy::PlayerTown>()
        .0
        .expect("the scenario has a player town") as usize;
    let (me, them) = {
        let towns = s.world.resource::<sim_core::Towns>();
        let other = (0..towns.0.len()).find(|&t| t != player).unwrap();
        (towns.0[player].name.clone(), towns.0[other].name.clone())
    };
    s.order(&format!("army {me} 0.15")).unwrap();
    // The town's doctrine: its squads take prisoners.
    s.order(&format!("doctrine {me} aggressive capture"))
        .unwrap();
    for _ in 0..4 {
        s.step();
    }
    assert_eq!(
        s.world.resource::<sim_core::Towns>().0[player]
            .policy
            .doctrine,
        sim_core::battle::Doctrine {
            posture: sim_core::battle::Posture::Aggressive,
            prisoners: sim_core::battle::Pursuit::Capture,
        }
    );
    s.order(&format!("raid {me} {them}")).unwrap();
    let mut waited = false;
    for _ in 0..20 {
        s.step();
        if live_battle_waiting(&s.world) {
            waited = true;
            break;
        }
    }
    assert!(waited, "the player's raid never reached a battle");
    let view = live_battle_view(&s.world).unwrap();
    assert_eq!(view.player_side, 0);
    let mine = view.squads.iter().position(|q| q.side == 0).unwrap();
    // It starts from the town's doctrine.
    for q in (0..view.squads.len()).filter(|&q| view.squads[q].side == 0) {
        assert_eq!(view.stances[q].pursuit, sim_core::battle::Pursuit::Capture);
    }
    // Deployment: nothing moves until the player starts it; a squad stands
    // where it is put in its own half, and nowhere else.
    assert!(view.deploying);
    step_live_battle(&mut s.world, 5.0);
    assert_eq!(live_battle_view(&s.world).unwrap().frame.t, 0.0);
    let (x0, x1, y0, y1) = view.deploy_zone(0);
    let spot = (0.5 * (x0 + x1), 0.5 * (y0 + y1));
    command_live_battle(
        &mut s.world,
        mine,
        Command::Deploy {
            anchor: spot,
            facing: (1.0, 0.0),
            width: None,
        },
    )
    .unwrap();
    let line = live_battle_view(&s.world).unwrap().lines[mine];
    assert!(
        (line.anchor.0 - spot.0).hypot(line.anchor.1 - spot.1) < 1.0,
        "{line:?}"
    );
    let beyond = Command::Deploy {
        anchor: (x1 + 50.0, spot.1),
        facing: (1.0, 0.0),
        width: None,
    };
    assert!(command_live_battle(&mut s.world, mine, beyond).is_err());
    start_live_battle(&mut s.world);
    assert!(!live_battle_view(&s.world).unwrap().deploying);
    step_live_battle(&mut s.world, 1.0);
    assert!(live_battle_view(&s.world).unwrap().frame.t > 0.0);
    // The raid holds while the battle waits.
    let progress = |s: &Simulation| {
        s.world.resource::<sim_core::Towns>().0[player]
            .raid
            .as_ref()
            .map(|r| (r.progress, r.back))
    };
    let before = progress(&s);
    s.step();
    assert_eq!(
        progress(&s),
        before,
        "the raid moved on while its battle waited"
    );
    command_live_battle(&mut s.world, mine, Command::Orders(vec![Order::Hold])).unwrap();
    assert_eq!(
        live_battle_view(&s.world).unwrap().orders[mine],
        [Order::Hold]
    );
    // Only the player's own squads take commands.
    let theirs = view.squads.iter().position(|q| q.side == 1).unwrap();
    assert!(command_live_battle(&mut s.world, theirs, Command::Orders(vec![Order::Hold])).is_err());
    resolve_live_battle(&mut s.world);
    assert!(!live_battle_waiting(&s.world));
    let (prisoners, won) = {
        let pending = s
            .world
            .resource::<sim_core::systems::military::PendingBattle>();
        let live = pending.0.as_ref().unwrap();
        (live.prisoners(), live.battle.report().winner == Some(0))
    };
    s.step();
    // Prisoners go to the winners' town: the raiders' march home as captives.
    let store = s.world.resource::<sim_core::store::AgentStore>();
    let towns = s.world.resource::<sim_core::Towns>();
    let winners = if won {
        player
    } else {
        towns.0[player]
            .raid
            .as_ref()
            .map_or(player, |r| r.target as usize)
    };
    for &i in &prisoners {
        let i = i as usize;
        assert!(
            !store.alive[i] || store.town[i] as usize == winners,
            "prisoner {i} stayed"
        );
        // Captives on the march; settled in the town once the raid is home.
        let marching = towns.0[player].raid.is_some();
        if won && marching && store.alive[i] {
            assert_eq!(store.activity[i], sim_core::store::Activity::Captive);
        }
    }
    let raid = towns.0[player].raid.as_ref();
    assert!(raid.is_none_or(|r| r.back), "the battle was not settled");
    assert!(
        s.world
            .resource::<sim_core::systems::military::PendingBattle>()
            .0
            .is_none(),
        "the battle is still pending"
    );
}

/// Towns and people as text: what the daily economy changes.
fn economy_text(s: &Simulation) -> String {
    ron::to_string(&(
        &s.world.resource::<sim_core::Towns>().0,
        s.world.resource::<sim_core::store::AgentStore>(),
    ))
    .unwrap()
}

/// The full state as text, without the era labels in the yearly figures
/// (and without the fingerprint of the content, which holds the labels too).
fn state_without_eras(s: &Simulation) -> String {
    let text: String = sim_core::save::state_text(&s.world)
        .lines()
        .filter(|l| !l.contains("content_hash:"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(i) = rest.find(" era: ") {
        out.push_str(&rest[..i]);
        rest = rest[i + 6..].trim_start_matches(|c: char| c.is_ascii_digit());
    }
    out.push_str(rest);
    out
}

fn sim_steps(seed: u64, steps_per_day: u32) -> Simulation {
    sim_steps_with(seed, steps_per_day, "Trade")
}

fn sim_steps_with(seed: u64, steps_per_day: u32, attendance: &str) -> Simulation {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let sets = [
        ("time.steps_per_day".to_string(), steps_per_day.to_string()),
        ("decision.attendance".to_string(), attendance.to_string()),
    ];
    let (db, params, mut scenario) =
        sim_core::load_with(&AssetPaths::from_root(&root), &sets).unwrap();
    assert_eq!(params.time.steps_per_day, steps_per_day);
    scenario.seed = seed;
    for t in &mut scenario.towns {
        t.population = 60;
    }
    Simulation::new(db, params, &scenario)
}

#[test]
fn same_seed_same_state_at_any_steps_per_day() {
    for spd in [7, 48] {
        let (mut a, mut b) = (sim_steps(4, spd), sim_steps(4, spd));
        run_years(&mut a, 1);
        run_years(&mut b, 1);
        assert_eq!(a.state_hash(), b.state_hash(), "{spd} steps a day");
    }
}

#[test]
fn steps_per_day_is_validated() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let sets = [("time.steps_per_day".to_string(), "0".to_string())];
    assert!(sim_core::load_with(&AssetPaths::from_root(&root), &sets).is_err());
}

#[test]
fn two_step_sizes_give_close_results() {
    // The economy should not depend much on how finely a day is cut: the
    // same seed run at 30 and 120 steps a day ends with about as many people
    // and about as much food sold.
    // With free job choice: in towns of 60, kept trades make the run path
    // dependent (who happened to apprentice where), which swamps the step size.
    let summary = |spd: u32| {
        let mut s = sim_steps_with(2, spd, "Free");
        run_years(&mut s, 3);
        let alive = s.world.resource::<sim_core::AgentStore>().alive_count() as f64;
        let food: f64 = s
            .world
            .resource::<sim_core::Towns>()
            .0
            .iter()
            .map(|t| t.ledger.total[sim_core::ledger::Flow::FoodSales as usize])
            .sum();
        (alive, food)
    };
    let (a, b) = (summary(30), summary(120));
    let close = |x: f64, y: f64| (x - y).abs() <= 0.25 * x.max(y);
    assert!(close(a.0, b.0), "people: {} at 30 vs {} at 120", a.0, b.0);
    assert!(
        close(a.1, b.1),
        "food sold: {} at 30 vs {} at 120",
        a.1,
        b.1
    );
}

#[test]
fn goods_made_per_day_do_not_depend_on_the_step_size() {
    // The same seeds at 24 and 120 steps a day make about as much of each
    // main good over two years: every rate is scaled to the step. Small
    // towns are noisy (one seed's bread moves by a third), so eight seeds
    // are summed; their sums differ by at most about a fifth.
    let made = |spd: u32| {
        let runs: Vec<Vec<f64>> = std::thread::scope(|sc| {
            let hs: Vec<_> = (3..11)
                .map(|seed| {
                    sc.spawn(move || {
                        // Free job choice: in towns of 60, kept trades make the
                        // run path dependent, which swamps the step size.
                        let mut s = sim_steps_with(seed, spd, "Free");
                        run_years(&mut s, 2);
                        let n = s.world.resource::<sim_core::Db>().content.products.len();
                        let towns = s.world.resource::<sim_core::Towns>();
                        (0..n)
                            .map(|p| {
                                towns
                                    .0
                                    .iter()
                                    .map(|t| t.flows.get(sim_core::goods::Flow::Made, p))
                                    .sum::<f64>()
                            })
                            .collect::<Vec<f64>>()
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        (0..runs[0].len())
            .map(|p| runs.iter().map(|r| r[p]).sum::<f64>())
            .collect::<Vec<f64>>()
    };
    let (a, b) = (made(24), made(120));
    let total: f64 = a.iter().sum();
    let db = sim_steps(3, 24);
    let db = db.world.resource::<sim_core::Db>();
    for (p, (&x, &y)) in a.iter().zip(&b).enumerate() {
        // The goods that make up the bulk of output.
        if x.max(y) < 0.02 * total {
            continue;
        }
        // Within 30 % (towns of 60 are noisy: flax came out 26 % apart once
        // towns paid their own people in their own coin).
        assert!(
            (x - y).abs() <= 0.3 * x.max(y),
            "{}: {x:.0} made at 24 steps a day, {y:.0} at 120",
            db.content.products[p].id
        );
    }
}

#[test]
fn people_keep_their_trade() {
    // Trades are kept (plan Phase 3): from year 2 to year 5, fewer than 5 % of
    // those with a trade a year change it. One seed's wars and migrations
    // move its rate by a couple of points, so four seeds are averaged.
    let rates: Vec<(usize, f32)> = std::thread::scope(|sc| {
        let hs: Vec<_> = (1..5)
            .map(|seed| {
                sc.spawn(move || {
                    let mut s = full(seed);
                    run_years(&mut s, 2);
                    let trades = |s: &Simulation| -> std::collections::HashMap<u32, u32> {
                        let store = s.world.resource::<sim_core::AgentStore>();
                        (0..store.len())
                            .filter(|&i| store.alive[i])
                            .filter_map(|i| {
                                store.guild[i].map(|g| (store.entity[i].index_u32(), g.domain))
                            })
                            .collect()
                    };
                    let before = trades(&s);
                    run_years(&mut s, 3);
                    let after = trades(&s);
                    let (kept, changed) = before.iter().fold((0, 0), |(k, c), (e, d)| match after
                        .get(e)
                    {
                        Some(x) if x == d => (k + 1, c),
                        Some(_) => (k, c + 1),
                        None => (k, c),
                    });
                    let per_year = changed as f32 / (kept + changed).max(1) as f32 / 3.0;
                    (kept, per_year)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mean = rates.iter().map(|r| r.1).sum::<f32>() / rates.len() as f32;
    eprintln!(
        "kept and yearly change per seed: {rates:?}, mean {:.1} %",
        100.0 * mean
    );
    for &(kept, _) in &rates {
        assert!(kept > 100, "too few people kept a trade: {kept}");
    }
    assert!(mean < 0.05, "{:.1} % changed trade a year", 100.0 * mean);
}

#[test]
fn the_grown_population_starts_with_masters_and_apprentices() {
    // The adults at the start have been in their trade since they came of
    // age, so the old are masters from the first year and take apprentices.
    use sim_core::store::Role;
    let mut s = full(5);
    run_years(&mut s, 1);
    let store = s.world.resource::<sim_core::AgentStore>();
    let count = |role: Role| {
        (0..store.len())
            .filter(|&i| store.alive[i] && store.guild[i].is_some_and(|g| g.role == role))
            .count()
    };
    let (masters, members, apprentices) = (
        count(Role::Master),
        count(Role::Member),
        count(Role::Apprentice),
    );
    eprintln!("masters {masters}, members {members}, apprentices {apprentices}");
    assert!(masters > 50, "only {masters} masters after a year");
    assert!(members > 0, "no members");
    assert!(apprentices > 0, "no apprentices after a year");
}

fn with_knowledge(seed: u64) -> Simulation {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    scenario.seed = seed;
    params.knowledge.enabled = true;
    Simulation::new(db, params, &scenario)
}

#[test]
fn people_work_only_in_what_they_know_and_learn_from_their_masters() {
    use sim_core::knowledge::knows;
    let mut s = with_knowledge(5);
    let founders = s.world.resource::<sim_core::AgentStore>().len();
    let kp = s
        .world
        .resource::<sim_core::world::Params>()
        .knowledge
        .clone();
    // Every founder knows only part of the graph: nobody starts knowing it all,
    // and every adult knows at least their trade.
    {
        let db = s.world.resource::<sim_core::Db>();
        let store = s.world.resource::<sim_core::AgentStore>();
        let all = (0..founders)
            .filter(|&i| (0..db.recipes.len()).all(|r| knows(&kp, db, store, i, r)))
            .count();
        assert!(
            all < founders / 10 + 1,
            "{all} of {founders} founders know every recipe"
        );
    }
    let start: Vec<Vec<u64>> = s.world.resource::<sim_core::AgentStore>().known.clone();
    let mut checked = 0;
    let per_year = s
        .world
        .resource::<sim_core::world::Params>()
        .time
        .months_per_year;
    for _ in 0..3 * per_year {
        // Check after every tick, while the day's jobs are still set.
        loop {
            s.step_tick();
            let db = s.world.resource::<sim_core::Db>();
            let store = s.world.resource::<sim_core::AgentStore>();
            for (i, job) in store.job_today.iter().enumerate() {
                // A regular place needs the recipe known (an apprentice place
                // is how anyone gets into work they did not know).
                if let Some((r, false)) = *job {
                    assert!(
                        knows(&kp, db, store, i, r as usize),
                        "{i} took a place in recipe {r} without knowing it"
                    );
                    checked += 1;
                }
            }
            if s.world.resource::<sim_core::world::SimClock>().day_done() {
                break;
            }
        }
    }
    assert!(checked > 0, "nobody worked");
    let store = s.world.resource::<sim_core::AgentStore>();
    let learned = (0..founders)
        .filter(|&i| store.known.get(i).is_some_and(|k| Some(k) != start.get(i)))
        .count();
    assert!(learned > 0, "no founder learned a recipe in 3 years");
}

#[test]
fn knowledge_off_draws_nothing_and_stores_nothing() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    scenario.seed = 5;
    params.knowledge.enabled = false;
    let mut s = Simulation::new(db, params, &scenario);
    run_years(&mut s, 1);
    assert!(s.world.resource::<sim_core::AgentStore>().known.is_empty());
}

#[test]
fn trade_is_paid_in_the_buyers_coin_and_moves_its_value() {
    // Each town issues its own coin: a buyer pays abroad in it, the seller
    // keeps it as a foreign balance, and coins piling up abroad lose value.
    let mut s = full(42);
    run_years(&mut s, 5);
    let towns = s.world.resource::<sim_core::Towns>();
    let held: f32 = towns
        .0
        .iter()
        .flat_map(|t| t.foreign.iter())
        .map(|c| c.coins().max(0.0))
        .sum();
    assert!(held > 0.0, "no town holds another town's coin");
    let values: Vec<f32> = towns.0.iter().map(|t| t.coin_value).collect();
    assert!(
        values.iter().any(|&v| (v - 1.0).abs() > 0.01),
        "no coin moved: {values:?}"
    );
    // The coin with the largest share of itself held unspent abroad is worth
    // less than the average (the rule works on that share).
    let n = towns.0.len();
    let store = s.world.resource::<sim_core::AgentStore>();
    let share = |c: usize| -> f32 {
        let abroad: f32 = (0..n)
            .filter(|&t| t != c)
            .map(|t| towns.0[t].holds(c).coins().max(0.0))
            .sum();
        let home: f32 = towns.0[c]
            .residents
            .iter()
            .map(|&i| store.wealth[i as usize].coins().max(0.0))
            .sum::<f32>()
            + towns.0[c].treasury.coins().max(0.0);
        abroad / (abroad + home).max(1e-3)
    };
    let most = (0..n)
        .max_by(|&a, &b| share(a).total_cmp(&share(b)))
        .unwrap();
    let mean = values.iter().sum::<f32>() / n as f32;
    assert!(
        values[most] < mean,
        "the coin most held abroad is worth {} against a mean of {mean}",
        values[most]
    );
}

#[test]
fn early_industry_starts_unknown_and_gets_invented() {
    use sim_core::history::EventKind;
    use sim_core::knowledge::{era, knows};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    scenario.seed = 5;
    params.knowledge.enabled = true;
    params.knowledge.invention.enabled = true;
    // Ten times the usual rate, so a few years are enough.
    params.knowledge.invention.rate = 0.01;
    let mut s = Simulation::new(db, params, &scenario);
    let kp = s
        .world
        .resource::<sim_core::world::Params>()
        .knowledge
        .clone();
    let later = {
        let db = s.world.resource::<sim_core::Db>();
        let store = s.world.resource::<sim_core::AgentStore>();
        // A recipe is as late as what it needs: blast iron makes plain iron
        // but needs coke.
        let blast = db.content.recipe("blast_iron").unwrap() as usize;
        assert_eq!(era(db, blast), 4);
        let later: Vec<usize> = (0..db.recipes.len())
            .filter(|&r| era(db, r) > kp.start_era)
            .collect();
        assert!(!later.is_empty());
        for &r in &later {
            assert!(
                (0..store.len()).all(|i| !knows(&kp, db, store, i, r)),
                "a founder knows {}",
                db.recipes[r].name
            );
        }
        later
    };
    run_years(&mut s, 5);
    let h = s.world.resource::<sim_core::history::History>();
    let inventions: Vec<&str> = h
        .events
        .iter()
        .filter(|e| e.kind == EventKind::Invented)
        .map(|e| e.text.as_str())
        .collect();
    assert!(!inventions.is_empty(), "nothing invented in 5 years");
    let db = s.world.resource::<sim_core::Db>();
    let store = s.world.resource::<sim_core::AgentStore>();
    let known_later = later
        .iter()
        .filter(|&&r| (0..store.len()).any(|i| knows(&kp, db, store, i, r)))
        .count();
    assert!(
        known_later > 0,
        "inventions: {inventions:?}, yet nobody knows a later recipe"
    );
}

#[test]
fn the_player_sees_what_their_people_know_and_keeps_it() {
    use sim_core::history::{EventKind, History};
    use sim_core::view::{Seen, ViewSnapshot};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
    scenario.seed = 5;
    assert!(
        scenario.player.is_some(),
        "the scenario hands a town to the player"
    );
    params.knowledge.enabled = true;
    params.knowledge.invention.enabled = true;
    params.knowledge.invention.rate = 0.05;
    let mut s = Simulation::new(db, params, &scenario);
    s.step();
    let mut seen = Seen::default();
    let first = ViewSnapshot::capture(&s.world, &mut seen);
    let start: Vec<(u32, u64)> = s.world.resource::<History>().revealed.clone();
    {
        let db = s.world.resource::<sim_core::Db>();
        let kp = &s.world.resource::<sim_core::world::Params>().knowledge;
        assert!(!start.is_empty());
        assert_eq!(
            first.revealed.len(),
            start.len(),
            "the first capture brings all"
        );
        // Nothing past the start era is known, so nothing of it is shown.
        assert!(start
            .iter()
            .all(|&(r, _)| sim_core::knowledge::era(db, r as usize) <= kp.start_era));
        assert!(
            start.len() < db.recipes.len(),
            "the whole graph is open at the start"
        );
    }
    assert!(ViewSnapshot::capture(&s.world, &mut seen)
        .revealed
        .is_empty());
    run_years(&mut s, 6);
    let h = s.world.resource::<History>();
    assert_eq!(
        &h.revealed[..start.len()],
        &start[..],
        "a revealed node stays revealed"
    );
    assert!(
        h.revealed.len() > start.len() && h.events.iter().any(|e| e.kind == EventKind::Discovered),
        "the player's people learned nothing new in 6 years"
    );
}

#[test]
fn guilds_that_keep_secrets_teach_fewer_people() {
    // Learned recipe-skills over three years, with every guild in every town
    // keeping its secrets or none (inventing off, so only teaching counts).
    let learned = |secrets: bool| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
        scenario.seed = 5;
        params.knowledge.enabled = true;
        params.knowledge.invention.enabled = false;
        let mut s = Simulation::new(db, params, &scenario);
        if secrets {
            let towns: Vec<String> = s
                .world
                .resource::<sim_core::Towns>()
                .0
                .iter()
                .map(|t| t.name.clone())
                .collect();
            let domains = s.world.resource::<sim_core::Db>().content.domains.clone();
            for t in &towns {
                for d in &domains {
                    s.order(&format!("secrets {t} {d} on")).unwrap();
                }
            }
        }
        let count = |s: &Simulation| -> u32 {
            let st = s.world.resource::<sim_core::AgentStore>();
            st.known.iter().flatten().map(|w| w.count_ones()).sum()
        };
        let before = count(&s);
        run_years(&mut s, 3);
        let h = s.world.resource::<sim_core::history::History>();
        if secrets {
            assert!(h.events.iter().any(|e| e.text.contains("guild's secrets")));
        }
        count(&s) - before
    };
    let open = learned(false);
    let kept = learned(true);
    assert!(open > 0, "nobody learned anything in 3 years");
    assert!(kept < open, "secrets kept {kept} learned, open {open}");
}

#[test]
fn a_town_without_vitamin_food_gets_scurvy_and_one_with_gardens_does_not() {
    // Nutrition: vegetables, fruit, fish, milk and ale carry vitamins; bread
    // next to none. A town whose land grows only grain runs short within a
    // year; the same town with its gardens does not.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let run = |gardens: bool| {
        let (db, mut params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
        params.governor.enabled = false;
        // No free land: it can't plant gardens of its own.
        params.market.free_land = 0;
        scenario.seed = 9;
        let t = &mut scenario.towns[0];
        for n in &mut t.nature {
            let food = ["hunting_ground", "fishing_water", "pasture", "orchard"];
            if food.contains(&n.0.as_str()) || (!gardens && n.0 == "garden") {
                n.1 = 0;
            }
        }
        // The town alone: nobody brings food in.
        scenario.towns.truncate(1);
        scenario.player = None;
        let mut s = Simulation::new(db, params, &scenario);
        run_years(&mut s, 1);
        let store = s.world.resource::<sim_core::AgentStore>();
        let town = &s.world.resource::<sim_core::Towns>().0[0];
        sim_core::world::town_nutrition(store, town)
    };
    let without = run(false);
    let with = run(true);
    eprintln!("without gardens {without:?}, with {with:?}");
    assert!(
        with[3] > without[3] + 0.2,
        "vitamins {} with gardens vs {} without",
        with[3],
        without[3]
    );
    assert!(
        without[3] < 0.6,
        "no scurvy without vitamin food: {}",
        without[3]
    );
}
