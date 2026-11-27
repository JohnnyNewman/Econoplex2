//! Who knows which recipe-skill (inventions step 2, docs/design/inventions.md).
//!
//! With `knowledge.enabled`, every person holds a set of known recipe-skills, one
//! bit per recipe. A person takes a regular place only in work they know. An
//! apprentice place is open to anyone who meets the recipe's `requires`, and an
//! apprentice (or a child working beside a parent) who works with someone who
//! knows the recipe learns it, with `learn_per_day` a day of such work. Pupils
//! learn the recipes their teacher knows in the school's domain. Knowledge lives
//! in people: it moves with migrants and captives and dies with its last holder.
//! Recipes with no `requires` start the graph and are known to everyone.
//!
//! Founders: every adult knows their family trade and what it requires, and each
//! other recipe the town can run with `start_share` (with what it requires);
//! recipes the town cannot run (no building or land for them) with
//! `start_share * foreign_share`.
//!
//! Off (the default), everyone counts as knowing everything and nothing draws a
//! random number, so runs are exactly as before.

use crate::db::Db;
use crate::store::AgentStore;
use rand::Rng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KnowledgeParams {
    pub enabled: bool,
    /// Chance a founder knows a recipe the town can run (besides their trade).
    pub start_share: f32,
    /// Factor on `start_share` for recipes the town cannot run.
    pub foreign_share: f32,
    /// Chance per day of work beside someone who knows the recipe that an
    /// apprentice or child learns it.
    pub learn_per_day: f32,
    /// Factor on `learn_per_day` for a pupil learning a teacher's recipe at school.
    pub school_share: f32,
}

impl Default for KnowledgeParams {
    fn default() -> Self {
        KnowledgeParams {
            enabled: false,
            start_share: 0.1,
            foreign_share: 0.2,
            learn_per_day: 0.03,
            school_share: 0.3,
        }
    }
}

impl KnowledgeParams {
    pub fn validate(&self) -> Result<(), String> {
        let unit = |x: f32| (0.0..=1.0).contains(&x);
        if !(unit(self.start_share)
            && unit(self.foreign_share)
            && unit(self.learn_per_day)
            && unit(self.school_share))
        {
            return Err("knowledge: shares and rates must be between 0 and 1".into());
        }
        Ok(())
    }
}

/// Whether bit `r` is set in a known set.
#[inline]
fn has(bits: &[u64], r: usize) -> bool {
    bits.get(r / 64).is_some_and(|w| w >> (r % 64) & 1 == 1)
}

/// Set bit `r`.
fn set(bits: &mut Vec<u64>, r: usize) {
    if bits.len() <= r / 64 {
        bits.resize(r / 64 + 1, 0);
    }
    bits[r / 64] |= 1 << (r % 64);
}

/// Whether person `i` knows recipe `r` (always, with knowledge off).
#[inline]
pub fn knows(kp: &KnowledgeParams, db: &Db, store: &AgentStore, i: usize, r: usize) -> bool {
    !kp.enabled || db.requires[r].is_empty() || store.known.get(i).is_some_and(|b| has(b, r))
}

/// Whether person `i` meets recipe `r`'s `requires`: it is on their frontier
/// or known, so they can apprentice in it.
pub fn can_learn(kp: &KnowledgeParams, db: &Db, store: &AgentStore, i: usize, r: usize) -> bool {
    !kp.enabled
        || db.requires[r]
            .iter()
            .all(|req| req.iter().any(|&o| knows(kp, db, store, i, o as usize)))
}

/// Person `i` learns recipe `r`. Returns whether it was new.
pub fn learn(store: &mut AgentStore, i: usize, r: usize) -> bool {
    if store.known.len() <= i {
        store.known.resize(i + 1, Vec::new());
    }
    let new = !has(&store.known[i], r);
    set(&mut store.known[i], r);
    new
}

/// Add recipe `r` and, recursively, one way to meet each of its requirements
/// (the first option already known, else the first) to a known set.
fn add_with_requires(db: &Db, bits: &mut Vec<u64>, r: usize) {
    if has(bits, r) {
        return;
    }
    set(bits, r);
    for req in &db.requires[r] {
        if req
            .iter()
            .any(|&o| has(bits, o as usize) || db.requires[o as usize].is_empty())
        {
            continue;
        }
        if let Some(&o) = req.first() {
            add_with_requires(db, bits, o as usize);
        }
    }
}

/// A founder's known set: their family trade `trade` (if any) and each recipe by
/// chance, `runnable[r]` telling whether the town can run it.
pub fn founder(
    kp: &KnowledgeParams,
    db: &Db,
    trade: Option<usize>,
    runnable: &[bool],
    rng: &mut impl Rng,
) -> Vec<u64> {
    let mut bits = Vec::new();
    if let Some(r) = trade {
        add_with_requires(db, &mut bits, r);
    }
    for (r, &here) in runnable.iter().enumerate() {
        let p = if here {
            kp.start_share
        } else {
            kp.start_share * kp.foreign_share
        };
        if rng.random::<f32>() < p {
            add_with_requires(db, &mut bits, r);
        }
    }
    bits
}

/// Chance of learning over `days` of work at `per_day`.
pub fn chance(per_day: f32, days: f32) -> f32 {
    1.0 - (1.0 - per_day).powf(days.max(0.0))
}

/// How many distinct recipe-skills the living people in `residents` know
/// between them (the town's knowledge).
pub fn town_known(db: &Db, store: &AgentStore, residents: &[u32]) -> usize {
    let words = db.recipes.len().div_ceil(64);
    let mut all = vec![0u64; words];
    for &i in residents {
        if let Some(b) = store.known.get(i as usize) {
            for (a, w) in all.iter_mut().zip(b) {
                *a |= w;
            }
        }
    }
    let roots = (0..db.recipes.len())
        .filter(|&r| db.requires[r].is_empty() && !has(&all, r))
        .count();
    all.iter().map(|w| w.count_ones() as usize).sum::<usize>() + roots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_grow_as_needed() {
        let mut b = Vec::new();
        assert!(!has(&b, 130));
        set(&mut b, 130);
        assert!(has(&b, 130) && !has(&b, 129) && b.len() == 3);
    }

    #[test]
    fn learning_chance_compounds_over_days() {
        assert_eq!(chance(0.1, 0.0), 0.0);
        assert!((chance(0.1, 1.0) - 0.1).abs() < 1e-6);
        assert!((chance(0.1, 2.0) - 0.19).abs() < 1e-6);
        assert!(chance(0.03, 30.0) > 0.5);
    }
}
