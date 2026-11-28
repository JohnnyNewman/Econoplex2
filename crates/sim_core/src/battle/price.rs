//! The price of a battle (docs/design/war-ideas.md, idea 2): every soldier is
//! a person with a name, a trade and battles behind them, and what a battle
//! costs is the work its dead and taken would have done.
//!
//! The battle itself knows nothing of people: the war system that sets it up
//! gives each member a [`Fighter`] (`systems::military`), in squad and member
//! order, and the replays and the live view carry them for the clients. A
//! fighter is lost when they die of the battle or are taken prisoner
//! ([`Battle::lost`]); the wounded who live go back to work.

use super::*;

/// One member of a battle as a person.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Fighter {
    /// Index in the agent store.
    pub id: u32,
    pub name: String,
    /// What they work at, as a worker ("baker"); "soldier" for someone who
    /// never worked a trade.
    pub trade: String,
    /// Battles fought, this one included.
    pub battles: u16,
    /// What a week of their work makes in their town, in loaves of bread:
    /// the value a worker-day at their trade adds there at today's prices,
    /// over the price of bread.
    pub weekly: f32,
}

impl Fighter {
    /// "Mara, baker, 3 battles".
    pub fn words(&self) -> String {
        format!(
            "{}, {}, {} {}",
            self.name,
            self.trade,
            self.battles,
            if self.battles == 1 {
                "battle"
            } else {
                "battles"
            }
        )
    }
}

/// What one side has lost so far: how many of each trade, and the week's
/// work they would have done.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Losses {
    /// (trade, how many), the most first, then by name.
    pub trades: Vec<(String, u32)>,
    /// Loaves of bread a week.
    pub weekly: f32,
}

/// Trades a line names before it sums up the rest.
const NAMED: usize = 4;

impl Losses {
    /// The losses of the members `lost(squad, member)` says, of the squads
    /// on `side` (`sides[squad]`), from `people` (by squad, member order).
    pub fn of(
        people: &[Vec<Fighter>],
        sides: &[u8],
        side: u8,
        lost: impl Fn(usize, usize) -> bool,
    ) -> Losses {
        let mut out = Losses::default();
        for (q, members) in people.iter().enumerate() {
            if sides.get(q) != Some(&side) {
                continue;
            }
            for (k, f) in members.iter().enumerate() {
                if !lost(q, k) {
                    continue;
                }
                out.weekly += f.weekly;
                match out.trades.iter_mut().find(|t| t.0 == f.trade) {
                    Some(t) => t.1 += 1,
                    None => out.trades.push((f.trade.clone(), 1)),
                }
            }
        }
        out.trades
            .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    pub fn count(&self) -> u32 {
        self.trades.iter().map(|t| t.1).sum()
    }

    /// `other`'s losses added to these (a town's over several battles).
    pub fn add(&mut self, other: &Losses) {
        self.weekly += other.weekly;
        for (trade, n) in &other.trades {
            match self.trades.iter_mut().find(|t| &t.0 == trade) {
                Some(t) => t.1 += n,
                None => self.trades.push((trade.clone(), *n)),
            }
        }
        self.trades
            .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    }

    /// "lost: 2 smiths, 5 farmers, about 300 loaves a week", or "lost:
    /// nobody".
    pub fn words(&self) -> String {
        self.words_as("lost")
    }

    /// The same under another word: "fallen: 2 smiths, ...".
    pub fn words_as(&self, what: &str) -> String {
        if self.trades.is_empty() {
            return format!("{what}: nobody");
        }
        let mut parts: Vec<String> = self
            .trades
            .iter()
            .take(NAMED)
            .map(|(t, n)| format!("{n} {}", if *n == 1 { t.clone() } else { plural(t) }))
            .collect();
        let rest: u32 = self.trades.iter().skip(NAMED).map(|t| t.1).sum();
        match rest {
            0 => {}
            1 => parts.push("1 other".into()),
            n => parts.push(format!("{n} others")),
        }
        let loaves = about(self.weekly);
        if loaves > 0.0 {
            let unit = if loaves == 1.0 { "loaf" } else { "loaves" };
            parts.push(format!("about {loaves:.0} {unit} a week"));
        }
        format!("{what}: {}", parts.join(", "))
    }
}

/// A round figure: 2 significant digits, at least 1.
fn about(x: f32) -> f32 {
    if x < 0.5 {
        return 0.0;
    }
    let step = 10f32.powi((x.log10().floor() as i32 - 1).max(0));
    (x / step).round() * step
}

/// A worker noun in the plural: "smiths", "quarrymen", "dairy maids".
pub fn plural(noun: &str) -> String {
    if let Some(stem) = noun.strip_suffix("man") {
        format!("{stem}men")
    } else if ["s", "sh", "ch", "x"].iter().any(|e| noun.ends_with(e)) {
        format!("{noun}es")
    } else {
        format!("{noun}s")
    }
}

impl Battle {
    /// Whether member `k` of squad `squad` is lost to their town: dead of the
    /// battle ([`Self::dies`]: before the end, only the killed outright) or
    /// taken prisoner.
    pub fn lost(&self, squad: usize, k: usize) -> bool {
        self.members(squad).get(k).is_some_and(|m| m.captured) || self.dies(squad, k)
    }

    /// Each side's losses so far, from the fighters `people`.
    pub fn losses(&self, people: &[Vec<Fighter>]) -> [Losses; 2] {
        let sides: Vec<u8> = (0..people.len())
            .map(|q| self.side(q).unwrap_or(u8::MAX))
            .collect();
        [0, 1].map(|s| Losses::of(people, &sides, s, |q, k| self.lost(q, k)))
    }
}

impl Replay {
    /// Each side's losses by battle time `t`: those lost at the end who had
    /// fallen by then. Empty without the fighters.
    pub fn losses_at(&self, t: f32) -> [Losses; 2] {
        let sides: Vec<u8> = self.squads.iter().map(|s| s.side).collect();
        let lost = |q: usize, k: usize| {
            self.lost
                .get(q)
                .and_then(|m| m.get(k))
                .copied()
                .flatten()
                .is_some_and(|at| at <= t)
        };
        [0, 1].map(|s| Losses::of(&self.people, &sides, s, lost))
    }

    /// The fighter at field point `at` by battle time `t`, in words with
    /// their squad: the soldier under it (on the soldier model, within a
    /// metre and a half), else the first of the squad under it still in the
    /// fight. `None` without the fighters.
    pub fn inspect(&self, t: f32, at: (f32, f32)) -> Option<String> {
        let d2 = |p: (f32, f32)| (p.0 - at.0).powi(2) + (p.1 - at.1).powi(2);
        let soldiers = self.soldiers_at(t);
        let soldier = soldiers
            .iter()
            .enumerate()
            .flat_map(|(q, m)| m.iter().enumerate().map(move |(k, s)| (q, k, *s)))
            .filter(|(_, _, s)| matches!(s.state, FighterState::Standing | FighterState::Routing))
            .map(|(q, k, s)| (q, k, d2(s.pos())))
            .filter(|&(_, _, d)| d <= 1.5 * 1.5)
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(q, k, _)| (q, k));
        let (q, k) = match soldier {
            Some(s) => s,
            None if soldiers.is_empty() => {
                // The squad model: the block under the pointer.
                let (fa, fb, u) = self.around(t)?;
                let q = (0..self.squads.len())
                    .filter_map(|q| {
                        let (a, b) = (fa.squads.get(q)?, fb.squads.get(q)?);
                        let n = if u < 0.5 { a.2 } else { b.2 };
                        let p = (a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u);
                        let r = (1.2 * (n as f32).sqrt()).max(4.0);
                        (n > 0 && d2(p) <= r * r).then_some((q, d2(p)))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))?
                    .0;
                let fell = |k: usize| {
                    self.lost
                        .get(q)
                        .and_then(|m| m.get(k))
                        .copied()
                        .flatten()
                        .is_some_and(|at| at <= t)
                };
                let k = (0..self.people.get(q)?.len()).find(|&k| !fell(k))?;
                (q, k)
            }
            None => return None,
        };
        let f = self.people.get(q)?.get(k)?;
        Some(format!("{} ({})", f.words(), self.squads.get(q)?.name))
    }
}

impl LiveView {
    /// Each side's losses so far (`lost`): while the battle goes on, those
    /// out of the fight; once it is over, the dead and the taken.
    pub fn losses(&self) -> [Losses; 2] {
        let sides: Vec<u8> = self.squads.iter().map(|s| s.side).collect();
        let lost = |q: usize, k: usize| {
            self.lost
                .get(q)
                .and_then(|m| m.get(k))
                .copied()
                .unwrap_or(false)
        };
        [0, 1].map(|s| Losses::of(&self.people, &sides, s, lost))
    }

    /// Soldier `k` of squad `q` as a person, "Mara, baker, 3 battles".
    pub fn fighter_words(&self, q: usize, k: usize) -> Option<String> {
        self.people.get(q)?.get(k).map(Fighter::words)
    }
}

/// The price ticker, for both clients: side `first`'s losses, then the
/// other's, a line each: "Ashford lost: 2 smiths, ...". `names` are each
/// side's town; `what` is "lost", or "fallen" while a live battle cannot
/// yet tell the dead from the wounded.
pub fn ticker(losses: &[Losses; 2], names: &[String; 2], first: u8, what: &str) -> String {
    let order = if first == 1 { [1, 0] } else { [0, 1] };
    order
        .iter()
        .map(|&s| format!("{} {}", names[s], losses[s].words_as(what)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Each side's name: the raiding town (or "The attackers") and the town
/// fought over, by `name` (a town's index to its name).
fn side_names(attacker: Option<u16>, town: u16, name: &dyn Fn(u16) -> String) -> [String; 2] {
    [
        attacker.map_or_else(|| "The attackers".to_string(), name),
        name(town),
    ]
}

impl Replay {
    /// The ticker at battle time `t`, attackers first ([`ticker`]); `None`
    /// for a battle of nobody in particular.
    pub fn ticker(&self, t: f32, name: &dyn Fn(u16) -> String) -> Option<String> {
        if self.people.is_empty() {
            return None;
        }
        let names = side_names(self.attacker, self.town, name);
        Some(ticker(&self.losses_at(t), &names, 0, "lost"))
    }
}

impl LiveView {
    /// The ticker now, the player's side first ([`ticker`]): while the
    /// battle goes on, those fallen so far; once it is over, those lost.
    /// `None` for a battle of nobody in particular.
    pub fn ticker(&self, name: &dyn Fn(u16) -> String) -> Option<String> {
        if self.people.is_empty() {
            return None;
        }
        let names = side_names(self.attacker, self.town, name);
        let what = if self.finished { "lost" } else { "fallen" };
        Some(ticker(&self.losses(), &names, self.player_side, what))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(trade: &str, weekly: f32) -> Fighter {
        Fighter {
            id: 0,
            name: "Mara".into(),
            trade: trade.into(),
            battles: 3,
            weekly,
        }
    }

    #[test]
    fn a_fighter_and_the_losses_read_as_the_idea_says() {
        assert_eq!(f("baker", 0.0).words(), "Mara, baker, 3 battles");
        let people = vec![
            vec![f("smith", 40.0), f("smith", 40.0), f("farmer", 30.0)],
            vec![f("farmer", 30.0); 5],
        ];
        let l = Losses::of(&people, &[0, 0], 0, |q, k| q == 1 || k < 2);
        assert_eq!(
            l.words(),
            "lost: 5 farmers, 2 smiths, about 230 loaves a week"
        );
        assert_eq!(l.count(), 7);
        assert_eq!(
            Losses::of(&people, &[0, 0], 1, |_, _| true).words(),
            "lost: nobody"
        );
        let one = Losses::of(&[vec![f("smith", 1.0)]], &[0], 0, |_, _| true);
        assert_eq!(one.words(), "lost: 1 smith, about 1 loaf a week");
        assert_eq!(plural("quarryman"), "quarrymen");
        assert_eq!(plural("dairy maid"), "dairy maids");
        assert_eq!(about(304.0), 300.0);
        assert_eq!(about(7.4), 7.0);
    }
}
