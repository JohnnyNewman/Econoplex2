//! Battle control: what the player can tell a squad and ask of a battle,
//! shaped for RTS mouse control in the clients (design note, "Battle
//! controls").
//!
//! - **Orders** ([`Order`]): a list per squad, carried out in turn. Right-click
//!   ground is a `MoveTo` (double-click: `run`), right-drag sets its facing
//!   and width, right-click an enemy is `Attack` (`Shoot` for archers),
//!   shift-right-click queues. [`LiveView::group_line`] and
//!   [`LiveView::group_shift`] turn one drag or click into moves for every
//!   selected squad.
//! - **Stances** ([`Stance`]): how a squad fights whatever its orders: fire
//!   at will, hold fire or skirmish; its own, tight or loose order; and what
//!   it does to enemies who run.
//! - **Doctrine** ([`Doctrine`]): how a side's squads on their own judgment
//!   go about it, and what they do to enemies who run.
//! - **Deployment**: before the first step, a squad can be put anywhere in
//!   its side's zone ([`deploy_zone`], `Battle::deploy`).
//! - **Ghost line** ([`LiveView::preview`]): the slots and arrival time of a
//!   proposed list of orders, for the clients to draw while dragging.
//! - **Matchups** ([`Kit::against`]): what squad A's weapons do to squad B,
//!   per damage type, after armour and shields.
//! - **Overlay**: each soldier's morale and fatigue in the live view
//!   ([`LiveView::condition`]).

use super::*;

/// Where a squad's front stands, as the clients draw it and an RTS control
/// drags it: the centre of the front rank (metres on the field), the way it
/// faces (unit), how wide it is and the metres between its files. On the
/// squad model, which has no front, the squad's centre, facing the enemy's
/// edge of the field, as wide as a square of its fighters.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SquadLine {
    pub anchor: (f32, f32),
    pub facing: (f32, f32),
    pub width: f32,
    pub spacing: f32,
}

/// When a squad's bows shoot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fire {
    /// At the ordered squad, else the nearest in range.
    #[default]
    AtWill,
    /// Not at all (to keep the arrows, or not to give the squad away).
    Hold,
    /// At will, and back away from enemies who come within
    /// `battle.skirmish_distance`, so they shoot rather than fight.
    Skirmish,
}

/// How far apart a squad's soldiers stand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Spacing {
    /// As its kit has it: archers loose, the rest tight.
    #[default]
    Own,
    /// Shoulder to shoulder: stands up to a charge.
    Tight,
    /// At `soldiers.loose_spacing`: fewer are hit where an arrow lands.
    Loose,
}

/// What a squad does to enemies who run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pursuit {
    /// Chase them and cut them down.
    #[default]
    Kill,
    /// Take them prisoner: chase them, and lay hands on those in reach
    /// instead of striking them (`Member::captured`). After a battle
    /// the winners' prisoners go to the winning town (military
    /// `take_prisoners`).
    Capture,
    /// Let them go: strike nobody who runs.
    LetGo,
}

/// How a squad fights, whatever its orders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stance {
    pub fire: Fire,
    pub spacing: Spacing,
    pub pursuit: Pursuit,
}

/// How a side's squads go about it on their own judgment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Posture {
    /// Hold their ground until an enemy comes within
    /// `battle.cautious_wait`, then fight as balanced.
    Cautious,
    /// As the battle AI always has.
    #[default]
    Balanced,
    /// Run in from twice as far as balanced squads do.
    Aggressive,
}

/// A side's way of fighting: its squads' posture on their own judgment, and
/// what they do to enemies who run. A town sets it in governance
/// (`Policy::doctrine`); every battle it fights starts from it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Doctrine {
    pub posture: Posture,
    pub prisoners: Pursuit,
}

impl Doctrine {
    /// The posture after this one (cautious, balanced, aggressive, round),
    /// for a governance button.
    pub fn next_posture(self) -> Self {
        let posture = match self.posture {
            Posture::Cautious => Posture::Balanced,
            Posture::Balanced => Posture::Aggressive,
            Posture::Aggressive => Posture::Cautious,
        };
        Self { posture, ..self }
    }

    /// The prisoner policy after this one (kill, capture, let go, round).
    pub fn next_prisoners(self) -> Self {
        let prisoners = match self.prisoners {
            Pursuit::Kill => Pursuit::Capture,
            Pursuit::Capture => Pursuit::LetGo,
            Pursuit::LetGo => Pursuit::Kill,
        };
        Self { prisoners, ..self }
    }
}

/// A doctrine in words, e.g. "doctrine cautious, take prisoners".
pub fn doctrine_words(d: Doctrine) -> String {
    let posture = match d.posture {
        Posture::Cautious => "cautious",
        Posture::Balanced => "balanced",
        Posture::Aggressive => "aggressive",
    };
    let prisoners = match d.prisoners {
        Pursuit::Kill => "kill runners",
        Pursuit::Capture => "take prisoners",
        Pursuit::LetGo => "let runners go",
    };
    format!("doctrine {posture}, {prisoners}")
}

/// What the player tells one squad of the live battle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// These orders, in turn, replacing what it was doing.
    Orders(Vec<Order>),
    Stance(Stance),
    /// Before the first step: stand the squad here (`Battle::deploy`).
    Deploy {
        anchor: (f32, f32),
        facing: (f32, f32),
        width: Option<f32>,
    },
}

/// Where a list of orders would put a squad: the slot of every soldier in
/// line at the last move (metres on the field), the seconds to get there,
/// and the way its front goes, from where it stands now (round the
/// obstacles, if the field has any).
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub slots: Vec<(f32, f32)>,
    pub eta: f32,
    pub path: Vec<(f32, f32)>,
}

/// How a soldier is, for the overlay: morale and fatigue, each 0 to 1 in
/// 256 steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    morale: u8,
    fatigue: u8,
}

impl Condition {
    pub fn new(morale: f32, fatigue: f32) -> Self {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Self {
            morale: q(morale),
            fatigue: q(fatigue),
        }
    }
    /// 1 steady, 0 broken.
    pub fn morale(&self) -> f32 {
        self.morale as f32 / 255.0
    }
    /// 0 rested, 1 spent.
    pub fn fatigue(&self) -> f32 {
        self.fatigue as f32 / 255.0
    }
}

/// What one squad's weapons do to another's soldiers, per damage type
/// (pierce, slash, blunt): health per blow and per arrow after armour (and
/// for arrows the shield), and the same per second of one fighter.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Matchup {
    pub blow: PerType,
    pub arrow: PerType,
    pub melee_per_second: f32,
    pub shots_per_second: f32,
}

impl Kit {
    /// What this kit does to soldiers wearing `defender`'s armour, for the
    /// matchup hints. A charge or a flank blow does more; this is a blow
    /// from the front, as the model strikes it.
    pub fn against(&self, defender: &Kit) -> Matchup {
        let a = &defender.armour;
        let blow: PerType = std::array::from_fn(|t| self.damage[t] * (1.0 - a[t]));
        let arrow: PerType =
            std::array::from_fn(|t| self.shot_damage[t] * (1.0 - a[t]) * (1.0 - defender.shield));
        Matchup {
            blow,
            arrow,
            melee_per_second: blow.iter().sum::<f32>() * self.strike_rate,
            shots_per_second: arrow.iter().sum::<f32>() * self.shot_rate,
        }
    }
}

/// Where side `side` may deploy on `field`: x from, x to, y from, y to
/// (metres). Its own half; defenders behind a wall stay inside it.
pub fn deploy_zone(field: &Field, side: u8) -> (f32, f32, f32, f32) {
    let half = 0.5 * field.width;
    let (x0, x1) = match (side, field.wall_x) {
        (0, Some(w)) => (0.0, half.min(w - 1.0)),
        (0, None) => (0.0, half),
        (_, Some(w)) => ((w + 1.0).max(half), field.width),
        (_, None) => (half, field.width),
    };
    (x0, x1, 0.0, field.height)
}

/// Metres of clear ground a squad going round an enemy keeps from its side.
const FLANK_CLEAR: f32 = 20.0;

/// A unit vector, or `or` for one too short to turn.
fn unit((x, y): (f32, f32), or: (f32, f32)) -> (f32, f32) {
    let l = x.hypot(y);
    if l > 1e-6 {
        (x / l, y / l)
    } else {
        or
    }
}

/// Turned a quarter to the right, as the soldier model turns a facing.
fn right((x, y): (f32, f32)) -> (f32, f32) {
    (y, -x)
}

impl LiveView {
    /// Fronts the battle panels offer, by name and files (`reform`).
    pub const FRONTS: [(&'static str, u16); 4] =
        [("Column", 4), ("Deep", 10), ("Line", 20), ("Wide", 40)];

    /// Whether it is fought on the soldier model, where a squad has a front
    /// to set and sides to go round.
    pub fn on_soldiers(&self) -> bool {
        !self.frame.soldiers.is_empty()
    }

    /// The order in force for squad `q`.
    pub fn order_now(&self, q: usize) -> Order {
        self.orders.get(q).map_or(Order::Auto, |o| order_now(o))
    }

    /// Orders that send squad `q` round enemy squad `j`'s side and into it:
    /// out to a lane `FLANK_CLEAR` beyond `j`'s end on the side nearer `q`,
    /// halfway along at a slant if it is not out there yet, then beside `j`
    /// a little behind its front, facing in, then `Attack(j)`. With mouse
    /// control, the same is shift-queued moves and an attack.
    pub fn flank(&self, q: usize, j: usize) -> Vec<Order> {
        let (Some(&(sx, sy, ..)), Some(&(jx, jy, ..)), Some(own), Some(line)) = (
            self.frame.squads.get(q),
            self.frame.squads.get(j),
            self.lines.get(q),
            self.lines.get(j),
        ) else {
            return vec![Order::Attack(j)];
        };
        let (fx, fy) = line.facing;
        let (rx, ry) = right(line.facing);
        let (dx, dy) = (sx - jx, sy - jy);
        let side = if dx * rx + dy * ry < 0.0 { -1.0 } else { 1.0 };
        let (along, out) = (dx * fx + dy * fy, (dx * rx + dy * ry) * side);
        let wide = 0.5 * line.width + 0.5 * own.width + FLANK_CLEAR;
        let lane = |ahead: f32| {
            (
                jx + rx * side * wide + fx * ahead,
                jy + ry * side * wide + fy * ahead,
            )
        };
        let mut plan = Vec::new();
        if out < wide - 5.0 && along > 0.0 {
            plan.push(Order::move_to(lane(0.5 * along)));
        }
        plan.push(Order::MoveTo {
            anchor: lane(-10.0),
            facing: Some((-rx * side, -ry * side)),
            width: None,
            run: false,
        });
        plan.push(Order::Attack(j));
        plan
    }

    /// Orders that re-form squad `q` where it stands, `files` wide, then
    /// carry on with what it was doing: what mouse control does when the
    /// player drags a squad's line wider or narrower in place.
    pub fn reform(&self, q: usize, files: u16) -> Vec<Order> {
        let Some(line) = self.lines.get(q) else {
            return Vec::new();
        };
        let mut plan = vec![Order::MoveTo {
            anchor: line.anchor,
            facing: Some(line.facing),
            width: Some(files as f32 * line.spacing),
            run: false,
        }];
        // Then on, in the new width, with what it was doing (its own
        // judgment if nothing).
        plan.extend(self.orders.get(q).into_iter().flatten().copied());
        if plan.len() == 1 {
            plan.push(Order::Auto);
        }
        plan
    }

    /// A drag from `from` to `to` (metres on the field) over squads
    /// `squads`: one line along the drag, facing to its left (as seen from
    /// above with the field's +y up, so a drag to the right faces up), each
    /// squad a share of it by its front, in the order they stand along it now,
    /// with a pace between. A drag shorter than the squads' fronts together
    /// keeps their widths and starts at `from`.
    pub fn group_line(
        &self,
        squads: &[usize],
        from: (f32, f32),
        to: (f32, f32),
        run: bool,
    ) -> Vec<(usize, Order)> {
        let along = unit((to.0 - from.0, to.1 - from.1), (0.0, 1.0));
        let facing = (-along.1, along.0);
        let length = (to.0 - from.0).hypot(to.1 - from.1);
        let mut qs: Vec<(usize, SquadLine)> = squads
            .iter()
            .filter_map(|&q| self.lines.get(q).map(|l| (q, *l)))
            .collect();
        if qs.is_empty() {
            return Vec::new();
        }
        let at = |l: &SquadLine| l.anchor.0 * along.0 + l.anchor.1 * along.1;
        qs.sort_by(|a, b| at(&a.1).total_cmp(&at(&b.1)).then(a.0.cmp(&b.0)));
        let gap = qs.iter().map(|q| q.1.spacing).fold(0.0, f32::max);
        let fronts: f32 = qs.iter().map(|q| q.1.width).sum();
        let room = length - gap * (qs.len() - 1) as f32;
        let scale = if room > fronts { room / fronts } else { 1.0 };
        let mut x = 0.0;
        qs.iter()
            .map(|&(q, l)| {
                let w = l.width * scale;
                let mid = x + 0.5 * w;
                x += w + gap;
                (
                    q,
                    Order::MoveTo {
                        anchor: (from.0 + along.0 * mid, from.1 + along.1 * mid),
                        facing: Some(facing),
                        width: Some(w),
                        run,
                    },
                )
            })
            .collect()
    }

    /// A click at `to` for squads `squads`: each moves as the group's centre
    /// moves to `to`, keeping where it stands in the group, its facing and
    /// its width.
    pub fn group_shift(&self, squads: &[usize], to: (f32, f32), run: bool) -> Vec<(usize, Order)> {
        let lines: Vec<(usize, SquadLine)> = squads
            .iter()
            .filter_map(|&q| self.lines.get(q).map(|l| (q, *l)))
            .collect();
        let n = lines.len().max(1) as f32;
        let c = lines.iter().fold((0.0, 0.0), |c, (_, l)| {
            (c.0 + l.anchor.0 / n, c.1 + l.anchor.1 / n)
        });
        lines
            .into_iter()
            .map(|(q, l)| {
                let anchor = (to.0 + l.anchor.0 - c.0, to.1 + l.anchor.1 - c.1);
                (
                    q,
                    Order::MoveTo {
                        anchor,
                        facing: Some(l.facing),
                        width: Some(l.width),
                        run,
                    },
                )
            })
            .collect()
    }

    /// The ghost line of `orders` for squad `q`: the slots of its soldiers in
    /// line where the last move leaves them, and the seconds to walk (or run)
    /// the moves from its front now, at the squad's pace now (`paces`).
    /// Among obstacles, each move goes where the battle would send it (off
    /// a building, to the nearest free spot), by the way round the
    /// obstacles a squad would take, and no slot stands in a building.
    /// `None` if the orders have no move.
    pub fn preview(&self, q: usize, orders: &[Order]) -> Option<Preview> {
        let line = *self.lines.get(q)?;
        let (walk, run) = *self.paces.get(q)?;
        let standing = self.frame.squads.get(q).map_or(0, |f| f.2 as usize);
        let side = self.squads.get(q).map_or(0, |s| s.side);
        let nav = self.nav.as_deref();
        let mut at = line.anchor;
        let mut path = vec![at];
        let (mut facing, mut width, mut eta, mut moved) = (line.facing, line.width, 0.0, false);
        for o in orders {
            let o = match nav {
                Some(nav) => o.clear_of(nav, side),
                None => *o,
            };
            let Order::MoveTo {
                anchor,
                facing: f,
                width: w,
                run: r,
            } = o
            else {
                continue;
            };
            let from = soldiers::V2::new(at.0, at.1);
            let to = soldiers::V2::new(anchor.0, anchor.1);
            let way = nav
                .and_then(|n| n.path(from, to, side))
                .unwrap_or_else(|| vec![to]);
            let d = Nav::length(from, &way);
            let last = if way.len() > 1 { way[way.len() - 2] } else { from };
            path.extend(way.iter().map(|p| (p.x, p.y)));
            eta += d / if r { run } else { walk }.max(0.1);
            facing = f.unwrap_or(unit((to.x - last.x, to.y - last.y), facing));
            width = w.unwrap_or(width);
            at = anchor;
            moved = true;
        }
        if !moved {
            return None;
        }
        let files = ((width / line.spacing).round() as usize).clamp(1, standing.max(1));
        let (rx, ry) = right(facing);
        let body = if self.kits.get(q).is_some_and(|k| k.mounted) {
            0.9
        } else {
            0.4
        };
        let slots = (0..standing)
            .map(|k| {
                let (rank, file) = ((k / files) as f32, (k % files) as f32);
                let lat = (file - (files as f32 - 1.0) * 0.5) * line.spacing;
                let back = rank * line.spacing;
                let p = (
                    at.0 + rx * lat - facing.0 * back,
                    at.1 + ry * lat - facing.1 * back,
                );
                match nav {
                    Some(n) => {
                        let c = n.push_out(soldiers::V2::new(p.0, p.1), body, side);
                        (c.x, c.y)
                    }
                    None => p,
                }
            })
            .collect();
        Some(Preview { slots, eta, path })
    }

    /// Where side `side` may deploy (`deploy_zone`).
    pub fn deploy_zone(&self, side: u8) -> (f32, f32, f32, f32) {
        deploy_zone(
            &Field {
                width: self.field.0,
                height: self.field.1,
                wall_x: self.wall_x,
                obstacles: Vec::new(),
            },
            side,
        )
    }

    /// What squad `a`'s weapons do to squad `b`'s soldiers (`Kit::against`).
    pub fn matchup(&self, a: usize, b: usize) -> Option<Matchup> {
        Some(self.kits.get(a)?.against(self.kits.get(b)?))
    }
}
