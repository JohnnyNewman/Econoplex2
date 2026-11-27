//! What a renderer may see of the simulation: a plain-data snapshot per tick.
//!
//! A view (the 3D client, a web front end, a test) never touches the ECS world.
//! After each tick the owner of the [`crate::Simulation`] calls
//! [`ViewSnapshot::capture`] and hands the copy over, for example to another
//! thread. Capturing only reads, so it cannot change the simulation: the state
//! hash is the same with or without a view (see the test below).
//!
//! The other two ways across the boundary already exist: the event log
//! ([`crate::history::History`], delivered here as the events new since the last
//! capture) and the order queue ([`crate::policy::OrderQueue`]).

use crate::db::Db;
use crate::history::{Event, History, Sample};
use crate::world::{SimClock, Towns};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

/// One building, as a view sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildingView {
    /// Stable while the building stands (the entity's bits), so a view can tell
    /// new buildings from old ones between snapshots.
    pub id: u64,
    /// Product id of the building type, e.g. "mill".
    pub kind: String,
    /// Map position.
    pub pos: (f32, f32),
    /// How far the run under way in the building has got, 0..1 (the most
    /// advanced one if several teams work there); `None` when it stands idle.
    /// Inputs flow in as the team works; each whole run's output goes to
    /// market when the bar fills, and the rest carries on.
    #[serde(default)]
    pub progress: Option<f32>,
    /// How busy buildings of this kind in this town have been lately: the
    /// share of their places in use, averaged over about a month
    /// (`Town::building_load`). Steady whatever the hour, unlike `progress`.
    #[serde(default)]
    pub load: f32,
}

/// A town's government levers, with the defaults from models.ron filled in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PolicyView {
    pub wealth_tax: f32,
    pub income_tax: f32,
    pub wage_share: f32,
    pub public_spending: f32,
    /// "open", "encouraged" or "closed".
    pub immigration: String,
    /// "auto", "on" or "off".
    pub minting: String,
    /// "auto" or "defend".
    pub war: String,
    /// Buildings ordered and not yet built.
    pub commissions: u32,
}

/// One town, as a view sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TownView {
    pub name: String,
    pub pos: (f32, f32),
    pub population: u32,
    /// In the town's own coin.
    pub treasury: f32,
    /// What one of the town's coins is worth in the common unit.
    #[serde(default)]
    pub coin_value: f32,
    /// Other towns' coin the town holds, by issuing town.
    #[serde(default)]
    pub foreign: Vec<f32>,
    /// Metal coin the town holds (worth its face value everywhere).
    #[serde(default)]
    pub metal: f32,
    /// Houses by kind (product id, how many), and how well they shelter the
    /// town's people (0..1).
    #[serde(default)]
    pub houses: Vec<(String, u32)>,
    #[serde(default)]
    pub shelter: f32,
    pub buildings: Vec<BuildingView>,
    /// The town this one pays tribute to, if conquered.
    pub ruler: Option<u16>,
    pub policy: PolicyView,
    /// The town's culture, one value per ideological axis (0..1, 0.5 neutral;
    /// see `decision::culture`).
    #[serde(default)]
    pub culture: Vec<f32>,
    /// The newest era among the town's buildings (a display label: the 3D view
    /// builds in that era's style).
    pub era: u8,
    /// Residents' mean wealth (coin per head).
    pub mean_wealth: f32,
    /// Goods in the market: (product def, stock, price), for goods in stock.
    pub market: Vec<(u32, f32, f32)>,
    /// Value shipped to each town by caravan (at the buyer's price), as a daily
    /// average over about a month; indexed by town.
    pub exports: Vec<f32>,
    /// Residents' mean satisfaction per need on the ladder, in
    /// `sim_data::Need::ALL` order (empty unless `market.consumption: Needs`).
    #[serde(default)]
    pub needs: Vec<f32>,
    /// Food in the granary (food value), and the days of food it aims for.
    #[serde(default)]
    pub granary_food: f32,
    #[serde(default)]
    pub granary_days: f32,
    /// Whether seed grain is held back (order `reserve TOWN on`).
    #[serde(default)]
    pub seed_reserve: bool,
    /// Food in the market and in larders, as days of the residents' food.
    #[serde(default)]
    pub food_days: f32,
    /// Person-days cold and starving since the start (cold: with the needs
    /// ladder) and since the last yearly report (starving).
    #[serde(default)]
    pub cold_days: u32,
    #[serde(default)]
    pub starving: u32,
    /// Tools and weapons in the armory: (product def, count).
    #[serde(default)]
    pub armory: Vec<(u32, u32)>,
    /// Crop standing on the town's fields, as a share of a year's growth
    /// (`farming.rs`): rises from spring, drops as it is reaped. 0 for a town
    /// without fields.
    #[serde(default)]
    pub crops: f32,
    /// Buildings being put up, with how far along they are.
    #[serde(default)]
    pub sites: Vec<SiteView>,
    /// The town's pastures: map position and how much grass stands on each
    /// (0..1 of what it can hold). Flocks graze there (3D view).
    #[serde(default)]
    pub pastures: Vec<((f32, f32), f32)>,
}

/// A building being put up (a construction recipe under way).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SiteView {
    /// Product id of the building, e.g. "mill".
    pub kind: String,
    /// Map position: where it will stand once done (the order's place, or
    /// the town's next building place).
    pub pos: (f32, f32),
    /// Work done, 0..1.
    pub progress: f32,
}

/// What a person is doing, as a view sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Doing {
    Child,
    Idle,
    Resting,
    /// Worked today, at a recipe of this skill domain (0..`ViewSnapshot::domains`).
    Working(u32),
    Soldier,
    Captive,
}

/// All living people, as parallel arrays (one entry per person; cheap to copy
/// and easy to upload to the GPU).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PeopleView {
    /// Index in the simulation's agent store: stable while the person lives,
    /// so a view can follow someone from one snapshot to the next.
    pub id: Vec<u32>,
    pub town: Vec<u16>,
    /// Where the person is heading (map position): their workplace, home, or
    /// the squad's goal.
    pub target: Vec<(f32, f32)>,
    pub doing: Vec<Doing>,
    /// Each person's household: the store index of who keeps their home
    /// (`ladder::keepers`). Everyone with the same keeper lives in one house.
    #[serde(default)]
    pub home: Vec<u32>,
    /// Each person's own culture: the first three ideological axes of their
    /// mind (tradition, faith, liberty; 0..1, 0.5 neutral).
    #[serde(default)]
    pub culture: Vec<[f32; 3]>,
    /// Each person's savings (coin), for how well they dress.
    #[serde(default)]
    pub wealth: Vec<f32>,
}

/// How much of the event log and the monthly history a view already has.
#[derive(Debug, Clone, Copy, Default)]
pub struct Seen {
    pub events: usize,
    pub samples: usize,
    /// Battles fought (`BattleLog::total`) when last captured.
    pub battles: usize,
}

/// Everything a view needs for one tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewSnapshot {
    pub tick: u64,
    /// Economy step within the simulated day, and steps a day.
    #[serde(default)]
    pub step: u32,
    #[serde(default)]
    pub steps_per_day: u32,
    /// Simulated day of the year, which is its month (0 = the first).
    pub day_of_year: u32,
    /// Simulated days in a year: one per month.
    pub months_per_year: u32,
    pub towns: Vec<TownView>,
    pub people: PeopleView,
    /// Number of skill domains (for colouring `Doing::Working`).
    pub domains: u32,
    /// The skill domains' names, in order (the 3D view picks a work motion by
    /// domain: a scythe for agriculture, a hammer for smithing, ...).
    #[serde(default)]
    pub domain_names: Vec<String>,
    /// Events logged since the previous capture (oldest first).
    pub events: Vec<Event>,
    /// Monthly history samples (one per town) recorded since the previous
    /// capture.
    pub samples: Vec<Sample>,
    /// The "why" findings for one town (see `crate::explain`), if the view
    /// asked for them; set by the owner of the simulation after capturing.
    #[serde(default)]
    pub why: Option<(u16, Vec<crate::explain::Finding>)>,
    /// The product space as one town sees it (see [`town_skills`]), if the
    /// view asked; set like `why`.
    #[serde(default)]
    pub skills: Option<(u16, TownSkills)>,
    /// Battles fought since the previous capture, to play back.
    #[serde(default)]
    pub battles: Vec<crate::battle::Replay>,
}

/// The product space's fixed part: the skills, their domains, and the links
/// between related skills (the embedding's similarity, as in the 2D game).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProductSpace {
    pub skills: Vec<String>,
    /// Each skill's domain (index into `domains`).
    pub domain: Vec<u16>,
    pub domains: Vec<String>,
    /// (skill, skill, similarity) for every pair more similar than 0.2.
    pub edges: Vec<(u16, u16, f32)>,
}

impl ProductSpace {
    pub fn new(db: &Db) -> Self {
        let n = db.content.skills.len();
        let mut edges = Vec::new();
        for a in 0..n {
            for b in (a + 1)..n {
                let s = db.embedding.similarity(a, b);
                if s > 0.2 {
                    edges.push((a as u16, b as u16, s));
                }
            }
        }
        ProductSpace {
            skills: db.content.skills.iter().map(|s| s.name.clone()).collect(),
            domain: db
                .content
                .skills
                .iter()
                .map(|s| db.content.domain(&s.domain).unwrap_or(0) as u16)
                .collect(),
            domains: db.content.domains.clone(),
            edges,
        }
    }
}

/// One town's place in the product space.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TownSkills {
    /// The residents' mean level in each skill.
    pub level: Vec<f32>,
    /// Skills the town reached: it made something with them last year, or its
    /// people know them (mean level above 0.2).
    pub lit: Vec<bool>,
}

/// Where a town stands in the product space (read-only).
pub fn town_skills(world: &World, town: u16) -> TownSkills {
    let db = world.resource::<Db>();
    let towns = world.resource::<Towns>();
    let store = world.resource::<crate::AgentStore>();
    let n = db.content.skills.len();
    let Some(t) = towns.0.get(town as usize) else {
        return TownSkills::default();
    };
    let mut level = vec![0.0f32; n];
    let mut count = 0.0f32;
    for &i in &t.residents {
        let i = i as usize;
        if !store.alive[i] {
            continue;
        }
        for (s, v) in db.skill_profile(&store.cap[i]).iter().enumerate() {
            level[s] += v;
        }
        count += 1.0;
    }
    level.iter_mut().for_each(|v| *v /= count.max(1.0));
    let mut lit: Vec<bool> = level.iter().map(|&v| v > 0.2).collect();
    for r in &db.recipes {
        let made = r.outputs.iter().any(|&(p, _)| {
            t.produced_last_year
                .get(p as usize)
                .is_some_and(|&q| q > 0.0)
        });
        if made {
            if let Some(l) = lit.get_mut(r.skill_idx as usize) {
                *l = true;
            }
        }
    }
    TownSkills { level, lit }
}

/// Per town, its pastures: position and grass standing (0..1 of capacity).
fn pastures_by_town(world: &World) -> Vec<Vec<((f32, f32), f32)>> {
    let (Some(towns), Some(db)) = (world.get_resource::<Towns>(), world.get_resource::<Db>())
    else {
        return Vec::new();
    };
    let Some(kind) = db.content.nature.iter().position(|k| k.id == "pasture") else {
        return vec![Vec::new(); towns.0.len()];
    };
    // A pasture that grows as a crop holds up to `crop_capacity` times its
    // capacity (farming.rs).
    let hold = world
        .get_resource::<crate::world::Params>()
        .filter(|p| p.farming.crops.iter().any(|c| c == "pasture"))
        .map_or(1.0, |p| p.farming.crop_capacity.max(1.0));
    towns
        .0
        .iter()
        .map(|t| {
            t.nature_sites
                .iter()
                .filter_map(|&e| {
                    let n = world.get::<crate::components::NaturalResource>(e)?;
                    if n.kind as usize != kind {
                        return None;
                    }
                    let p = world.get::<crate::components::Pos>(e)?;
                    let grass = if n.capacity > 0.0 {
                        (n.amount / (n.capacity * hold)).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    Some(((p.x, p.y), grass))
                })
                .collect()
        })
        .collect()
}

/// Per town, the crop standing on its fields as a share of a year's growth
/// (sum over its crop sites; the 3D view draws the fields by it).
fn crops_by_town(world: &World) -> Vec<f32> {
    let (Some(towns), Some(db), Some(params)) = (
        world.get_resource::<Towns>(),
        world.get_resource::<Db>(),
        world.get_resource::<crate::world::Params>(),
    ) else {
        return Vec::new();
    };
    let crops = &params.farming.crops;
    let is_crop: Vec<bool> = db
        .content
        .nature
        .iter()
        .map(|k| crops.contains(&k.id))
        .collect();
    let cal = params.calendar();
    let year_days = cal.days() * cal.per_year as f32;
    towns
        .0
        .iter()
        .map(|t| {
            let (mut standing, mut year) = (0.0, 0.0);
            for &e in &t.nature_sites {
                let Some(n) = world.get::<crate::components::NaturalResource>(e) else {
                    continue;
                };
                if is_crop.get(n.kind as usize).copied().unwrap_or(false) {
                    standing += n.amount;
                    year += n.regrowth * year_days;
                }
            }
            if year > 0.0 {
                standing / year
            } else {
                0.0
            }
        })
        .collect()
}

/// Per town, its building sites: each construction recipe with work under
/// way (left by the last team, `Town::wip`, or a team on it now). A site
/// stands where the building will: the place its order gave, else the
/// town's next building places in turn.
fn sites_by_town(world: &World) -> Vec<Vec<SiteView>> {
    let (Some(towns), Some(db)) = (world.get_resource::<Towns>(), world.get_resource::<Db>())
    else {
        return Vec::new();
    };
    let mut work: Vec<Vec<f32>> = towns
        .0
        .iter()
        .map(|t| {
            let mut w = t.wip.clone();
            w.resize(db.recipes.len(), 0.0);
            w
        })
        .collect();
    if let Some(mut q) = world.try_query::<&crate::components::Task>() {
        for t in q.iter(world) {
            if let Some(w) = work
                .get_mut(t.town as usize)
                .and_then(|w| w.get_mut(t.recipe as usize))
            {
                *w += t.progress;
            }
        }
    }
    towns
        .0
        .iter()
        .zip(&work)
        .map(|(town, w)| {
            let mut next = town.built;
            db.recipes
                .iter()
                .zip(w)
                .filter_map(|(m, &done)| {
                    let b = m.builds?;
                    if done <= 1e-4 {
                        return None;
                    }
                    let ordered = town
                        .policy
                        .commissions
                        .iter()
                        .find(|c| c.building == b)
                        .and_then(|c| c.pos);
                    let pos = ordered.unwrap_or_else(|| {
                        next += 1;
                        town.building_pos(next - 1)
                    });
                    Some(SiteView {
                        kind: db.content.products[b as usize].id.clone(),
                        pos,
                        progress: done.clamp(0.0, 1.0),
                    })
                })
                .collect()
        })
        .collect()
}

impl ViewSnapshot {
    /// Copy what a view needs out of the world. `seen` says how much of the
    /// event log and history the view already has; it is advanced past what is
    /// returned here.
    pub fn capture(world: &World, seen: &mut Seen) -> Self {
        let clock = world.resource::<SimClock>();
        let needs = needs_by_town(world);
        let armories = armory_counts(world);
        let crops = crops_by_town(world);
        let sites = sites_by_town(world);
        let pastures = pastures_by_town(world);
        let towns = world.resource::<Towns>();
        let db = world.resource::<Db>();
        let store = world.resource::<crate::AgentStore>();
        let progress = batch_progress(world);
        let mut people = PeopleView::default();
        let keepers = crate::ladder::keepers(store);
        for i in 0..store.len() {
            if !store.alive[i] {
                continue;
            }
            use crate::store::Activity;
            people.id.push(i as u32);
            people.town.push(store.town[i]);
            people.target.push(store.target[i]);
            people.home.push(keepers.get(i).map_or(i, |&k| k) as u32);
            let ideology = &store.mind[i][sim_data::dims::mind::IDEOLOGY_START..];
            people.culture.push(std::array::from_fn(|k| {
                ideology.get(k).copied().unwrap_or(0.5)
            }));
            people.wealth.push(store.wealth[i].coins());
            // Workers are Idle again by the end of the tick; what they did is
            // in `worked_today`.
            people
                .doing
                .push(match (store.worked_today[i], store.activity[i]) {
                    (Some(r), _) => Doing::Working(db.recipes[r as usize].domain),
                    (None, Activity::Child) => Doing::Child,
                    (None, Activity::Resting) => Doing::Resting,
                    (None, Activity::Soldier) => Doing::Soldier,
                    (None, Activity::Captive) => Doing::Captive,
                    (None, Activity::Idle | Activity::Working(_)) => Doing::Idle,
                });
        }
        let (events, samples) = world
            .get_resource::<History>()
            .map(|h| {
                let new = h.events.get(seen.events..).unwrap_or_default().to_vec();
                let samples = h.samples.get(seen.samples..).unwrap_or_default().to_vec();
                seen.events = h.events.len();
                seen.samples = h.samples.len();
                (new, samples)
            })
            .unwrap_or_default();
        let battles = world
            .get_resource::<crate::battle::BattleLog>()
            .map(|log| {
                let new = log.since(seen.battles);
                seen.battles = log.total;
                new
            })
            .unwrap_or_default();
        ViewSnapshot {
            tick: clock.tick,
            step: clock.step,
            steps_per_day: clock.steps_per_day,
            day_of_year: clock.day_of_year(),
            months_per_year: clock.months_per_year,
            towns: towns
                .0
                .iter()
                .enumerate()
                .map(|(ti, t)| TownView {
                    needs: needs.get(ti).cloned().unwrap_or_default(),
                    granary_food: crate::granary::held(t, db),
                    granary_days: t.policy.granary_days,
                    seed_reserve: t.policy.food_reserve,
                    food_days: {
                        let hunger = world
                            .get_resource::<crate::world::Params>()
                            .map_or(0.12, |p| p.needs.hunger_per_day)
                            .max(1e-3);
                        let larders: f32 = t
                            .residents
                            .iter()
                            .flat_map(|&i| store.larder[i as usize].iter())
                            .map(|&(f, q)| {
                                q * db.foods.iter().find(|x| x.0 == f).map_or(0.0, |x| x.1)
                            })
                            .sum();
                        (t.food_stock(db) + larders) / (t.residents.len().max(1) as f32 * hunger)
                    },
                    cold_days: t.cold_days,
                    starving: t.starving,
                    armory: armories.get(ti).cloned().unwrap_or_default(),
                    crops: crops.get(ti).copied().unwrap_or(0.0),
                    sites: sites.get(ti).cloned().unwrap_or_default(),
                    pastures: pastures.get(ti).cloned().unwrap_or_default(),
                    name: t.name.clone(),
                    pos: t.pos,
                    population: t.residents.len() as u32,
                    treasury: t.treasury.coins(),
                    coin_value: t.coin_value,
                    foreign: t.foreign.iter().map(|c| c.coins()).collect(),
                    metal: t.metal.coins(),
                    houses: t
                        .houses
                        .iter()
                        .enumerate()
                        .filter(|&(_, &n)| n > 0)
                        .map(|(p, &n)| (db.content.products[p].id.clone(), n))
                        .collect(),
                    shelter: t.shelter(db),
                    buildings: t
                        .buildings
                        .iter()
                        .map(|&(def, e, pos)| BuildingView {
                            id: e.to_bits(),
                            kind: db.content.products[def as usize].id.clone(),
                            pos,
                            progress: progress
                                .iter()
                                .filter(|b| b.0 == ti as u16 && b.1 == def && b.2 == pos)
                                .map(|b| b.3)
                                .reduce(f32::max),
                            load: t.building_load.get(def as usize).copied().unwrap_or(0.0),
                        })
                        .collect(),
                    ruler: t.ruler,
                    culture: t.culture.clone(),
                    era: t
                        .buildings
                        .iter()
                        .map(|&(def, _, _)| db.content.products[def as usize].era)
                        .max()
                        .unwrap_or(0),
                    mean_wealth: {
                        let n = t.residents.len().max(1) as f32;
                        t.residents
                            .iter()
                            .map(|&i| store.wealth[i as usize].coins())
                            .sum::<f32>()
                            / n
                    },
                    policy: {
                        let pol = &t.policy;
                        let defaults = world.get_resource::<crate::world::Params>();
                        let d = |f: fn(&crate::config::ModelParams) -> f32| {
                            defaults.map_or(0.0, |p| f(&p.0))
                        };
                        PolicyView {
                            wealth_tax: pol.wealth_tax.unwrap_or(d(|p| p.market.wealth_tax)),
                            income_tax: pol.income_tax,
                            wage_share: pol.wage_share.unwrap_or(d(|p| p.work.wage_share)),
                            public_spending: pol
                                .public_spending
                                .unwrap_or(d(|p| p.market.public_spending)),
                            immigration: format!("{:?}", pol.immigration).to_lowercase(),
                            minting: format!("{:?}", pol.minting).to_lowercase(),
                            war: format!("{:?}", pol.war).to_lowercase(),
                            commissions: pol.commissions.len() as u32,
                        }
                    },
                    exports: t.ledger.exports.clone(),
                    market: (0..t.stock.len())
                        .filter(|&p| t.stock[p] > 0.5)
                        .filter(|&p| db.category(p as u32) != sim_data::Category::Building)
                        .map(|p| (p as u32, t.stock[p], t.price[p]))
                        .collect(),
                })
                .collect(),
            people,
            domains: db.domain_dirs.len() as u32,
            domain_names: db.content.domains.clone(),
            events,
            samples,
            why: None,
            skills: None,
            battles,
        }
    }

    /// Fraction of the year gone at the middle of this month, 0..1 (for
    /// seasons).
    pub fn season(&self) -> f32 {
        (self.day_of_year as f32 + 0.5) / self.months_per_year.max(1) as f32
    }
}

/// One person, as the inspector of a view shows them (`person`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersonInfo {
    /// Index in the agent store (as `PeopleView::id`).
    pub id: u32,
    pub town: u16,
    pub age_years: f32,
    pub female: bool,
    /// What they do: "child", "idle", "resting", "soldier", "captive", or
    /// "working".
    pub activity: String,
    /// The recipe they worked at today, if any.
    pub job: Option<String>,
    pub wealth: f32,
    /// Satisfaction per need (name, 0..1), if the needs ladder is on.
    pub needs: Vec<(String, f32)>,
    /// Food kept from their own work (product name, units).
    pub larder: Vec<(String, f32)>,
    /// Household goods held (product name, units).
    pub owned: Vec<(String, f32)>,
    /// What they carry or wear (slot, product name).
    pub gear: Vec<(String, String)>,
    pub partner: Option<u32>,
    pub parents: Option<(u32, u32)>,
    pub children: Vec<u32>,
}

/// Everything a view shows about the living person with store index `id`;
/// `None` if there is no such person.
pub fn person(world: &World, id: u32) -> Option<PersonInfo> {
    let store = world.get_resource::<crate::AgentStore>()?;
    let db = world.resource::<Db>();
    let i = id as usize;
    if i >= store.len() || !store.alive[i] {
        return None;
    }
    let product = |k: u32| {
        db.content
            .products
            .get(k as usize)
            .map_or_else(|| format!("#{k}"), |p| p.name.clone())
    };
    let activity = match store.activity[i] {
        crate::store::Activity::Child => "child",
        crate::store::Activity::Idle => "idle",
        crate::store::Activity::Resting => "resting",
        crate::store::Activity::Working(_) => "working",
        crate::store::Activity::Soldier => "soldier",
        crate::store::Activity::Captive => "captive",
    };
    let job = store.worked_today[i]
        .or(store.last_recipe[i])
        .and_then(|r| db.recipes.get(r as usize))
        .map(|r| r.name.clone());
    let needs = person_needs(world, i);
    let goods = |list: &[(u32, f32)]| -> Vec<(String, f32)> {
        list.iter()
            .filter(|(_, q)| *q > 0.0)
            .map(|&(k, q)| (product(k), q))
            .collect()
    };
    let mut gear = Vec::new();
    for (slot, name) in [
        (crate::goods::Slot::Tool, "tool"),
        (crate::goods::Slot::Weapon, "weapon"),
        (crate::goods::Slot::Armour, "armour"),
        (crate::goods::Slot::Shield, "shield"),
        (crate::goods::Slot::Mount, "mount"),
    ] {
        if let Some(e) = crate::goods::gear(store, i, slot) {
            let item = world
                .get::<crate::components::Product>(e)
                .map_or_else(|| "item".to_string(), |p| product(p.def));
            gear.push((name.to_string(), item));
        }
    }
    let children = (0..store.len())
        .filter(|&c| store.alive[c] && store.parents[c].is_some_and(|(a, b)| a == id || b == id))
        .map(|c| c as u32)
        .collect();
    Some(PersonInfo {
        id,
        town: store.town[i],
        age_years: store.age_months[i] as f32
            / world.resource::<SimClock>().months_per_year.max(1) as f32,
        female: store.female[i],
        activity: activity.to_string(),
        job,
        wealth: store.wealth[i].coins(),
        needs,
        larder: store.larder.get(i).map_or_else(Vec::new, |l| goods(l)),
        owned: store.owned.get(i).map_or_else(Vec::new, |o| goods(o)),
        gear,
        partner: store.partner[i],
        parents: store.parents[i],
        children,
    })
}

/// One person's satisfaction per need, if the needs ladder is on.
fn person_needs(world: &World, i: usize) -> Vec<(String, f32)> {
    let Some(params) = world.get_resource::<crate::world::Params>() else {
        return Vec::new();
    };
    let crate::consumption::ConsumptionParams::Needs(lp) = &params.market.consumption else {
        return Vec::new();
    };
    let (Some(env), Some(store)) = (
        world.get_resource::<crate::env::Environment>(),
        world.get_resource::<crate::AgentStore>(),
    ) else {
        return Vec::new();
    };
    let sats = crate::ladder::all(
        store,
        world.resource::<Towns>(),
        world.resource::<Db>(),
        params,
        env,
        lp,
    );
    let Some(Some(sat)) = sats.get(i) else {
        return Vec::new();
    };
    sim_data::Need::ALL
        .iter()
        .zip(sat.iter())
        .map(|(n, &v)| (format!("{n:?}").to_lowercase(), v))
        .collect()
}

/// Mean satisfaction per need by town, if the needs ladder is on.
fn needs_by_town(world: &World) -> Vec<Vec<f32>> {
    let Some(params) = world.get_resource::<crate::world::Params>() else {
        return Vec::new();
    };
    let crate::consumption::ConsumptionParams::Needs(lp) = &params.market.consumption else {
        return Vec::new();
    };
    let (Some(env), Some(store)) = (
        world.get_resource::<crate::env::Environment>(),
        world.get_resource::<crate::AgentStore>(),
    ) else {
        return Vec::new();
    };
    let towns = world.resource::<Towns>();
    let db = world.resource::<Db>();
    let sats = crate::ladder::all(store, towns, db, params, env, lp);
    let n = sim_data::Need::ALL.len();
    let mut sum = vec![vec![0.0f32; n]; towns.0.len()];
    let mut count = vec![0.0f32; towns.0.len()];
    for (i, s) in sats.iter().enumerate() {
        let Some(s) = s else { continue };
        let t = store.town[i] as usize;
        for k in 0..n {
            sum[t][k] += s[k];
        }
        count[t] += 1.0;
    }
    sum.into_iter()
        .zip(count)
        .map(|(v, c)| v.into_iter().map(|x| x / c.max(1.0)).collect())
        .collect()
}

/// Tools and weapons in each town's armory, by product.
/// Every batch under way in a building: (town, building def, position,
/// progress 0..1).
fn batch_progress(world: &World) -> Vec<(u16, u32, (f32, f32), f32)> {
    use crate::components::{Pos, Task};
    let db = world.resource::<Db>();
    let Some(mut q) = world.try_query::<(&Task, &Pos)>() else {
        return Vec::new();
    };
    q.iter(world)
        .filter_map(|(t, p)| {
            let b = db.recipes.get(t.recipe as usize)?.building?;
            Some((
                t.town,
                b,
                (p.x, p.y),
                (t.progress / t.required.max(1e-6)).clamp(0.0, 1.0),
            ))
        })
        .collect()
}

fn armory_counts(world: &World) -> Vec<Vec<(u32, u32)>> {
    let towns = world.resource::<Towns>();
    towns
        .0
        .iter()
        .map(|t| {
            let mut counts: Vec<(u32, u32)> = Vec::new();
            for &e in &t.armory {
                let Some(p) = world.get::<crate::components::Product>(e) else {
                    continue;
                };
                match counts.iter_mut().find(|c| c.0 == p.def) {
                    Some(c) => c.1 += 1,
                    None => counts.push((p.def, 1)),
                }
            }
            counts.sort();
            counts
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AssetPaths, Simulation};

    fn sim() -> Simulation {
        let paths = AssetPaths::discover();
        let (db, params, scenario) = crate::load(&paths).unwrap();
        Simulation::new(db, params, &scenario)
    }

    #[test]
    fn a_town_sees_its_skills_in_the_product_space() {
        let mut s = sim();
        for _ in 0..60 {
            s.step();
        }
        let space = ProductSpace::new(s.world.resource::<Db>());
        let mine = town_skills(&s.world, 0);
        let n = space.skills.len();
        assert_eq!(
            (mine.level.len(), mine.lit.len(), space.domain.len()),
            (n, n, n)
        );
        assert!(
            mine.lit.iter().any(|&l| l),
            "a working town has reached some skills"
        );
        assert!(mine.level.iter().all(|v| v.is_finite() && *v >= 0.0));
        assert!(space
            .edges
            .iter()
            .all(|&(a, b, w)| a < b && (b as usize) < n && w > 0.2));
        // Read-only: asking doesn't change the simulation.
        let before = s.state_hash();
        let _ = town_skills(&s.world, 0);
        assert_eq!(before, s.state_hash());
    }

    #[test]
    fn workplaces_show_their_work_during_the_working_day() {
        let mut s = sim();
        let mut seen = Seen::default();
        for _ in 0..30 {
            s.step();
        }
        // Over a day, some step finds workplaces with a run under way.
        let spd = s.world.resource::<crate::world::SimClock>().steps_per_day;
        let mut most = 0;
        for _ in 0..spd {
            s.step_tick();
            let snap = ViewSnapshot::capture(&s.world, &mut seen);
            let working = snap
                .towns
                .iter()
                .flat_map(|t| t.buildings.iter())
                .filter(|b| b.progress.is_some())
                .count();
            most = most.max(working);
        }
        assert!(most > 0);
        // At any hour, busy workplaces show how busy they have been lately.
        let snap = ViewSnapshot::capture(&s.world, &mut seen);
        assert!(snap
            .towns
            .iter()
            .flat_map(|t| t.buildings.iter())
            .any(|b| b.load > 0.05));
    }

    #[test]
    fn capturing_does_not_change_the_simulation() {
        let (mut a, mut b) = (sim(), sim());
        let mut seen = Seen::default();
        for _ in 0..40 {
            a.step();
            let _ = ViewSnapshot::capture(&a.world, &mut seen);
            b.step();
        }
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn snapshots_show_towns_buildings_and_new_events_once() {
        let mut s = sim();
        let mut seen = Seen::default();
        let first = ViewSnapshot::capture(&s.world, &mut seen);
        assert!(!first.towns.is_empty());
        assert!(first.towns.iter().all(|t| !t.buildings.is_empty()));
        assert!(first.towns.iter().all(|t| t.population > 0));
        assert!(first.towns.iter().all(|t| !t.market.is_empty()));
        assert!(first.towns.iter().all(|t| t.policy.wage_share > 0.0));
        let mut all_events = first.events.len();
        for _ in 0..400 {
            s.step();
            let snap = ViewSnapshot::capture(&s.world, &mut seen);
            assert!(snap.events.iter().all(|e| e.tick <= snap.tick));
            all_events += snap.events.len();
        }
        // Every event was delivered exactly once.
        assert_eq!(all_events, s.world.resource::<History>().events.len());
        // At midday people are at work (a day ends at midnight, when they
        // are not).
        let spd = s.world.resource::<crate::world::SimClock>().steps_per_day;
        for _ in 0..spd / 2 {
            s.step_tick();
        }
        // Every living person is there once, with a target.
        let alive = s
            .world
            .resource::<crate::AgentStore>()
            .alive
            .iter()
            .filter(|a| **a)
            .count();
        let snap0 = ViewSnapshot::capture(&s.world, &mut seen);
        assert_eq!(snap0.people.id.len(), alive);
        assert_eq!(snap0.people.target.len(), alive);
        assert!(snap0
            .people
            .doing
            .iter()
            .any(|d| matches!(d, Doing::Working(_))));
        // Building ids are unique.
        let snap = ViewSnapshot::capture(&s.world, &mut seen);
        let mut ids: Vec<u64> = snap
            .towns
            .iter()
            .flat_map(|t| t.buildings.iter().map(|b| b.id))
            .collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }
}
