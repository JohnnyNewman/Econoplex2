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
    pub treasury: f32,
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
    /// Whether seed grain is held back (always under the town food rules).
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
    /// Each person's own culture: the first three ideological axes of their
    /// mind (tradition, faith, liberty; 0..1, 0.5 neutral).
    #[serde(default)]
    pub culture: Vec<[f32; 3]>,
}

/// How much of the event log and the monthly history a view already has.
#[derive(Debug, Clone, Copy, Default)]
pub struct Seen {
    pub events: usize,
    pub samples: usize,
}

/// Everything a view needs for one tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewSnapshot {
    pub tick: u64,
    pub day_of_year: u32,
    pub days_per_year: u32,
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

impl ViewSnapshot {
    /// Copy what a view needs out of the world. `seen` says how much of the
    /// event log and history the view already has; it is advanced past what is
    /// returned here.
    pub fn capture(world: &World, seen: &mut Seen) -> Self {
        let clock = world.resource::<SimClock>();
        let needs = needs_by_town(world);
        let armories = armory_counts(world);
        let towns = world.resource::<Towns>();
        let db = world.resource::<Db>();
        let store = world.resource::<crate::AgentStore>();
        let mut people = PeopleView::default();
        for i in 0..store.len() {
            if !store.alive[i] {
                continue;
            }
            use crate::store::Activity;
            people.id.push(i as u32);
            people.town.push(store.town[i]);
            people.target.push(store.target[i]);
            let ideology = &store.mind[i][sim_data::dims::mind::IDEOLOGY_START..];
            people.culture.push(std::array::from_fn(|k| {
                ideology.get(k).copied().unwrap_or(0.5)
            }));
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
        ViewSnapshot {
            tick: clock.tick,
            day_of_year: clock.day_of_year(),
            days_per_year: clock.days_per_year,
            towns: towns
                .0
                .iter()
                .enumerate()
                .map(|(ti, t)| TownView {
                    needs: needs.get(ti).cloned().unwrap_or_default(),
                    granary_food: crate::granary::held(t, db),
                    granary_days: t.policy.granary_days,
                    seed_reserve: world
                        .get_resource::<crate::world::Params>()
                        .is_some_and(|p| crate::granary::keeps_reserve(t, p)),
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
                    name: t.name.clone(),
                    pos: t.pos,
                    population: t.residents.len() as u32,
                    treasury: t.treasury,
                    buildings: t
                        .buildings
                        .iter()
                        .map(|&(def, e, pos)| BuildingView {
                            id: e.to_bits(),
                            kind: db.content.products[def as usize].id.clone(),
                            pos,
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
                            .map(|&i| store.wealth[i as usize])
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
        }
    }

    /// Fraction of the year gone, 0..1 (for seasons).
    pub fn season(&self) -> f32 {
        self.day_of_year as f32 / self.days_per_year.max(1) as f32
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
        age_years: store.age_days[i] as f32
            / world.resource::<SimClock>().days_per_year.max(1) as f32,
        female: store.female[i],
        activity: activity.to_string(),
        job,
        wealth: store.wealth[i],
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
