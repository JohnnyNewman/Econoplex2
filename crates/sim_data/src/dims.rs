//! Vector dimensions shared by content tools and the simulation.
//!
//! Capability is fully latent. Mind, physique and state use named axes so players can read them.

/// Latent capability (skill) dimension when nothing else decides it. The simulation
/// sizes capability vectors from the content at load time (see [`cap_dim_for`]).
pub const DEFAULT_CAP_DIM: usize = 32;

/// Capability dimension for `skills` skills: one per skill, rounded up to a multiple
/// of 8 so vector loops stay SIMD-friendly. With at least one dimension per skill
/// the product space is exact (no compression).
pub fn cap_dim_for(skills: usize) -> usize {
    skills.max(1).div_ceil(8) * 8
}
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

/// Latent capability vector; its length is the content's capability dimension
/// (`Db::cap_dim`), the same for every agent and skill in a game.
pub type CapVec = Vec<f32>;
pub type MindVec = [f32; MIND_DIM];
pub type PhysVec = [f32; PHYS_DIM];
pub type StateVec = [f32; STATE_DIM];

#[inline]
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    // Four running sums let the compiler vectorize without reassociating floats
    // differently between builds, so results stay bit-identical.
    let mut acc = [0.0f32; 4];
    let (ca, ra) = a.split_at(a.len() / 4 * 4);
    let (cb, rb) = b.split_at(ca.len());
    for (x, y) in ca.chunks_exact(4).zip(cb.chunks_exact(4)) {
        for k in 0..4 {
            acc[k] += x[k] * y[k];
        }
    }
    let mut s = (acc[0] + acc[1]) + (acc[2] + acc[3]);
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

#[inline]
pub fn norm(a: &[f32]) -> f32 {
    dot(a, a).sqrt()
}
