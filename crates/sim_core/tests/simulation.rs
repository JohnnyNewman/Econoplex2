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
    let before: f32 = {
        let st = s.world.resource::<sim_core::AgentStore>();
        st.cap.iter().map(|c| c.iter().sum::<f32>()).sum::<f32>() / st.len() as f32
    };
    for _ in 0..240 {
        s.step();
    }
    let log = s.world.resource::<sim_core::world::WorkLog>();
    assert!(
        log.successes > 100,
        "only {} successful steps",
        log.successes
    );
    let st = s.world.resource::<sim_core::AgentStore>();
    let after: f32 = (0..st.len())
        .filter(|&i| st.alive[i])
        .map(|i| st.cap[i].iter().sum::<f32>())
        .sum::<f32>()
        / st.alive_count() as f32;
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
    // Money only grows by what the mints strike.
    let mut s = sim(4);
    let money = |s: &Simulation| {
        let towns = s.world.resource::<sim_core::Towns>();
        let minted: f64 = towns
            .0
            .iter()
            .map(|t| t.ledger.total[sim_core::ledger::Flow::Minted as usize])
            .sum();
        sim_core::world::total_money(s.world.resource::<sim_core::AgentStore>(), towns) - minted
    };
    let before = money(&s);
    for _ in 0..600 {
        s.step();
    }
    let after = money(&s);
    assert!(
        ((after - before) / before).abs() < 1e-3,
        "money changed from {before} to {after}"
    );
}

#[test]
fn first_year_has_no_famine() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut s = Simulation::from_paths(&AssetPaths::from_root(&root)).unwrap();
    let start = s.world.resource::<sim_core::AgentStore>().alive_count();
    let dpy = s.world.resource::<sim_core::SimClock>().days_per_year;
    for _ in 0..dpy {
        s.step();
    }
    let end = s.world.resource::<sim_core::AgentStore>().alive_count();
    assert!(
        end as f32 >= 0.97 * start as f32,
        "population fell from {start} to {end} in the first year"
    );
}

#[test]
fn towns_construct_buildings() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut s = Simulation::from_paths(&AssetPaths::from_root(&root)).unwrap();
    let dpy = s.world.resource::<sim_core::SimClock>().days_per_year;
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
    let dpy = s.world.resource::<sim_core::SimClock>().days_per_year;
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

#[test]
fn households_migrate_and_carry_their_skills() {
    let mut s = sim(7);
    let dpy = s.world.resource::<sim_core::SimClock>().days_per_year;
    let before: Vec<u16> = s.world.resource::<sim_core::AgentStore>().town.clone();
    for _ in 0..5 * dpy {
        s.step();
    }
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
    assert!(arrived > 0, "nobody migrated in 5 years");
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
    let mut s = sim(3);
    let dpy = s.world.resource::<sim_core::SimClock>().days_per_year;
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
    // Mountains of flour and a little wood, no bread: bakers get the wood, and
    // nobody mills more flour.
    for t in &mut s.world.resource_mut::<sim_core::Towns>().0 {
        t.stock[flour] = 1e6;
        t.stock[wood] = 2.0;
        t.stock[bread] = 0.0;
    }
    let before = s.world.resource::<sim_core::world::WorkLog>().runs.clone();
    for _ in 0..5 {
        for t in &mut s.world.resource_mut::<sim_core::Towns>().0 {
            t.stock[wood] = t.stock[wood].min(2.0);
        }
        s.step();
    }
    let after = &s.world.resource::<sim_core::world::WorkLog>().runs;
    let ran = |r: usize| after[r] - before[r];
    assert_eq!(ran(mill), 0, "mills kept making flour nobody needs");
    assert!(ran(bake) > 0, "no bread was baked");
    assert_eq!(
        ran(charcoal) + ran(planks),
        0,
        "other trades took the bakeries' wood"
    );
}

#[test]
fn towns_clear_new_land_up_to_their_limit() {
    let mut s = sim(5);
    let dpy = s.world.resource::<sim_core::SimClock>().days_per_year;
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
        towns.0[1].treasury += 500.0;
        towns.0[0].treasury -= 500.0;
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
        .life
        .days_per_year;
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
        .life
        .days_per_year;
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
    let queue = s.world.resource::<sim_core::policy::OrderQueue>();
    assert!(queue.pending.is_empty());
    assert_eq!(queue.log.len(), 4);
    assert!(queue.log.iter().all(|(tick, _)| *tick == 0));
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
        let dir = db.domain_dirs[db.content.domain("woodworking").unwrap() as usize];
        let kids: Vec<f32> = s.world.resource::<sim_core::Towns>().0[1]
            .residents
            .iter()
            .map(|&i| i as usize)
            .filter(|&i| store.activity[i] == sim_core::store::Activity::Child)
            .map(|i| store.cap[i].iter().zip(dir).map(|(a, b)| a * b).sum())
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
    // A good with spare capacity to grow (Timberwick's planks are already bound by
    // workshop space and construction demand).
    s.order("subsidy Greenvale cloth 1.0").unwrap();
    let mut base = full(6);
    let mut arrived = 0;
    let mut made = [0.0f32; 2];
    let cloth = s
        .world
        .resource::<sim_core::Db>()
        .content
        .product("cloth")
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
        made[0] += s.world.resource::<sim_core::Towns>().0[1].produced_last_year[cloth];
        made[1] += base.world.resource::<sim_core::Towns>().0[1].produced_last_year[cloth];
    }
    assert_eq!(arrived, 0);
    assert!(
        made[0] > made[1] * 1.2,
        "subsidized {} vs {} cloth",
        made[0],
        made[1]
    );
}

#[test]
fn a_defending_town_marches_only_when_ordered() {
    let mut s = full(7);
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
    for _ in 0..3 * 120 {
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
    for _ in 0..240 {
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
        .map(|t| t.treasury)
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
        let change = town.treasury - before[t];
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
    let mut s = full(42);
    // Ironhold has ore and a furnace; ordered to mint, it builds a mint and works it.
    s.order("mint Ironhold on").unwrap();
    s.order("build Ironhold mint").unwrap();
    run_years(&mut s, 4);
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

    // A town that turns minting off strikes nothing, even with a mint.
    let mut s = full(42);
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
    let mut s = full(42);
    run_years(&mut s, 8);
    let db = s.world.resource::<sim_core::Db>();
    let m = s.world.resource::<Metrics>();
    let best = |t: usize| m.years.iter().map(|y| y.towns[t].era).max().unwrap();
    // Ironhold, with ore and coal, reaches early industry; the others at least
    // stone, clay and cloth.
    assert_eq!(db.content.eras[best(0) as usize], "Early industry");
    assert!((1..4).all(|t| best(t) >= 1));
    // Bronze is cast somewhere, from traded copper and tin.
    let bronze = db.content.product("bronze").unwrap() as usize;
    let towns = s.world.resource::<sim_core::Towns>();
    assert!(towns
        .0
        .iter()
        .any(|t| t.produced_last_year[bronze] > 0.0 || t.produced_this_year[bronze] > 0.0));
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
        (s.state_hash(), eras)
    };
    let (a, eras_a) = run(false);
    let (b, eras_b) = run(true);
    assert_eq!(a, b, "relabelling eras changed the simulation");
    // The labels themselves did change.
    assert_ne!(eras_a, eras_b);
}
