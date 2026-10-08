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
    for i in 0..store.len() {
        if !store.alive[i] {
            continue;
        }
        store.worked_today[i] = None;
        let town = &mut towns.0[store.town[i] as usize];
        let st = &mut store.state[i];

        st[HUNGER] += p.hunger_per_day;
        if st[HUNGER] > p.eat_threshold {
            for &(food, value) in &db.foods {
                let f = food as usize;
                if town.stock[f] >= 1.0 {
                    town.stock[f] -= 1.0;
                    st[HUNGER] = (st[HUNGER] - value).max(0.0);
                    let pay = town.price[f].min(store.wealth[i].max(0.0));
                    store.wealth[i] -= pay;
                    town.treasury += pay;
                    break;
                }
            }
        }
        if st[HUNGER] > 1.0 {
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
            Activity::Idle | Activity::Child => {
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
