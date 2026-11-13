//! Where goods come from and where they go: the goods ledger.
//!
//! Step 2 of the economy redesign (items, time, needs). Every good that exists
//! is held somewhere:
//!
//! | Holder | Today's storage |
//! | --- | --- |
//! | a town's market | `Town::stock` |
//! | a workshop | `Firm::stock` |
//! | a person's larder | `AgentStore::larder` |
//! | a raid's loot on its way home | `Raid::loot` |
//!
//! Goods only appear or disappear through the flows below; moving between
//! holders (a sale, a caravan, plunder, a larder handed on at death) changes
//! no total. Each town counts the flows that happen in it, and
//! [`audit`] checks that, for every good, what is held equals what was seeded
//! and made minus what left. A good whose numbers do not add up is being made
//! or lost somewhere outside the rules.
//!
//! Household goods (`AgentStore::owned`) count as bought (they leave the
//! market for a home and wear out there), and tools and weapons are entities
//! as well as a count in the market (see `Flow::Equipped`); the next steps
//! turn both into proper held items.

use crate::store::AgentStore;
use crate::world::Towns;
use serde::{Deserialize, Serialize};

/// Ways goods enter or leave the economy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// In the town's stock when the world was made.
    Seeded,
    /// Output of a recipe (before any of it is kept, sold or delivered).
    Made,
    /// Eaten by someone.
    Eaten,
    /// Used up as a recipe input.
    Input,
    /// Bought by a household for its home (it wears out there).
    Bought,
    /// Delivered for a government order (public use).
    Procured,
    /// Became a building in the town.
    Built,
    /// Became new land (a cleared field, a planted woodlot).
    Opened,
    /// Struck coin: became money in the treasury.
    Minted,
    /// A tool taken from the market's count by the person who uses it.
    Equipped,
    /// Rotted in store.
    Spoiled,
}

impl Flow {
    pub const ALL: [Flow; 11] = [
        Flow::Seeded,
        Flow::Made,
        Flow::Eaten,
        Flow::Input,
        Flow::Bought,
        Flow::Procured,
        Flow::Built,
        Flow::Opened,
        Flow::Minted,
        Flow::Equipped,
        Flow::Spoiled,
    ];
    pub const COUNT: usize = Self::ALL.len();

    /// Whether the flow brings goods in (+1) or takes them out (-1).
    pub fn sign(self) -> f64 {
        match self {
            Flow::Seeded | Flow::Made => 1.0,
            _ => -1.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Flow::Seeded => "seeded",
            Flow::Made => "made",
            Flow::Eaten => "eaten",
            Flow::Input => "input",
            Flow::Bought => "bought",
            Flow::Procured => "procured",
            Flow::Built => "built",
            Flow::Opened => "opened",
            Flow::Minted => "minted",
            Flow::Equipped => "equipped",
            Flow::Spoiled => "spoiled",
        }
    }
}

/// A town's goods flows since the start, per good and flow (units).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoodsFlows {
    /// `[good * Flow::COUNT + flow]`.
    v: Vec<f64>,
}

impl GoodsFlows {
    /// Flows of a town that starts with `stock`.
    pub fn seeded(stock: &[f32]) -> Self {
        let mut f = GoodsFlows::default();
        for (p, &q) in stock.iter().enumerate() {
            if q != 0.0 {
                f.add(Flow::Seeded, p, q);
            }
        }
        f
    }

    pub fn add(&mut self, flow: Flow, good: usize, q: f32) {
        let i = good * Flow::COUNT + flow as usize;
        if self.v.len() <= i {
            self.v.resize((good + 1) * Flow::COUNT, 0.0);
        }
        self.v[i] += q as f64;
    }

    pub fn get(&self, flow: Flow, good: usize) -> f64 {
        self.v
            .get(good * Flow::COUNT + flow as usize)
            .copied()
            .unwrap_or(0.0)
    }

    /// In minus out for `good`: what should be held of it.
    pub fn net(&self, good: usize) -> f64 {
        Flow::ALL
            .iter()
            .map(|&f| f.sign() * self.get(f, good))
            .sum()
    }
}

/// What is held of each good, wherever it is.
pub fn census(store: &AgentStore, towns: &Towns, goods: usize) -> Vec<f64> {
    let mut held = vec![0.0f64; goods];
    for t in &towns.0 {
        for (p, &q) in t.stock.iter().enumerate().take(goods) {
            held[p] += q as f64;
        }
        for f in &t.firms {
            for (p, &q) in f.stock.iter().enumerate().take(goods) {
                held[p] += q as f64;
            }
        }
        if let Some(raid) = &t.raid {
            for (p, &q) in raid.loot.iter().enumerate().take(goods) {
                held[p] += q as f64;
            }
        }
    }
    for i in 0..store.len() {
        if !store.alive[i] {
            continue;
        }
        for &(p, q) in &store.larder[i] {
            if (p as usize) < goods {
                held[p as usize] += q as f64;
            }
        }
    }
    held
}

/// One good whose holdings do not match its flows.
#[derive(Debug, Clone)]
pub struct Mismatch {
    pub good: usize,
    pub held: f64,
    /// Seeded plus made minus everything that left.
    pub expected: f64,
}

impl Mismatch {
    /// Held minus expected: positive = goods appeared from nowhere.
    pub fn gap(&self) -> f64 {
        self.held - self.expected
    }
}

/// Goods whose holdings differ from their flows by more than `tolerance`
/// (relative to what passed through them, at least one unit).
pub fn audit(store: &AgentStore, towns: &Towns, goods: usize, tolerance: f64) -> Vec<Mismatch> {
    let held = census(store, towns, goods);
    (0..goods)
        .filter_map(|p| {
            let expected: f64 = towns.0.iter().map(|t| t.flows.net(p)).sum();
            let through: f64 = towns
                .0
                .iter()
                .map(|t| t.flows.get(Flow::Seeded, p) + t.flows.get(Flow::Made, p))
                .sum();
            let gap = (held[p] - expected).abs();
            (gap > (tolerance * through).max(1.0)).then_some(Mismatch {
                good: p,
                held: held[p],
                expected,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flows_add_up() {
        let mut f = GoodsFlows::default();
        f.add(Flow::Seeded, 2, 10.0);
        f.add(Flow::Made, 2, 5.0);
        f.add(Flow::Eaten, 2, 3.0);
        f.add(Flow::Input, 0, 1.0);
        assert_eq!(f.net(2), 12.0);
        assert_eq!(f.net(0), -1.0);
        assert_eq!(f.net(7), 0.0);
    }
}
