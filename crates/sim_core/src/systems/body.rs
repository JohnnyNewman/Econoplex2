//! Capability limits and forgetting; physique training toward the genetic potential.

use crate::config::PhysiqueParams;
use crate::db::Db;
use crate::models::Models;
use crate::store::AgentStore;
use crate::world::{Params, SimClock};
use bevy_ecs::prelude::*;
use sim_data::dims::PHYS_DIM;

/// Fraction of genetic potential reachable at a given age: grows to 1 at `peak_age`,
/// then declines after `decline_age`.
pub fn age_curve(age_years: f32, p: &PhysiqueParams) -> f32 {
    if age_years < p.peak_age {
        0.3 + 0.7 * (age_years / p.peak_age)
    } else if age_years > p.decline_age {
        (1.0 - p.decline_per_year * (age_years - p.decline_age)).max(0.2)
    } else {
        1.0
    }
}

pub fn body(
    mut store: ResMut<AgentStore>,
    db: Res<Db>,
    models: Res<Models>,
    params: Res<Params>,
    clock: Res<SimClock>,
) {
    let p = &params.physique;
    let dpy = clock.days_per_year;
    let store = &mut *store;
    for i in 0..store.len() {
        if !store.alive[i] {
            continue;
        }
        models.capacity.apply(&mut store.cap[i]);

        let ceiling = age_curve(store.age_years(i, dpy), p);
        let worked = store.worked_today[i].map(|r| db.recipes[r as usize].phys);
        for k in 0..PHYS_DIM {
            let max = store.potential[i][k] * ceiling;
            let floor = max * p.untrained_level;
            let x = &mut store.phys[i][k];
            let demand = worked.map_or(0.0, |d| d[k]);
            if demand > 0.0 {
                *x += p.train_rate * demand * (max - *x);
            } else {
                *x += p.disuse_rate * (floor - *x);
            }
            *x = x.min(max);
        }
    }
}
