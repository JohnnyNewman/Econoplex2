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
//! 10. `military::military` musters, soldier pay and drill, raids and battles
//! 11. `migrate::migrate`   households move to towns that suit them better
//! 12. `life::lifecycle`    aging, death, partnering, births with genetics
//! 13. `metrics::record`    yearly statistics, economic complexity

/// Maps `f` over `items`, in parallel when there are enough of them to pay
/// for handing them to the thread pool (a step often has only a few). The
/// result is the same either way.
pub fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync + Send) -> Vec<R> {
    use rayon::prelude::*;
    if items.len() < 128 {
        items.iter().map(f).collect()
    } else {
        items.par_iter().map(f).collect()
    }
}

pub mod body;
pub mod decide;
pub mod life;
pub mod market;
pub mod migrate;
pub mod military;
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
    pub const MILITARY: u64 = 8;
    pub const MIGRATE: u64 = 9;
    pub const KNOWLEDGE: u64 = 10;
}

/// The tribute town `town` owes its ruler this round, in its own coin: a share
/// of its treasury, or, when it issues its coin, a share of what it taxed
/// over the round (`cal` spans the round).
pub fn tribute_due(
    town: &crate::world::Town,
    market: &crate::config::MarketParams,
    mp: &crate::config::MilitaryParams,
    cal: crate::calendar::Calendar,
) -> f32 {
    use crate::ledger::Flow;
    match market.treasury {
        crate::config::Treasury::Holds => town.treasury.coins().max(0.0) * mp.tribute_share,
        crate::config::Treasury::Issues => {
            let taxed: f32 = [Flow::WealthTax, Flow::IncomeTax, Flow::Rent, Flow::Estates]
                .iter()
                .map(|&f| town.ledger.monthly[f as usize].max(0.0))
                .sum();
            taxed * cal.days() * mp.tribute_share
        }
    }
}
