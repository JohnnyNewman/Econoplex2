//! State dynamics: hunger, eating, health, fatigue recovery, happiness.

use crate::db::Db;
use crate::store::{Activity, AgentStore};
use crate::world::{Params, SimClock, Towns};
use bevy_ecs::prelude::*;
use sim_data::dims::state::*;

pub fn sense(
    mut store: ResMut<AgentStore>,
    mut towns: ResMut<Towns>,
    db: Res<Db>,
    params: Res<Params>,
    mut clock: ResMut<SimClock>,
) {
    clock.tick += 1;
    let p = &params.needs;
    let store = &mut *store;

    // Food that a recipe turns into more food (grain into flour for bread) is held
    // back for one day of that work, so hungry people don't eat the seed of the
    // next harvest and lock the town into famine.
    let reserve: Vec<Vec<f32>> = towns
        .0
        .iter()
        .map(|town| super::decide::food_reserve(&db, town, &params))
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
        town.rent += (target - town.rent) / crowding.stickiness_days();
    }

    for i in 0..store.len() {
        if !store.alive[i] {
            continue;
        }
        store.worked_today[i] = None;
        if !matches!(store.activity[i], Activity::Child | Activity::Captive) {
            let town = &mut towns.0[store.town[i] as usize];
            let pay = town.rent.min(store.wealth[i].max(0.0));
            if pay > 0.0 {
                store.wealth[i] -= pay;
                town.treasury += pay;
                town.ledger.add(crate::ledger::Flow::Rent, pay);
            }
        }
        let town = &mut towns.0[store.town[i] as usize];
        let st = &mut store.state[i];

        st[HUNGER] += p.hunger_per_day;
        if st[HUNGER] > p.eat_threshold {
            // The household's own food first.
            if let Some((_, value)) =
                crate::decision::subsistence::take_meal(&mut store.larder[i], &db.foods)
            {
                st[HUNGER] = (st[HUNGER] - value).max(0.0);
            }
        }
        // With firms, residents buy food straight from the firms that hold it
        // first, then from the market.
        if st[HUNGER] > p.eat_threshold && params.market.firms.on() {
            'buy: for &(food, value) in &db.foods {
                let f = food as usize;
                for firm in town.firms.iter_mut() {
                    if firm.stock.get(f).is_some_and(|&s| s >= 1.0) {
                        firm.stock[f] -= 1.0;
                        st[HUNGER] = (st[HUNGER] - value).max(0.0);
                        let pay = town.price[f].min(store.wealth[i].max(0.0));
                        store.wealth[i] -= pay;
                        firm.cash += pay;
                        firm.book(crate::firms::FirmFlow::DirectSales, pay);
                        town.consumed_year.resize(town.stock.len(), 0.0);
                        town.consumed_year[f] += 1.0;
                        break 'buy;
                    }
                }
            }
        }
        if st[HUNGER] > p.eat_threshold {
            for &(food, value) in &db.foods {
                let f = food as usize;
                if town.stock[f] >= 1.0 + reserve[store.town[i] as usize][f] {
                    town.stock[f] -= 1.0;
                    town.consumed_year.resize(town.stock.len(), 0.0);
                    town.consumed_year[f] += 1.0;
                    st[HUNGER] = (st[HUNGER] - value).max(0.0);
                    let pay = town.price[f].min(store.wealth[i].max(0.0));
                    store.wealth[i] -= pay;
                    town.treasury += pay;
                    town.ledger.add(crate::ledger::Flow::FoodSales, pay);
                    break;
                }
            }
        }
        if st[HUNGER] > 1.0 {
            towns.0[store.town[i] as usize].starving += 1;
            st[HUNGER] = st[HUNGER].min(1.5);
            st[HEALTH] -= p.starvation_damage;
        } else if st[HUNGER] < 0.6 {
            st[HEALTH] = (st[HEALTH] + p.health_regen).min(1.0);
        }

        match store.activity[i] {
            Activity::Resting => {
                st[FATIGUE] = (st[FATIGUE] - p.fatigue_rest_recovery).max(0.0);
                store.activity[i] = Activity::Idle;
            }
            Activity::Idle | Activity::Child | Activity::Soldier | Activity::Captive => {
                st[FATIGUE] = (st[FATIGUE] - p.fatigue_idle_recovery).max(0.0)
            }
            Activity::Working(_) => {}
        }

        let target =
            0.7 - 0.5 * st[HUNGER] - 0.3 * st[FATIGUE] + 0.2 * (store.wealth[i] / 50.0).tanh();
        st[HAPPINESS] =
            (st[HAPPINESS] + p.happiness_rate * (target - st[HAPPINESS])).clamp(0.0, 1.0);
    }
}
