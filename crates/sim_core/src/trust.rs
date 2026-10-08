//! Trust: personal ties between agents.
//!
//! Each agent remembers a handful of people (`TIES` slots) with a trust value in
//! [-1, 1]. Working together builds trust, a successful job more than a failed
//! one; chance meetings build it between like-minded people and erode it between
//! people far apart in outlook. Ties fade without contact, and a stronger new tie
//! pushes out the weakest one. Keeping a fixed number of slots keeps memory and
//! time linear in the number of agents.
//!
//! Trust then feeds back: teams form around trusted coworkers and work better
//! together, apprentices learn faster from masters they trust, squads that trust
//! each other fight harder, and households follow the people they trust when they
//! move. Cultural clusters thereby turn into clusters of capability.

use crate::store::AgentStore;

pub const TIES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tie {
    pub other: u32,
    pub value: f32,
}

impl Tie {
    pub const NONE: Tie = Tie {
        other: u32::MAX,
        value: 0.0,
    };
}

impl AgentStore {
    /// How much `a` trusts `b` (0 when they have no tie).
    pub fn trust(&self, a: usize, b: usize) -> f32 {
        self.ties[a]
            .iter()
            .find(|t| t.other == b as u32)
            .map_or(0.0, |t| t.value)
    }

    /// Change the trust between `a` and `b` by `delta`, both ways.
    pub fn bump_trust(&mut self, a: usize, b: usize, delta: f32) {
        if a == b {
            return;
        }
        self.bump_one(a, b, delta);
        self.bump_one(b, a, delta);
    }

    fn bump_one(&mut self, a: usize, b: usize, delta: f32) {
        let ties = &mut self.ties[a];
        if let Some(t) = ties.iter_mut().find(|t| t.other == b as u32) {
            t.value = (t.value + delta).clamp(-1.0, 1.0);
            return;
        }
        // A new acquaintance replaces the weakest tie if it already matters more.
        let (k, weakest) = ties
            .iter()
            .enumerate()
            .min_by(|x, y| x.1.value.abs().total_cmp(&y.1.value.abs()))
            .map(|(k, t)| (k, t.value.abs()))
            .expect("TIES > 0");
        let value = delta.clamp(-1.0, 1.0);
        if ties[k].other == u32::MAX || value.abs() > weakest {
            ties[k] = Tie {
                other: b as u32,
                value,
            };
        }
    }

    /// Fade every tie toward zero and forget the dead and the faded.
    pub fn fade_trust(&mut self, factor: f32) {
        for i in 0..self.len() {
            if !self.alive[i] {
                continue;
            }
            for t in &mut self.ties[i] {
                if t.other == u32::MAX {
                    continue;
                }
                t.value *= factor;
                if !self.alive[t.other as usize] || t.value.abs() < 1e-3 {
                    *t = Tie::NONE;
                }
            }
        }
    }

    /// Mean trust each member of `group` places in the rest of the group (`sorted`
    /// is the same group in ascending order, for lookups). Distrust counts as zero.
    pub fn cohesion(&self, group: &[u32], sorted: &[u32]) -> f32 {
        if group.len() < 2 {
            return 0.0;
        }
        let mut total = 0.0;
        for &a in group {
            for t in &self.ties[a as usize] {
                if t.value > 0.0 && t.other != a && sorted.binary_search(&t.other).is_ok() {
                    total += t.value;
                }
            }
        }
        (total / group.len() as f32).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(n: usize) -> AgentStore {
        let mut s = AgentStore::default();
        for _ in 0..n {
            s.alive.push(true);
            s.ties.push([Tie::NONE; TIES]);
        }
        s
    }

    #[test]
    fn ties_build_fade_and_make_room() {
        let mut s = store(TIES + 3);
        s.bump_trust(0, 1, 0.3);
        assert_eq!(s.trust(1, 0), 0.3);
        s.bump_trust(0, 1, 0.9);
        assert_eq!(s.trust(0, 1), 1.0);
        // Fill every slot of agent 0, then a weak newcomer does not displace anyone.
        for b in 2..=TIES {
            s.bump_trust(0, b, 0.2);
        }
        s.bump_trust(0, TIES + 1, 0.1);
        assert_eq!(s.trust(0, TIES + 1), 0.0);
        s.bump_trust(0, TIES + 2, 0.5);
        assert_eq!(s.trust(0, TIES + 2), 0.5);
        assert_eq!(s.trust(0, 1), 1.0);
        s.alive[1] = false;
        s.fade_trust(0.5);
        assert_eq!(s.trust(0, 1), 0.0);
        assert_eq!(s.trust(0, TIES + 2), 0.25);
        let group = [0, TIES as u32 + 2];
        assert!(s.cohesion(&group, &group) > 0.0);
    }
}
