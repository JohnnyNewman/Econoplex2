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
    mut items: Query<(&Product, &mut TownId), With<Item>>,
) {
    let mp = &params.market;
    let np = db.content.products.len();
    // Demand-driven pricing: the stock each good's users need.
    if mp.mint_money_per_capita > 0.0 {
        for town in towns.0.iter_mut() {
            let money = town.treasury
                + town.firms.iter().map(|f| f.cash).sum::<f32>()
                + town
                    .residents
                    .iter()
                    .map(|&i| store.wealth[i as usize])
                    .sum::<f32>();
            town.money_per_capita = money / town.residents.len().max(1) as f32;
        }
    }
    firm_finances(&mut towns, &mut store, &db, &params);
    crate::consumption::spend(&mut towns, &mut store, &db, &params, clock.tick);
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
                })
                .collect()
        });
    for town in towns.0.iter_mut() {
        town.input_use.resize(np, 0.0);
        town.input_today.resize(np, 0.0);
        for p in 0..np {
            let today = std::mem::take(&mut town.input_today[p]);
            town.input_use[p] += (today - town.input_use[p]) / crate::ledger::MONTH_DAYS;
        }
        town.goods_use.resize(np, 0.0);
        town.goods_today.resize(np, 0.0);
        for p in 0..np {
            let today = std::mem::take(&mut town.goods_today[p]);
            town.goods_use[p] += (today - town.goods_use[p]) / crate::ledger::MONTH_DAYS;
        }
    }
    // Public pay rates: yesterday's prices for jobs nobody has tried lately.
    super::work::post_pay(&mut towns, &db, &params);

    for (ti, town) in towns.0.iter_mut().enumerate() {
        let pop = town.residents.len().max(1) as f32;
        let mint_wanted = town.mint_wanted(mp);
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
                // A building is worth paying for when the ones the town has are nearly
                // full, or, at a discount, when it has none (a new line of work).
                // Only while what it makes is scarce: its goods sell above base price.
                let base = db.base_price[p];
                let scarcity = db
                    .recipes
                    .iter()
                    .filter(|m| m.building == Some(p as u32))
                    .map(|m| {
                        let (now, normal) = m.outputs.iter().fold((0.0, 0.0), |acc, &(o, q)| {
                            (
                                acc.0 + town.price[o as usize] * q,
                                acc.1 + db.base_price[o as usize] * q,
                            )
                        });
                        now / normal.max(1e-6)
                    })
                    .fold(0.0f32, f32::max);
                let demand = (scarcity - 1.0).clamp(0.0, 1.0);
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
                town.price[p] = if town.building_count(p as u32) == 0 {
                    base * mp.new_building_appeal
                } else {
                    let excess = (town.building_load[p] - mp.build_threshold)
                        / (1.0 - mp.build_threshold).max(1e-3);
                    base * excess.clamp(0.0, 1.0) * demand
                };
                continue;
            }
            let per_capita = if db.is_food[p] {
                mp.food_target_per_capita
            } else {
                mp.target_per_capita
            };
            // Goods the government ordered raise the stock the market wants.
            let wanted = match (&derived, mp.pricing) {
                (Some(d), crate::config::Pricing::Demand { cover_days, .. }) => {
                    // Inputs: what trades use; final goods: the per-resident
                    // target; food both; plus what households buy.
                    let households = cover_days * town.goods_use.get(p).copied().unwrap_or(0.0);
                    households
                        + if db.is_input[p] {
                            d[ti][p] + if db.is_food[p] { per_capita * pop } else { 0.0 }
                        } else {
                            per_capita * pop
                        }
                }
                _ => per_capita * pop,
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
                    town.price[p] = (old * (mp.price_adjust * pressure).exp())
                        .clamp(base * mp.min_price_factor, base * cap);
                }
            }
            if old > 0.0 && town.price[p] > 0.0 {
                town.price_vol.resize(3, Default::default());
                let k = class_index(db.content.products[p].price_class());
                town.price_vol[k].add((town.price[p] / old).ln().abs());
            }
            if let crate::config::ScarcityCap::Loosen { rate, max_factor } = mp.scarcity_cap {
                town.price_cap.resize(np, mp.max_price_factor);
                let c = &mut town.price_cap[p];
                if pressure > 0.0 && town.price[p] >= base * *c * 0.999 {
                    *c = (*c * (1.0 + rate)).min(max_factor.max(mp.max_price_factor));
                } else {
                    *c = (*c * (1.0 - rate)).max(mp.max_price_factor);
                }
            }
        }
    }

    let dpy = clock.days_per_year as f32;
    for town in towns.0.iter_mut() {
        for &(food, _) in &db.foods {
            town.stock[food as usize] *= 1.0 - mp.food_spoilage / dpy;
        }
    }

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
        let excess = store.wealth[i] - mp.tax_free_wealth;
        if excess > 0.0 {
            let tax = excess * rates[store.town[i] as usize];
            store.wealth[i] -= tax;
            let town = &mut towns.0[store.town[i] as usize];
            town.treasury += tax;
            town.ledger.add(Flow::WealthTax, tax);
        }
    }

    // Public spending: a treasury above its reserve pays the excess out to residents.
    for town in towns.0.iter_mut() {
        let n = town.residents.len();
        let excess = town.treasury - mp.treasury_reserve_per_capita * n as f32;
        if n == 0 || excess <= 0.0 {
            continue;
        }
        let rate = town.policy.public_spending.unwrap_or(mp.public_spending);
        let spend = excess * (rate / dpy).min(1.0);
        town.treasury -= spend;
        town.ledger.add(Flow::PublicSpending, spend);
        let share = spend / n as f32;
        for &i in &town.residents {
            store.wealth[i as usize] += share;
        }
    }

    // Today's prices, for the year's average (price indices).
    for town in towns.0.iter_mut() {
        town.price_sum.resize(np, 0.0);
        for p in 0..np {
            town.price_sum[p] += town.price[p] as f64;
        }
        town.price_days += 1;
    }

    // Caravans: move bulk goods from cheap to expensive towns when the margin covers transport.
    if mp.trade_interval > 0 && clock.tick.is_multiple_of(mp.trade_interval as u64) {
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
                    let amount = mp
                        .trade_amount
                        .min(available * 0.5)
                        .min(towns.0[b].treasury.max(0.0) / pb)
                        .floor();
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
                                .position(|e| items.get(*e).is_ok_and(|(pr, _)| pr.def == p as u32))
                                .expect("counted above");
                            let e = towns.0[a].armory.swap_remove(k);
                            items.get_mut(e).unwrap().1 .0 = b as u16;
                            towns.0[b].armory.push(e);
                        }
                    }
                    towns.0[a].stock[p] = (towns.0[a].stock[p] - amount).max(0.0);
                    towns.0[a].treasury += amount * pb;
                    towns.0[a].ledger.add(Flow::TradeSales, amount * pb);
                    towns.0[a].ledger.export(b, amount * pb);
                    let tb = &mut towns.0[b];
                    tb.quality[p] =
                        (tb.stock[p] * tb.quality[p] + amount * q) / (tb.stock[p] + amount);
                    tb.stock[p] += amount;
                    tb.treasury -= amount * pb;
                    tb.ledger.add(Flow::TradePurchases, amount * pb);
                }
            }
        }
    }

    food_imports(&mut towns, &db, &params, &terrain, clock.tick);

    for mut n in sites.iter_mut() {
        n.amount = (n.amount + n.regrowth).min(n.capacity);
    }
}

/// The market buys good `p` from the town's firms (in firm order) up to its
/// target stock, at the town price, while the treasury can pay; then delivers
/// government orders from its stock.
fn buy_from_firms(town: &mut crate::world::Town, p: usize, target: f32) {
    let price = town.price[p].max(1e-6);
    let mut want = (target - town.stock[p]).max(0.0);
    for k in 0..town.firms.len() {
        if want < 1e-3 || town.treasury <= 0.0 {
            break;
        }
        let have = town.firms[k].stock.get(p).copied().unwrap_or(0.0);
        let q = have.min(want).min(town.treasury / price);
        if q <= 0.0 {
            continue;
        }
        let pay = q * price;
        town.firms[k].stock[p] -= q;
        town.firms[k].cash += pay;
        town.firms[k].book(crate::firms::FirmFlow::Sales, pay);
        town.treasury -= pay;
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
        town.policy.procurement.retain(|o| o.1 > 1e-3);
    }
}

/// With workshops as firms: start-up cash from the treasury for firms that have
/// none yet, and each firm's daily payout above its reserve to its owners (the
/// town's guild members of its domain; the treasury if there are none), less
/// the profit tax.
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
        ..
    } = params.market.firms
    else {
        return;
    };
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
            if f.cash < 0.2 * start_cash {
                f.short_days += 1;
            } else {
                f.short_days = 0;
                f.closed = false;
            }
            if f.started && f.short_days < close_days {
                continue;
            }
            f.started = true;
            f.short_days = 0;
            let mut need = (start_cash - f.cash).max(0.0);
            let grant = need.min(0.5 * town.treasury.max(0.0) / n);
            if grant > 0.0 {
                town.treasury -= grant;
                town.ledger.add(Flow::FirmGrants, grant);
                f.cash += grant;
                f.capital += grant;
                f.book(crate::firms::FirmFlow::Grants, grant);
                need -= grant;
            }
            if let Some(m) = owners.get(&domain[k]).filter(|m| !m.is_empty()) {
                let ask = need / m.len() as f32;
                for &i in m {
                    let put = ask.min(0.2 * store.wealth[i as usize].max(0.0));
                    store.wealth[i as usize] -= put;
                    f.cash += put;
                    f.capital += put;
                    f.book(crate::firms::FirmFlow::Capital, put);
                }
            }
            f.closed = f.cash < 0.2 * start_cash;
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
                let pay = o.1.min(town.firms[k].cash.max(0.0));
                if pay <= 0.0 {
                    break;
                }
                town.firms[k].cash -= pay;
                super::work::receive(town, o.0, pay);
                o.1 -= pay;
            }
            owed.retain(|o| o.1 > 1e-4);
            town.firms[k].payables = owed;
        }
        // Loans: each day's instalment, interest first, out of the firm's cash. A
        // closed firm's overdue loans default: the rest is written off.
        let dpy = params.life.days_per_year as f32;
        for f in town.firms.iter_mut() {
            let closed = f.closed;
            let mut loans = std::mem::take(&mut f.loans);
            for l in loans.iter_mut() {
                let (interest, principal) = l.due(dpy);
                let paid_i = interest.min(f.cash.max(0.0));
                let paid_p = principal.min((f.cash - paid_i).max(0.0));
                f.cash -= paid_i + paid_p;
                f.book(crate::firms::FirmFlow::Interest, paid_i);
                f.book(crate::firms::FirmFlow::Repayments, paid_p);
                town.treasury += paid_i + paid_p;
                town.ledger.add(Flow::LoanInterest, paid_i);
                town.ledger.add(Flow::LoanRepayments, paid_p);
                l.left -= paid_p;
                l.days_left = l.days_left.saturating_sub(1);
                if l.days_left == 0 && l.left > 1e-3 {
                    if closed {
                        town.loan_defaults += l.left;
                        l.left = 0.0;
                    } else {
                        // Overdue: the rest falls due over another month.
                        l.days_left = 30;
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
            let out = f.payout(reserve_days, payout_rate);
            if out <= 0.0 {
                continue;
            }
            f.cash -= out;
            let members = owners.get(&domain[k]);
            let tax = match members {
                Some(m) if !m.is_empty() => {
                    out * town.policy.profit_tax.unwrap_or(profit_tax).clamp(0.0, 1.0)
                }
                _ => out,
            };
            town.treasury += tax;
            town.ledger.add(Flow::ProfitTax, tax);
            f.book(crate::firms::FirmFlow::ProfitTax, tax);
            if let Some(m) = members.filter(|m| !m.is_empty()) {
                let each = (out - tax) / m.len() as f32;
                for &i in m {
                    store.wealth[i as usize] += each;
                }
                f.book(crate::firms::FirmFlow::Dividends, out - tax);
            }
        }
    }
}

/// Emergency food imports. Every trade round, a town importing (policy on, or
/// auto while short of food) buys food from neighbours with stock to spare, at
/// the seller's price plus transport, paying from its treasury and owing the
/// rest. Each day a town with debts pays a share of its treasury above its
/// reserve toward them; debts grow with interest.
fn food_imports(towns: &mut Towns, db: &Db, params: &Params, terrain: &Terrain, tick: u64) {
    use crate::policy::Imports;
    let mp = &params.market;
    let ip = &mp.imports;
    let n = towns.0.len();
    let dpy = params.life.days_per_year as f32;
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
        let pay =
            ((towns.0[b].treasury - reserve).max(0.0) * ip.repay_share.clamp(0.0, 1.0)).min(owed);
        if pay <= 0.0 {
            continue;
        }
        for a in 0..n {
            let share = pay * towns.0[b].debts[a] / owed;
            if share <= 0.0 {
                continue;
            }
            towns.0[b].debts[a] -= share;
            towns.0[b].treasury -= share;
            towns.0[b].ledger.add(Flow::DebtRepaid, share);
            towns.0[a].treasury += share;
            towns.0[a].ledger.add(Flow::DebtReceived, share);
        }
    }
    if mp.trade_interval == 0 || !tick.is_multiple_of(mp.trade_interval as u64) {
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
                let paid = cost.min(towns.0[b].treasury.max(0.0));
                let q = towns.0[a].quality[f];
                towns.0[a].stock[f] -= amount;
                towns.0[a].treasury += paid;
                towns.0[a].ledger.add(Flow::TradeSales, paid);
                towns.0[a].ledger.export(b, cost);
                let tb = &mut towns.0[b];
                tb.quality[f] = (tb.stock[f] * tb.quality[f] + amount * q) / (tb.stock[f] + amount);
                tb.stock[f] += amount;
                tb.treasury -= paid;
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

/// For demand-driven pricing, each town's target stock of every good used as an
/// input: `cover_days` × (its trades' daily use + what profitable trades held up
/// only by its shortage would use). Also closes the day's input count.
fn derived_targets(
    towns: &Towns,
    db: &Db,
    params: &Params,
    sites: &Query<&mut NaturalResource>,
) -> Option<Vec<Vec<f32>>> {
    let crate::config::Pricing::Demand {
        cover_days,
        blocked_workers,
    } = params.market.pricing
    else {
        return None;
    };
    let np = db.content.products.len();
    Some(
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
            .collect(),
    )
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
    items: &Query<(&Product, &mut TownId), With<Item>>,
) -> usize {
    armory
        .iter()
        .filter(|e| items.get(**e).is_ok_and(|(p, _)| p.def == def))
        .count()
}
