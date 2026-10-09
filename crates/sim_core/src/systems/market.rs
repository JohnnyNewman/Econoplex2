//! Prices from stock levels, spoilage, taxes and public spending, caravan trade
//! between towns, nature regrowth.

use crate::components::{Item, NaturalResource, Product, TownId};
use crate::db::Db;
use crate::store::AgentStore;
use crate::world::{Params, SimClock, Towns};
use bevy_ecs::prelude::*;
use sim_data::Category;

pub fn settle(
    mut towns: ResMut<Towns>,
    mut store: ResMut<AgentStore>,
    db: Res<Db>,
    params: Res<Params>,
    clock: Res<SimClock>,
    mut sites: Query<&mut NaturalResource>,
    mut items: Query<(&Product, &mut TownId), With<Item>>,
) {
    let mp = &params.market;
    let np = db.content.products.len();

    for town in towns.0.iter_mut() {
        let pop = town.residents.len().max(1) as f32;
        for p in 0..np {
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
            let target = (per_capita * pop).max(1.0);
            let pressure = ((target - town.stock[p]) / target).clamp(-1.0, 1.0);
            let base = db.base_price[p];
            town.price[p] = (town.price[p] * (mp.price_adjust * pressure).exp())
                .clamp(base * mp.min_price_factor, base * mp.max_price_factor);
        }
    }

    let dpy = clock.days_per_year as f32;
    for town in towns.0.iter_mut() {
        for &(food, _) in &db.foods {
            town.stock[food as usize] *= 1.0 - mp.food_spoilage / dpy;
        }
    }

    // Wealth tax: savings above the allowance flow back to the town treasury.
    let rate = mp.wealth_tax / dpy;
    for i in 0..store.len() {
        if !store.alive[i] {
            continue;
        }
        let excess = store.wealth[i] - mp.tax_free_wealth;
        if excess > 0.0 {
            let tax = excess * rate;
            store.wealth[i] -= tax;
            towns.0[store.town[i] as usize].treasury += tax;
        }
    }

    // Public spending: a treasury above its reserve pays the excess out to residents.
    for town in towns.0.iter_mut() {
        let n = town.residents.len();
        let excess = town.treasury - mp.treasury_reserve_per_capita * n as f32;
        if n == 0 || excess <= 0.0 {
            continue;
        }
        let spend = excess * (mp.public_spending / dpy).min(1.0);
        town.treasury -= spend;
        let share = spend / n as f32;
        for &i in &town.residents {
            store.wealth[i as usize] += share;
        }
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
                    if pb <= pa * (1.0 + mp.transport_cost) {
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
                    let tb = &mut towns.0[b];
                    tb.quality[p] =
                        (tb.stock[p] * tb.quality[p] + amount * q) / (tb.stock[p] + amount);
                    tb.stock[p] += amount;
                    tb.treasury -= amount * pb;
                }
            }
        }
    }

    for mut n in sites.iter_mut() {
        n.amount = (n.amount + n.regrowth).min(n.capacity);
    }
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
