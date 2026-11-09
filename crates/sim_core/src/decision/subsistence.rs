//! Subsistence: food workers' households keep part of what they make and eat it.
//!
//! Interface: when a team finishes a run of a food recipe, how much of each edible
//! output its households keep (the rest goes to the town's stores and is sold), and
//! how much a household may hold. An agent eats from its own larder before buying
//! from the town. In the utility model, a food job then also covers the worker's
//! own food need, not only through pay.
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

/// Takes one unit of the first good in `foods` (best first) the larder holds at
/// least one of; returns that good.
pub fn take_meal(larder: &mut Vec<(u32, f32)>, foods: &[(u32, f32)]) -> Option<(u32, f32)> {
    for &(f, value) in foods {
        if let Some(k) = larder.iter().position(|e| e.0 == f && e.1 >= 1.0) {
            larder[k].1 -= 1.0;
            if larder[k].1 < 1e-6 {
                larder.swap_remove(k);
            }
            return Some((f, value));
        }
    }
    None
}

/// How much of its own food need a worker covers over the coming weeks by keeping
/// `meals_per_day` meals a day, each removing `relief` hunger, when it gets hungry
/// by `hunger_per_day` a day (0..1).
pub fn food_cover(meals_per_day: f32, relief: f32, hunger_per_day: f32) -> f32 {
    if hunger_per_day <= 0.0 {
        return 1.0;
    }
    (meals_per_day * relief / hunger_per_day).clamp(0.0, 1.0)
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
        // Best food first.
        let foods = [(7, 0.5), (3, 0.3)];
        assert_eq!(take_meal(&mut l, &foods), Some((7, 0.5)));
        assert_eq!(take_meal(&mut l, &foods), Some((3, 0.3)));
        assert_eq!(l, vec![(3, 3.0)]);
    }

    #[test]
    fn keeping_food_covers_the_food_need() {
        assert_eq!(food_cover(0.0, 0.4, 0.2), 0.0);
        assert!((food_cover(0.25, 0.4, 0.2) - 0.5).abs() < 1e-6);
        assert_eq!(food_cover(2.0, 0.4, 0.2), 1.0);
    }
}
