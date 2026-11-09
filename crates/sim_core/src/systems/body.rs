//! Capability limits and forgetting; physique training toward the genetic potential.

use crate::config::PhysiqueParams;
use crate::db::Db;
use crate::models::Models;
use crate::store::AgentStore;
use crate::world::{Params, SimClock};
use bevy_ecs::prelude::*;
use rayon::prelude::*;
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
    // Each agent's update touches only its own vectors: run them in parallel.
    let (alive, age, worked_today, potential) = (
        &store.alive,
        &store.age_days,
        &store.worked_today,
        &store.potential,
    );
    store
        .cap
        .par_iter_mut()
        .zip(store.phys.par_iter_mut())
        .enumerate()
        .for_each(|(i, (cap, phys))| {
            if !alive[i] {
                return;
            }
            models.capacity.apply(cap);

            let ceiling = age_curve(age[i] as f32 / dpy as f32, p);
            let worked = worked_today[i].map(|r| db.recipes[r as usize].phys);
            for k in 0..PHYS_DIM {
                let max = potential[i][k] * ceiling;
                let floor = max * p.untrained_level;
                let x = &mut phys[k];
                let demand = worked.map_or(0.0, |d| d[k]);
                if demand > 0.0 {
                    *x += p.train_rate * demand * (max - *x);
                } else {
                    *x += p.disuse_rate * (floor - *x);
                }
                *x = x.min(max);
            }
        });
}
