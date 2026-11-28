//! A town's wall on the field: its stretches and gates are thin obstacles
//! (`Shape::Segment`), each with hit points (`Field::walls`). Both models
//! use what is here:
//!
//! - **Attackers** facing a closed ring pick a stretch or a gate
//!   ([`Ring::assault`]) and batter it from outside: the members at it who
//!   strike nobody strike the wall instead, by their weapon's damage times
//!   `BattleParams::wall_taken` (blunt best). At no hit points it is a
//!   breach (`Passage::Open`), and every way is planned again.
//! - **Defenders** stand inside it ([`Field::defenders_spot`]). While it is
//!   closed they man it where the enemy comes ([`Ring::man`]); once it has a
//!   breach, the nearest go to plug it. They strike down over a stretch at
//!   a man below (the higher ground, `wall_ground`), and he cannot strike
//!   back or hold up his shield; arrows at them from outside meet its cover
//!   (`wall_cover`).
//!
//! A gap (a stretch the town has not built yet) is a breach from the start.

use super::*;

/// A stretch of wall or a gate on the field: its obstacle's `id`, hit
/// points left and at most (`hp` 0: a breach, or not built), and whether
/// it is a gate.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WallHp {
    pub id: u64,
    pub hp: f32,
    pub max: f32,
    pub gate: bool,
}

impl WallHp {
    /// Who passes it as it stands: nobody, the defenders (a gate they
    /// hold), or everybody (a breach or a gap).
    pub fn passage(&self) -> Passage {
        if self.hp <= 0.0 {
            Passage::Open
        } else if self.gate {
            Passage::Gate(1)
        } else {
            Passage::Blocked
        }
    }
}

/// Metres inside a wall the defenders' front stands when they deploy.
const INSIDE: f32 = 5.0;

impl Field {
    /// Whether the field has a town wall on it.
    pub fn walled(&self) -> bool {
        !self.walls.is_empty()
    }

    /// Where a defending squad meant for height `y` forms up: in a walled
    /// town, its front `INSIDE` metres behind the wall at that height (or
    /// at the nearest height the wall spans); otherwise at
    /// `defenders_x`.
    pub fn defenders_spot(&self, y: f32) -> (f32, f32) {
        let Some(ring) = Ring::of(self) else {
            return (self.defenders_x(), y);
        };
        let r = ring.radius;
        let y = y.clamp(ring.centre.y - 0.7 * r, ring.centre.y + 0.7 * r);
        let x = ring
            .pieces
            .iter()
            .filter_map(|p| {
                let (a, b) = (p.a, p.b);
                if (a.y - y) * (b.y - y) > 0.0 || (a.y - b.y).abs() < 1e-6 {
                    return None;
                }
                Some(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x))
            })
            .fold(f32::INFINITY, f32::min);
        if x.is_finite() {
            (x + INSIDE, y)
        } else {
            (self.defenders_x(), y)
        }
    }

    /// Whether the wall is whole: walls on the field, none breached or
    /// missing.
    pub fn wall_closed(&self) -> bool {
        self.walled() && self.walls.iter().all(|w| w.hp > 0.0)
    }

    /// `amount` hit points off wall piece `k`; the obstacles as they stand
    /// now if that breached it.
    pub(crate) fn batter(&mut self, k: usize, amount: f32) -> bool {
        let w = &mut self.walls[k];
        if w.hp <= 0.0 || amount <= 0.0 {
            return false;
        }
        w.hp -= amount;
        if w.hp > 0.0 {
            return false;
        }
        w.hp = 0.0;
        let id = w.id;
        for o in self.obstacles.iter_mut().filter(|o| o.id == id) {
            o.passage = Passage::Open;
        }
        true
    }
}

/// One wall piece as the squads see it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Piece {
    /// Index in `Field::walls`.
    pub k: usize,
    pub a: V2,
    pub b: V2,
    pub mid: V2,
    /// Unit, from the ring's centre out through this piece.
    pub out: V2,
    pub half_width: f32,
}

impl Piece {
    /// Distance from `p` to the piece's line.
    pub fn dist(&self, p: V2) -> f32 {
        let d = self.b - self.a;
        let t = ((p - self.a).dot(d) / d.len2().max(1e-6)).clamp(0.0, 1.0);
        (self.a + d * t - p).len()
    }

    /// The point `off` metres out from (`off` < 0: in from) its middle.
    pub fn beside(&self, off: f32) -> V2 {
        self.mid + self.out * (off.signum() * self.half_width + off)
    }

    pub fn len(&self) -> f32 {
        (self.b - self.a).len()
    }

    /// Whether the way from `p` to `q` crosses the piece's line.
    pub fn crosses(&self, p: V2, q: V2) -> bool {
        let side = |o: V2, u: V2, v: V2| (u.x - o.x) * (v.y - o.y) - (u.y - o.y) * (v.x - o.x);
        side(self.a, self.b, p) * side(self.a, self.b, q) < 0.0
            && side(p, q, self.a) * side(p, q, self.b) < 0.0
    }
}

/// A field's wall: every piece of it, its centre and its mean radius.
#[derive(Clone, Debug)]
pub(crate) struct Ring {
    pub pieces: Vec<Piece>,
    pub centre: V2,
    pub radius: f32,
    /// For [`Ring::inside`]: no piece comes nearer the centre than `near`,
    /// none reaches further than `far`, and whether the centre is inside.
    near: f32,
    far: f32,
    centre_inside: bool,
}

impl Ring {
    pub fn of(field: &Field) -> Option<Self> {
        if field.walls.is_empty() {
            return None;
        }
        let mut pieces = Vec::with_capacity(field.walls.len());
        for (k, w) in field.walls.iter().enumerate() {
            let Some(o) = field.obstacles.iter().find(|o| o.id == w.id) else {
                continue;
            };
            let Shape::Segment { a, b, half_width } = o.shape else {
                continue;
            };
            let (a, b) = (V2::new(a.0, a.1), V2::new(b.0, b.1));
            pieces.push(Piece {
                k,
                a,
                b,
                mid: (a + b) * 0.5,
                out: V2::ZERO,
                half_width,
            });
        }
        if pieces.is_empty() {
            return None;
        }
        let n = pieces.len() as f32;
        let centre = pieces.iter().fold(V2::ZERO, |s, p| s + p.mid) * (1.0 / n);
        let radius = pieces.iter().map(|p| (p.mid - centre).len()).sum::<f32>() / n;
        for p in &mut pieces {
            p.out = (p.mid - centre).norm_or(V2::new(-1.0, 0.0));
        }
        let near = pieces
            .iter()
            .map(|p| p.dist(centre))
            .fold(f32::INFINITY, f32::min);
        let far = pieces
            .iter()
            .map(|p| (p.a - centre).len().max((p.b - centre).len()))
            .fold(0.0, f32::max);
        let mut ring = Self {
            pieces,
            centre,
            radius,
            near,
            far,
            centre_inside: false,
        };
        ring.centre_inside = ring.crossings(centre);
        Some(ring)
    }

    /// Whether `p` stands inside the wall's line (between its pieces' ends
    /// and the centre). Nearer the centre than any piece, `p` is where the
    /// centre is; further out than every piece's ends, outside; only
    /// between the two are the pieces counted.
    pub fn inside(&self, p: V2) -> bool {
        let d = (p - self.centre).len();
        if d < self.near {
            self.centre_inside
        } else if d > self.far {
            false
        } else {
            self.crossings(p)
        }
    }

    /// Whether a ray from `p` crosses the pieces an odd number of times.
    fn crossings(&self, p: V2) -> bool {
        let mut inside = false;
        for q in &self.pieces {
            let (a, b) = (q.a, q.b);
            if (a.y > p.y) != (b.y > p.y) {
                let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
                if p.x < x {
                    inside = !inside;
                }
            }
        }
        inside
    }

    /// The piece an attacker at `at` assaults: the nearest standing one,
    /// a gate counted `GATE_PULL` metres nearer (the weaker point), never
    /// one off the field.
    pub fn assault(&self, field: &Field, at: V2) -> Option<usize> {
        self.pieces
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                field.walls[p.k].hp > 0.0
                    && p.mid.x > 0.0
                    && p.mid.y > 0.0
                    && p.mid.x < field.width
                    && p.mid.y < field.height
            })
            .map(|(n, p)| {
                let pull = if field.walls[p.k].gate {
                    GATE_PULL
                } else {
                    0.0
                };
                (n, (p.mid - at).len() - pull)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
            .map(|(n, _)| n)
    }

    /// The piece nearest `p`, standing (`open` false) or breached.
    pub fn nearest(&self, field: &Field, p: V2, open: bool) -> Option<usize> {
        self.pieces
            .iter()
            .enumerate()
            .filter(|(_, q)| (field.walls[q.k].hp <= 0.0) == open)
            .map(|(n, q)| (n, q.dist(p)))
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
            .map(|(n, _)| n)
    }

    /// Where a defending squad meets an enemy at `enemy`: just inside the
    /// piece nearest him.
    pub fn man(&self, enemy: V2) -> V2 {
        self.man_at(enemy, MAN_OFF)
    }

    /// The point `off` metres inside the piece nearest `enemy`.
    pub fn man_at(&self, enemy: V2, off: f32) -> V2 {
        let p = self
            .pieces
            .iter()
            .min_by(|a, b| a.dist(enemy).total_cmp(&b.dist(enemy)))
            .expect("a ring has pieces");
        p.beside(-off)
    }
}

/// Metres nearer a gate counts for an attacker choosing where to batter.
const GATE_PULL: f32 = 30.0;
/// Metres from a wall's face a squad's front stands to batter it, or to man
/// it from inside.
pub(crate) const ASSAULT_OFF: f32 = 0.8;
pub(crate) const MAN_OFF: f32 = 0.8;
/// Metres past his reach a defender on the wall strikes down at a man below.
pub(crate) const OVER_WALL: f32 = 1.5;
/// Metres behind a town's wall that a defender has its cover against
/// arrows from below: no further than his last metres toward the archer.
pub(crate) const COVER: f32 = 4.0;
/// Defending squads that go to plug each breach: the nearest ones.
pub(crate) const PLUG: usize = 2;
/// Share of their range from the defenders at which archers attacking a
/// town's wall stand and shoot (all of them reach the men on it).
pub(crate) const NEAR: f32 = 0.6;
/// Metres outside the wall riders wait while it stands.
pub(crate) const RIDERS_OFF: f32 = 30.0;
