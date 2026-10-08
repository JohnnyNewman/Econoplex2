//! Wall-clock time per system, for finding what slows big worlds down.
//!
//! A `mark::<K>` system runs after system K in the tick and books the time since
//! the previous mark to it. Timing never feeds back into the simulation, so it
//! does not affect determinism.

use bevy_ecs::prelude::*;
use std::time::{Duration, Instant};

pub const SYSTEMS: [&str; 13] = [
    "sense",
    "decide",
    "assign",
    "produce",
    "learn",
    "socialize",
    "body",
    "settle",
    "organize",
    "military",
    "migrate",
    "lifecycle",
    "metrics",
];

#[derive(Resource)]
pub struct Profile {
    pub total: [Duration; SYSTEMS.len()],
    last: Instant,
}

impl Default for Profile {
    fn default() -> Self {
        Profile {
            total: [Duration::ZERO; SYSTEMS.len()],
            last: Instant::now(),
        }
    }
}

/// Start of the tick.
pub fn start(mut p: ResMut<Profile>) {
    p.last = Instant::now();
}

/// End of system `K`.
pub fn mark<const K: usize>(mut p: ResMut<Profile>) {
    let now = Instant::now();
    let dt = now - p.last;
    p.total[K] += dt;
    p.last = now;
}
