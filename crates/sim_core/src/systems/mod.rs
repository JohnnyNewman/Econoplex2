//! The tick pipeline. Systems run in this fixed order every simulation day:
//!
//! 1. `needs::sense`        state vectors: hunger, fatigue, happiness, health; eating
//! 2. `decide::decide`      mind-weighted utility over candidate jobs, softmax choice
//! 3. `work::assign`        form teams, reserve inputs and nature, spawn task entities
//! 4. `work::produce`       advance tasks; resolve success, quality and outputs
//! 5. `work::learn`         practice and master-to-apprentice diffusion
//! 6. `social::socialize`   bounded-confidence alignment between coworkers and neighbors
//! 7. `body::body`          capacity limits, forgetting, physique training and aging
//! 8. `market::settle`      prices, trade between towns, nature regrowth
//! 9. `organize::organize`  guild membership and roles, tool equipment
//! 10. `life::lifecycle`    aging, death, partnering, births with genetics
//! 11. `metrics::record`    yearly statistics, economic complexity

pub mod body;
pub mod decide;
pub mod life;
pub mod market;
pub mod needs;
pub mod organize;
pub mod social;
pub mod work;

/// Random-stream ids so every system draws from its own reproducible sequence.
pub mod streams {
    pub const DECIDE: u64 = 1;
    pub const ASSIGN: u64 = 2;
    pub const PRODUCE: u64 = 3;
    pub const SOCIAL: u64 = 4;
    pub const MARKET: u64 = 5;
    pub const LIFE: u64 = 6;
    pub const ORGANIZE: u64 = 7;
}
