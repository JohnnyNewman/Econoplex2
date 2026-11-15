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
}

/// A fought battle, for the 3D view to play back where it happened.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
}

/// A battle being fought live, as the game shows it and the player commands it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiveView {
    pub town: u16,
    pub at: (f32, f32),
    pub facing: (f32, f32),
    pub field: (f32, f32),
    pub squads: Vec<ReplaySquad>,
    pub orders: Vec<Order>,
    pub morale: Vec<f32>,
    pub frame: ReplayFrame,
    /// The side the player fights on (0 attacks, 1 defends).
    pub player_side: u8,
    pub finished: bool,
    pub winner: Option<u8>,
}

/// Battles fought, for the view (the newest few; `total` counts all of them so
/// a reader can tell which it has seen).
#[derive(bevy_ecs::prelude::Resource, Clone, Debug, Default)]
pub struct BattleLog {
    pub replays: std::collections::VecDeque<Replay>,
    pub total: usize,
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
