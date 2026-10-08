//! Player levers: town policies set by orders.
//!
//! A player (or a script, or later a network peer) never touches the world
//! directly: it queues [`Order`]s, and the first system of each tick applies them
//! in queue order. Orders are plain data, logged with the tick they took effect,
//! so a game can be replayed or kept in lockstep from its order log.
//!
//! The levers:
//! - **Subsidy**: the treasury pays a share of a product's base price on top of the
//!   market price to whoever makes it, and keeps buying it up to a larger stock,
//!   so more people choose that work.
//! - **School**: children of school age learn one skill domain from the town's
//!   best teacher, who is paid a fee per pupil and day from the treasury.
//! - **Immigration**: closed borders turn migrants away; encouraged immigration
//!   adds to the town's appeal and pays newcomers a settlement grant.
//! - **Guild charter**: a chartered guild draws people into its trade and its
//!   masters teach faster.

use crate::db::Db;
use crate::world::{SimClock, Towns};
use bevy_ecs::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Immigration {
    #[default]
    Open,
    Closed,
    Encouraged,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct School {
    pub domain: u32,
    /// Most pupils taught per day.
    pub seats: u32,
}

/// A town's policy. Every town starts with no subsidies, no school, open borders
/// and no charters.
#[derive(Debug, Clone, Default)]
pub struct Policy {
    /// Extra share of market value paid per unit made, by product.
    pub subsidy: Vec<f32>,
    pub school: Option<School>,
    pub immigration: Immigration,
    /// Chartered guilds, by skill domain.
    pub charters: Vec<bool>,
}

impl Policy {
    pub fn new(products: usize, domains: usize) -> Self {
        Policy {
            subsidy: vec![0.0; products],
            school: None,
            immigration: Immigration::Open,
            charters: vec![false; domains],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Order {
    Subsidy { town: u16, product: u32, share: f32 },
    School { town: u16, school: Option<School> },
    Immigration { town: u16, policy: Immigration },
    Charter { town: u16, domain: u32, on: bool },
}

impl Order {
    pub fn town(&self) -> u16 {
        match *self {
            Order::Subsidy { town, .. }
            | Order::School { town, .. }
            | Order::Immigration { town, .. }
            | Order::Charter { town, .. } => town,
        }
    }

    /// Parse a text order, as typed in the CLI or a script:
    ///
    /// ```text
    /// subsidy <town> <product> <share>       e.g. subsidy Greenvale bread 0.3
    /// school <town> <domain> <seats>         school <town> off
    /// immigration <town> open|closed|encouraged
    /// charter <town> <domain> on|off
    /// ```
    pub fn parse(text: &str, db: &Db, towns: &Towns) -> Result<Order, String> {
        let w: Vec<&str> = text.split_whitespace().collect();
        let town = |name: &str| {
            towns
                .0
                .iter()
                .position(|t| t.name.eq_ignore_ascii_case(name))
                .map(|t| t as u16)
                .ok_or_else(|| format!("no town `{name}`"))
        };
        let domain = |name: &str| {
            db.content
                .domain(name)
                .ok_or_else(|| format!("no skill domain `{name}`"))
        };
        let number = |s: &str| {
            s.parse::<f32>()
                .map_err(|_| format!("`{s}` is not a number"))
        };
        match w.as_slice() {
            ["subsidy", t, p, share] => Ok(Order::Subsidy {
                town: town(t)?,
                product: db
                    .content
                    .product(p)
                    .ok_or_else(|| format!("no product `{p}`"))?,
                share: number(share)?,
            }),
            ["school", t, "off"] => Ok(Order::School {
                town: town(t)?,
                school: None,
            }),
            ["school", t, d, seats] => Ok(Order::School {
                town: town(t)?,
                school: Some(School {
                    domain: domain(d)?,
                    seats: number(seats)?.max(0.0) as u32,
                }),
            }),
            ["immigration", t, p] => Ok(Order::Immigration {
                town: town(t)?,
                policy: match *p {
                    "open" => Immigration::Open,
                    "closed" => Immigration::Closed,
                    "encouraged" => Immigration::Encouraged,
                    other => {
                        return Err(format!(
                            "immigration is open, closed or encouraged, not `{other}`"
                        ))
                    }
                },
            }),
            ["charter", t, d, on @ ("on" | "off")] => Ok(Order::Charter {
                town: town(t)?,
                domain: domain(d)?,
                on: *on == "on",
            }),
            _ => Err(format!("cannot read order `{text}`")),
        }
    }
}

/// Orders waiting for the next tick, and every order applied so far.
#[derive(Resource, Default)]
pub struct OrderQueue {
    pub pending: Vec<Order>,
    pub log: Vec<(u64, Order)>,
}

/// The town the player governs, if any (from the scenario).
#[derive(Resource, Default)]
pub struct PlayerTown(pub Option<u16>);

/// First system of the tick: apply queued orders in order.
pub fn apply_orders(
    mut queue: ResMut<OrderQueue>,
    mut towns: ResMut<Towns>,
    params: Res<crate::world::Params>,
    clock: Res<SimClock>,
) {
    let max = params.policy.max_subsidy;
    let orders = std::mem::take(&mut queue.pending);
    for order in orders {
        let Some(town) = towns.0.get_mut(order.town() as usize) else {
            continue;
        };
        let p = &mut town.policy;
        match order {
            Order::Subsidy { product, share, .. } => {
                if let Some(s) = p.subsidy.get_mut(product as usize) {
                    *s = share.clamp(0.0, max);
                }
            }
            Order::School { school, .. } => {
                p.school = school.filter(|s| (s.domain as usize) < p.charters.len());
            }
            Order::Immigration { policy, .. } => p.immigration = policy,
            Order::Charter { domain, on, .. } => {
                if let Some(c) = p.charters.get_mut(domain as usize) {
                    *c = on;
                }
            }
        }
        queue.log.push((clock.tick, order));
    }
}
