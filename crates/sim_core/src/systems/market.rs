//! Prices from stock levels, spoilage, taxes and public spending, caravan trade
//! between towns, nature regrowth.

use crate::components::{Item, NaturalResource, Product, TownId};
use crate::db::Db;
use crate::ledger::Flow;
use crate::store::AgentStore;
use crate::terrain::Terrain;
use crate::world::{Params, SimClock, Towns};
use bevy_ecs::prelude::*;
use sim_data::Category;

#[allow(clippy::too_many_arguments)]
pub fn settle(
    mut towns: ResMut<Towns>,
    mut store: ResMut<AgentStore>,
    db: Res<Db>,
    params: Res<Params>,
    clock: Res<SimClock>,
    terrain: Res<Terrain>,
    mut sites: Query<&mut NaturalResource>,
    mut items: Query<(&Product, &mut TownId, Option<&mut crate::goods::Owner>), With<Item>>,
    env: Res<crate::env::Environment>,
) {
    let mp = &params.market;
    let cal = clock.calendar();
    let np = db.content.products.len();
    // The market's count of tools and weapons is what its armory holds
    // (weapons carried by soldiers are no longer counted).
    for town in towns.0.iter_mut() {
        let mut held = vec![0.0f32; np];
        for &e in &town.armory {
            if let Ok((pr, _, _)) = items.get(e) {
                held[pr.def as usize] += 1.0;
            }
        }
        for (p, &h) in held.iter().enumerate() {
            if db.category(p as u32).is_item() && town.stock[p] != h {
                let gone = town.stock[p] - h;
                town.stock[p] = h;
                town.flows.add(crate::goods::Flow::Equipped, p, gone);
            }
        }
    }
    // Demand-driven pricing: the stock each good's users need.
    if mp.mint_money_per_capita > 0.0 {
        for town in towns.0.iter_mut() {
            let money = town.treasury.coins()
                + town.firms.iter().map(|f| f.cash.coins()).sum::<f32>()
                + town
                    .residents
                    .iter()
                    .map(|&i| store.wealth[i as usize].coins())
                    .sum::<f32>();
            town.money_per_capita = money / town.residents.len().max(1) as f32;
        }
    }
    firm_finances(&mut towns, &mut store, &db, &params);
    crate::consumption::spend(&mut towns, &mut store, &db, &params, &env, clock.steps());
    let derived = derived_targets(&towns, &db, &params, &sites);
    // Each good's price dynamics: its class's, or its own override.
    let dynamics: Option<Vec<crate::pricing::ClassParams>> =
        mp.price_classes.as_ref().map(|classes| {
            db.content
                .products
                .iter()
                .map(|p| {
                    mp.price_overrides
                        .iter()
                        .find(|o| o.0 == p.id)
                        .map(|o| o.1)
                        .unwrap_or(*classes.get(p.price_class()))
                        .over(cal)
                })
                .collect()
        });
    // Daily use, averaged over about a month of days: this step's use over
    // the days it stands for.
    let month = cal.smoothing(crate::ledger::MONTH_DAYS);
    for town in towns.0.iter_mut() {
        town.input_use.resize(np, 0.0);
        town.input_today.resize(np, 0.0);
        for p in 0..np {
            let today = std::mem::take(&mut town.input_today[p]) / cal.days();
            town.input_use[p] += (today - town.input_use[p]) * month;
        }
        town.goods_use.resize(np, 0.0);
        town.goods_today.resize(np, 0.0);
        for p in 0..np {
            let today = std::mem::take(&mut town.goods_today[p]) / cal.days();
            town.goods_use[p] += (today - town.goods_use[p]) * month;
        }
    }
    // Public pay rates: yesterday's prices for jobs nobody has tried lately.
    super::work::post_pay(&mut towns, &db, &params);
    // The recipes that make each good from inputs, and how much of it a run
    // makes (for the cost floor below).
    let mut makers: Vec<Vec<(usize, f32)>> = vec![Vec::new(); np];
    for (r, m) in db.recipes.iter().enumerate() {
        if m.inputs.is_empty() {
            continue;
        }
        for &(o, q) in &m.outputs {
            match makers[o as usize].iter_mut().find(|x| x.0 == r) {
                Some(x) => x.1 += q,
                None => makers[o as usize].push((r, q)),
            }
        }
    }

    for (ti, town) in towns.0.iter_mut().enumerate() {
        let pop = town.residents.len().max(1) as f32;
        let mint_wanted = town.mint_wanted(mp);
        // The pay of the town's poorer jobs (the lower quartile of those that
        // pay), at the start of the step.
        let poor_pay = {
            let mut paid: Vec<f32> = town.pay_rate.iter().copied().filter(|&x| x > 0.0).collect();
            if paid.is_empty() {
                0.0
            } else {
                let q = paid.len() / 4;
                *paid.select_nth_unstable_by(q, f32::total_cmp).1
            }
        };
        for p in 0..np {
            if db.coin[p] > 0.0 {
                // Coin is worth its face value, while the town wants more of it.
                town.price[p] = if mint_wanted { db.coin[p] } else { 0.0 };
                continue;
            }
            if let Some(kind) = db.opens[p] {
                town.price[p] = land_price(&db, town, mp, p, kind, &sites);
                continue;
            }
            if db.category(p as u32) == Category::Building {
                // A building is worth what its places would earn (see
                // `BuildingValue`), or, at a discount, when the town has none (a
                // new line of work).
                let base = db.base_price[p];
                // A building the government ordered is paid for at full price.
                if town
                    .policy
                    .commissions
                    .iter()
                    .any(|c| c.building == p as u32)
                {
                    town.price[p] = base;
                    continue;
                }
                // A mint is worth building when the town wants coin and has none.
                let mints = db.recipes.iter().any(|m| {
                    m.building == Some(p as u32)
                        && m.outputs.iter().any(|o| db.coin[o.0 as usize] > 0.0)
                });
                if mints {
                    town.price[p] = if mint_wanted && town.building_count(p as u32) == 0 {
                        base
                    } else {
                        0.0
                    };
                    continue;
                }
                if town.building_count(p as u32) == 0 {
                    town.price[p] = base * mp.new_building_appeal;
                    continue;
                }
                let crate::config::BuildingValue {
                    payback_days,
                    max_factor,
                } = mp.building_value;
                // What the workers who would fill it earn now: the pay of
                // the town's poorer jobs.
                let typical = poor_pay;
                let best = db
                    .recipes
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.building == Some(p as u32))
                    .map(|(r, _)| town.pay_rate.get(r).copied().unwrap_or(0.0))
                    .fold(0.0f32, f32::max);
                let slots = db.content.products[p].slots.max(1) as f32;
                // New places only earn while the ones the town has are full.
                let full = town.building_load[p] >= mp.build_threshold;
                let gain = if full {
                    slots * (best - typical).max(0.0) * payback_days
                } else {
                    0.0
                };
                town.price[p] = gain.min(base * max_factor);
                continue;
            }
            let per_capita = if db.is_food[p] {
                mp.food_target_per_capita
            } else {
                mp.target_per_capita
            };
            // Goods the government ordered raise the stock the market wants.
            // Inputs: what trades use; final goods: the per-resident target;
            // food both; plus what households buy.
            let cover_days = mp.pricing.cover_days;
            let households = cover_days * town.goods_use.get(p).copied().unwrap_or(0.0);
            let wanted = households
                + if db.is_input[p] {
                    derived[ti][p] + if db.is_food[p] { per_capita * pop } else { 0.0 }
                } else {
                    per_capita * pop
                };
            let target = wanted.max(1.0) + town.policy.ordered(p);
            // With workshops as firms, the market buys from their stock what it
            // wants, while the treasury can pay, and fills government orders.
            if mp.firms.on()
                && town
                    .firms
                    .iter()
                    .any(|f| f.stock.get(p).is_some_and(|&s| s > 0.0))
            {
                buy_from_firms(town, p, target);
            }
            let pressure = ((target - town.stock[p]) / target).clamp(-1.0, 1.0);
            let base = db.base_price[p];
            let cap = town
                .price_cap
                .get(p)
                .copied()
                .unwrap_or(mp.max_price_factor);
            let old = town.price[p];
            match &dynamics {
                Some(d) => {
                    town.pressure.resize(np, 0.0);
                    let mut c = d[p];
                    // A loosened scarcity cap widens the class's ceiling too.
                    c.ceiling = c.ceiling.max(cap * c.ceiling / mp.max_price_factor);
                    town.price[p] = c.step(old, base, pressure, &mut town.pressure[p]);
                }
                None => {
                    town.price[p] = (old * (cal.flow(mp.price_adjust) * pressure).exp())
                        .clamp(base * mp.min_price_factor, base * cap);
                }
            }
            // Nobody sells below cost: a made good is worth at least its inputs
            // at today's prices plus a day's pay of the town's poorer jobs for
            // the work, by its cheapest recipe. Without this flour fell below
            // grain in winter and milling paid nothing while bread ran short.
            let cost = makers[p]
                .iter()
                .filter(|&&(_, out)| out > 0.0)
                .map(|&(r, out)| {
                    let m = &db.recipes[r];
                    let inputs: f32 = m
                        .inputs
                        .iter()
                        .map(|&(i, q)| town.price[i as usize] * q)
                        .sum();
                    (inputs + poor_pay * m.duration) / out
                })
                .fold(f32::INFINITY, f32::min);
            if cost.is_finite() {
                town.price[p] = town.price[p].max(cost.min(base * cap));
            }
            if old > 0.0 && town.price[p] > 0.0 {
                town.price_vol.resize(3, Default::default());
                let k = class_index(db.content.products[p].price_class());
                // Recorded as a move per day, the step's spread over its days.
                town.price_vol[k].add((town.price[p] / old).ln().abs() / cal.days());
            }
            if let crate::config::ScarcityCap::Loosen { rate, max_factor } = mp.scarcity_cap {
                town.price_cap.resize(np, mp.max_price_factor);
                let c = &mut town.price_cap[p];
                if pressure > 0.0 && town.price[p] >= base * *c * 0.999 {
                    *c = (*c * (1.0 + rate).powf(cal.days()))
                        .min(max_factor.max(mp.max_price_factor));
                } else {
                    *c = (*c * cal.keep(1.0 - rate)).max(mp.max_price_factor);
                }
            }
        }
    }

    // Yearly rates over the steps of a year.
    let dpy = cal.per_year() as f32;
    for town in towns.0.iter_mut() {
        for &(food, _) in &db.foods {
            let f = food as usize;
            let rotted = town.stock[f] * (mp.food_spoilage / dpy);
            town.stock[f] *= 1.0 - mp.food_spoilage / dpy;
            town.flows.add(crate::goods::Flow::Spoiled, f, rotted);
        }
    }
    crate::granary::tend(&mut towns, &db, &params, dpy, env.season);

    // Wealth tax: savings above the allowance flow back to the town treasury.
    // Each town's own rate.
    let rates: Vec<f32> = towns
        .0
        .iter()
        .map(|t| t.policy.wealth_tax.unwrap_or(mp.wealth_tax) / dpy)
        .collect();
    for i in 0..store.len() {
        if !store.alive[i] {
            continue;
        }
        let excess = store.wealth[i].coins() - mp.tax_free_wealth;
        if excess > 0.0 {
            let tax = excess * rates[store.town[i] as usize];
            store.wealth[i] -= crate::coin::Coin::from_coins(tax);
            let town = &mut towns.0[store.town[i] as usize];
            town.treasury += crate::coin::Coin::from_coins(tax);
            town.ledger.add(Flow::WealthTax, tax);
        }
    }

    // Public spending: a treasury above its reserve pays the excess out to residents.
    for town in towns.0.iter_mut() {
        let n = town.residents.len();
        let excess = town.treasury.coins() - mp.treasury_reserve_per_capita * n as f32;
        if n == 0 || excess <= 0.0 {
            continue;
        }
        let rate = town.policy.public_spending.unwrap_or(mp.public_spending);
        let spend = excess * (rate / dpy).min(1.0);
        // Equal shares to the unit; what does not divide stays in the treasury.
        let (share, _) = crate::coin::Coin::from_coins(spend).split(n);
        town.treasury -= crate::coin::Coin(share.0 * n as i64);
        town.ledger
            .add(Flow::PublicSpending, share.coins() * n as f32);
        for &i in &town.residents {
            store.wealth[i as usize] += share;
        }
    }

    // Today's prices, for the year's average (price indices).
    for town in towns.0.iter_mut() {
        town.index_year.price_sum.resize(np, 0.0);
        for p in 0..np {
            town.index_year.price_sum[p] += town.price[p] as f64;
        }
        town.index_year.price_days += 1;
    }

    // Caravans: move bulk goods from cheap to expensive towns when the margin covers transport.
    let trade_round = clock.every_months(mp.trade_interval);
    if trade_round {
        let n = towns.0.len();
        for p in 0..np {
            let cat = db.category(p as u32);
            if cat == Category::Weapon || cat == Category::Building {
                continue;
            }
            for a in 0..n {
                for b in 0..n {
                    if a == b {
                        continue;
                    }
                    let (pa, pb) = (towns.0[a].price[p], towns.0[b].price[p]);
                    // Transport costs grow with the length of the road, around
                    // rivers and over hills; no caravan goes where no road leads.
                    let road = terrain.road(a, b);
                    if !road.is_finite() || pb <= pa * (1.0 + mp.transport_cost * road / 1000.0) {
                        continue;
                    }
                    // The buyer pays the destination price; the margin over the source
                    // price pays for transport and stays with the seller's merchants.
                    let available = if cat.is_item() {
                        armory_count(&towns.0[a].armory, p as u32, &items) as f32
                    } else {
                        towns.0[a].stock[p]
                    };
                    let load = match mp.caravans {
                        crate::config::Caravans::Fixed => mp.trade_amount.min(available * 0.5),
                        // The seller's surplus over its own target, up to what
                        // the buyer lacks of its target.
                        crate::config::Caravans::Balance { max_load } => {
                            let surplus = available
                                - super::decide::target_stock(&db, &towns.0[a], &params, p);
                            let lack = super::decide::target_stock(&db, &towns.0[b], &params, p)
                                - towns.0[b].stock[p];
                            surplus.min(lack).min(max_load).max(0.0)
                        }
                    };
                    let amount = load.min(towns.0[b].treasury.coins().max(0.0) / pb).floor();
                    if amount < 1.0 {
                        continue;
                    }
                    let q = towns.0[a].quality[p];
                    if cat.is_item() {
                        // Tools travel as entities between the armories.
                        for _ in 0..amount as u32 {
                            let k = towns.0[a]
                                .armory
                                .iter()
                                .position(|e| {
                                    items.get(*e).is_ok_and(|(pr, _, _)| pr.def == p as u32)
                                })
                                .expect("counted above");
                            let e = towns.0[a].armory.swap_remove(k);
                            let (_, mut tid, owner) = items.get_mut(e).unwrap();
                            tid.0 = b as u16;
                            if let Some(mut o) = owner {
                                o.0 = crate::goods::Holder::Armory(b as u16);
                            }
                            towns.0[b].armory.push(e);
                        }
                    }
                    towns.0[a].stock[p] = (towns.0[a].stock[p] - amount).max(0.0);
                    towns.0[a].treasury += crate::coin::Coin::from_coins(amount * pb);
                    towns.0[a].ledger.add(Flow::TradeSales, amount * pb);
                    towns.0[a].ledger.export(b, amount * pb);
                    let tb = &mut towns.0[b];
                    tb.quality[p] =
                        (tb.stock[p] * tb.quality[p] + amount * q) / (tb.stock[p] + amount);
                    tb.stock[p] += amount;
                    tb.treasury -= crate::coin::Coin::from_coins(amount * pb);
                    tb.ledger.add(Flow::TradePurchases, amount * pb);
                }
            }
        }
    }

    food_imports(&mut towns, &db, &params, &terrain, trade_round);

    crate::farming::regrow(&towns, &db, &params, &env, &mut sites);
}

/// The market buys good `p` from the town's firms (in firm order) up to its
/// target stock, at the town price, while the treasury can pay; then delivers
/// government orders from its stock.
fn buy_from_firms(town: &mut crate::world::Town, p: usize, target: f32) {
    let price = town.price[p].max(1e-6);
    let mut want = (target - town.stock[p]).max(0.0);
    for k in 0..town.firms.len() {
        if want < 1e-3 || town.treasury.coins() <= 0.0 {
            break;
        }
        let have = town.firms[k].stock.get(p).copied().unwrap_or(0.0);
        let q = have.min(want).min(town.treasury.coins() / price);
        if q <= 0.0 {
            continue;
        }
        let pay = q * price;
        town.firms[k].stock[p] -= q;
        town.firms[k].cash += crate::coin::Coin::from_coins(pay);
        town.firms[k].book(crate::firms::FirmFlow::Sales, pay);
        town.treasury -= crate::coin::Coin::from_coins(pay);
        town.ledger.add(Flow::MarketPurchases, pay);
        town.stock[p] += q;
        want -= q;
    }
    if let Some(o) = town
        .policy
        .procurement
        .iter_mut()
        .find(|o| o.0 as usize == p)
    {
        let delivered = town.stock[p].min(o.1);
        o.1 -= delivered;
        town.stock[p] -= delivered;
        town.procured[p] += delivered;
        town.flows.add(crate::goods::Flow::Procured, p, delivered);
        town.policy.procurement.retain(|o| o.1 > 1e-3);
    }
}

/// With workshops as firms: start-up cash from the treasury for firms that have
/// none yet, and each firm's daily payout above its reserve to its owners (the
/// town's guild members of its domain; the treasury if there are none), less
/// the profit tax.
/// Workshops' stock targets: each firm keeps `input_days` of the inputs it has
/// been using in its own store, bought ahead from the market with the cash
/// above its wage reserve.
fn firm_input_stocks(
    towns: &mut crate::world::Towns,
    db: &Db,
    cal: crate::calendar::Calendar,
    reserve_days: f32,
    input_days: f32,
) {
    let np = db.content.products.len();
    let month = cal.smoothing(30.0);
    for town in towns.0.iter_mut() {
        for k in 0..town.firms.len() {
            let f = &mut town.firms[k];
            f.input_rate.resize(np, 0.0);
            f.input_used.resize(np, 0.0);
            for p in 0..np {
                f.input_rate[p] += (f.input_used[p] / cal.days() - f.input_rate[p]) * month;
                f.input_used[p] = 0.0;
            }
            if input_days <= 0.0 || f.closed {
                continue;
            }
            f.inputs.resize(np, 0.0);
            let mut spare = f.cash.coins() - reserve_days * f.wage_bill;
            for p in 0..np {
                let rate = town.firms[k].input_rate[p];
                if rate <= 1e-3 || spare <= 0.0 {
                    continue;
                }
                let price = town.price[p].max(1e-3);
                let want = rate * input_days - town.firms[k].inputs[p];
                let q = want.min(town.stock[p].max(0.0)).min(spare / price).floor();
                if q < 1.0 {
                    continue;
                }
                let cost = q * price;
                town.stock[p] -= q;
                town.treasury += crate::coin::Coin::from_coins(cost);
                town.ledger.add(crate::ledger::Flow::InputSales, cost);
                let f = &mut town.firms[k];
                f.inputs[p] += q;
                f.cash -= crate::coin::Coin::from_coins(cost);
                f.book(crate::firms::FirmFlow::Inputs, cost);
                spare -= cost;
            }
        }
    }
}

pub fn firm_finances(
    towns: &mut crate::world::Towns,
    store: &mut AgentStore,
    db: &Db,
    params: &Params,
) {
    let crate::firms::FirmsParams::Workshops {
        start_cash,
        reserve_days,
        payout_rate,
        profit_tax,
        close_days,
        input_days,
        ..
    } = params.market.firms
    else {
        return;
    };
    let cal = params.calendar();
    firm_input_stocks(towns, db, cal, reserve_days, input_days);
    // Each firm's domain: its guild, or the domain of its first recipe.
    let domain: Vec<u32> = (0..db.firm_keys.len())
        .map(|k| match db.firm_keys[k] {
            crate::firms::FirmKey::Guild(d) => d,
            _ => db
                .recipes
                .iter()
                .enumerate()
                .find(|(r, _)| db.recipe_firm[*r] == k)
                .map_or(0, |(_, m)| m.domain),
        })
        .collect();
    for town in towns.0.iter_mut() {
        if town.firms.len() != db.firm_keys.len() {
            town.firms = crate::firms::new_firms(db);
        }
        // Owners per domain.
        let mut owners: std::collections::BTreeMap<u32, Vec<u32>> = Default::default();
        for &i in &town.residents {
            if let Some(g) = store.guild[i as usize] {
                owners.entry(g.domain).or_default().push(i);
            }
        }
        // Capital: a new firm, or one short of cash for `close_days`, raises
        // `start_cash`: from the treasury (at most a fair share of half of it),
        // then from its owners (each at most a fifth of their wealth). A firm that
        // still has too little closes until capital comes.
        let n = town.firms.len().max(1) as f32;
        for (k, f) in town.firms.iter_mut().enumerate() {
            if f.cash.coins() < 0.2 * start_cash {
                f.short_days += cal.days();
            } else {
                f.short_days = 0.0;
                f.closed = false;
            }
            if f.started && f.short_days < close_days as f32 {
                continue;
            }
            f.started = true;
            f.short_days = 0.0;
            let mut need = (start_cash - f.cash.coins()).max(0.0);
            let grant = need.min(0.5 * town.treasury.coins().max(0.0) / n);
            if grant > 0.0 {
                town.treasury -= crate::coin::Coin::from_coins(grant);
                town.ledger.add(Flow::FirmGrants, grant);
                f.cash += crate::coin::Coin::from_coins(grant);
                f.capital += grant;
                f.book(crate::firms::FirmFlow::Grants, grant);
                need -= grant;
            }
            if let Some(m) = owners.get(&domain[k]).filter(|m| !m.is_empty()) {
                let ask = need / m.len() as f32;
                for &i in m {
                    let put = ask.min(0.2 * store.wealth[i as usize].coins().max(0.0));
                    store.wealth[i as usize] -= crate::coin::Coin::from_coins(put);
                    f.cash += crate::coin::Coin::from_coins(put);
                    f.capital += put;
                    f.book(crate::firms::FirmFlow::Capital, put);
                }
            }
            f.closed = f.cash.coins() < 0.2 * start_cash;
        }
        // Trade credit: debts on account are paid from cash, oldest first. A
        // closed firm's debts are written off (its creditors lose them).
        for k in 0..town.firms.len() {
            if town.firms[k].payables.is_empty() {
                continue;
            }
            let mut owed = std::mem::take(&mut town.firms[k].payables);
            if town.firms[k].closed {
                town.credit_defaults += owed.iter().map(|o| o.1).sum::<f32>();
                continue;
            }
            for o in owed.iter_mut() {
                let pay = o.1.min(town.firms[k].cash.coins().max(0.0));
                if pay <= 0.0 {
                    break;
                }
                town.firms[k].cash -= crate::coin::Coin::from_coins(pay);
                super::work::receive(town, o.0, pay);
                o.1 -= pay;
            }
            owed.retain(|o| o.1 > 1e-4);
            town.firms[k].payables = owed;
        }
        // Loans: each step's instalment, interest first, out of the firm's cash.
        // A closed firm's overdue loans default: the rest is written off.
        for f in town.firms.iter_mut() {
            let closed = f.closed;
            let mut loans = std::mem::take(&mut f.loans);
            for l in loans.iter_mut() {
                let (interest, principal) = l.due(cal);
                let paid_i = interest.min(f.cash.coins().max(0.0));
                let paid_p = principal.min((f.cash.coins() - paid_i).max(0.0));
                f.cash -= crate::coin::Coin::from_coins(paid_i + paid_p);
                f.book(crate::firms::FirmFlow::Interest, paid_i);
                f.book(crate::firms::FirmFlow::Repayments, paid_p);
                town.treasury += crate::coin::Coin::from_coins(paid_i + paid_p);
                town.ledger.add(Flow::LoanInterest, paid_i);
                town.ledger.add(Flow::LoanRepayments, paid_p);
                l.left -= paid_p;
                l.days_left = (l.days_left - cal.days()).max(0.0);
                if l.days_left <= 0.0 && l.left > 1e-3 {
                    if closed {
                        town.loan_defaults += l.left;
                        l.left = 0.0;
                    } else {
                        // Overdue: the rest falls due over another month.
                        l.days_left = 30.0;
                    }
                }
            }
            loans.retain(|l| l.left > 1e-3);
            f.loans = loans;
        }
        for (k, f) in town.firms.iter_mut().enumerate() {
            // No payouts while a firm owes the treasury or its suppliers.
            if !f.loans.is_empty() || !f.payables.is_empty() {
                continue;
            }
            let out = f.payout(reserve_days, cal.share(payout_rate));
            if out <= 0.0 {
                continue;
            }
            let out_coin = crate::coin::Coin::from_coins(out);
            f.cash -= out_coin;
            let members = owners.get(&domain[k]);
            let tax = match members {
                Some(m) if !m.is_empty() => {
                    out * town.policy.profit_tax.unwrap_or(profit_tax).clamp(0.0, 1.0)
                }
                _ => out,
            };
            // The owners share the rest equally; units that do not divide go
            // to the treasury with the tax.
            let mut tax_coin = crate::coin::Coin::from_coins(tax).min(out_coin);
            if let Some(m) = members.filter(|m| !m.is_empty()) {
                let (each, rest) = (out_coin - tax_coin).split(m.len());
                tax_coin += rest;
                for &i in m {
                    store.wealth[i as usize] += each;
                }
            } else {
                tax_coin = out_coin;
            }
            town.treasury += tax_coin;
            town.ledger.add(Flow::ProfitTax, tax);
            f.book(crate::firms::FirmFlow::ProfitTax, tax);
            if let Some(_m) = members.filter(|m| !m.is_empty()) {
                f.book(crate::firms::FirmFlow::Dividends, out - tax);
            }
        }
    }
}

/// Emergency food imports. Every trade round, a town importing (policy on, or
/// auto while short of food) buys food from neighbours with stock to spare, at
/// the seller's price plus transport, paying from its treasury and owing the
/// rest. Each step a town with debts pays the step's share of its treasury above its
/// reserve toward them; debts grow with interest.
fn food_imports(towns: &mut Towns, db: &Db, params: &Params, terrain: &Terrain, round: bool) {
    use crate::policy::Imports;
    let mp = &params.market;
    let ip = &mp.imports;
    let n = towns.0.len();
    let cal = params.calendar();
    let dpy = cal.per_year() as f32;
    for t in towns.0.iter_mut() {
        t.debts.resize(n, 0.0);
    }
    // Interest and repayment.
    for b in 0..n {
        if towns.0[b].debts.iter().all(|&d| d <= 0.0) {
            continue;
        }
        for d in towns.0[b].debts.iter_mut() {
            *d *= 1.0 + ip.debt_rate.max(0.0) / dpy;
        }
        let reserve = mp.treasury_reserve_per_capita * towns.0[b].residents.len() as f32;
        let owed: f32 = towns.0[b].debts.iter().sum();
        let pay = ((towns.0[b].treasury.coins() - reserve).max(0.0) * cal.share(ip.repay_share))
            .min(owed);
        if pay <= 0.0 {
            continue;
        }
        for a in 0..n {
            let share = pay * towns.0[b].debts[a] / owed;
            if share <= 0.0 {
                continue;
            }
            towns.0[b].debts[a] -= share;
            towns.0[b].treasury -= crate::coin::Coin::from_coins(share);
            towns.0[b].ledger.add(Flow::DebtRepaid, share);
            towns.0[a].treasury += crate::coin::Coin::from_coins(share);
            towns.0[a].ledger.add(Flow::DebtReceived, share);
        }
    }
    if !round {
        return;
    }
    for b in 0..n {
        let pop = towns.0[b].residents.len().max(1) as f32;
        let importing = match towns.0[b].policy.food_imports {
            Imports::Off => false,
            Imports::On => true,
            Imports::Auto => {
                towns.0[b].food_stock(db) / pop < ip.hungry_below * mp.food_target_per_capita
            }
        };
        if !importing {
            continue;
        }
        // Food still wanted, in food value.
        let mut need = (mp.food_target_per_capita * pop - towns.0[b].food_stock(db)).max(0.0);
        for &(f, value) in &db.foods {
            let f = f as usize;
            for a in 0..n {
                if a == b || need <= 0.0 {
                    continue;
                }
                let road = terrain.road(a, b);
                if !road.is_finite() {
                    continue;
                }
                let keep =
                    ip.seller_keeps * mp.food_target_per_capita * towns.0[a].residents.len() as f32;
                let spare = towns.0[a].stock[f] - keep / value.max(1e-3);
                let amount = ip
                    .amount
                    .min(spare * 0.5)
                    .min(need / value.max(1e-3))
                    .floor();
                if amount < 1.0 {
                    continue;
                }
                let price = towns.0[a].price[f] * (1.0 + mp.transport_cost * road / 1000.0);
                let cost = amount * price;
                let paid = cost.min(towns.0[b].treasury.coins().max(0.0));
                let q = towns.0[a].quality[f];
                towns.0[a].stock[f] -= amount;
                towns.0[a].treasury += crate::coin::Coin::from_coins(paid);
                towns.0[a].ledger.add(Flow::TradeSales, paid);
                towns.0[a].ledger.export(b, cost);
                let tb = &mut towns.0[b];
                tb.quality[f] = (tb.stock[f] * tb.quality[f] + amount * q) / (tb.stock[f] + amount);
                tb.stock[f] += amount;
                tb.treasury -= crate::coin::Coin::from_coins(paid);
                tb.ledger.add(Flow::TradePurchases, paid);
                tb.debts[a] += cost - paid;
                need -= amount * value;
            }
        }
    }
}

/// Index of a price class in `Town::price_vol`.
pub fn class_index(c: crate::pricing::PriceClass) -> usize {
    match c {
        crate::pricing::PriceClass::Essential => 0,
        crate::pricing::PriceClass::Commodity => 1,
        crate::pricing::PriceClass::Labour => 2,
    }
}

/// Each town's target stock of every good used as an input: `cover_days` ×
/// (its trades' daily use + what profitable trades held up only by its
/// shortage would use).
fn derived_targets(
    towns: &Towns,
    db: &Db,
    params: &Params,
    sites: &Query<&mut NaturalResource>,
) -> Vec<Vec<f32>> {
    let crate::config::Pricing {
        cover_days,
        blocked_workers,
    } = params.market.pricing;
    let np = db.content.products.len();
    towns
        .0
        .iter()
        .map(|town| {
            let mut nature = vec![0.0f32; db.content.nature.len()];
            for &e in &town.nature_sites {
                if let Ok(n) = sites.get(e) {
                    nature[n.kind as usize] += n.amount;
                }
            }
            let mut want = vec![0.0f32; np];
            for (p, w) in want.iter_mut().enumerate() {
                *w = town.input_use.get(p).copied().unwrap_or(0.0);
            }
            for (r, m) in db.recipes.iter().enumerate() {
                if m.inputs.is_empty()
                    || m.builds.is_some()
                    || m.building.is_some_and(|b| town.building_count(b) == 0)
                    || m.nature.is_some_and(|k| nature[k as usize] <= 0.0)
                    || super::work::posted_pay(town, db, params, r) <= 0.0
                {
                    continue;
                }
                for &(p, q) in &m.inputs {
                    if town.stock[p as usize] < q {
                        want[p as usize] += q * blocked_workers / m.duration.max(1.0);
                    }
                }
            }
            want.iter().map(|w| w * cover_days).collect()
        })
        .collect()
}

/// New land is worth opening while the town's sites of that kind are worn down
/// (harvested faster than they regrow) and what they yield sells above base
/// price; at a discount when the town has none yet; never once its land is used up.
fn land_price(
    db: &Db,
    town: &crate::world::Town,
    mp: &crate::config::MarketParams,
    p: usize,
    kind: u32,
    sites: &Query<&mut NaturalResource>,
) -> f32 {
    if town.free_land == 0 {
        return 0.0;
    }
    let (amount, capacity) = town
        .nature_sites
        .iter()
        .filter_map(|&e| sites.get(e).ok())
        .filter(|n| n.kind == kind)
        .fold((0.0, 0.0), |acc, n| (acc.0 + n.amount, acc.1 + n.capacity));
    let base = db.base_price[p];
    if capacity <= 0.0 {
        return base * mp.new_building_appeal;
    }
    let scarcity = db
        .recipes
        .iter()
        .filter(|m| m.nature == Some(kind))
        .flat_map(|m| m.outputs.iter())
        .map(|&(o, _)| town.price[o as usize] / db.base_price[o as usize].max(1e-6))
        .fold(0.0f32, f32::max);
    let demand = (scarcity - 1.0).clamp(0.0, 1.0);
    let worn = 1.0 - amount / capacity;
    let excess = (worn - mp.land_threshold) / (1.0 - mp.land_threshold).max(1e-3);
    base * excess.clamp(0.0, 1.0) * demand
}

fn armory_count(
    armory: &[Entity],
    def: u32,
    items: &Query<(&Product, &mut TownId, Option<&mut crate::goods::Owner>), With<Item>>,
) -> usize {
    armory
        .iter()
        .filter(|e| items.get(**e).is_ok_and(|(p, _, _)| p.def == def))
        .count()
}
