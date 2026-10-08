//! Global simulation resources and world setup from a scenario.

use crate::components::*;
use crate::config::{ModelParams, Scenario};
use crate::db::Db;
use crate::models::Models;
use crate::store::{AgentStore, NewAgent};
use bevy_ecs::prelude::*;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use sim_data::dims::*;
use sim_data::Category;

/// Active model parameters.
#[derive(Resource, Clone)]
pub struct Params(pub ModelParams);

impl std::ops::Deref for Params {
    type Target = ModelParams;
    fn deref(&self) -> &ModelParams {
        &self.0
    }
}

impl Params {
    pub fn mastery(&self) -> f32 {
        match self.learning {
            crate::config::LearningParams::Diminishing { mastery, .. } => mastery,
        }
    }
}

/// Simulation calendar. One tick = one in-game day.
#[derive(Resource, Debug, Clone, Copy)]
pub struct SimClock {
    pub tick: u64,
    pub days_per_year: u32,
}

impl SimClock {
    pub fn year(&self) -> u64 {
        self.tick / self.days_per_year as u64
    }
    pub fn day_of_year(&self) -> u32 {
        (self.tick % self.days_per_year as u64) as u32
    }
}

/// World seed. Each system derives its own stream per tick, so results do not
/// depend on system scheduling.
#[derive(Resource, Debug, Clone, Copy)]
pub struct SimSeed(pub u64);

impl SimSeed {
    pub fn rng(&self, tick: u64, stream: u64) -> ChaCha8Rng {
        let mut s = self.0 ^ 0x5DEE_CE66_D1CE_4E5B;
        s = s.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(tick);
        s = s.wrapping_mul(0xBF58_476D_1CE4_E5B9).wrapping_add(stream);
        ChaCha8Rng::seed_from_u64(s)
    }
}

pub struct Town {
    pub name: String,
    pub pos: (f32, f32),
    pub org: Entity,
    pub stock: Vec<f32>,
    pub quality: Vec<f32>,
    pub price: Vec<f32>,
    pub treasury: f32,
    /// (building product def, entity, position)
    pub buildings: Vec<(u32, Entity, (f32, f32))>,
    pub nature_sites: Vec<Entity>,
    /// Items stored in the town (tools and weapons).
    pub armory: Vec<Entity>,
    /// Guild organization per skill domain.
    pub guilds: Vec<Entity>,
    pub residents: Vec<u32>,
    pub produced_this_year: Vec<f32>,
    pub produced_last_year: Vec<f32>,
    /// Share of each building type's slots in use, averaged over about a month
    /// (indexed by product).
    pub building_load: Vec<f32>,
    /// Buildings put up by construction so far.
    pub built: u32,
    /// Plots of land still free to clear or plant, and plots opened so far.
    pub free_land: u32,
    pub cleared: u32,
    /// The town's squad organization and its soldiers (store indices).
    pub squad_org: Entity,
    pub squad: Vec<u32>,
    pub raid: Option<Raid>,
    /// No new raid before this tick.
    pub raid_ready: u64,
    /// Food stock yesterday and its daily change, averaged over about two months.
    pub food_last: f32,
    pub food_trend: f32,
    /// Person-days spent starving since the last yearly report.
    pub starving: u32,
    pub war: WarCounters,
    /// The town this one was conquered by and pays tribute to, if any.
    pub ruler: Option<u16>,
    /// People who moved in or out since the last yearly report.
    pub moves: MoveCounters,
}

/// A squad on the march: out to `target`, battle on arrival, then home.
#[derive(Debug, Clone)]
pub struct Raid {
    pub target: u16,
    pub soldiers: Vec<u32>,
    pub depart: u64,
    pub arrive: u64,
    pub home: u64,
    pub fought: bool,
    /// People taken in battle, marched home with the squad.
    pub captives: Vec<u32>,
    /// Goods carried home (indexed by product).
    pub loot: Vec<f32>,
}

/// Military events since the last yearly report.
#[derive(Debug, Clone, Copy, Default)]
pub struct WarCounters {
    pub raids: u32,
    pub raids_won: u32,
    pub raids_lost: u32,
    pub attacked: u32,
    pub defended: u32,
    pub fallen: u32,
    /// People this town carried off, and people carried off from it.
    pub captives_taken: u32,
    pub captives_lost: u32,
    /// Towns this one conquered, and times it was conquered.
    pub conquests: u32,
    pub conquered: u32,
    /// Times this town threw off its ruler.
    pub revolts: u32,
    pub tribute_paid: f32,
    pub tribute_received: f32,
}

/// Migration since the last yearly report.
#[derive(Debug, Clone, Copy, Default)]
pub struct MoveCounters {
    pub arrived: u32,
    pub left: u32,
}

impl Town {
    pub fn food_stock(&self, db: &Db) -> f32 {
        db.foods
            .iter()
            .map(|(p, f)| self.stock[*p as usize] * f)
            .sum()
    }

    pub fn building_count(&self, def: u32) -> usize {
        self.buildings.iter().filter(|b| b.0 == def).count()
    }

    /// Where the next constructed building goes: rings outside the starting ones.
    pub fn next_building_pos(&self) -> (f32, f32) {
        let ring = (self.built / 8) as f32;
        let a = (self.built % 8) as f32 / 8.0 * std::f32::consts::TAU + 0.39 * (ring + 1.0);
        let r = 105.0 + 30.0 * ring;
        (self.pos.0 + r * a.cos(), self.pos.1 + r * a.sin())
    }

    pub fn building(&self, def: u32) -> Option<(Entity, (f32, f32))> {
        self.buildings
            .iter()
            .find(|b| b.0 == def)
            .map(|b| (b.1, b.2))
    }
}

#[derive(Resource, Default)]
pub struct Towns(pub Vec<Town>);

/// All money in the world: agents' wealth plus town treasuries. Every transfer
/// moves money between these, so this stays constant over a run.
pub fn total_money(store: &AgentStore, towns: &Towns) -> f64 {
    let agents: f64 = (0..store.len())
        .filter(|&i| store.alive[i])
        .map(|i| store.wealth[i] as f64)
        .sum();
    agents + towns.0.iter().map(|t| t.treasury as f64).sum::<f64>()
}

/// Choices made in the decide step, consumed by assign.
#[derive(Resource, Default)]
pub struct Decisions {
    /// (agent, recipe)
    pub jobs: Vec<(u32, u32)>,
}

/// One finished work step, consumed by learn and socialize.
#[derive(Debug, Clone)]
pub struct WorkEvent {
    pub recipe: u32,
    pub workers: Vec<u32>,
    pub apprentices: Vec<u32>,
    pub success: bool,
    pub quality: f32,
}

#[derive(Resource, Default)]
pub struct WorkLog {
    pub events: Vec<WorkEvent>,
    pub next_serial: u64,
    /// Lifetime counters.
    pub attempts: u64,
    pub successes: u64,
    /// Completed runs per recipe (all towns), lifetime.
    pub runs: Vec<u64>,
    /// Times each recipe was chosen in the decide step, lifetime.
    pub chosen: Vec<u64>,
}

pub const STARTING_SKILL_NOISE: f32 = 0.12;

/// Build the world: lookup tables, models, towns, buildings, nature and agents.
pub fn setup(world: &mut World, db: Db, params: ModelParams, scenario: &Scenario) {
    let models = Models::from_params(&params);
    let dpy = params.life.days_per_year;
    let seed = SimSeed(scenario.seed);
    let mut rng = seed.rng(0, 0xC0FFEE);
    let np = db.content.products.len();
    let nr = db.recipes.len();

    let mut store = AgentStore::default();
    let mut towns = Towns::default();

    for (ti, spec) in scenario.towns.iter().enumerate() {
        let t = ti as u16;
        let (tx, ty) = spec.position;
        let org = world
            .spawn((
                Organization {
                    kind: OrgKind::Town,
                    name: spec.name.clone(),
                    domain: None,
                },
                TownId(t),
                Pos { x: tx, y: ty },
            ))
            .id();

        let mut stock = vec![0.0; np];
        for (p, q) in &spec.stock {
            let i = db
                .content
                .product(p)
                .unwrap_or_else(|| panic!("scenario: unknown product `{p}`"));
            stock[i as usize] = *q;
        }
        // Buildings on a ring around the town center.
        let mut buildings = Vec::new();
        for (bi, b) in spec.buildings.iter().enumerate() {
            let def = db
                .content
                .product(b)
                .unwrap_or_else(|| panic!("scenario: unknown building `{b}`"));
            assert_eq!(
                db.category(def),
                Category::Building,
                "scenario: `{b}` is not a building"
            );
            let a = bi as f32 / spec.buildings.len() as f32 * std::f32::consts::TAU;
            let pos = (tx + 70.0 * a.cos(), ty + 70.0 * a.sin());
            let e = world
                .spawn((
                    Product { def, quality: 0.6 },
                    Structure {
                        footprint: (24.0, 24.0),
                    },
                    TownId(t),
                    Pos { x: pos.0, y: pos.1 },
                ))
                .id();
            buildings.push((def, e, pos));
        }
        // Nature sites scattered further out.
        let mut nature_sites = Vec::new();
        for (kind, count) in &spec.nature {
            let k = db
                .content
                .nature_kind(kind)
                .unwrap_or_else(|| panic!("scenario: unknown nature `{kind}`"));
            let def = &db.content.nature[k as usize];
            for _ in 0..*count {
                let a = rng.random::<f32>() * std::f32::consts::TAU;
                let r = 140.0 + rng.random::<f32>() * 120.0;
                let e = world
                    .spawn((
                        NaturalResource {
                            kind: k,
                            amount: def.capacity,
                            capacity: def.capacity,
                            regrowth: def.regrowth,
                        },
                        TownId(t),
                        Pos {
                            x: tx + r * a.cos(),
                            y: ty + r * a.sin(),
                        },
                    ))
                    .id();
                nature_sites.push(e);
            }
        }
        let guilds = db
            .content
            .domains
            .iter()
            .enumerate()
            .map(|(d, name)| {
                world
                    .spawn((
                        Organization {
                            kind: OrgKind::Guild,
                            name: format!("{} {} guild", spec.name, name),
                            domain: Some(d as u32),
                        },
                        TownId(t),
                    ))
                    .id()
            })
            .collect();

        towns.0.push(Town {
            name: spec.name.clone(),
            pos: (tx, ty),
            org,
            stock,
            quality: vec![0.5; np],
            price: db.base_price.clone(),
            treasury: 1000.0,
            buildings,
            nature_sites,
            armory: Vec::new(),
            guilds,
            residents: Vec::new(),
            produced_this_year: vec![0.0; np],
            produced_last_year: vec![0.0; np],
            building_load: vec![0.0; np],
            built: 0,
            free_land: params.market.free_land,
            cleared: 0,
            squad_org: world
                .spawn((
                    Organization {
                        kind: OrgKind::Squad,
                        name: format!("{} squad", spec.name),
                        domain: None,
                    },
                    TownId(t),
                ))
                .id(),
            squad: Vec::new(),
            raid: None,
            raid_ready: 0,
            food_last: 0.0,
            food_trend: 0.0,
            starving: 0,
            war: WarCounters::default(),
            ruler: None,
            moves: MoveCounters::default(),
        });

        // Starting trades follow what the town can actually run, with food work weighted
        // up so a new town can feed itself while the labor market finds its balance.
        let trade_weights: Vec<f32> = db
            .recipes
            .iter()
            .map(|m| {
                let has_building = m.building.is_none_or(|b| {
                    spec.buildings
                        .iter()
                        .any(|n| db.content.product(n) == Some(b))
                });
                let has_nature = m.nature.is_none_or(|k| {
                    spec.nature
                        .iter()
                        .any(|(n, c)| *c > 0 && db.content.nature_kind(n) == Some(k))
                });
                if has_building && has_nature {
                    1.0 + params.life.starting_food_bias * m.food_out
                } else {
                    0.1
                }
            })
            .collect();
        let weight_sum: f32 = trade_weights.iter().sum();

        for _ in 0..spec.population {
            let age_years = rng.random_range(0.0..50.0f32);
            let female = rng.random_bool(0.5);
            let potential: PhysVec = std::array::from_fn(|_| rng.random_range(0.35..0.95));
            let temperament: MindVec = std::array::from_fn(|_| rng.random::<f32>());
            let mut mind = temperament;
            // Each town starts with its own cultural center on the ideological axes.
            for (k, m) in mind.iter_mut().enumerate().skip(mind::IDEOLOGY_START) {
                let center = 0.5 + 0.25 * ((ti * 3 + k) as f32 * 1.7).sin();
                *m = (center + rng.random_range(-0.15..0.15f32)).clamp(0.0, 1.0);
            }
            let mut cap = [0.0; CAP_DIM];
            for x in cap.iter_mut() {
                *x = rng.random::<f32>() * STARTING_SKILL_NOISE;
            }
            // Adults start with some experience in one family trade.
            if age_years >= params.life.adult_age {
                let mut x = rng.random::<f32>() * weight_sum;
                let mut r = 0;
                while r + 1 < trade_weights.len() && x >= trade_weights[r] {
                    x -= trade_weights[r];
                    r += 1;
                }
                let amount = rng.random_range(0.2..0.6f32) * (age_years / 40.0).min(1.0);
                for k in 0..CAP_DIM {
                    cap[k] += amount * db.recipes[r].skill[k];
                }
            }
            let age_factor = crate::systems::body::age_curve(age_years, &params.physique);
            let phys: PhysVec = std::array::from_fn(|k| {
                potential[k] * age_factor * params.physique.untrained_level.max(0.6)
            });
            let entity = world.spawn_empty().id();
            let target = (
                tx + rng.random_range(-40.0..40.0f32),
                ty + rng.random_range(-40.0..40.0f32),
            );
            let idx = store.push(
                NewAgent {
                    entity,
                    town: t,
                    female,
                    age_days: (age_years * dpy as f32) as u32,
                    cap,
                    mind,
                    phys,
                    potential,
                    temperament,
                    aptitude: rng.random_range(0.6..1.4),
                    parents: None,
                    target,
                    wealth: 20.0,
                },
                age_years >= params.life.adult_age,
            );
            world.entity_mut(entity).insert((Agent { idx }, TownId(t)));
        }
    }

    for (i, &t) in store.town.iter().enumerate() {
        towns.0[t as usize].residents.push(i as u32);
    }

    world.insert_resource(store);
    world.insert_resource(towns);
    world.insert_resource(db);
    world.insert_resource(models);
    world.insert_resource(Params(params));
    world.insert_resource(crate::profile::Profile::default());
    world.insert_resource(SimClock {
        tick: 0,
        days_per_year: dpy,
    });
    world.insert_resource(seed);
    world.insert_resource(Decisions::default());
    world.insert_resource(WorkLog {
        runs: vec![0; nr],
        chosen: vec![0; nr],
        ..Default::default()
    });
    world.insert_resource(crate::metrics::Metrics::default());
}
