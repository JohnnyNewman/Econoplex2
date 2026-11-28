//! What a battle leaves behind: events, reports, replays and the battle log.

use super::*;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EventKind {
    Contact {
        by: usize,
        with: usize,
    },
    Charge {
        by: usize,
        into: usize,
        braced: bool,
    },
    Wavering(usize),
    Routing(usize),
    Fled(usize),
    Destroyed(usize),
    OutOfArrows(usize),
    /// A stretch of the town's wall or a gate battered down (its index in
    /// `Field::walls`).
    Breach(usize),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub t: f32,
    pub kind: EventKind,
}

/// One squad's result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SquadReport {
    pub name: String,
    pub side: u8,
    pub kit: String,
    pub start: usize,
    pub standing: usize,
    pub wounded: usize,
    pub dead: usize,
    pub state: SquadState,
    pub morale: f32,
}

/// What happened: who won, when, and each squad's losses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// The side left holding the field, or `None` if time ran out with both standing.
    pub winner: Option<u8>,
    pub seconds: f32,
    pub squads: Vec<SquadReport>,
    pub events: Vec<Event>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let who = match self.winner {
            Some(0) => "attackers win",
            Some(_) => "defenders win",
            None => "no decision",
        };
        writeln!(f, "{who} after {:.0} min", self.seconds / 60.0)?;
        for s in &self.squads {
            writeln!(
                f,
                "  {:<5} {:<14} {:<13} {:>3} in, {:>3} standing, {:>3} wounded, {:>3} dead, {:?}, morale {:.2}",
                if s.side == 0 { "att" } else { "def" },
                s.name,
                s.kit,
                s.start,
                s.standing,
                s.wounded,
                s.dead,
                s.state,
                s.morale
            )?;
        }
        Ok(())
    }
}

/// One squad in a battle replay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplaySquad {
    pub name: String,
    pub side: u8,
    pub mounted: bool,
    /// Fighters at the start.
    pub start: u16,
}

/// Where every squad stood at one moment of a battle: (x, y) in metres on the
/// field, fighters still standing, and its state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplayFrame {
    pub t: f32,
    pub squads: Vec<(f32, f32, u16, SquadState)>,
    /// On the soldier model, every soldier of each squad, in member order;
    /// empty on the squad model.
    pub soldiers: Vec<Vec<FrameSoldier>>,
    /// Hit points left of each piece of a town's wall on the field, in the
    /// order of the replay's `walls` (0: a breach, or a gap never built).
    #[serde(default)]
    pub walls: Vec<f32>,
}

/// One piece of a town's wall on a battlefield, for the clients to draw:
/// its ends and half its thickness (field metres), whether it is a gate,
/// and its hit points at most (0: a gap never built).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplayWall {
    pub a: (f32, f32),
    pub b: (f32, f32),
    pub half_width: f32,
    pub gate: bool,
    pub max: f32,
}

/// One soldier at one moment of a battle, packed into six bytes: where they
/// stand to a tenth of a metre, the way they face to 1/256 of a turn, and
/// how they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameSoldier {
    x: i16,
    y: i16,
    facing: u8,
    pub state: FighterState,
}

impl FrameSoldier {
    /// Metres per step of position.
    pub const STEP: f32 = 0.1;

    /// A soldier at (x, y) metres on the field, facing `facing` (any length).
    pub fn new(x: f32, y: f32, facing: (f32, f32), state: FighterState) -> Self {
        let q = |v: f32| {
            (v / Self::STEP)
                .round()
                .clamp(i16::MIN as f32, i16::MAX as f32) as i16
        };
        let turn = facing.1.atan2(facing.0) / std::f32::consts::TAU;
        Self {
            x: q(x),
            y: q(y),
            facing: (turn.rem_euclid(1.0) * 256.0).round() as u32 as u8,
            state,
        }
    }

    /// Where they stand, metres on the field.
    pub fn pos(&self) -> (f32, f32) {
        (self.x as f32 * Self::STEP, self.y as f32 * Self::STEP)
    }

    /// The way they face, unit.
    pub fn facing(&self) -> (f32, f32) {
        let a = self.facing as f32 / 256.0 * std::f32::consts::TAU;
        (a.cos(), a.sin())
    }

    /// Part way (`u` from 0 to 1) from `a` to `b`: the position along the
    /// line, the facing round the shorter way, the state of the nearer one.
    pub fn between(a: Self, b: Self, u: f32) -> Self {
        let lerp = |p: i16, q: i16| (p as f32 + (q as f32 - p as f32) * u).round() as i16;
        let turn = (b.facing.wrapping_sub(a.facing)) as i8;
        Self {
            x: lerp(a.x, b.x),
            y: lerp(a.y, b.y),
            facing: a.facing.wrapping_add((turn as f32 * u).round() as i8 as u8),
            state: if u < 0.5 { a.state } else { b.state },
        }
    }
}

/// A fought battle, for the views to play back where it happened.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Replay {
    /// The town fought over, and the map position of the field's centre.
    pub town: u16,
    pub at: (f32, f32),
    /// Map direction from the attackers' side of the field to the defenders'
    /// (the field's +x), unit length.
    pub facing: (f32, f32),
    /// Field size, metres.
    pub field: (f32, f32),
    pub squads: Vec<ReplaySquad>,
    pub frames: Vec<ReplayFrame>,
    pub winner: Option<u8>,
    /// Each squad's members as people, in member order (`price.rs`); empty
    /// for a battle of nobody in particular (a line-up).
    #[serde(default)]
    pub people: Vec<Vec<Fighter>>,
    /// The raiding town, if the attackers have one.
    #[serde(default)]
    pub attacker: Option<u16>,
    /// For each member lost to their town (dead or taken), the battle time
    /// they fell; `None` for those who came through. By squad, member order.
    #[serde(default)]
    pub lost: Vec<Vec<Option<f32>>>,
    /// The town's wall on the field, if it has one.
    #[serde(default)]
    pub walls: Vec<ReplayWall>,
}

impl Replay {
    /// Drop the long approach, for a battle staged to be watched
    /// (`--battle` in the clients): start three frames before two enemy
    /// squads first come within 60 m, with the clock from there. Returns
    /// where on the field (metres) those two meet: halfway between them.
    pub fn skip_approach(&mut self) -> Option<(f32, f32)> {
        let squads = &self.squads;
        let (k, at) = self.frames.iter().enumerate().find_map(|(k, f)| {
            f.squads.iter().enumerate().find_map(|(a, p)| {
                f.squads
                    .iter()
                    .enumerate()
                    .find(|&(b, q)| {
                        squads[a].side != squads[b].side && (p.0 - q.0).hypot(p.1 - q.1) < 60.0
                    })
                    .map(|(_, q)| (k, (0.5 * (p.0 + q.0), 0.5 * (p.1 + q.1))))
            })
        })?;
        let from = k.saturating_sub(3);
        let t0 = self.frames[from].t;
        self.frames.drain(..from);
        for f in &mut self.frames {
            f.t -= t0;
        }
        Some(at)
    }

    /// The frames before and after battle time `t`, and how far (0 to 1)
    /// `t` lies between them; `None` with no frames.
    pub fn around(&self, t: f32) -> Option<(&ReplayFrame, &ReplayFrame, f32)> {
        let last = self.frames.len().checked_sub(1)?;
        let k = self.frames.partition_point(|f| f.t <= t);
        let a = &self.frames[k.saturating_sub(1).min(last)];
        let b = self.frames.get(k).unwrap_or(a);
        let u = if b.t > a.t {
            ((t - a.t) / (b.t - a.t)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        Some((a, b, u))
    }

    /// Every soldier of each squad at battle time `t`, between the frames
    /// around it; empty for a battle fought on the squad model.
    pub fn soldiers_at(&self, t: f32) -> Vec<Vec<FrameSoldier>> {
        let Some((a, b, u)) = self.around(t) else {
            return Vec::new();
        };
        a.soldiers
            .iter()
            .zip(&b.soldiers)
            .map(|(ma, mb)| {
                ma.iter()
                    .zip(mb)
                    .map(|(&p, &q)| FrameSoldier::between(p, q, u))
                    .collect()
            })
            .collect()
    }
}

/// A battle being fought live, as the game shows it and the player commands it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiveView {
    pub town: u16,
    pub at: (f32, f32),
    pub facing: (f32, f32),
    pub field: (f32, f32),
    pub squads: Vec<ReplaySquad>,
    /// Each squad's orders left, the first in force (empty: its own judgment).
    pub orders: Vec<Vec<Order>>,
    /// Where each squad's front stands.
    pub lines: Vec<SquadLine>,
    pub stances: Vec<Stance>,
    /// Each squad's marching and running pace now, metres per second.
    pub paces: Vec<(f32, f32)>,
    /// Each squad's kit, and its attack bonus in melee and with arrows at
    /// half range (its members' mean), for the matchup hints.
    pub kits: Vec<Kit>,
    pub bonus: Vec<(f32, f32)>,
    /// Each soldier's sheet in the combat rules' terms (armour class, attack
    /// bonus, hit points, weapon dice), by squad in member order.
    pub sheets: Vec<Vec<super::rules::Sheet>>,
    pub wall_x: Option<f32>,
    /// The town's wall on the field, if it has one (its hit points now are
    /// `frame.walls`).
    #[serde(default)]
    pub walls: Vec<ReplayWall>,
    /// The field's obstacles, for the ghost line to find its way round
    /// (`None`: an open field). Not sent: a view that needs it shares the
    /// battle's.
    #[serde(skip)]
    pub nav: Option<std::sync::Arc<Nav>>,
    pub morale: Vec<f32>,
    /// Each soldier's morale and fatigue, by squad in member order, for the
    /// overlay (empty on the squad model, which has neither per soldier).
    pub condition: Vec<Vec<Condition>>,
    /// Now, with every soldier on the soldier model (`frame.soldiers`).
    pub frame: ReplayFrame,
    /// The side the player fights on (0 attacks, 1 defends).
    pub player_side: u8,
    /// Before the first step: the player stands their squads in their half
    /// (`deploy_zone`, `Command::Deploy`) and starts the battle when ready.
    pub deploying: bool,
    pub finished: bool,
    pub winner: Option<u8>,
    /// Each squad's members as people, in member order (`price.rs`; set by
    /// the war system, empty for a line-up).
    #[serde(default)]
    pub people: Vec<Vec<Fighter>>,
    /// The raiding town, if the attackers have one.
    #[serde(default)]
    pub attacker: Option<u16>,
    /// By squad: while the battle goes on, each member out of the fight
    /// (down or taken: who of the down will die is rolled at the end);
    /// once it is over, each lost to their town (`Battle::lost`).
    #[serde(default)]
    pub lost: Vec<Vec<bool>>,
}

/// How a soldier is, as the clients draw them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FighterState {
    /// In the line: formed, fighting or wavering.
    Standing,
    Routing,
    /// Lying where they fell.
    Down,
    /// Left the field.
    Fled,
}

/// Battles fought, for the view (the newest few; `total` counts all of them so
/// a reader can tell which it has seen).
#[derive(bevy_ecs::prelude::Resource, Clone, Debug, Default)]
pub struct BattleLog {
    pub replays: std::collections::VecDeque<Replay>,
    pub total: usize,
    /// What every battle fought cost each side, oldest first.
    pub costs: Vec<Cost>,
}

/// What one battle cost: where and when it was fought, who won, and each
/// side's losses (`price.rs`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    /// `SimClock::tick` (simulated days) it was fought on.
    pub day: u64,
    pub attacker: u16,
    /// The town fought over.
    pub town: u16,
    pub winner: Option<u8>,
    /// How many fought on each side.
    pub fought: [u32; 2],
    pub losses: [Losses; 2],
}

impl BattleLog {
    /// Battles kept for a view that falls behind.
    pub const KEEP: usize = 16;

    pub fn push(&mut self, replay: Replay) {
        self.replays.push_back(replay);
        self.total += 1;
        while self.replays.len() > Self::KEEP {
            self.replays.pop_front();
        }
    }

    /// Replays after the first `seen` battles ever fought.
    pub fn since(&self, seen: usize) -> Vec<Replay> {
        let first = self.total - self.replays.len();
        self.replays
            .iter()
            .skip(seen.saturating_sub(first))
            .cloned()
            .collect()
    }
}
