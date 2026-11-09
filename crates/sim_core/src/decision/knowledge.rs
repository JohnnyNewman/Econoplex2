//! Knowledge: which towns an agent knows about, and how well.
//!
//! Interface: for each town, the agent's sources (the trust and family ties it has
//! there, the trade between home and that town, and its memory of having lived
//! there). Output: how well the agent knows the town, 0..1 (home is 1). A household
//! only weighs towns it knows at least `min_known`, and its estimates of a town's
//! pay and food are off by up to `noise` × (1 − knowledge), so a little-known town
//! may look better or worse than it is.
//!
//! Memories of past homes fade with a time constant of `memory_days`.

use serde::{Deserialize, Serialize};

/// `decision.knowledge` in models.ron.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KnowledgeParams {
    /// Everyone knows every town exactly (the original rule).
    All,
    /// Agents know towns through ties, trade and past residence.
    Limited {
        min_known: f32,
        /// Value of goods shipped per day (both ways, monthly average) at which
        /// trade gets a town about two-thirds of the way to `trade_known`.
        trade_scale: f32,
        /// How well trade alone can make a town known: hearsay from traders is
        /// not like living there.
        #[serde(default = "trade_known_default")]
        trade_known: f32,
        /// Time constant of the memory of a past home, in days.
        memory_days: f32,
        /// Largest relative error of an estimate about an unknown town.
        noise: f32,
    },
}

fn trade_known_default() -> f32 {
    0.4
}

impl Default for KnowledgeParams {
    fn default() -> Self {
        KnowledgeParams::Limited {
            min_known: 0.1,
            trade_scale: 2.0,
            trade_known: 0.4,
            memory_days: 3650.0,
            noise: 0.5,
        }
    }
}

/// What an agent has to go on about one town.
pub struct Sources {
    /// 0..1: its social network there (`migration::network`).
    pub network: f32,
    /// Value of goods shipped between home and the town per day.
    pub trade: f32,
    /// 0..1: memory of having lived there.
    pub lived: f32,
}

impl KnowledgeParams {
    /// How well the agent knows a town (not home), 0..1.
    pub fn known(&self, s: &Sources) -> f32 {
        match *self {
            KnowledgeParams::All => 1.0,
            KnowledgeParams::Limited {
                trade_scale,
                trade_known,
                ..
            } => {
                let trade = trade_known.clamp(0.0, 1.0)
                    * (1.0 - (-s.trade.max(0.0) / trade_scale.max(1e-6)).exp());
                s.network.max(trade).max(s.lived).clamp(0.0, 1.0)
            }
        }
    }

    /// Whether a town known this well is worth weighing at all.
    pub fn considers(&self, known: f32) -> bool {
        match *self {
            KnowledgeParams::All => true,
            KnowledgeParams::Limited { min_known, .. } => known >= min_known,
        }
    }

    /// Relative error of an estimate, for a random draw `e` in -1..1.
    pub fn error(&self, known: f32, e: f32) -> f32 {
        match *self {
            KnowledgeParams::All => 0.0,
            KnowledgeParams::Limited { noise, .. } => noise * (1.0 - known).clamp(0.0, 1.0) * e,
        }
    }

    /// Factor by which a memory fades over `days`.
    pub fn fade(&self, days: f32) -> f32 {
        match *self {
            KnowledgeParams::All => 1.0,
            KnowledgeParams::Limited { memory_days, .. } => (-days / memory_days.max(1.0)).exp(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn towns_are_known_through_ties_trade_and_memory() {
        let k = KnowledgeParams::default();
        let none = Sources {
            network: 0.0,
            trade: 0.0,
            lived: 0.0,
        };
        assert_eq!(k.known(&none), 0.0);
        assert!(!k.considers(k.known(&none)));
        let traded = Sources { trade: 4.0, ..none };
        assert!(k.considers(k.known(&traded)));
        // Trade alone never makes a town fully known.
        let lots = Sources { trade: 1e6, ..none };
        assert!(k.known(&lots) <= 0.4 + 1e-6);
        let lived = Sources {
            network: 0.0,
            trade: 0.0,
            lived: 0.9,
        };
        assert!((k.known(&lived) - 0.9).abs() < 1e-6);
        assert_eq!(KnowledgeParams::All.known(&lived), 1.0);
    }

    #[test]
    fn estimates_of_little_known_towns_are_noisier_and_memories_fade() {
        let k = KnowledgeParams::default();
        assert!(k.error(0.2, 1.0).abs() > k.error(0.9, 1.0).abs());
        assert_eq!(k.error(1.0, 1.0), 0.0);
        assert_eq!(KnowledgeParams::All.error(0.0, 1.0), 0.0);
        assert!(k.fade(365.0) < 1.0 && k.fade(365.0) > k.fade(3650.0));
    }
}
