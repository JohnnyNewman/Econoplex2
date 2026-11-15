//! Subsistence: food workers' households keep part of what they make and eat it.
//!
//! Interface: when a team finishes a run of a food recipe, how much of each edible
//! output its households keep (the rest goes to the town's stores and is sold), and
//! how much a household may hold. An agent eats from its own larder before buying
//! from the town.
//!
//! Accounting stays exact: kept food is still counted as produced, it is simply
//! held by the household instead of the town; the market value the team is paid
//! for is only what it delivered. A larder that would overflow sends the rest to
//! the town, and a dead agent's larder goes back to the town's stores.

use serde::{Deserialize, Serialize};

/// `decision.subsistence` in models.ron.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum SubsistenceParams {
    /// Everything goes to the town's stores (the original rule).
    #[default]
    None,
    /// Households keep `share` of each edible output, up to `larder_meals` meals.
    Keep { share: f32, larder_meals: f32 },
}

impl SubsistenceParams {
    pub fn share(&self) -> f32 {
        match self {
            SubsistenceParams::None => 0.0,
            SubsistenceParams::Keep { share, .. } => share.clamp(0.0, 1.0),
        }
    }

    pub fn larder_meals(&self) -> f32 {
        match self {
            SubsistenceParams::None => 0.0,
            SubsistenceParams::Keep { larder_meals, .. } => larder_meals.max(0.0),
        }
    }
}

/// Puts `q` units of good `good` into a larder holding at most `cap` units in all;
/// returns what did not fit.
pub fn store(larder: &mut Vec<(u32, f32)>, good: u32, q: f32, cap: f32) -> f32 {
    let held: f32 = larder.iter().map(|e| e.1).sum();
    let fits = q.min((cap - held).max(0.0));
    if fits > 0.0 {
        match larder.iter_mut().find(|e| e.0 == good) {
            Some(e) => e.1 += fits,
            None => larder.push((good, fits)),
        }
    }
    q - fits
}

/// How much of a food a meal takes: enough to still `hunger`, at most one
/// unit (food is a stack: a loaf is cut, not eaten whole at every meal).
pub fn portion(hunger: f32, value: f32) -> f32 {
    (hunger / value.max(1e-6)).clamp(0.0, 1.0)
}

/// A meal from a larder or a pack: a portion of the first good in `foods`
/// (best first) it holds enough of. Returns (good, its food value, quantity).
pub fn take_portion(
    larder: &mut Vec<(u32, f32)>,
    foods: &[(u32, f32)],
    hunger: f32,
) -> Option<(u32, f32, f32)> {
    for &(f, value) in foods {
        let q = portion(hunger, value);
        if q <= 0.0 {
            continue;
        }
        if let Some(k) = larder.iter().position(|e| e.0 == f && e.1 >= q - 1e-6) {
            let q = q.min(larder[k].1);
            larder[k].1 -= q;
            if larder[k].1 < 1e-6 {
                larder.swap_remove(k);
            }
            return Some((f, value, q));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_larder_holds_what_fits_and_hands_back_the_rest() {
        let mut l = Vec::new();
        assert_eq!(store(&mut l, 3, 4.0, 5.0), 0.0);
        assert!((store(&mut l, 7, 4.0, 5.0) - 3.0).abs() < 1e-6);
        assert_eq!(l, vec![(3, 4.0), (7, 1.0)]);
        // Best food first, a portion that stills the hunger: 0.25 hunger takes
        // half of a 0.5 food; a full unit is the most one meal takes.
        let foods = [(7, 0.5), (3, 0.3)];
        assert_eq!(take_portion(&mut l, &foods, 0.25), Some((7, 0.5, 0.5)));
        // Half a unit of the best food left is too little for 0.6 hunger.
        assert_eq!(take_portion(&mut l, &foods, 0.6), Some((3, 0.3, 1.0)));
        assert_eq!(l, vec![(3, 3.0), (7, 0.5)]);
    }
}
