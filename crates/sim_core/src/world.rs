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

/// The scenario the world was set up from (kept for saving).
#[derive(Resource, Debug, Clone)]
pub struct ScenarioSpec(pub Scenario);

/// The town the scenario hands to the player, if any.
pub fn player_town(scenario: &Scenario) -> crate::policy::PlayerTown {
    crate::policy::PlayerTown(scenario.player.as_ref().map(|name| {
        scenario
            .towns
            .iter()
            .position(|t| &t.name == name)
            .unwrap_or_else(|| panic!("scenario: player town `{name}` is not a town"))
            as u16
    }))
}

/// Simulation calendar. Each day is cut into `ticks_per_day` ticks; `tick`
/// counts days (the economy runs once a day, on the day's first tick) and
/// `sub` is the tick within the day. See `env` for hours, seasons and weather.
#[derive(Resource, Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct SimClock {
    /// Days started so far (day 1 is the first day the economy runs).
    pub tick: u64,
    pub days_per_year: u32,
    #[serde(default = "one")]
    pub ticks_per_day: u32,
    /// Tick within the current day, 0 .. ticks_per_day. Before the first day it
    /// is the last tick of day 0, so the first step starts day 1.
    #[serde(default)]
    pub sub: u32,
    #[serde(default = "twelve")]
    pub months_per_year: u32,
    /// Game days started so far (each runs `economy_days_per_day` days of
    /// economy, counted by `tick`).
    #[serde(default)]
    pub game_day: u64,
    #[serde(default = "one")]
    pub economy_days_per_day: u32,
}

fn one() -> u32 {
    1
}

fn twelve() -> u32 {
    12
}

impl SimClock {
    pub fn new(days_per_year: u32, time: &crate::env::TimeParams) -> Self {
        let tpd = time.ticks_per_day.max(1);
        SimClock {
            tick: 0,
            days_per_year,
            ticks_per_day: tpd,
            sub: tpd - 1,
            months_per_year: time.months_per_year.max(1),
            game_day: 0,
            economy_days_per_day: time.economy_days_per_day.max(1),
        }
    }
    /// Days of economy started so far (the same as `tick`).
    pub fn day(&self) -> u64 {
        self.tick
    }
    /// Hour at the middle of the current tick, 0 .. 24.
    pub fn hour(&self) -> f32 {
        crate::env::hour(self)
    }
    /// Month of the year, 0 .. months_per_year.
    pub fn month(&self) -> u32 {
        let dpm = (self.days_per_year / self.months_per_year.max(1)).max(1);
        (self.day_of_year() / dpm).min(self.months_per_year.max(1) - 1)
    }
    /// Whether this tick is the last of its day (a whole day has run).
    pub fn day_done(&self) -> bool {
        self.sub + 1 >= self.ticks_per_day.max(1)
    }
    pub fn year(&self) -> u64 {
        self.tick / self.days_per_year as u64
    }
    pub fn day_of_year(&self) -> u32 {
        (self.tick % self.days_per_year as u64) as u32
    }
}

/// World seed. Each system derives its own stream per tick, so results do not
/// depend on system scheduling.
#[derive(Resource, Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct SimSeed(pub u64);

/// A tiny counter-based generator (SplitMix64) for per-agent streams in parallel
/// loops: seeding costs a few multiplications, unlike a ChaCha key setup, and the
/// numbers depend only on seed, tick, stream and agent, never on thread timing.
#[derive(Debug, Clone)]
pub struct AgentRng(u64);

impl rand::RngCore for AgentRng {
    fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn fill_bytes(&mut self, dst: &mut [u8]) {
        for chunk in dst.chunks_mut(8) {
            let v = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }
}

impl SimSeed {
    /// Agent `i`'s own stream of `stream` for this tick.
    pub fn agent_rng(&self, tick: u64, stream: u64, i: usize) -> AgentRng {
        let mut s = self.0 ^ 0xA076_1D64_78BD_642F;
        s = s.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(tick);
        s = s.wrapping_mul(0xBF58_476D_1CE4_E5B9).wrapping_add(stream);
        s = s.wrapping_mul(0x94D0_49BB_1331_11EB).wrapping_add(i as u64);
        let mut r = AgentRng(s);
        // Mix the seed once so neighbouring agents' streams decorrelate.
        let _ = rand::RngCore::next_u64(&mut r);
        r
    }

    pub fn rng(&self, tick: u64, stream: u64) -> ChaCha8Rng {
        let mut s = self.0 ^ 0x5DEE_CE66_D1CE_4E5B;
        s = s.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(tick);
        s = s.wrapping_mul(0xBF58_476D_1CE4_E5B9).wrapping_add(stream);
        ChaCha8Rng::seed_from_u64(s)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
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
    /// Per good: what the town made this year, and last year (units), while
    /// it already held more than its target stock of the good at its floor
    /// price: made and paid for, but only piling up. A diagnostic (metrics'
    /// `glut`).
    #[serde(default)]
    pub glut_made_this_year: Vec<f32>,
    #[serde(default)]
    pub glut_made_last_year: Vec<f32>,
    /// Worker-days of finished work this year, and the part spent on recipes
    /// whose every output was glutted (over target, at its floor price): labour
    /// that only piles goods up. Diagnostics; `[this year, last year]`.
    #[serde(default)]
    pub work_days: [f32; 2],
    #[serde(default)]
    pub glut_work_days: [f32; 2],
    /// Per good: the last year (counted from 1) the town made any of it; 0 =
    /// never. For capability: what the town can make (see metrics).
    #[serde(default)]
    pub last_made: Vec<u32>,
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
    /// Person-days spent cold (less than half the heat needed) since the start
    /// (with the needs ladder).
    #[serde(default)]
    pub cold_days: u32,
    pub war: WarCounters,
    /// The town this one was conquered by and pays tribute to, if any.
    pub ruler: Option<u16>,
    /// People who moved in or out since the last yearly report.
    pub moves: MoveCounters,
    /// Levers the town's government has set (see `policy.rs`).
    pub policy: crate::policy::Policy,
    /// What a worker-day at each recipe has paid here lately (public knowledge; a
    /// moving average of realised pay, failures included).
    #[serde(default)]
    pub pay_rate: Vec<f32>,
    /// What a successful worker-day at each recipe has paid here lately (failures
    /// left out); see `decision.valuation.pay_basis`.
    #[serde(default)]
    pub success_pay: Vec<f32>,
    /// Each good's current price cap, as a factor of its base price (see
    /// `market.scarcity_cap`). Empty: `max_price_factor` for all.
    #[serde(default)]
    pub price_cap: Vec<f32>,
    /// Inputs the town's trades took today, and their daily average over about a
    /// month, by product (see `market.pricing`).
    #[serde(default)]
    pub input_today: Vec<f32>,
    #[serde(default)]
    pub input_use: Vec<f32>,
    /// Smoothed price pressure per product (for `pricing::Inertia::Smoothed`).
    #[serde(default)]
    pub pressure: Vec<f32>,
    /// Daily price changes this year, per price class (essential, commodity,
    /// labour); see `pricing::Volatility`.
    #[serde(default)]
    pub price_vol: Vec<crate::pricing::Volatility>,
    /// This year so far, for the price indices (see `price_index`): what residents
    /// consumed and trades used, by product; the sum of daily prices and the days
    /// counted; worker-days paid.
    #[serde(default)]
    pub consumed_year: Vec<f32>,
    #[serde(default)]
    pub inputs_year: Vec<f32>,
    #[serde(default)]
    pub price_sum: Vec<f64>,
    #[serde(default)]
    pub price_days: u32,
    #[serde(default)]
    pub worker_days: f32,
    /// The town's workshops as firms, one per `Db::firm_keys` (see `firms`; only
    /// active with `market.firms: Workshops`).
    #[serde(default)]
    pub firms: Vec<crate::firms::Firm>,
    /// Loan principal written off this year (firms that closed owing it).
    #[serde(default)]
    pub loan_defaults: f32,
    /// Household goods bought today, and their daily average over about a month.
    /// Trade credit written off this year (firms that closed owing it).
    #[serde(default)]
    pub credit_defaults: f32,
    /// The town's money stock (treasury, residents, firms) per resident,
    /// yesterday (for `market.mint_money_per_capita`).
    #[serde(default)]
    pub money_per_capita: f32,
    /// What this town owes each other town for food imported on credit.
    #[serde(default)]
    pub debts: Vec<f32>,
    #[serde(default)]
    pub goods_today: Vec<f32>,
    #[serde(default)]
    pub goods_use: Vec<f32>,
    /// Market factor on the base wage share, per recipe (see `decision::wages`).
    #[serde(default)]
    pub wage_factor: Vec<f32>,
    /// Share of each job's places offered to apprentices (see `decision::matching`).
    #[serde(default)]
    pub apprentice_share: Vec<f32>,
    /// Daily rent each adult resident pays (see `decision::crowding`).
    #[serde(default)]
    pub rent: f32,
    /// The town's culture, one value per ideological axis; follows its residents
    /// slowly (see `decision::culture`). Empty until first set.
    #[serde(default)]
    pub culture: Vec<f32>,
    /// Goods delivered for government orders since the start, by product.
    #[serde(default)]
    pub procured: Vec<f32>,
    /// Food kept out of the market by the town (`granary.rs`), by product.
    #[serde(default)]
    pub granary: Vec<f32>,
    /// Today's share of each stockpile target that applies (`granary.rs`):
    /// below 1 for crops between harvests. Empty: 1 for every good.
    #[serde(default)]
    pub stockpile_scale: Vec<f32>,
    /// Per good and month: made minus used per day, learned over the years
    /// (`granary.rs`), and the ledger's running net it was last taken from.
    #[serde(default)]
    pub flow_profile: Vec<f32>,
    #[serde(default)]
    pub flow_seen: Vec<f64>,
    #[serde(default)]
    pub flow_seen_day: u32,
    /// Goods made, used and lost in this town since the start (`goods.rs`).
    #[serde(default)]
    pub flows: crate::goods::GoodsFlows,
    /// Payments into and out of the treasury, by kind (see `ledger.rs`).
    #[serde(default)]
    pub ledger: crate::ledger::Ledger,
}

/// A squad on the march: out to `target`, battle on arrival, then home.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    /// The squad the soldiers march as (`goods::Squads`), with `march: Weather`.
    #[serde(default)]
    pub squad: Option<u32>,
    /// Share of the road behind it, 0 at home .. 1 at the target.
    #[serde(default)]
    pub progress: f32,
    /// Fought and marching home.
    #[serde(default)]
    pub back: bool,
}

/// Military events since the last yearly report.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct MoveCounters {
    pub arrived: u32,
    pub left: u32,
}

impl Town {
    /// Open government orders that take food, or what food is made from (grain,
    /// flour, wood for the ovens), while the town holds less food per resident
    /// than its market aims for. Orders are never capped; this is the warning.
    pub fn orders_eating_food(&self, db: &Db, params: &crate::config::ModelParams) -> Vec<u32> {
        let per_capita = self.food_stock(db) / self.residents.len().max(1) as f32;
        if per_capita >= params.market.food_target_per_capita {
            return Vec::new();
        }
        let feeds = |p: usize| {
            db.is_food[p]
                || (0..db.recipes.len()).any(|r| {
                    crate::systems::decide::makes_food(db, r)
                        && db.recipes[r].inputs.iter().any(|i| i.0 as usize == p)
                })
        };
        self.policy
            .procurement
            .iter()
            .filter(|o| o.1 > 0.0 && feeds(o.0 as usize))
            .map(|o| o.0)
            .collect()
    }

    /// The stockpile target for good `p` today: the ruler's amount, scaled by
    /// the season (`granary.rs`).
    pub fn stockpile_target(&self, p: usize) -> f32 {
        self.policy.stockpiled(p) * self.stockpile_scale.get(p).copied().unwrap_or(1.0)
    }

    pub fn food_stock(&self, db: &Db) -> f32 {
        db.foods
            .iter()
            .map(|(p, f)| self.stock[*p as usize] * f)
            .sum()
    }

    /// Unsold stock of good `p` held by the town's firms.
    pub fn firm_stock(&self, p: usize) -> f32 {
        self.firms
            .iter()
            .map(|f| f.stock.get(p).copied().unwrap_or(0.0))
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

    /// Whether the town wants new coin: by policy, or on automatic while its
    /// treasury holds less than `mint_below_per_capita` per resident.
    pub fn mint_wanted(&self, mp: &crate::config::MarketParams) -> bool {
        match self.policy.minting {
            crate::policy::Minting::On => true,
            crate::policy::Minting::Off => false,
            crate::policy::Minting::Auto => {
                let n = self.residents.len().max(1) as f32;
                self.treasury < mp.mint_below_per_capita * n
                    || (mp.mint_money_per_capita > 0.0
                        && self.money_per_capita < mp.mint_money_per_capita)
            }
        }
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
    agents
        + towns
            .0
            .iter()
            .map(|t| t.treasury as f64 + t.firms.iter().map(|f| f.cash as f64).sum::<f64>())
            .sum::<f64>()
}

/// Choices made in the decide step, consumed by assign.
#[derive(Resource, Default, serde::Serialize, serde::Deserialize)]
pub struct Decisions {
    /// (agent, recipe)
    pub jobs: Vec<(u32, u32)>,
    /// Agents who took their job as apprentices (paid `apprentice_wage`).
    #[serde(default)]
    pub apprentices: Vec<u32>,
}

/// One finished work step, consumed by learn and socialize.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkEvent {
    pub recipe: u32,
    pub town: u16,
    pub workers: Vec<u32>,
    pub apprentices: Vec<u32>,
    pub success: bool,
    pub quality: f32,
}

#[derive(Resource, Default, serde::Serialize, serde::Deserialize)]
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
pub fn setup(world: &mut World, mut db: Db, params: ModelParams, scenario: &Scenario) {
    db.group_firms(params.market.firms.grouping());
    // A world generated from the seed, if the scenario asks for one.
    let resolved = crate::worldgen::resolve(scenario, &db);
    let scenario = &resolved;
    let models = Models::from_params(&params);
    let dpy = params.life.days_per_year;
    let params_time = params.time.clone();
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
        let flows = crate::goods::GoodsFlows::seeded(&stock);
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
            treasury: params.market.starting_coins,
            buildings,
            nature_sites,
            armory: Vec::new(),
            guilds,
            residents: Vec::new(),
            produced_this_year: vec![0.0; np],
            produced_last_year: vec![0.0; np],
            glut_made_this_year: vec![0.0; np],
            glut_made_last_year: vec![0.0; np],
            work_days: [0.0; 2],
            glut_work_days: [0.0; 2],
            last_made: vec![0; np],
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
            cold_days: 0,
            war: WarCounters::default(),
            ruler: None,
            moves: MoveCounters::default(),
            policy: crate::policy::Policy::new(db.content.products.len(), db.content.domains.len()),
            ledger: crate::ledger::Ledger::new(scenario.towns.len()),
            procured: vec![0.0; db.content.products.len()],
            granary: Vec::new(),
            stockpile_scale: Vec::new(),
            flow_profile: Vec::new(),
            flow_seen: Vec::new(),
            flow_seen_day: 0,
            flows,
            pay_rate: vec![0.0; db.recipes.len()],
            success_pay: vec![0.0; db.recipes.len()],
            price_cap: Vec::new(),
            input_today: Vec::new(),
            input_use: Vec::new(),
            pressure: Vec::new(),
            price_vol: Vec::new(),
            consumed_year: Vec::new(),
            inputs_year: Vec::new(),
            price_sum: Vec::new(),
            price_days: 0,
            worker_days: 0.0,
            firms: crate::firms::new_firms(&db),
            loan_defaults: 0.0,
            credit_defaults: 0.0,
            money_per_capita: 0.0,
            debts: Vec::new(),
            goods_today: Vec::new(),
            goods_use: Vec::new(),
            wage_factor: vec![1.0; db.recipes.len()],
            apprentice_share: vec![0.0; db.recipes.len()],
            rent: 0.0,
            culture: spec.culture.clone().unwrap_or_default(),
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
                let center = spec
                    .culture
                    .as_ref()
                    .and_then(|c| c.get(k - mind::IDEOLOGY_START).copied())
                    .unwrap_or_else(|| 0.5 + 0.25 * ((ti * 3 + k) as f32 * 1.7).sin());
                *m = (center + rng.random_range(-0.15..0.15f32)).clamp(0.0, 1.0);
            }
            let mut cap = vec![0.0; db.cap_dim];
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
                for (c, s) in cap.iter_mut().zip(db.recipes[r].skill.iter()) {
                    *c += amount * s;
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

    let goal = crate::goal::MatchState::new(scenario.goal.as_ref(), &db, scenario.towns.len())
        .unwrap_or_else(|e| panic!("{e}"));
    world.insert_resource(goal);
    world.insert_resource(store);
    world.insert_resource(towns);
    world.insert_resource(db);
    world.insert_resource(models);
    world.insert_resource(crate::decision::DecisionModels::from_model_params(&params));
    world.insert_resource(Params(params));
    world.insert_resource(crate::terrain::Terrain::build(scenario));
    world.insert_resource(crate::profile::Profile::default());
    world.insert_resource(crate::policy::OrderQueue::default());
    world.insert_resource(player_town(scenario));
    world.insert_resource(ScenarioSpec(scenario.clone()));
    world.insert_resource(crate::governor::Governors::new(scenario.towns.len()));
    world.insert_resource(crate::history::History::default());
    world.insert_resource(SimClock::new(dpy, &params_time));
    world.insert_resource(crate::env::Environment::default());
    world.insert_resource(crate::goods::Squads::default());
    world.insert_resource(seed);
    world.insert_resource(Decisions::default());
    world.insert_resource(WorkLog {
        runs: vec![0; nr],
        chosen: vec![0; nr],
        ..Default::default()
    });
    world.insert_resource(crate::metrics::Metrics::default());
}
