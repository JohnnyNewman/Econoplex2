//! Econoplex simulation core.
//!
//! Every unit is an economic agent whose skills, mind, physique and state are
//! vectors in [`store::AgentStore`]. One simulation tick is one in-game day and
//! runs the [`SimTick`] schedule (see [`systems`] for the pipeline). The core is
//! headless: the CLI runs it directly, the game runs it from Bevy's `FixedUpdate`.

pub mod components;
pub mod config;
pub mod db;
pub mod metrics;
pub mod models;
pub mod store;
pub mod systems;
pub mod world;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::{ScheduleLabel, SingleThreadedExecutor};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub use config::{ModelParams, Scenario};
pub use db::Db;
pub use store::AgentStore;
pub use world::{SimClock, Towns};

/// The schedule that advances the simulation by one day.
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub struct SimTick;

/// Build the tick schedule. Systems run single-threaded in a fixed order so the
/// simulation is reproducible from its seed; the heavy loops are written over
/// flat arrays and can be parallelized internally later.
pub fn build_schedule() -> Schedule {
    use systems::*;
    let mut schedule = Schedule::new(SimTick);
    schedule.set_executor(SingleThreadedExecutor::new());
    schedule.add_systems(
        (
            needs::sense,
            decide::decide,
            work::assign,
            work::produce,
            work::learn,
            social::socialize,
            body::body,
            market::settle,
            organize::organize,
            military::military,
            migrate::migrate,
            life::lifecycle,
            metrics::record,
        )
            .chain(),
    );
    schedule
}

/// Where content and config live.
#[derive(Debug, Clone)]
pub struct AssetPaths {
    pub content: PathBuf,
    pub models: PathBuf,
    pub scenario: PathBuf,
}

impl AssetPaths {
    pub fn from_root(root: &Path) -> Self {
        AssetPaths {
            content: root.join("content"),
            models: root.join("config/models.ron"),
            scenario: root.join("config/scenario.ron"),
        }
    }

    /// Find an `assets` folder in the working directory or one of its parents,
    /// falling back to the repository's own assets.
    pub fn discover() -> Self {
        let mut dir = std::env::current_dir().ok();
        while let Some(d) = dir {
            let candidate = d.join("assets");
            if candidate.join("config/models.ron").exists() {
                return Self::from_root(&candidate);
            }
            dir = d.parent().map(Path::to_path_buf);
        }
        Self::from_root(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets"))
    }
}

pub fn load(paths: &AssetPaths) -> Result<(Db, ModelParams, Scenario), Box<dyn std::error::Error>> {
    let content = sim_data::Content::load_dir(&paths.content)?;
    let params: ModelParams = config::load_ron(&paths.models)?;
    params.validate()?;
    let scenario: Scenario = config::load_ron(&paths.scenario)?;
    let db = Db::build(
        content,
        params.embedding.ancestor_decay,
        &params.embedding.method,
    );
    Ok((db, params, scenario))
}

/// A self-contained headless simulation: an ECS world plus its tick schedule.
pub struct Simulation {
    pub world: World,
    pub schedule: Schedule,
}

impl Simulation {
    pub fn new(db: Db, params: ModelParams, scenario: &Scenario) -> Self {
        let mut world = World::new();
        world::setup(&mut world, db, params, scenario);
        Simulation {
            world,
            schedule: build_schedule(),
        }
    }

    pub fn from_paths(paths: &AssetPaths) -> Result<Self, Box<dyn std::error::Error>> {
        let (db, params, scenario) = load(paths)?;
        Ok(Self::new(db, params, &scenario))
    }

    pub fn step(&mut self) {
        self.schedule.run(&mut self.world);
    }

    pub fn tick(&self) -> u64 {
        self.world.resource::<SimClock>().tick
    }

    pub fn state_hash(&self) -> u64 {
        state_hash(&self.world)
    }
}

/// Hash of the full simulation state that matters for determinism checks.
pub fn state_hash(world: &World) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    world.resource::<SimClock>().tick.hash(&mut h);
    let s = world.resource::<AgentStore>();
    let mut f = |x: f32| x.to_bits().hash(&mut h);
    for i in 0..s.len() {
        s.cap[i].iter().for_each(|x| f(*x));
        s.mind[i].iter().for_each(|x| f(*x));
        s.phys[i].iter().for_each(|x| f(*x));
        s.state[i].iter().for_each(|x| f(*x));
        f(s.wealth[i]);
    }
    for t in &world.resource::<Towns>().0 {
        t.stock.iter().for_each(|x| f(*x));
        t.price.iter().for_each(|x| f(*x));
    }
    s.alive.hash(&mut h);
    s.age_days.hash(&mut h);
    h.finish()
}
