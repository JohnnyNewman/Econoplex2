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
    let mut s = sim(4);
    let money = |s: &Simulation| {
        sim_core::world::total_money(
            s.world.resource::<sim_core::AgentStore>(),
            s.world.resource::<sim_core::Towns>(),
        )
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
