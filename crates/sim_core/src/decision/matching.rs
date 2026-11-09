//! Job matching: who gets which place when more agents want a job than it has.
//!
//! Interface, per town: each agent's ranked list of acceptable options (best first;
//! an option is a job and whether the agent applies as a regular worker or an
//! apprentice), the regular and apprentice places of each job, and how a job ranks
//! its applicants. Output: each agent's match, or none (it rests).
//!
//! `FirstCome` is today's rule (in agent order, the first choice still open), kept in
//! `decide`. `DeferredAcceptance` is the agent-proposing Gale-Shapley algorithm: agents
//! propose down their lists, each place keeps its best applicants so far and bumps
//! the rest, until nobody has anywhere left to propose. The result is stable and
//! does not depend on the agents' order.

use serde::{Deserialize, Serialize};

/// An option on an agent's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Opt {
    /// Index of the job in the town's job list.
    pub job: u32,
    pub apprentice: bool,
}

/// Places a job offers today.
#[derive(Debug, Clone, Copy, Default)]
pub struct Places {
    pub regular: u32,
    pub apprentice: u32,
}

/// `decision.matching` in models.ron.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum MatchingParams {
    /// In agent order, each takes its first choice still open (today's rule).
    #[default]
    FirstCome,
    /// Stable matching; regular places go to the applicants who would do the job
    /// best, apprentice places to those with the most to gain. Apprentices are paid
    /// `apprentice_wage` of a regular worker's share. Each job's share of
    /// apprentice places is the workplace's own choice (see `ApprenticeShare`).
    DeferredAcceptance {
        apprentice_wage: f32,
        share: ApprenticeShareParams,
    },
}

impl MatchingParams {
    pub fn apprentice_wage(&self) -> f32 {
        match self {
            MatchingParams::FirstCome => 1.0,
            MatchingParams::DeferredAcceptance {
                apprentice_wage, ..
            } => *apprentice_wage,
        }
    }
}

/// How a workplace sets its share of apprentice places, each month, within bounds:
/// it adds places while cheap labour now plus a trained worker later is worth more
/// than the output an apprentice doesn't yet deliver.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApprenticeShareParams {
    pub min: f32,
    pub max: f32,
    pub step: f32,
    /// An apprentice's output, as a share of a regular worker's.
    pub trainee_output: f32,
    /// Share of trained apprentices expected to stay and work in the town.
    pub retention: f32,
    /// Discounted days of skilled work a day of training is worth later.
    pub payoff_days: f32,
}

impl Default for ApprenticeShareParams {
    fn default() -> Self {
        ApprenticeShareParams {
            min: 0.0,
            max: 0.3,
            step: 0.05,
            trainee_output: 0.5,
            retention: 0.8,
            payoff_days: 0.5,
        }
    }
}

impl ApprenticeShareParams {
    /// Net gain to the workplace, per apprentice-day, of taking an apprentice
    /// instead of a regular worker, in units of a regular worker's daily value
    /// added. `wage_share` is what a regular worker gets of that value; `shortage`
    /// (0 = none) how scarce skilled workers are.
    pub fn gain(&self, wage_share: f32, apprentice_wage: f32, shortage: f32) -> f32 {
        let now = (self.trainee_output - 1.0) + wage_share * (1.0 - apprentice_wage);
        let later = self.retention * (1.0 - wage_share) * self.payoff_days * (1.0 + shortage);
        now + later
    }

    pub fn adjust(&self, share: f32, gain: f32) -> f32 {
        let s = if gain > 0.0 {
            share + self.step
        } else {
            share - self.step
        };
        s.clamp(self.min, self.max)
    }
}

/// Stable matching. `lists[a]` is agent `a`'s ranked options (best first). `places`
/// is indexed by job. `rank(a, opt)` is how much the job behind `opt` wants agent
/// `a` (higher is better; ties go to the lower agent index). Returns each agent's
/// match.
pub fn deferred_acceptance(
    lists: &[Vec<Opt>],
    places: &[Places],
    rank: impl Fn(usize, Opt) -> f32,
) -> Vec<Option<Opt>> {
    let n = lists.len();
    let mut next = vec![0usize; n];
    let mut matched: Vec<Option<Opt>> = vec![None; n];
    // Held applicants per (job, apprentice): (score, agent).
    let mut held: Vec<[Vec<(f32, usize)>; 2]> = vec![[Vec::new(), Vec::new()]; places.len()];
    let mut free: Vec<usize> = (0..n).rev().collect();
    while let Some(a) = free.pop() {
        let Some(&opt) = lists[a].get(next[a]) else {
            continue;
        };
        next[a] += 1;
        let j = opt.job as usize;
        let cap = if opt.apprentice {
            places[j].apprentice
        } else {
            places[j].regular
        } as usize;
        if cap == 0 {
            free.push(a);
            continue;
        }
        let slot = &mut held[j][opt.apprentice as usize];
        slot.push((rank(a, opt), a));
        if slot.len() > cap {
            // Bump the weakest (lowest score; on ties, the higher agent index).
            let (k, _) = slot
                .iter()
                .enumerate()
                .min_by(|x, y| x.1 .0.total_cmp(&y.1 .0).then(y.1 .1.cmp(&x.1 .1)))
                .unwrap();
            let (_, out) = slot.swap_remove(k);
            matched[out] = None;
            free.push(out);
        }
        if slot.iter().any(|&(_, b)| b == a) {
            matched[a] = Some(opt);
        }
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(job: u32) -> Opt {
        Opt {
            job,
            apprentice: false,
        }
    }
    fn app(job: u32) -> Opt {
        Opt {
            job,
            apprentice: true,
        }
    }

    #[test]
    fn the_best_applicant_gets_a_scarce_place() {
        // Three agents want job 0 (one place); job 1 has room.
        let lists = vec![vec![reg(0), reg(1)], vec![reg(0), reg(1)], vec![reg(0)]];
        let places = [
            Places {
                regular: 1,
                apprentice: 0,
            },
            Places {
                regular: 5,
                apprentice: 0,
            },
        ];
        let skill = [0.2, 0.9, 0.5];
        let m = deferred_acceptance(&lists, &places, |a, _| skill[a]);
        assert_eq!(m, vec![Some(reg(1)), Some(reg(0)), None]);
    }

    #[test]
    fn the_result_does_not_depend_on_agent_order() {
        let lists = vec![
            vec![reg(0), reg(1)],
            vec![reg(1), reg(0)],
            vec![reg(0), reg(1)],
        ];
        let places = [
            Places {
                regular: 1,
                apprentice: 0,
            },
            Places {
                regular: 1,
                apprentice: 0,
            },
        ];
        let skill = [0.3, 0.8, 0.6];
        let m = deferred_acceptance(&lists, &places, |a, _| skill[a]);
        // Reverse the agents and their skills: same people, same places.
        let rl: Vec<Vec<Opt>> = lists.iter().rev().cloned().collect();
        let rs: Vec<f32> = skill.iter().rev().copied().collect();
        let mut r = deferred_acceptance(&rl, &places, |a, _| rs[a]);
        r.reverse();
        assert_eq!(m, r);
    }

    #[test]
    fn apprentice_places_go_to_those_with_most_to_gain() {
        // Two beginners and an expert all apply as apprentices to one place.
        let lists = vec![vec![app(0)], vec![app(0)], vec![app(0)]];
        let places = [Places {
            regular: 0,
            apprentice: 1,
        }];
        let room_to_learn = [0.9, 0.1, 0.6];
        let m = deferred_acceptance(&lists, &places, |a, _| room_to_learn[a]);
        assert_eq!(m, vec![Some(app(0)), None, None]);
    }

    #[test]
    fn a_workplace_takes_apprentices_when_it_pays() {
        let p = ApprenticeShareParams::default();
        // Cheap apprentices and a shortage of skilled hands: worth it.
        assert!(p.gain(0.5, 0.4, 1.0) > 0.0);
        // Apprentices paid like regulars, no shortage, little payoff: not worth it.
        let stingy = ApprenticeShareParams {
            payoff_days: 0.0,
            ..p.clone()
        };
        assert!(stingy.gain(0.5, 1.0, 0.0) < 0.0);
        assert_eq!(p.adjust(0.3, 1.0), 0.3);
        assert_eq!(p.adjust(0.0, -1.0), 0.0);
    }
}
