//! Counter-based random draws: each draw depends only on the battle's seed,
//! the substep, the soldier and the purpose, never on the order soldiers are
//! processed in or on threads.

/// What a draw is for, so two purposes of one soldier in one substep never
/// share a stream.
#[derive(Clone, Copy)]
#[repr(u64)]
pub(super) enum Purpose {
    /// The first cooldown, so a squad does not strike in step.
    Start = 1,
    /// A blow's or a charge's to-hit.
    Hit,
    /// A braced spear's blow on a charger.
    Counter,
    /// Whom an archer aims at.
    Aim,
    /// Where an arrow lands, and whether a shield or the wall stops it.
    Scatter,
}

/// A SplitMix64 stream, as `AgentRng`.
pub(super) struct Draw(u64);

impl Draw {
    pub(super) fn new(seed: u64, substep: u64, soldier: usize, purpose: Purpose) -> Self {
        let mut s = seed ^ 0x2545_F491_4F6C_DD1D;
        s = s.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(substep);
        s = s
            .wrapping_mul(0xBF58_476D_1CE4_E5B9)
            .wrapping_add(soldier as u64);
        s = s
            .wrapping_mul(0x94D0_49BB_1331_11EB)
            .wrapping_add(purpose as u64);
        let mut d = Draw(s);
        // Mix once so neighbouring soldiers' streams decorrelate.
        d.next_u64();
        d
    }

    pub(super) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub(super) fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * (1.0 / (1u64 << 24) as f32)
    }

    /// Uniform in 0..n.
    pub(super) fn below(&mut self, n: usize) -> usize {
        (((self.next_u64() >> 32) * n as u64) >> 32) as usize
    }

    /// Roughly standard normal: the sum of four uniforms, scaled. Only adds
    /// and multiplies, so it is the same on every platform.
    pub(super) fn normal(&mut self) -> f32 {
        let s = self.f32() + self.f32() + self.f32() + self.f32();
        (s - 2.0) * 1.732_050_8
    }
}
