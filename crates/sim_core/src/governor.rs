//! AI town governments.
//!
//! Every town the player doesn't govern has a governor: a few rules that look only at
//! the town's own numbers and pull the same levers the player has, by queuing the
//! same orders (`policy::Order`). Orders apply at the start of the next day, exactly
//! like the player's. A governor never sets an outcome; it changes taxes, subsidies,
//! spending, government orders and minting, and the agents respond.
//!
//! ```text
//! observe (smoothed)            decide (monthly)                 lever (order)
//! food per resident     low  -> subsidize food, clear a field -> subsidy TOWN bread 0.2
//! a good's price/base   high -> subsidize it if the town makes -> subsidy TOWN cloth 0.2
//! treasury per resident low  -> raise wealth, then income tax  -> tax TOWN wealth 0.75
//!                              ... and mint                     -> mint TOWN on
//! treasury per resident high -> lower taxes, spend more         -> spending TOWN 2.5
//! idle adults           high -> order public works             -> procure TOWN plank 60
//! ```
//!
//! Observations are moving averages, and a lever moves one step per decision, so
//! towns don't swing back and forth. Thresholds, step sizes and personalities
//! (frugal, balanced, expansionist) are in the `governor` section of `models.ron`; a
//! town's personality is set in `scenario.ron`.

use crate::db::Db;
use crate::policy::{Minting, Order, OrderQueue, PlayerTown, Tax};
use crate::store::{Activity, AgentStore};
use crate::world::{Params, ScenarioSpec, SimClock, Towns};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Decisions kept per town for the HUD.
pub const KEEP_DECISIONS: usize = 6;

/// What one governor remembers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GovernorState {
    /// Smoothed food per resident, treasury per resident and idle share of adults.
    pub food: f32,
    pub treasury: f32,
    pub idle: f32,
    /// Smoothed price / base price, by product.
    pub price_ratio: Vec<f32>,
    /// Checks in a row with the treasury low (for minting).
    pub broke_checks: u32,
    /// Latest decisions, newest last: (tick, what and why).
    pub decisions: VecDeque<(u64, String)>,
}

#[derive(Resource, Debug, Clone, Default, Serialize, Deserialize)]
pub struct Governors {
    pub towns: Vec<GovernorState>,
}

/// A governor's leaning; multiplies thresholds and step sizes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Personality {
    /// Above 1: taxes rise sooner and the treasury is spent later.
    pub thrift: f32,
    /// Above 1: bigger subsidies and public works.
    pub ambition: f32,
}

impl Default for Personality {
    fn default() -> Self {
        Personality {
            thrift: 1.0,
            ambition: 1.0,
        }
    }
}

impl Governors {
    pub fn new(towns: usize) -> Self {
        Governors {
            towns: vec![GovernorState::default(); towns],
        }
    }
}

fn note(state: &mut GovernorState, tick: u64, text: String) {
    state.decisions.push_back((tick, text));
    while state.decisions.len() > KEEP_DECISIONS {
        state.decisions.pop_front();
    }
}

/// Watch every AI town daily (smoothed), decide every `interval` days.
#[allow(clippy::too_many_arguments)]
pub fn govern(
    towns: Res<Towns>,
    store: Res<AgentStore>,
    db: Res<Db>,
    params: Res<Params>,
    clock: Res<SimClock>,
    player: Res<PlayerTown>,
    scenario: Res<ScenarioSpec>,
    mut governors: ResMut<Governors>,
    mut queue: ResMut<OrderQueue>,
) {
    let gp = &params.governor;
    if !gp.enabled {
        return;
    }
    let np = db.content.products.len();
    governors
        .towns
        .resize(towns.0.len(), GovernorState::default());
    let a = 1.0 / gp.smoothing_days.max(1.0);

    // Daily observation, smoothed.
    for (t, town) in towns.0.iter().enumerate() {
        let st = &mut governors.towns[t];
        let pop = town.residents.len().max(1) as f32;
        let food = town.food_stock(&db) / pop;
        let treasury = town.treasury / pop;
        let (mut adults, mut idle) = (0usize, 0usize);
        for &i in &town.residents {
            let i = i as usize;
            match store.activity[i] {
                Activity::Child | Activity::Captive | Activity::Soldier => {}
                Activity::Resting => adults += 1,
                _ => {
                    adults += 1;
                    if store.worked_today[i].is_none() {
                        idle += 1;
                    }
                }
            }
        }
        let idle = idle as f32 / adults.max(1) as f32;
        if st.price_ratio.len() != np {
            // First look: start from today's values.
            st.price_ratio = vec![1.0; np];
            st.food = food;
            st.treasury = treasury;
            st.idle = idle;
        }
        st.food += a * (food - st.food);
        st.treasury += a * (treasury - st.treasury);
        st.idle += a * (idle - st.idle);
        for p in 0..np {
            let r = town.price[p] / db.base_price[p].max(1e-6);
            st.price_ratio[p] += a * (r - st.price_ratio[p]);
        }
    }

    if gp.interval == 0 || clock.tick == 0 || !clock.tick.is_multiple_of(gp.interval as u64) {
        return;
    }
    let mp = &params.market;
    let max_subsidy = params.policy.max_subsidy;
    for (t, town) in towns.0.iter().enumerate() {
        if player.0 == Some(t as u16) && !gp.govern_player_town {
            continue;
        }
        let personality = scenario
            .0
            .towns
            .get(t)
            .and_then(|s| s.governor.as_ref())
            .and_then(|name| gp.personalities.iter().find(|p| &p.0 == name))
            .map_or_else(Personality::default, |p| p.1);
        let tid = t as u16;
        let p = &town.policy;
        let st = &mut governors.towns[t];
        let tick = clock.tick;
        let mut orders: Vec<(Order, String)> = Vec::new();
        let sub_step = gp.subsidy_step * personality.ambition;
        let sub_max = (gp.max_subsidy * personality.ambition).min(max_subsidy);

        // --- food: subsidize what feeds people while the town goes hungry
        let food_target = mp.food_target_per_capita;
        let hungry = st.food < gp.food_low * food_target;
        let fed = st.food > gp.food_ok * food_target;
        for &(f, _) in &db.foods {
            let f = f as usize;
            let now = p.subsidy[f];
            if hungry && now < sub_max {
                let share = (now + sub_step).min(sub_max);
                orders.push((
                    Order::Subsidy {
                        town: tid,
                        product: f as u32,
                        share,
                    },
                    format!(
                        "subsidized {} to {:.0}%: food is short",
                        db.content.products[f].name.to_lowercase(),
                        share * 100.0
                    ),
                ));
            } else if fed && now > 0.0 {
                let share = (now - sub_step).max(0.0);
                orders.push((
                    Order::Subsidy {
                        town: tid,
                        product: f as u32,
                        share,
                    },
                    format!(
                        "cut the {} subsidy to {:.0}%: enough food",
                        db.content.products[f].name.to_lowercase(),
                        share * 100.0
                    ),
                ));
            }
        }

        // A hungry town short of money buys food from its neighbours on credit;
        // once fed, it stops.
        if hungry && p.food_imports == crate::policy::Imports::Off {
            orders.push((
                Order::Imports {
                    town: tid,
                    imports: crate::policy::Imports::Auto,
                },
                "started food imports from neighbours: food is short".into(),
            ));
        } else if fed && p.food_imports == crate::policy::Imports::Auto {
            orders.push((
                Order::Imports {
                    town: tid,
                    imports: crate::policy::Imports::Off,
                },
                "stopped food imports: the town is fed".into(),
            ));
        }

        // A hungry town with land to spare also has a field cleared: subsidies don't
        // help when the fields are the limit.
        if hungry && town.free_land > 0 {
            let field = (0..np).find(|&g| {
                db.opens[g].is_some_and(|k| db.content.nature[k as usize].id == "field")
            });
            if let Some(g) = field {
                if !p.commissions.iter().any(|c| c.building == g as u32) {
                    orders.push((
                        Order::Build {
                            town: tid,
                            building: g as u32,
                            pos: None,
                        },
                        "commissioned a new field: food is short".into(),
                    ));
                }
            }
        }

        // --- shortages: subsidize goods the town makes but can't make enough of
        let mut short: Vec<(usize, f32)> = (0..np)
            .filter(|&g| {
                !db.is_food[g]
                    && db.coin[g] == 0.0
                    && db.category(g as u32) != sim_data::Category::Building
                    && town.produced_last_year[g] > 0.0
                    && st.price_ratio[g] >= gp.shortage_ratio
            })
            .map(|g| (g, st.price_ratio[g]))
            .collect();
        short.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
        let subsidized = (0..np)
            .filter(|&g| !db.is_food[g] && p.subsidy[g] > 0.0)
            .count();
        let room = gp.max_subsidized.saturating_sub(subsidized);
        for &(g, ratio) in short.iter().take(room.max(1)) {
            if p.subsidy[g] > 0.0 && p.subsidy[g] >= sub_max {
                continue;
            }
            if p.subsidy[g] == 0.0 && room == 0 {
                continue;
            }
            let share = (p.subsidy[g] + sub_step).min(sub_max);
            orders.push((
                Order::Subsidy {
                    town: tid,
                    product: g as u32,
                    share,
                },
                format!(
                    "subsidized {} to {:.0}%: it sells at {:.1}x its base price",
                    db.content.products[g].name.to_lowercase(),
                    share * 100.0,
                    ratio
                ),
            ));
        }
        for g in 0..np {
            if !db.is_food[g] && p.subsidy[g] > 0.0 && st.price_ratio[g] <= gp.recovered_ratio {
                let share = (p.subsidy[g] - sub_step).max(0.0);
                orders.push((
                    Order::Subsidy {
                        town: tid,
                        product: g as u32,
                        share,
                    },
                    format!(
                        "cut the {} subsidy to {:.0}%: the price is back to normal",
                        db.content.products[g].name.to_lowercase(),
                        share * 100.0
                    ),
                ));
            }
        }

        // --- treasury: tax when broke, mint if it stays broke; spend when flush
        let wealth_tax = p.wealth_tax.unwrap_or(mp.wealth_tax);
        let default_profit_tax = match mp.firms {
            crate::firms::FirmsParams::Workshops { profit_tax, .. } => profit_tax,
            crate::firms::FirmsParams::Treasury => 0.0,
        };
        let low = gp.treasury_low * personality.thrift;
        let high = gp.treasury_high * personality.thrift;
        if st.treasury < low {
            st.broke_checks += 1;
            if wealth_tax < params.policy.max_wealth_tax {
                let rate = (wealth_tax + gp.tax_step).min(params.policy.max_wealth_tax);
                orders.push((
                    Order::Tax {
                        town: tid,
                        tax: Tax::Wealth,
                        rate: Some(rate),
                    },
                    format!("raised the wealth tax to {rate:.2}/yr: treasury low"),
                ));
            } else if params.market.firms.on()
                && p.profit_tax.unwrap_or(default_profit_tax) < gp.max_profit_tax
            {
                let rate = (p.profit_tax.unwrap_or(default_profit_tax) + gp.income_tax_step)
                    .min(gp.max_profit_tax);
                orders.push((
                    Order::Tax {
                        town: tid,
                        tax: Tax::Profit,
                        rate: Some(rate),
                    },
                    format!(
                        "raised the profit tax to {:.0}%: treasury low, wealth tax at its cap",
                        rate * 100.0
                    ),
                ));
            } else if p.income_tax < gp.max_income_tax {
                let rate = (p.income_tax + gp.income_tax_step).min(gp.max_income_tax);
                orders.push((
                    Order::Tax {
                        town: tid,
                        tax: Tax::Income,
                        rate: Some(rate),
                    },
                    format!(
                        "raised the income tax to {:.0}%: treasury low, wealth tax at its cap",
                        rate * 100.0
                    ),
                ));
            }
            if st.broke_checks >= gp.mint_after_checks && p.minting != Minting::On {
                orders.push((
                    Order::Mint {
                        town: tid,
                        minting: Minting::On,
                    },
                    "set the mint to strike coin: treasury low for months".into(),
                ));
            }
        } else {
            st.broke_checks = 0;
            if p.minting == Minting::On {
                orders.push((
                    Order::Mint {
                        town: tid,
                        minting: Minting::Auto,
                    },
                    "set the mint back to auto: treasury recovered".into(),
                ));
            }
        }
        if st.treasury > high {
            if params.market.firms.on()
                && p.profit_tax.unwrap_or(default_profit_tax) > default_profit_tax
            {
                let rate = (p.profit_tax.unwrap_or(default_profit_tax) - gp.income_tax_step)
                    .max(default_profit_tax);
                orders.push((
                    Order::Tax {
                        town: tid,
                        tax: Tax::Profit,
                        rate: Some(rate),
                    },
                    format!(
                        "lowered the profit tax to {:.0}%: treasury full",
                        rate * 100.0
                    ),
                ));
            } else if p.income_tax > 0.0 {
                let rate = (p.income_tax - gp.income_tax_step).max(0.0);
                orders.push((
                    Order::Tax {
                        town: tid,
                        tax: Tax::Income,
                        rate: Some(rate),
                    },
                    format!(
                        "lowered the income tax to {:.0}%: treasury full",
                        rate * 100.0
                    ),
                ));
            } else if wealth_tax > mp.wealth_tax {
                let rate = (wealth_tax - gp.tax_step).max(mp.wealth_tax);
                orders.push((
                    Order::Tax {
                        town: tid,
                        tax: Tax::Wealth,
                        rate: Some(rate),
                    },
                    format!("lowered the wealth tax to {rate:.2}/yr: treasury full"),
                ));
            }
            let spending = p.public_spending.unwrap_or(mp.public_spending);
            let target = mp.public_spending * gp.max_spending_factor / personality.thrift;
            if spending < target {
                let rate = (spending + gp.spending_step).min(target);
                orders.push((
                    Order::Spending {
                        town: tid,
                        rate: Some(rate),
                    },
                    format!("raised spending to {rate:.1}/yr of the surplus: treasury full"),
                ));
            }
        } else if p.public_spending.is_some_and(|s| s > mp.public_spending) && st.treasury < low {
            orders.push((
                Order::Spending {
                    town: tid,
                    rate: None,
                },
                "cut spending back to the default: treasury low".into(),
            ));
        }

        // --- idle hands: public works, if there is money for wages
        if st.idle > gp.idle_high && st.treasury > low && p.procurement.is_empty() {
            let works = gp
                .public_works
                .iter()
                .filter_map(|id| db.content.product(id))
                .find(|&g| town.produced_last_year[g as usize] > 0.0);
            if let Some(g) = works {
                let amount = (gp.works_amount * personality.ambition).round();
                orders.push((
                    Order::Procure {
                        town: tid,
                        product: g,
                        amount,
                    },
                    format!(
                        "ordered {amount:.0} {} for public works: {:.0}% of adults idle",
                        db.content.products[g as usize].name.to_lowercase(),
                        st.idle * 100.0
                    ),
                ));
            }
        }

        // --- credit: lend to a firm in a short trade, while the treasury is flush
        if params.market.firms.on() && town.treasury > high * town.residents.len() as f32 {
            let lent = crate::firms::outstanding(&town.firms);
            if lent + gp.loan_amount <= gp.max_loan_share * town.treasury {
                let short = |k: usize| {
                    db.recipes
                        .iter()
                        .enumerate()
                        .filter(|(r, _)| db.recipe_firm[*r] == k)
                        .flat_map(|(_, m)| m.outputs.iter())
                        .map(|&(p, _)| st.price_ratio.get(p as usize).copied().unwrap_or(1.0))
                        .fold(0.0f32, f32::max)
                };
                let pick = (0..town.firms.len())
                    .filter(|&k| {
                        let f = &town.firms[k];
                        f.loans.is_empty() && f.cash < gp.loan_amount
                    })
                    .map(|k| (k, short(k)))
                    .filter(|&(_, s)| s >= gp.shortage_ratio)
                    .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)));
                if let Some((k, s)) = pick {
                    orders.push((
                        Order::Loan {
                            town: tid,
                            firm: k as u32,
                            amount: gp.loan_amount,
                            rate: gp.loan_rate,
                            days: gp.loan_days,
                        },
                        format!(
                            "lent {:.0} to the {} at {:.0}%: its goods sell at {:.1}x base",
                            gp.loan_amount,
                            crate::firms::firm_name(&db, db.firm_keys[k]),
                            gp.loan_rate * 100.0,
                            s
                        ),
                    ));
                }
            }
        }

        for (order, why) in orders {
            queue.pending.push(order);
            note(st, tick, why);
        }
    }
}
