//! Vector dimensions shared by content tools and the simulation.
//!
//! Capability is fully latent. Mind, physique and state use named axes so players can read them.

/// Latent capability (skill) dimension.
pub const CAP_DIM: usize = 32;
/// Mind: 5 personality axes (fixed by temperament) + 3 ideological axes (socially mutable).
pub const MIND_DIM: usize = 8;
pub const PHYS_DIM: usize = 6;
pub const STATE_DIM: usize = 4;

pub const MIND_NAMES: [&str; MIND_DIM] = [
    "ambition",
    "diligence",
    "curiosity",
    "sociability",
    "risk",
    "tradition",
    "faith",
    "liberty",
];
pub const PHYS_NAMES: [&str; PHYS_DIM] = [
    "strength",
    "endurance",
    "agility",
    "dexterity",
    "perception",
    "constitution",
];
pub const STATE_NAMES: [&str; STATE_DIM] = ["hunger", "fatigue", "happiness", "health"];

pub mod mind {
    pub const AMBITION: usize = 0;
    pub const DILIGENCE: usize = 1;
    pub const CURIOSITY: usize = 2;
    pub const SOCIABILITY: usize = 3;
    pub const RISK: usize = 4;
    pub const TRADITION: usize = 5;
    pub const FAITH: usize = 6;
    pub const LIBERTY: usize = 7;
    /// First ideological axis; axes from here on change through social influence.
    pub const IDEOLOGY_START: usize = TRADITION;
}

pub mod state {
    pub const HUNGER: usize = 0;
    pub const FATIGUE: usize = 1;
    pub const HAPPINESS: usize = 2;
    pub const HEALTH: usize = 3;
}

pub fn phys_index(name: &str) -> Option<usize> {
    PHYS_NAMES.iter().position(|n| *n == name)
}

pub type CapVec = [f32; CAP_DIM];
pub type MindVec = [f32; MIND_DIM];
pub type PhysVec = [f32; PHYS_DIM];
pub type StateVec = [f32; STATE_DIM];

#[inline]
pub fn dot<const N: usize>(a: &[f32; N], b: &[f32; N]) -> f32 {
    let mut s = 0.0;
    for i in 0..N {
        s += a[i] * b[i];
    }
    s
}

#[inline]
pub fn norm<const N: usize>(a: &[f32; N]) -> f32 {
    dot(a, a).sqrt()
}
