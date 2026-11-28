//! The win-chance forecast (docs/design/war-ideas.md, idea 1): from where a
//! live battle stands, about 20 short simulations of the rest of it on the
//! squad model, for the player's current orders and for a few others, say
//! how likely each is to win and what it costs: "if you charge now: 70% win
//! (52-84%), about 40 of yours dead".
//!
//! - [`Battle::snapshot`] takes the battle as it stands, on either model, as
//!   squad specs: each squad where it stands, its morale, its stance and
//!   orders, and its members with their wounds. Those already down, or who
//!   have left the field, are out of play: they neither fight again nor
//!   count among the dead the forecast gives. A squad that has fled or is
//!   destroyed keeps its place (orders name squads by index) with nobody in
//!   play. Arrows start full again, the one thing the snapshot loses.
//! - [`Snapshot::run`] fights the rest once on the squad model, with a
//!   [`Proposal`] for the player's squads, from a seed of its own.
//! - [`Forecaster`] does it off the frame: a worker thread takes the newest
//!   snapshot and runs every proposal a few seeds at a time, so its
//!   [`Forecast`]s firm up as the runs come in, and starts again on a newer
//!   snapshot.
//!
//! How sure it is: each forecast gives a 90% interval for the win chance
//! (Wilson's, from its runs), and it is the squad model's guess at a battle
//! the soldier model fights, so take it as a guide.

use super::*;
use std::sync::mpsc;
use std::sync::Mutex;

/// Runs per proposal.
pub const RUNS: u32 = 20;
/// Runs per proposal in each round of the worker, so the forecasts show
/// early and firm up.
const ROUND: u32 = 2;

/// What the player's squads do in a forecast.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Proposal {
    /// Their current orders.
    AsOrdered,
    /// Each attacks the nearest enemy squad still fighting.
    Charge,
    Hold,
    Withdraw,
}

impl Proposal {
    pub const ALL: [Proposal; 4] = [
        Proposal::AsOrdered,
        Proposal::Charge,
        Proposal::Hold,
        Proposal::Withdraw,
    ];

    pub fn words(self) -> &'static str {
        match self {
            Proposal::AsOrdered => "as ordered",
            Proposal::Charge => "if you charge now",
            Proposal::Hold => "if you hold",
            Proposal::Withdraw => "if you withdraw",
        }
    }
}

/// A battle as it stands, ready to be fought on from here on the squad
/// model ([`Battle::snapshot`]).
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// The battle's parameters, on the squad model, with the time left.
    pub params: BattleParams,
    pub field: Field,
    /// Each squad where it stands, with its members as they are; those out
    /// of play have no health left.
    pub specs: Vec<SquadSpec>,
    /// Who of each squad is in play (fighting or able to), member order.
    pub in_play: Vec<Vec<bool>>,
    pub orders: Vec<Vec<Order>>,
    pub stances: Vec<Stance>,
    pub doctrines: [Doctrine; 2],
    /// Whether each squad is still fighting.
    pub fighting: Vec<bool>,
    /// The side the forecast is for.
    pub side: u8,
    /// Battle time it was taken at, and the battle's seed.
    pub t: f32,
    pub seed: u64,
}

impl Battle {
    /// The battle as it stands, for a forecast for side `side`.
    pub fn snapshot(&self, side: u8) -> Snapshot {
        let m = self.model();
        let soldiers = match self {
            Self::Squads(_) => Vec::new(),
            Self::Soldiers(b) => b.frame_soldiers(),
        };
        let mut specs = Vec::with_capacity(m.count());
        let mut in_play = Vec::with_capacity(m.count());
        let mut fighting = Vec::with_capacity(m.count());
        for s in 0..m.count() {
            let mut spec = m.spec(s).clone();
            let state = m.state(s);
            let on_field = matches!(state, SquadState::Fighting | SquadState::Routing);
            let play: Vec<bool> = spec
                .members
                .iter()
                .enumerate()
                .map(|(k, member)| {
                    let left = soldiers
                        .get(s)
                        .and_then(|f| f.get(k))
                        .is_some_and(|f| f.state == FighterState::Fled);
                    on_field && member.fights() && !left
                })
                .collect();
            for (member, &p) in spec.members.iter_mut().zip(&play) {
                if !p {
                    member.health = member.health.min(0.0);
                }
            }
            spec.pos = m.position(s);
            spec.morale = m.morale(s);
            specs.push(spec);
            in_play.push(play);
            fighting.push(state == SquadState::Fighting);
        }
        let params = BattleParams {
            model: BattleModel::Squads,
            max_seconds: (m.params().max_seconds - m.seconds()).max(m.params().dt),
            ..m.params().clone()
        };
        Snapshot {
            params,
            field: m.field().clone(),
            specs,
            in_play,
            orders: (0..m.count()).map(|s| m.orders(s).to_vec()).collect(),
            stances: (0..m.count()).map(|s| m.stance(s)).collect(),
            doctrines: [m.doctrine(0), m.doctrine(1)],
            fighting,
            side,
            t: m.seconds(),
            seed: m.seed(),
        }
    }
}

/// One run's result: whether the side won, and how many in play died on
/// it and on the other side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunResult {
    pub won: bool,
    pub dead: u32,
    pub enemy_dead: u32,
}

impl Snapshot {
    /// What squad `q` of the side is told under `p`, or `None` to keep its
    /// orders.
    fn orders_for(&self, p: Proposal, q: usize) -> Option<Vec<Order>> {
        if self.specs[q].side != self.side || !self.fighting[q] {
            return None;
        }
        match p {
            Proposal::AsOrdered => None,
            Proposal::Hold => Some(vec![Order::Hold]),
            Proposal::Withdraw => Some(vec![Order::Withdraw]),
            Proposal::Charge => {
                let at = self.specs[q].pos;
                let d2 = |e: usize| {
                    let p = self.specs[e].pos;
                    (p.0 - at.0).powi(2) + (p.1 - at.1).powi(2)
                };
                let e = (0..self.specs.len())
                    .filter(|&e| self.specs[e].side != self.side && self.fighting[e])
                    .min_by(|&a, &b| d2(a).total_cmp(&d2(b)))?;
                Some(vec![Order::Attack(e)])
            }
        }
    }

    /// Fight the rest once on the squad model under `p`, run `k`'s seed.
    pub fn run(&self, p: Proposal, k: u64) -> RunResult {
        let seed = self
            .seed
            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
            .wrapping_add(k.wrapping_mul(0xbf58_476d_1ce4_e5b9))
            .wrapping_add(self.t.to_bits() as u64);
        let mut b = Battle::new(
            self.params.clone(),
            self.field.clone(),
            self.specs.clone(),
            seed,
        );
        for (side, d) in self.doctrines.iter().enumerate() {
            b.set_doctrine(side as u8, *d);
        }
        for q in 0..self.specs.len() {
            b.set_stance(q, self.stances[q]);
            match self.orders_for(p, q) {
                Some(o) => b.set_orders(q, &o),
                None => b.set_orders(q, &self.orders[q]),
            }
        }
        let report = b.run();
        let mut dead = [0u32; 2];
        for (q, play) in self.in_play.iter().enumerate() {
            let mine = self.specs[q].side == self.side;
            for (k, _) in play.iter().enumerate().filter(|(_, &p)| p) {
                if b.dies(q, k) {
                    dead[usize::from(!mine)] += 1;
                }
            }
        }
        RunResult {
            won: report.winner == Some(self.side),
            dead: dead[0],
            enemy_dead: dead[1],
        }
    }

    /// `n` runs under `p`, from run `from` on, tallied.
    pub fn forecast(&self, p: Proposal, from: u32, n: u32) -> Forecast {
        let mut f = Forecast::new(p, self.t);
        for k in from..from + n {
            f.add(self.run(p, k as u64));
        }
        f
    }
}

/// What the runs of one proposal say so far.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Forecast {
    pub proposal: Proposal,
    /// Battle time of the snapshot it is from.
    pub t: f32,
    pub runs: u32,
    pub wins: u32,
    /// Dead of the side, and of the other, over all runs.
    pub dead: u32,
    pub enemy_dead: u32,
}

impl Forecast {
    pub fn new(proposal: Proposal, t: f32) -> Self {
        Self {
            proposal,
            t,
            runs: 0,
            wins: 0,
            dead: 0,
            enemy_dead: 0,
        }
    }

    pub fn add(&mut self, r: RunResult) {
        self.runs += 1;
        self.wins += u32::from(r.won);
        self.dead += r.dead;
        self.enemy_dead += r.enemy_dead;
    }

    /// Another tally of the same proposal and snapshot added to this one.
    pub fn merge(&mut self, o: &Forecast) {
        self.runs += o.runs;
        self.wins += o.wins;
        self.dead += o.dead;
        self.enemy_dead += o.enemy_dead;
    }

    /// The share of runs won.
    pub fn chance(&self) -> f32 {
        self.wins as f32 / self.runs.max(1) as f32
    }

    /// A 90% interval for the win chance (Wilson's score interval).
    pub fn interval(&self) -> (f32, f32) {
        let n = self.runs.max(1) as f32;
        let p = self.chance();
        let z = 1.645f32;
        let z2 = z * z;
        let centre = (p + z2 / (2.0 * n)) / (1.0 + z2 / n);
        let half = z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / (1.0 + z2 / n);
        ((centre - half).max(0.0), (centre + half).min(1.0))
    }

    /// Mean dead of the side, and of the other, per run.
    pub fn mean_dead(&self) -> (f32, f32) {
        let n = self.runs.max(1) as f32;
        (self.dead as f32 / n, self.enemy_dead as f32 / n)
    }

    /// "if you charge now: 70% win (52-84%), about 40 of yours dead, 25 of
    /// theirs".
    pub fn words(&self) -> String {
        let (lo, hi) = self.interval();
        let (mine, theirs) = self.mean_dead();
        format!(
            "{}: {:.0}% win ({:.0}-{:.0}%), about {:.0} of yours dead, {:.0} of theirs",
            self.proposal.words(),
            100.0 * self.chance(),
            100.0 * lo,
            100.0 * hi,
            mine,
            theirs
        )
    }
}

/// The forecast in words ([`words`]), saying under the heading the
/// weather it counts with, if any ("Rain: bows slowed", from
/// `LiveView::conditions`): for both clients.
pub fn words_in(forecasts: &[Forecast], conditions: Option<&str>) -> String {
    let all = words(forecasts);
    match conditions {
        Some(c) => match all.split_once('\n') {
            Some((head, rest)) => format!("{head}\nIn the weather: {c}\n{rest}"),
            None => format!("{all}\nIn the weather: {c}"),
        },
        None => all,
    }
}

/// The forecast in words, a line a proposal under a heading saying how many
/// runs it rests on: for both clients and the CLI.
pub fn words(forecasts: &[Forecast]) -> String {
    let runs = forecasts.iter().map(|f| f.runs).min().unwrap_or(0);
    let mut out = format!("Forecast ({runs} of {RUNS} runs each on the squad model; 90% range):");
    for f in forecasts {
        out.push('\n');
        out.push_str(&f.words());
    }
    out
}

/// The forecasts so far, and for which snapshot.
#[derive(Clone, Debug, Default)]
struct Shared {
    forecasts: Vec<Forecast>,
    /// Whether the worker has finished every run of the newest snapshot.
    done: bool,
}

/// Runs forecasts on a worker thread, off the frame: [`Self::feed`] it the
/// newest snapshot, read [`Self::latest`]. Dropping it stops the thread.
pub struct Forecaster {
    /// A snapshot to forecast from, or `None` to stop forecasting.
    send: mpsc::Sender<Option<Snapshot>>,
    shared: Arc<Mutex<Shared>>,
    /// Battle time of the last snapshot fed.
    fed: Option<f32>,
}

impl Default for Forecaster {
    fn default() -> Self {
        Self::new()
    }
}

impl Forecaster {
    /// Battle seconds between snapshots worth a new forecast.
    pub const EVERY: f32 = 10.0;

    pub fn new() -> Self {
        let (send, recv) = mpsc::channel::<Option<Snapshot>>();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let out = shared.clone();
        std::thread::Builder::new()
            .name("battle forecast".into())
            .spawn(move || work(recv, out))
            .expect("cannot start the forecast thread");
        Self {
            send,
            shared,
            fed: None,
        }
    }

    /// Whether a snapshot at battle time `t` is worth feeding: the first,
    /// one [`Self::EVERY`] seconds on, or one from an earlier time (a new
    /// battle).
    pub fn wants(&self, t: f32) -> bool {
        self.fed.is_none_or(|f| t < f || t >= f + Self::EVERY)
    }

    /// Forecast from this snapshot from now on, dropping any older one.
    pub fn feed(&mut self, s: Snapshot) {
        self.fed = Some(s.t);
        let _ = self.send.send(Some(s));
    }

    /// Forget the battle: the next snapshot is wanted at once, and nothing
    /// is shown meanwhile.
    pub fn clear(&mut self) {
        if self.fed.take().is_none() {
            return;
        }
        let _ = self.send.send(None);
        if let Ok(mut s) = self.shared.lock() {
            *s = Shared::default();
        }
    }

    /// The forecasts so far (empty before the first runs are in), and
    /// whether they are complete.
    pub fn latest(&self) -> (Vec<Forecast>, bool) {
        self.shared
            .lock()
            .map(|s| (s.forecasts.clone(), s.done))
            .unwrap_or_default()
    }
}

/// The worker: for the newest snapshot, a round of a few runs of every
/// proposal at a time until each has [`RUNS`]; a newer snapshot starts over.
fn work(recv: mpsc::Receiver<Option<Snapshot>>, out: Arc<Mutex<Shared>>) {
    let mut current: Option<Snapshot> = None;
    let mut tally: Vec<Forecast> = Vec::new();
    loop {
        // The newest snapshot sent, waiting for one when idle.
        let next = if current.is_none() {
            match recv.recv() {
                Ok(s) => Some(s),
                Err(_) => return,
            }
        } else {
            None
        };
        match next.into_iter().chain(recv.try_iter()).last() {
            Some(Some(s)) => {
                tally = Proposal::ALL
                    .iter()
                    .map(|&p| Forecast::new(p, s.t))
                    .collect();
                current = Some(s);
            }
            // Stopped: forget the battle (`clear` has emptied the shared
            // forecasts already).
            Some(None) => {
                current = None;
                continue;
            }
            None => {}
        }
        let Some(s) = &current else { continue };
        let done_runs = tally.first().map_or(0, |f| f.runs);
        let n = ROUND.min(RUNS - done_runs);
        for f in tally.iter_mut() {
            f.merge(&s.forecast(f.proposal, done_runs, n));
        }
        let done = tally.first().is_some_and(|f| f.runs >= RUNS);
        if let Ok(mut o) = out.lock() {
            o.forecasts = tally.clone();
            o.done = done;
        }
        if done {
            current = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interval_narrows_with_runs_and_holds_the_rate() {
        let mut f = Forecast::new(Proposal::Charge, 0.0);
        for k in 0..20 {
            f.add(RunResult {
                won: k % 10 < 7,
                dead: 2,
                enemy_dead: 1,
            });
        }
        let (lo, hi) = f.interval();
        assert!((f.chance() - 0.7).abs() < 1e-6);
        assert!(lo < 0.7 && hi > 0.7 && hi - lo < 0.4, "{lo} {hi}");
        assert_eq!(
            f.words(),
            "if you charge now: 70% win (52-84%), about 2 of yours dead, 1 of theirs"
        );
        let mut all = Forecast::new(Proposal::Hold, 0.0);
        for _ in 0..20 {
            all.add(RunResult {
                won: true,
                dead: 0,
                enemy_dead: 0,
            });
        }
        assert!(all.interval().1 == 1.0 && all.interval().0 > 0.8);
    }

    /// A snapshot of a battle under way fights on: the strong side's
    /// forecast wins, withdrawing loses, and those already down stay out.
    #[test]
    fn a_snapshot_forecasts_the_rest_of_the_battle() {
        for model in [BattleModel::Squads, BattleModel::Soldiers] {
            let (field, specs) = parse_lineup("swordsmen*60 vs levy*20").unwrap();
            let params = BattleParams {
                model,
                ..BattleParams::default()
            };
            let mut b = Battle::new(params, field, specs, 4);
            while b.seconds() < 90.0 && !b.finished() {
                b.step();
            }
            let s = b.snapshot(0);
            assert_eq!(s.specs.len(), 2);
            assert!((s.params.max_seconds + s.t - b.params().max_seconds).abs() < 1e-3);
            for (spec, play) in s.specs.iter().zip(&s.in_play) {
                for (m, &p) in spec.members.iter().zip(play) {
                    assert_eq!(p, m.health > 0.0, "{model:?}");
                }
            }
            let as_ordered = s.forecast(Proposal::AsOrdered, 0, 6);
            assert!(as_ordered.chance() > 0.8, "{model:?} {as_ordered:?}");
            let withdraw = s.forecast(Proposal::Withdraw, 0, 6);
            assert_eq!(withdraw.wins, 0, "{model:?} {withdraw:?}");
            // The same runs give the same results.
            assert_eq!(
                s.forecast(Proposal::Charge, 0, 3),
                s.forecast(Proposal::Charge, 0, 3)
            );
        }
    }

    #[test]
    fn the_forecaster_firms_up_off_the_thread() {
        let (field, specs) = parse_lineup("spearmen*20 vs swordsmen*20").unwrap();
        let b = Battle::new(BattleParams::default(), field, specs, 1);
        let mut f = Forecaster::new();
        assert!(f.wants(0.0));
        f.feed(b.snapshot(1));
        assert!(!f.wants(5.0) && f.wants(10.0));
        let start = std::time::Instant::now();
        loop {
            let (got, done) = f.latest();
            if done {
                assert_eq!(got.len(), Proposal::ALL.len());
                assert!(got.iter().all(|g| g.runs == RUNS));
                assert!(words(&got).starts_with("Forecast (20 of 20 runs"));
                break;
            }
            assert!(
                start.elapsed().as_secs() < 120,
                "the forecast never finished"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
