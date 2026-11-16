//! State dynamics: hunger, eating, health, fatigue recovery, happiness.
//!
//! Every economy step people get the step's hunger (`calendar.rs`) and eat
//! a meal when they are hungry, from the first source that has food; health,
//! fatigue and happiness move by the step's share of a day. A rest lasts
//! until the next simulated day (`rest_until`), then the person chooses again.

use crate::db::Db;
use crate::store::{Activity, AgentStore};
use crate::world::{Params, Towns};
use bevy_ecs::prelude::*;
use sim_data::dims::state::*;

/// Most meals one person eats in a step (more than one only when a step
/// stands for more than a day).
const MAX_MEALS: usize = 64;

#[allow(clippy::too_many_arguments)]
pub fn sense(
    mut store: ResMut<AgentStore>,
    mut towns: ResMut<Towns>,
    db: Res<Db>,
    params: Res<Params>,
    env: Res<crate::env::Environment>,
    clock: Res<crate::world::SimClock>,
    mut squads: ResMut<crate::goods::Squads>,
) {
    let p = &params.needs;
    let cal = params.calendar();
    let now = clock.steps();
    // Days of work a rest skips this step (indoor working hours).
    let rest_days = crate::env::work_days_this_step(&clock, &env, &params.time, false);
    // Starving is counted once a simulated day, as person-months.
    let count_starving = clock.day_start();
    let ladder = match &params.market.consumption {
        crate::consumption::ConsumptionParams::Needs(lp) => Some(lp),
        _ => None,
    };
    let store = &mut *store;

    // Food that a recipe turns into more food (grain into flour for bread) is held
    // back for a reserve of that work, so hungry people don't eat the seed of the
    // next harvest and lock the town into famine.
    let reserve: Vec<Vec<f32>> = towns
        .0
        .iter()
        .map(|town| {
            if town.policy.food_reserve {
                super::decide::food_reserve(&db, town, &params)
            } else {
                vec![0.0; town.stock.len()]
            }
        })
        .collect();

    // Rent from crowding, the same for every adult in a town. Rents are sticky:
    // they follow crowding over about two months, so a town that just filled up
    // does not send everyone away at once.
    let crowding = &params.decision.crowding;
    for town in towns.0.iter_mut() {
        let housing = crowding.housing(
            town.nature_sites.len() + town.free_land as usize,
            town.buildings.len(),
        );
        let target = crowding.rent(town.residents.len(), housing);
        town.rent += (target - town.rent) * cal.smoothing(crowding.stickiness_days());
    }

    for i in 0..store.len() {
        if !store.alive[i] {
            continue;
        }
        store.worked_today[i] = None;
        if !matches!(store.activity[i], Activity::Child | Activity::Captive) {
            let town = &mut towns.0[store.town[i] as usize];
            let pay = cal.flow(town.rent).min(store.wealth[i].coins().max(0.0));
            if pay > 0.0 {
                store.wealth[i] -= crate::coin::Coin::from_coins(pay);
                town.treasury += crate::coin::Coin::from_coins(pay);
                town.ledger.add(crate::ledger::Flow::Rent, pay);
            }
        }
        let town = &mut towns.0[store.town[i] as usize];
        let st = &mut store.state[i];

        st[HUNGER] += cal.flow(p.hunger_per_day);
        // Meals until the hunger is stilled: each meal is a portion from the
        // first source that has food (squad packs, larder, firms, the market).
        for _ in 0..MAX_MEALS {
            if st[HUNGER] <= p.eat_threshold {
                break;
            }
            let before = st[HUNGER];
            // On the march, people eat from their squad's packs first.
            if st[HUNGER] > p.eat_threshold && !squads.list.is_empty() {
                if let Some(id) = squads.squad_of(i) {
                    if let Some((food, value, q)) =
                        crate::goods::squad_meal(&mut squads, id, &db.foods, st[HUNGER])
                    {
                        town.flows.add(crate::goods::Flow::Eaten, food as usize, q);
                        st[HUNGER] = (st[HUNGER] - q * value).max(0.0);
                    }
                }
            }
            if st[HUNGER] > p.eat_threshold {
                // The household's own food first.
                if let Some((food, value, q)) = crate::decision::subsistence::take_portion(
                    &mut store.larder[i],
                    &db.foods,
                    st[HUNGER],
                ) {
                    town.flows.add(crate::goods::Flow::Eaten, food as usize, q);
                    st[HUNGER] = (st[HUNGER] - q * value).max(0.0);
                }
            }
            // With firms, residents buy food straight from the firms that hold it
            // first, then from the market.
            if st[HUNGER] > p.eat_threshold && params.market.firms.on() {
                'buy: for &(food, value) in &db.foods {
                    let f = food as usize;
                    let q = crate::decision::subsistence::portion(st[HUNGER], value);
                    for firm in town.firms.iter_mut() {
                        if firm.stock.get(f).is_some_and(|&s| s >= q) {
                            firm.stock[f] -= q;
                            town.flows.add(crate::goods::Flow::Eaten, f, q);
                            st[HUNGER] = (st[HUNGER] - q * value).max(0.0);
                            let pay = (q * town.price[f]).min(store.wealth[i].coins().max(0.0));
                            store.wealth[i] -= crate::coin::Coin::from_coins(pay);
                            firm.cash += crate::coin::Coin::from_coins(pay);
                            firm.book(crate::firms::FirmFlow::DirectSales, pay);
                            town.index_year.consumed.resize(town.stock.len(), 0.0);
                            town.index_year.consumed[f] += q;
                            break 'buy;
                        }
                    }
                }
            }
            if st[HUNGER] > p.eat_threshold {
                for &(food, value) in &db.foods {
                    let f = food as usize;
                    let q = crate::decision::subsistence::portion(st[HUNGER], value);
                    if q > 0.0 && town.stock[f] >= q + reserve[store.town[i] as usize][f] {
                        town.stock[f] -= q;
                        town.flows.add(crate::goods::Flow::Eaten, f, q);
                        town.index_year.consumed.resize(town.stock.len(), 0.0);
                        town.index_year.consumed[f] += q;
                        st[HUNGER] = (st[HUNGER] - q * value).max(0.0);
                        let pay = (q * town.price[f]).min(store.wealth[i].coins().max(0.0));
                        store.wealth[i] -= crate::coin::Coin::from_coins(pay);
                        town.treasury += crate::coin::Coin::from_coins(pay);
                        town.ledger.add(crate::ledger::Flow::FoodSales, pay);
                        break;
                    }
                }
            }
            if st[HUNGER] >= before {
                break;
            }
        }

        if st[HUNGER] > 1.0 {
            if count_starving {
                towns.0[store.town[i] as usize].starving += 1;
            }
            st[HUNGER] = st[HUNGER].min(1.5);
            st[HEALTH] -= cal.flow(p.starvation_damage);
        } else if st[HUNGER] < 0.6 {
            st[HEALTH] = (st[HEALTH] + cal.flow(p.health_regen)).min(1.0);
        }

        // Work tires (work::produce); everyone recovers a little each day of
        // time, at work or not, as when a step was a day. A rest restores
        // more for each day of work it skips, so it counts in the working
        // hours (a night needs no rest day).
        match store.activity[i] {
            Activity::Resting => {
                st[FATIGUE] = (st[FATIGUE] - rest_days * p.fatigue_rest_recovery).max(0.0);
                if now >= store.rest_until[i] {
                    store.activity[i] = Activity::Idle;
                }
            }
            Activity::Idle
            | Activity::Child
            | Activity::Soldier
            | Activity::Captive
            | Activity::Working(_) => {
                st[FATIGUE] = (st[FATIGUE] - cal.flow(p.fatigue_idle_recovery)).max(0.0)
            }
        }

        if ladder.is_some() {
            // The needs ladder sets happiness (below).
            continue;
        }
        let target = 0.7 - 0.5 * st[HUNGER] - 0.3 * st[FATIGUE]
            + 0.2 * (store.wealth[i].coins() / 50.0).tanh();
        st[HAPPINESS] = (st[HAPPINESS] + cal.share(p.happiness_rate) * (target - st[HAPPINESS]))
            .clamp(0.0, 1.0);
    }
    if let Some(lp) = ladder {
        // Wear, fuel and the ladder's happiness move once a day of time.
        if let Some(day) = clock.day_of_time() {
            crate::ladder::daily(
                store,
                &mut towns,
                &db,
                &params,
                &env,
                lp,
                day,
                count_starving,
            );
        }
    }
}
