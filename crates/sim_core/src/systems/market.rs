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
            if db.category(p as u32) == Category::Building {
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
