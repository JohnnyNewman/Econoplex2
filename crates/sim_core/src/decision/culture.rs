//! Culture: each town's culture vector and how well an agent fits it.
//!
//! An agent's personal culture vector is the ideological block of its mind
//! (`mind::IDEOLOGY_START..MIND_DIM`; today tradition, faith and liberty, each 0..1).
//! These are the axes that already change through social influence, are passed on
//! to children and drive partner choice, so culture is what people actually believe
//! and not a separate label. New ideological axes added to the mind block become
//! culture axes automatically.
//!
//! A town's culture vector is bottom-up: it follows the average of its adult
//! residents slowly (an exponential moving average with a time constant of
//! `memory_days`), so a town keeps its character when newcomers arrive and changes
//! only as they settle in and its people change their minds. A scenario may seed a
//! town's starting culture; otherwise it starts at its residents' average.
//!
//! Fit is the centred cosine: both vectors are measured from the neutral point 0.5
//! of each axis, then compared by direction. It is 1 for the same outlook, 0 for
//! unrelated outlooks or a neutral agent or town, and -1 for opposite outlooks.

use serde::{Deserialize, Serialize};

/// `decision.culture` in models.ron.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CultureParams {
    /// Time constant (days) over which a town's culture follows its residents.
    pub memory_days: f32,
}

impl Default for CultureParams {
    fn default() -> Self {
        CultureParams { memory_days: 365.0 }
    }
}

/// The neutral point of every culture axis.
pub const NEUTRAL: f32 = 0.5;

/// Centred cosine between an agent's culture and a town's, in -1..1.
pub fn fit(agent: &[f32], town: &[f32]) -> f32 {
    let (mut dot, mut na, mut nt) = (0.0f32, 0.0f32, 0.0f32);
    for (&a, &t) in agent.iter().zip(town) {
        let (a, t) = (a - NEUTRAL, t - NEUTRAL);
        dot += a * t;
        na += a * a;
        nt += t * t;
    }
    let norm = (na * nt).sqrt();
    if norm < 1e-9 {
        0.0
    } else {
        (dot / norm).clamp(-1.0, 1.0)
    }
}

/// Moves a town's culture a day toward its residents' average.
pub fn follow(culture: &mut [f32], residents_mean: &[f32], memory_days: f32) {
    let a = 1.0 / memory_days.max(1.0);
    for (c, &m) in culture.iter_mut().zip(residents_mean) {
        *c += a * (m - *c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_is_a_cosine_between_minus_one_and_one() {
        assert!((fit(&[0.9, 0.9, 0.1], &[0.8, 0.8, 0.2]) - 1.0).abs() < 1e-5);
        assert!((fit(&[0.9, 0.5, 0.5], &[0.1, 0.5, 0.5]) + 1.0).abs() < 1e-5);
        assert!(fit(&[0.9, 0.5, 0.5], &[0.5, 0.9, 0.5]).abs() < 1e-5);
        // A neutral agent fits nowhere in particular.
        assert_eq!(fit(&[0.5, 0.5, 0.5], &[0.9, 0.1, 0.3]), 0.0);
        // Only direction counts, not how strongly it is held.
        assert!((fit(&[0.6, 0.5, 0.5], &[1.0, 0.5, 0.5]) - 1.0).abs() < 1e-5);
        for k in 0..50 {
            let a: Vec<f32> = (0..3)
                .map(|j| ((k * 7 + j * 3) % 11) as f32 / 10.0)
                .collect();
            let t: Vec<f32> = (0..3)
                .map(|j| ((k * 5 + j * 9) % 13) as f32 / 12.0)
                .collect();
            let f = fit(&a, &t);
            assert!((-1.0..=1.0).contains(&f));
        }
    }

    #[test]
    fn a_town_culture_follows_its_people_slowly() {
        let mut c = [0.5, 0.5, 0.5];
        follow(&mut c, &[1.0, 0.5, 0.0], 100.0);
        assert!((c[0] - 0.505).abs() < 1e-6 && (c[2] - 0.495).abs() < 1e-6);
        for _ in 0..2000 {
            follow(&mut c, &[1.0, 0.5, 0.0], 100.0);
        }
        assert!((c[0] - 1.0).abs() < 1e-3);
    }
}
