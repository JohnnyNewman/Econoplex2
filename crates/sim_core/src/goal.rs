//! How a match is won.
//!
//! The scenario names a product at the top of a production chain, an amount and a
//! time limit. The first town that has made that many in total wins, provided it
//! is its own master (not paying tribute to a ruler) and did not go hungry that year
//! (at most `max_starving` of its person-days starving). Towns are checked at each
//! year end. If nobody gets there in time, the free town closest to the goal wins
//! (ties go to the higher economic complexity index).
//!
//! The default goal (`scenario.ron`) is five fine steel longswords: it takes ore,
//! charcoal, steel, blades, hilts and skilled smiths, so towns must specialize,
//! trade and train people to get there.

use crate::config::GoalSpec;
use crate::db::Db;
use crate::metrics::Metrics;
use crate::world::{ScenarioSpec, SimClock, Towns};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WinReason {
    /// Reached the goal.
    Goal,
    /// Closest to the goal when time ran out.
    TimeUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MatchResult {
    pub winner: u16,
    pub year: u64,
    pub reason: WinReason,
}

#[derive(Resource, Debug, Clone, Default, Serialize, Deserialize)]
pub struct MatchState {
    /// Goal product, resolved from the scenario (none: no goal, endless play).
    pub product: Option<u32>,
    /// Goal product made by each town in all finished years.
    pub made: Vec<f32>,
    pub result: Option<MatchResult>,
}

impl MatchState {
    pub fn new(goal: Option<&GoalSpec>, db: &Db, towns: usize) -> Result<Self, String> {
        let product = goal
            .map(|g| {
                db.content
                    .product_index
                    .get(&g.product)
                    .copied()
                    .ok_or_else(|| format!("scenario goal: unknown product `{}`", g.product))
            })
            .transpose()?;
        Ok(MatchState {
            product,
            made: vec![0.0; towns],
            result: None,
        })
    }

    /// Made so far, including the year in progress.
    pub fn progress(&self, towns: &Towns, t: usize) -> f32 {
        let this_year = self
            .product
            .map_or(0.0, |p| towns.0[t].produced_this_year[p as usize]);
        self.made.get(t).copied().unwrap_or(0.0) + this_year
    }
}

/// Year-end check. Runs after metrics, so the year's stats are in.
pub fn evaluate(
    towns: Res<Towns>,
    clock: Res<SimClock>,
    metrics: Res<Metrics>,
    scenario: Res<ScenarioSpec>,
    mut state: ResMut<MatchState>,
) {
    if clock.tick == 0 || clock.day_of_year() != 0 {
        return;
    }
    let (Some(goal), Some(p)) = (scenario.0.goal.as_ref(), state.product) else {
        return;
    };
    state.made.resize(towns.0.len(), 0.0);
    for (t, town) in towns.0.iter().enumerate() {
        state.made[t] += town.produced_last_year[p as usize];
    }
    if state.result.is_some() {
        return;
    }
    let Some(stats) = metrics.latest() else {
        return;
    };
    let year = clock.year();
    let free = |t: usize| towns.0[t].ruler.is_none();
    let eci = |t: usize| stats.towns.get(t).map_or(0.0, |s| s.eci);
    let better = |a: &usize, b: &usize| {
        state.made[*a]
            .total_cmp(&state.made[*b])
            .then(eci(*a).total_cmp(&eci(*b)))
            .then(b.cmp(a))
    };
    let reached = (0..towns.0.len())
        .filter(|&t| free(t) && state.made[t] >= goal.amount)
        .filter(|&t| {
            stats
                .towns
                .get(t)
                .is_some_and(|s| s.starving <= goal.max_starving)
        })
        .max_by(better);
    let result = match reached {
        Some(t) => Some((t, WinReason::Goal)),
        None if year >= goal.years as u64 => (0..towns.0.len())
            .filter(|&t| free(t))
            .max_by(better)
            .map(|t| (t, WinReason::TimeUp)),
        None => None,
    };
    state.result = result.map(|(t, reason)| MatchResult {
        winner: t as u16,
        year,
        reason,
    });
}
