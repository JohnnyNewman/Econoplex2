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
use crate::history::{Event, History};
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
    /// Goods in the market: (product def, stock, price), for goods in stock.
    pub market: Vec<(u32, f32, f32)>,
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
    /// Events logged since the previous capture (oldest first).
    pub events: Vec<Event>,
}

impl ViewSnapshot {
    /// Copy what a view needs out of the world. `events_seen` is how many events
    /// the view already has; it is advanced past the ones returned here.
    pub fn capture(world: &World, events_seen: &mut usize) -> Self {
        let clock = world.resource::<SimClock>();
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
        let events = world
            .get_resource::<History>()
            .map(|h| {
                let new = h.events.get(*events_seen..).unwrap_or_default().to_vec();
                *events_seen = h.events.len();
                new
            })
            .unwrap_or_default();
        ViewSnapshot {
            tick: clock.tick,
            day_of_year: clock.day_of_year(),
            days_per_year: clock.days_per_year,
            towns: towns
                .0
                .iter()
                .map(|t| TownView {
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
                    market: (0..t.stock.len())
                        .filter(|&p| t.stock[p] > 0.5)
                        .filter(|&p| db.category(p as u32) != sim_data::Category::Building)
                        .map(|p| (p as u32, t.stock[p], t.price[p]))
                        .collect(),
                })
                .collect(),
            people,
            domains: db.domain_dirs.len() as u32,
            events,
        }
    }

    /// Fraction of the year gone, 0..1 (for seasons).
    pub fn season(&self) -> f32 {
        self.day_of_year as f32 / self.days_per_year.max(1) as f32
    }
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
    fn capturing_does_not_change_the_simulation() {
        let (mut a, mut b) = (sim(), sim());
        let mut seen = 0;
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
        let mut seen = 0;
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
