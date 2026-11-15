//! Household consumption: households buy goods for their own use.
//!
//! `market.consumption` in models.ron. `None` keeps the original rule:
//! households buy only food (when hungry) and tools (to work). `Needs` (the
//! default) is the needs ladder (`ladder.rs`): households buy what serves
//! their needs.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum ConsumptionParams {
    #[default]
    None,
    /// The needs ladder (`ladder.rs`): households buy what serves their needs.
    Needs(crate::ladder::LadderParams),
}

/// Households buy for their own use under the chosen model (see the module docs).
pub fn spend(
    towns: &mut crate::world::Towns,
    store: &mut crate::store::AgentStore,
    db: &crate::db::Db,
    params: &crate::world::Params,
    env: &crate::env::Environment,
    tick: u64,
) {
    if let ConsumptionParams::Needs(lp) = &params.market.consumption {
        crate::ladder::spend(towns, store, db, params, env, lp, tick);
    }
}
