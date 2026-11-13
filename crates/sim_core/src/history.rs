//! History as data: an event log and monthly time series per town.
//!
//! One system, `record`, runs at the end of each day. It compares each town
//! with how it was the day before and logs what changed:
//! - raids won and lost, conquests, revolts;
//! - buildings finished, land opened;
//! - famines (starving starts / ends after a quiet month);
//! - migration waves (many arrivals in a month);
//! - every government order (player's and AI's).
//!
//! Every `SAMPLE_DAYS` it also records a sample per town: population, food per
//! resident, treasury, mean wealth, soldiers, buildings, starving person-days in
//! the month, bread price and ruler.
//!
//! Nothing in the simulation reads it back; it is for the HUD, exports
//! (`--events`, `--history` in the CLI) and analysis. Saves keep it.

use crate::db::Db;
use crate::policy::{Order, OrderQueue};
use crate::store::AgentStore;
use crate::world::{SimClock, Towns};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

/// Days between samples.
pub const SAMPLE_DAYS: u64 = 30;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EventKind {
    RaidWon,
    RaidLost,
    Conquered,
    Freed,
    Built,
    LandOpened,
    FamineStarted,
    FamineEnded,
    MigrationWave,
    Order,
}

impl EventKind {
    pub fn name(&self) -> &'static str {
        match self {
            EventKind::RaidWon => "raid won",
            EventKind::RaidLost => "raid lost",
            EventKind::Conquered => "conquered",
            EventKind::Freed => "freed",
            EventKind::Built => "built",
            EventKind::LandOpened => "land opened",
            EventKind::FamineStarted => "famine",
            EventKind::FamineEnded => "famine over",
            EventKind::MigrationWave => "migration wave",
            EventKind::Order => "order",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub tick: u64,
    pub town: u16,
    pub kind: EventKind,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Sample {
    pub tick: u64,
    pub town: u16,
    pub population: u32,
    pub food_per_capita: f32,
    pub treasury: f32,
    pub mean_wealth: f32,
    pub soldiers: u32,
    pub buildings: u32,
    /// Person-days spent starving since the last sample.
    pub starving: u32,
    pub bread_price: f32,
    pub ruler: Option<u16>,
}

/// What the recorder remembers of each town from the day before.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Snapshot {
    raids_won: u32,
    raids_lost: u32,
    ruler: Option<u16>,
    buildings: usize,
    land: usize,
    starving: u32,
    starving_at_sample: u32,
    famine: bool,
    quiet_days: u32,
    arrived: u32,
    arrived_at_sample: u32,
}

#[derive(Resource, Debug, Clone, Default, Serialize, Deserialize)]
pub struct History {
    pub events: Vec<Event>,
    pub samples: Vec<Sample>,
    snapshots: Vec<Snapshot>,
    /// Orders already logged (index into `OrderQueue::log`).
    orders_seen: usize,
}

impl History {
    /// The last `n` events, newest first.
    pub fn recent(&self, n: usize) -> impl Iterator<Item = &Event> {
        self.events.iter().rev().take(n)
    }
}

/// A short description of an order.
pub fn describe(order: &Order, db: &Db, towns: &Towns) -> String {
    let product = |p: u32| {
        db.content
            .products
            .get(p as usize)
            .map_or("?".to_string(), |d| d.name.to_lowercase())
    };
    let town = |t: u16| towns.0.get(t as usize).map_or("?", |t| t.name.as_str());
    match order {
        Order::Subsidy {
            product: p, share, ..
        } => {
            format!("subsidy on {} {:.0}%", product(*p), share * 100.0)
        }
        Order::Tax { tax, rate, .. } => match rate {
            Some(r) => format!("{tax:?} tax {r:.2}").to_lowercase(),
            None => format!("{tax:?} tax auto").to_lowercase(),
        },
        Order::Procure {
            product: p, amount, ..
        } => {
            format!("ordered {amount:.0} {}", product(*p))
        }
        Order::Build { building, .. } => format!("commissioned a {}", product(*building)),
        Order::Raid { target, .. } => format!("ordered a raid on {}", town(*target)),
        Order::Loan { firm, amount, .. } => format!(
            "lent {amount:.0} to the {}",
            db.firm_keys
                .get(*firm as usize)
                .map_or("?".into(), |&k| crate::firms::firm_name(db, k))
        ),
        Order::Imports { imports, .. } => format!("food imports {imports:?}").to_lowercase(),
        Order::Granary { days, .. } => format!("granary {days:.0} days of food"),
        Order::Reserve { on, .. } => {
            format!("seed reserve {}", if *on { "on" } else { "off" })
        }
        Order::Immigration { policy, .. } => format!("immigration {policy:?}").to_lowercase(),
        Order::Mint { minting, .. } => format!("minting {minting:?}").to_lowercase(),
        Order::Army { share, .. } => match share {
            Some(s) => format!("army {:.0}% of adults", s * 100.0),
            None => "army auto".into(),
        },
        Order::War { war, .. } => format!("war {war:?}").to_lowercase(),
        Order::Wages { share, .. } => match share {
            Some(s) => format!("wages {:.0}% of value", s * 100.0),
            None => "wages auto".into(),
        },
        Order::Spending { rate, .. } => match rate {
            Some(r) => format!("public spending {r:.1}"),
            None => "public spending auto".into(),
        },
        Order::School { school, .. } => match school {
            Some(s) => format!(
                "school for {} ({} seats)",
                db.content
                    .domains
                    .get(s.domain as usize)
                    .map_or("?", |d| d.as_str()),
                s.seats
            ),
            None => "closed the school".into(),
        },
        Order::Charter { domain, on, .. } => format!(
            "{} the {} guild charter",
            if *on { "granted" } else { "revoked" },
            db.content
                .domains
                .get(*domain as usize)
                .map_or("?", |d| d.as_str())
        ),
    }
}

/// Days without starving before a famine counts as over.
const QUIET_DAYS: u32 = 30;
/// Arrivals in a month, as a share of the population, that make a wave.
const WAVE_SHARE: f32 = 0.05;

#[allow(clippy::too_many_arguments)]
pub fn record(
    mut history: ResMut<History>,
    towns: Res<Towns>,
    store: Res<AgentStore>,
    db: Res<Db>,
    clock: Res<SimClock>,
    queue: Res<OrderQueue>,
) {
    let tick = clock.tick;
    let h = &mut *history;
    // Government orders given since yesterday (first: they came before today).
    for (otick, order) in queue.log.iter().skip(h.orders_seen) {
        let t = order.town() as usize;
        h.events.push(Event {
            tick: *otick,
            town: t as u16,
            kind: EventKind::Order,
            text: format!(
                "{}: {}",
                towns.0.get(t).map_or("?", |t| t.name.as_str()),
                describe(order, &db, &towns)
            ),
        });
    }
    h.orders_seen = queue.log.len();

    let first = h.snapshots.len() != towns.0.len();
    if first {
        h.snapshots = towns
            .0
            .iter()
            .map(|t| Snapshot {
                raids_won: t.war.raids_won,
                raids_lost: t.war.raids_lost,
                ruler: t.ruler,
                buildings: t.buildings.len(),
                land: t.nature_sites.len(),
                starving: t.starving,
                starving_at_sample: t.starving,
                arrived: t.moves.arrived,
                arrived_at_sample: t.moves.arrived,
                ..Default::default()
            })
            .collect();
    }
    let push = |events: &mut Vec<Event>, town: usize, kind: EventKind, text: String| {
        events.push(Event {
            tick,
            town: town as u16,
            kind,
            text,
        });
    };
    let bread = db.content.product("bread").map(|p| p as usize);
    for (t, town) in towns.0.iter().enumerate() {
        let s = &mut h.snapshots[t];
        // Counters reset at year end (metrics): follow them down.
        if town.war.raids_won < s.raids_won {
            s.raids_won = 0;
        }
        if town.war.raids_lost < s.raids_lost {
            s.raids_lost = 0;
        }
        if town.starving < s.starving {
            s.starving = 0;
            s.starving_at_sample = 0;
        }
        if town.moves.arrived < s.arrived {
            s.arrived = 0;
            s.arrived_at_sample = 0;
        }
        if town.war.raids_won > s.raids_won {
            push(
                &mut h.events,
                t,
                EventKind::RaidWon,
                format!("{} won a raid", town.name),
            );
        }
        if town.war.raids_lost > s.raids_lost {
            push(
                &mut h.events,
                t,
                EventKind::RaidLost,
                format!("{}'s raid failed", town.name),
            );
        }
        if town.ruler != s.ruler {
            match town.ruler {
                Some(r) => push(
                    &mut h.events,
                    t,
                    EventKind::Conquered,
                    format!(
                        "{} was conquered by {}",
                        town.name, towns.0[r as usize].name
                    ),
                ),
                None => push(
                    &mut h.events,
                    t,
                    EventKind::Freed,
                    format!("{} threw off its ruler", town.name),
                ),
            }
        }
        if town.buildings.len() > s.buildings {
            for b in &town.buildings[s.buildings.min(town.buildings.len())..] {
                push(
                    &mut h.events,
                    t,
                    EventKind::Built,
                    format!(
                        "{} finished a {}",
                        town.name,
                        db.content.products[b.0 as usize].name.to_lowercase()
                    ),
                );
            }
        }
        if town.nature_sites.len() > s.land {
            push(
                &mut h.events,
                t,
                EventKind::LandOpened,
                format!("{} opened new land", town.name),
            );
        }
        // Famine: people starving today, after a quiet month.
        let starving_today = town.starving > s.starving;
        if starving_today {
            s.quiet_days = 0;
            if !s.famine {
                s.famine = true;
                push(
                    &mut h.events,
                    t,
                    EventKind::FamineStarted,
                    format!("famine in {}", town.name),
                );
            }
        } else if s.famine {
            s.quiet_days += 1;
            if s.quiet_days >= QUIET_DAYS {
                s.famine = false;
                push(
                    &mut h.events,
                    t,
                    EventKind::FamineEnded,
                    format!("the famine in {} is over", town.name),
                );
            }
        }
        s.raids_won = town.war.raids_won;
        s.raids_lost = town.war.raids_lost;
        s.ruler = town.ruler;
        s.buildings = town.buildings.len();
        s.land = town.nature_sites.len();
        s.starving = town.starving;
        s.arrived = town.moves.arrived;

        // Monthly sample.
        if tick.is_multiple_of(SAMPLE_DAYS) {
            let pop = town.residents.len();
            let arrivals = town.moves.arrived - s.arrived_at_sample;
            if pop > 0 && arrivals as f32 > WAVE_SHARE * pop as f32 {
                push(
                    &mut h.events,
                    t,
                    EventKind::MigrationWave,
                    format!("{arrivals} people moved to {} this month", town.name),
                );
            }
            h.samples.push(Sample {
                tick,
                town: t as u16,
                population: pop as u32,
                food_per_capita: town.food_stock(&db) / pop.max(1) as f32,
                treasury: town.treasury,
                mean_wealth: town
                    .residents
                    .iter()
                    .map(|&i| store.wealth[i as usize])
                    .sum::<f32>()
                    / pop.max(1) as f32,
                soldiers: town.squad.len() as u32,
                buildings: town.buildings.len() as u32,
                starving: town.starving - s.starving_at_sample,
                bread_price: bread.map_or(0.0, |b| town.price[b]),
                ruler: town.ruler,
            });
            s.starving_at_sample = town.starving;
            s.arrived_at_sample = town.moves.arrived;
        }
    }
}
