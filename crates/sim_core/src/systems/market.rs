//! Prices from stock levels, caravan trade between towns, nature regrowth.

use crate::components::NaturalResource;
use crate::db::Db;
use crate::world::{Params, SimClock, Towns};
use bevy_ecs::prelude::*;
use sim_data::Category;

pub fn settle(
    mut towns: ResMut<Towns>,
    db: Res<Db>,
    params: Res<Params>,
    clock: Res<SimClock>,
    mut sites: Query<&mut NaturalResource>,
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

    // Caravans: move bulk goods from cheap to expensive towns when the margin covers transport.
    if mp.trade_interval > 0 && clock.tick.is_multiple_of(mp.trade_interval as u64) {
        let n = towns.0.len();
        for p in 0..np {
            if db.category(p as u32).is_item() || db.category(p as u32) == Category::Building {
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
                    let amount = mp.trade_amount.min(towns.0[a].stock[p] * 0.5).floor();
                    if amount < 1.0 {
                        continue;
                    }
                    let q = towns.0[a].quality[p];
                    towns.0[a].stock[p] -= amount;
                    towns.0[a].treasury += amount * pa;
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
