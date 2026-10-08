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
    for _ in 0..20 * dpy {
        s.step();
        let towns = s.world.resource::<sim_core::Towns>();
        raids += towns.0.iter().filter(|t| t.raid.is_some()).count();
        soldiers = soldiers.max(towns.0.iter().map(|t| t.squad.len()).sum());
    }
    assert!(soldiers > 0, "no town raised a squad");
    assert!(raids > 0, "no raid in 20 years");
}
