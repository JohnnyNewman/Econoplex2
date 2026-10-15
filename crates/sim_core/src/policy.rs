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
//! - **Army**: the share of adults kept under arms, and whether the squad raids on
//!   its own judgment or only defends; a raid order sends it against a chosen town.
//! - **Construction**: a commissioned building is paid for at full price whatever
//!   the market thinks, and goes up where the order placed it.

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

/// Who decides when the squad marches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum War {
    /// The squad raids whenever it sees a good chance (the AI towns).
    #[default]
    Auto,
    /// The squad only marches when ordered to.
    Defend,
}

/// A building the town has ordered, and where to put it (`None`: the next free spot).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Commission {
    pub building: u32,
    pub pos: Option<(f32, f32)>,
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
    /// Share of adults under arms; `None` keeps the default from `models.ron`.
    pub army_share: Option<f32>,
    pub war: War,
    /// A raid ordered against this town, launched as soon as the squad can march.
    pub raid_order: Option<u16>,
    /// Buildings ordered and not yet built, in order.
    pub commissions: Vec<Commission>,
}

impl Policy {
    pub fn new(products: usize, domains: usize) -> Self {
        Policy {
            subsidy: vec![0.0; products],
            school: None,
            immigration: Immigration::Open,
            charters: vec![false; domains],
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Order {
    Subsidy {
        town: u16,
        product: u32,
        share: f32,
    },
    School {
        town: u16,
        school: Option<School>,
    },
    Immigration {
        town: u16,
        policy: Immigration,
    },
    Charter {
        town: u16,
        domain: u32,
        on: bool,
    },
    Army {
        town: u16,
        share: Option<f32>,
    },
    War {
        town: u16,
        war: War,
    },
    Raid {
        town: u16,
        target: u16,
    },
    Build {
        town: u16,
        building: u32,
        pos: Option<(f32, f32)>,
    },
}

impl Order {
    pub fn town(&self) -> u16 {
        match *self {
            Order::Subsidy { town, .. }
            | Order::School { town, .. }
            | Order::Immigration { town, .. }
            | Order::Charter { town, .. }
            | Order::Army { town, .. }
            | Order::War { town, .. }
            | Order::Raid { town, .. }
            | Order::Build { town, .. } => town,
        }
    }

    /// Parse a text order, as typed in the CLI or a script:
    ///
    /// ```text
    /// subsidy <town> <product> <share>       e.g. subsidy Greenvale bread 0.3
    /// school <town> <domain> <seats>         school <town> off
    /// immigration <town> open|closed|encouraged
    /// charter <town> <domain> on|off
    /// army <town> <share>|auto               e.g. army Greenvale 0.1
    /// war <town> auto|defend
    /// raid <town> <target>
    /// build <town> <building> [<x> <y>]
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
            ["army", t, "auto"] => Ok(Order::Army {
                town: town(t)?,
                share: None,
            }),
            ["army", t, share] => Ok(Order::Army {
                town: town(t)?,
                share: Some(number(share)?),
            }),
            ["war", t, w @ ("auto" | "defend")] => Ok(Order::War {
                town: town(t)?,
                war: if *w == "auto" { War::Auto } else { War::Defend },
            }),
            ["raid", t, target] => Ok(Order::Raid {
                town: town(t)?,
                target: town(target)?,
            }),
            ["build", t, b, rest @ ..] if rest.is_empty() || rest.len() == 2 => {
                let building = db
                    .content
                    .product(b)
                    .filter(|&p| db.category(p) == sim_data::Category::Building)
                    .ok_or_else(|| format!("no building `{b}`"))?;
                let pos = match rest {
                    [x, y] => Some((number(x)?, number(y)?)),
                    _ => None,
                };
                Ok(Order::Build {
                    town: town(t)?,
                    building,
                    pos,
                })
            }
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
            Order::Army { share, .. } => {
                p.army_share = share.map(|s| s.clamp(0.0, params.policy.max_army_share));
            }
            Order::War { war, .. } => p.war = war,
            Order::Raid { town: t, target } => {
                p.raid_order = (target != t).then_some(target);
            }
            Order::Build { building, pos, .. } => p.commissions.push(Commission { building, pos }),
        }
        queue.log.push((clock.tick, order));
    }
}
