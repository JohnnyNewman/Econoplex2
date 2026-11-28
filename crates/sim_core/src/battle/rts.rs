//! RTS mouse and key controls over a live battle, shared by both clients so
//! they behave the same (design note battle-per-soldier.md, "Battle
//! controls"). The clients feed in the pointer (on screen, and where it
//! falls on the field) and keys; this turns them into a selection and into
//! [`Command`]s for `command_live_battle`.
//!
//! - **Left button:** click a squad of yours to select it (shift adds or
//!   removes it); drag a box to select every squad of yours inside it
//!   (shift adds them); click empty ground to select nothing.
//! - **Right button**, with squads selected: click an enemy squad to attack
//!   it (archers shoot it); click ground to move there (several squads keep
//!   their places in the group, their facing and width); drag to lay out
//!   the line from where the press was to where it is let go, facing to the
//!   left of the drag (`LiveView::group_line`), its ghost and arrival time
//!   shown while dragging. A second click within [`DOUBLE_CLICK`] seconds
//!   runs (so does a drag started by one). With shift held as the button
//!   comes up, the move or attack is queued after the squad's moves
//!   ([`queued`]) instead of replacing its orders; Backspace drops the
//!   queue ([`clear_queue`]).
//! - **Deployment:** before a live battle's first step (`LiveView::deploying`)
//!   the same clicks and drags stand the squads where they are to start,
//!   inside the player's half ([`deployed`]), until the player starts it.
//! - **Keys:** Ctrl+1..9 make the selection a group, 1..9 select it again,
//!   twice quickly to look at it; stance keys set the selected squads'
//!   stance ([`StanceKey`]).

use super::*;

/// Seconds between two right-clicks for the second to make the squads run.
pub const DOUBLE_CLICK: f64 = 0.35;
/// Seconds between two taps of a group key for the second to look at it.
pub const DOUBLE_TAP: f64 = 0.4;
/// Screen pixels the pointer must move with a button down to drag.
pub const DRAG_PX: f32 = 8.0;
/// Metres from a soldier within which a click picks their squad.
pub const PICK_REACH: f32 = 2.5;

/// Whether squad `q` takes the player's commands: theirs, standing and
/// fighting, in a battle not over.
pub fn selectable(v: &LiveView, q: usize) -> bool {
    !v.finished
        && v.squads.get(q).is_some_and(|s| s.side == v.player_side)
        && v.frame
            .squads
            .get(q)
            .is_some_and(|f| f.2 > 0 && f.3 == SquadState::Fighting)
}

/// Whether squad `q` is still on the field (fighting or running).
fn on_field(v: &LiveView, q: usize) -> bool {
    v.frame
        .squads
        .get(q)
        .is_some_and(|f| f.2 > 0 && matches!(f.3, SquadState::Fighting | SquadState::Routing))
}

/// The centre of squads `qs` on the field, if any is on it.
pub fn centre(v: &LiveView, qs: &[usize]) -> Option<(f32, f32)> {
    let at: Vec<(f32, f32)> = qs
        .iter()
        .filter(|&&q| on_field(v, q))
        .map(|&q| (v.frame.squads[q].0, v.frame.squads[q].1))
        .collect();
    let n = at.len() as f32;
    (n > 0.0).then(|| {
        (
            at.iter().map(|p| p.0).sum::<f32>() / n,
            at.iter().map(|p| p.1).sum::<f32>() / n,
        )
    })
}

/// The squad at field point `p` (metres): on the soldier model, the one
/// whose soldier on their feet stands nearest, within [`PICK_REACH`]; on the
/// squad model, the nearest whose disc of fighters takes in `p`.
pub fn squad_at(v: &LiveView, p: (f32, f32)) -> Option<usize> {
    let d = |x: f32, y: f32| (x - p.0).hypot(y - p.1);
    let mut best: Option<(usize, f32)> = None;
    if v.on_soldiers() {
        for (q, men) in v.frame.soldiers.iter().enumerate() {
            for m in men {
                if !matches!(m.state, FighterState::Standing | FighterState::Routing) {
                    continue;
                }
                let (x, y) = m.pos();
                let r = d(x, y);
                if r <= PICK_REACH && best.is_none_or(|b| r < b.1) {
                    best = Some((q, r));
                }
            }
        }
    } else {
        for (q, f) in v.frame.squads.iter().enumerate() {
            if !on_field(v, q) {
                continue;
            }
            let spacing = v.lines.get(q).map_or(1.0, |l| l.spacing);
            let disc = (f.2 as f32).sqrt() * spacing * 0.5 + PICK_REACH;
            let r = d(f.0, f.1);
            if r <= disc && best.is_none_or(|b| r < b.1) {
                best = Some((q, r));
            }
        }
    }
    best.map(|b| b.0)
}

/// The corners of squad `q`'s block on the field, for drawing it selected:
/// its front as wide as it is, as deep as its ranks (on the squad model,
/// which has no front, a square around its centre).
pub fn outline(v: &LiveView, q: usize) -> Option<[(f32, f32); 4]> {
    let l = v.lines.get(q)?;
    let n = v.frame.squads.get(q)?.2.max(1) as f32;
    let files = (l.width / l.spacing).round().max(1.0);
    let depth = (n / files).ceil() * l.spacing;
    let (fx, fy) = l.facing;
    let (rx, ry) = (fy, -fx);
    let half = 0.5 * l.width + 0.5;
    let (front, back) = if v.on_soldiers() {
        (0.5, depth)
    } else {
        (0.5 * depth + 0.5, 0.5 * depth + 0.5)
    };
    let (ax, ay) = l.anchor;
    let p = |lat: f32, ahead: f32| (ax + rx * lat + fx * ahead, ay + ry * lat + fy * ahead);
    Some([
        p(-half, front),
        p(half, front),
        p(half, -back),
        p(-half, -back),
    ])
}

/// A drag that would lay squads `sel` out `ahead` metres toward the enemy,
/// facing them, a pace between squads: from, to (field points). The clients'
/// `--rts-demo ghost` shows its ghost, for screenshots without a mouse.
pub fn line_ahead(v: &LiveView, sel: &[usize], ahead: f32) -> Option<((f32, f32), (f32, f32))> {
    let c = centre(v, sel)?;
    let enemies: Vec<usize> = (0..v.squads.len())
        .filter(|&q| v.squads[q].side != v.player_side)
        .collect();
    let e = centre(v, &enemies)?;
    let (dx, dy) = (e.0 - c.0, e.1 - c.1);
    let l = dx.hypot(dy).max(1e-3);
    let (fx, fy) = (dx / l, dy / l);
    // The line runs to the right of the way it faces.
    let (ax, ay) = (fy, -fx);
    let len: f32 = sel
        .iter()
        .filter_map(|&q| v.lines.get(q))
        .map(|l| l.width + 1.0)
        .sum::<f32>()
        * 1.3;
    let mid = (c.0 + fx * ahead, c.1 + fy * ahead);
    Some((
        (mid.0 - ax * 0.5 * len, mid.1 - ay * 0.5 * len),
        (mid.0 + ax * 0.5 * len, mid.1 + ay * 0.5 * len),
    ))
}

/// Squads the player has selected, and the control groups.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub squads: Vec<usize>,
    pub groups: [Vec<usize>; 9],
    /// The last group recalled, and when (seconds), for a double tap.
    last_recall: Option<(usize, f64)>,
}

impl Selection {
    /// A left-click on `hit` (the squad under the pointer, if any).
    pub fn click(&mut self, v: &LiveView, hit: Option<usize>, shift: bool) {
        match (hit.filter(|&q| selectable(v, q)), shift) {
            (Some(q), true) => {
                if let Some(k) = self.squads.iter().position(|&s| s == q) {
                    self.squads.remove(k);
                } else {
                    self.squads.push(q);
                }
            }
            (Some(q), false) => self.squads = vec![q],
            (None, true) => {}
            (None, false) => self.squads.clear(),
        }
    }

    /// A box dragged over `hits`.
    pub fn boxed(&mut self, v: &LiveView, hits: impl IntoIterator<Item = usize>, shift: bool) {
        let hits: Vec<usize> = hits.into_iter().filter(|&q| selectable(v, q)).collect();
        if !shift {
            self.squads.clear();
        }
        for q in hits {
            if !self.squads.contains(&q) {
                self.squads.push(q);
            }
        }
    }

    /// Ctrl+`n + 1`: the selection becomes group `n`.
    pub fn set_group(&mut self, n: usize) {
        if let Some(g) = self.groups.get_mut(n) {
            *g = self.squads.clone();
        }
    }

    /// `n + 1`: select group `n`. True on the second tap within
    /// [`DOUBLE_TAP`] seconds, when the camera should look at it.
    pub fn recall(&mut self, v: &LiveView, n: usize, now: f64) -> bool {
        let Some(g) = self.groups.get_mut(n) else {
            return false;
        };
        g.retain(|&q| selectable(v, q));
        if g.is_empty() {
            return false;
        }
        self.squads = g.clone();
        let twice = self
            .last_recall
            .is_some_and(|(m, t)| m == n && now - t <= DOUBLE_TAP);
        self.last_recall = if twice { None } else { Some((n, now)) };
        twice
    }

    /// Forget squads that no longer take commands.
    pub fn tidy(&mut self, v: &LiveView) {
        self.squads.retain(|&q| selectable(v, q));
        for g in &mut self.groups {
            g.retain(|&q| selectable(v, q));
        }
    }
}

/// Whether squad `q` shoots what it is sent against.
fn shoots(v: &LiveView, q: usize) -> bool {
    v.kits.get(q).is_some_and(|k| k.range > 0.0 && k.arrows > 0)
}

/// A right-click at field point `p` for squads `sel`: an enemy squad there
/// is attacked (shot, by archers); ground is moved to, one squad with its
/// front's centre there, several keeping their places in the group.
pub fn right_click(v: &LiveView, sel: &[usize], p: (f32, f32), run: bool) -> Vec<(usize, Command)> {
    let sel: Vec<usize> = sel.iter().copied().filter(|&q| selectable(v, q)).collect();
    let enemy = squad_at(v, p).filter(|&e| v.squads[e].side != v.player_side && on_field(v, e));
    if let Some(e) = enemy {
        return sel
            .iter()
            .map(|&q| {
                let o = if shoots(v, q) {
                    Order::Shoot(e)
                } else {
                    Order::Attack(e)
                };
                (q, Command::Orders(vec![o]))
            })
            .collect();
    }
    let moves = match sel.as_slice() {
        [] => Vec::new(),
        [q] => vec![(
            *q,
            Order::MoveTo {
                anchor: p,
                facing: None,
                width: None,
                run,
            },
        )],
        _ => v.group_shift(&sel, p, run),
    };
    moves
        .into_iter()
        .map(|(q, o)| (q, Command::Orders(vec![o])))
        .collect()
}

/// A right-drag from `from` to `to` (field points) for squads `sel`: one
/// line along it (`LiveView::group_line`).
pub fn right_drag(
    v: &LiveView,
    sel: &[usize],
    from: (f32, f32),
    to: (f32, f32),
    run: bool,
) -> Vec<(usize, Command)> {
    let sel: Vec<usize> = sel.iter().copied().filter(|&q| selectable(v, q)).collect();
    v.group_line(&sel, from, to, run)
        .into_iter()
        .map(|(q, o)| (q, Command::Orders(vec![o])))
        .collect()
}

/// The ghost of a right-drag: each squad's slots where the line leaves
/// them, and the seconds to get there.
pub fn ghost(
    v: &LiveView,
    sel: &[usize],
    from: (f32, f32),
    to: (f32, f32),
    run: bool,
) -> Vec<(usize, Preview)> {
    right_drag(v, sel, from, to, run)
        .into_iter()
        .filter_map(|(q, c)| match c {
            Command::Orders(o) => v.preview(q, &o).map(|p| (q, p)),
            _ => None,
        })
        .collect()
}

/// Squad `q`'s orders with `new` queued after its moves. Only a move gives
/// way to the next order, so what follows the last move (its own judgment,
/// holding, an attack) is replaced.
pub fn queued(v: &LiveView, q: usize, new: &[Order]) -> Vec<Order> {
    let mut orders: Vec<Order> = v
        .orders
        .get(q)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .copied()
        .take_while(|o| matches!(o, Order::MoveTo { .. }))
        .collect();
    orders.extend_from_slice(new);
    orders
}

/// The selected squads' queues dropped: each keeps the order in force.
pub fn clear_queue(v: &LiveView, sel: &[usize]) -> Vec<(usize, Command)> {
    sel.iter()
        .filter(|&&q| selectable(v, q) && v.orders.get(q).is_some_and(|o| o.len() > 1))
        .map(|&q| (q, Command::Orders(vec![v.orders[q][0]])))
        .collect()
}

/// The way squad `q`'s orders take it: from its front's centre through each
/// move's end, to the enemy it is sent against last, if any. Fewer than two
/// points: nothing queued to draw.
pub fn queued_path(v: &LiveView, q: usize) -> Vec<(f32, f32)> {
    let (Some(orders), Some(line)) = (v.orders.get(q), v.lines.get(q)) else {
        return Vec::new();
    };
    let mut path = vec![line.anchor];
    for o in orders {
        match *o {
            Order::MoveTo { anchor, .. } => path.push(anchor),
            Order::Attack(e) | Order::Shoot(e) => {
                if let Some(f) = v.frame.squads.get(e) {
                    path.push((f.0, f.1));
                }
            }
            _ => {}
        }
    }
    if path.len() < 2 {
        path.clear();
    }
    path
}

/// Orders for squads before the battle starts become where they deploy:
/// each move's end, pulled inside the player's half, facing as the move
/// says (else as the squad faces now). Attacks and the like are dropped.
pub fn deployed(v: &LiveView, cmds: Vec<(usize, Command)>) -> Vec<(usize, Command)> {
    let (x0, x1, y0, y1) = v.deploy_zone(v.player_side);
    let margin = 2.0f32.min(0.5 * (x1 - x0)).min(0.5 * (y1 - y0));
    cmds.into_iter()
        .filter_map(|(q, c)| {
            let Command::Orders(o) = c else {
                return Some((q, c));
            };
            let Some(&Order::MoveTo {
                anchor,
                facing,
                width,
                ..
            }) = o.last()
            else {
                return None;
            };
            let facing = facing.or_else(|| v.lines.get(q).map(|l| l.facing))?;
            Some((
                q,
                Command::Deploy {
                    anchor: (
                        anchor.0.clamp(x0 + margin, x1 - margin),
                        anchor.1.clamp(y0 + margin, y1 - margin),
                    ),
                    facing,
                    width,
                },
            ))
        })
        .collect()
}

/// The ghost of a line dragged before the battle starts: each squad's slots
/// where it would deploy (no arrival time: it stands there at once).
pub fn deploy_ghost(
    v: &LiveView,
    sel: &[usize],
    from: (f32, f32),
    to: (f32, f32),
) -> Vec<(usize, Preview)> {
    deployed(v, right_drag(v, sel, from, to, false))
        .into_iter()
        .filter_map(|(q, c)| match c {
            Command::Deploy {
                anchor,
                facing,
                width,
            } => {
                let o = Order::MoveTo {
                    anchor,
                    facing: Some(facing),
                    width,
                    run: false,
                };
                v.preview(q, &[o]).map(|g| (q, Preview { eta: 0.0, ..g }))
            }
            _ => None,
        })
        .collect()
}

/// Two waypoints queued for each of squads `sel`: off to one side, then on
/// toward the enemy. The clients' `--rts-demo queue` shows their path, for
/// screenshots without a mouse.
pub fn demo_queue(v: &LiveView, sel: &[usize]) -> Vec<(usize, Command)> {
    let Some((from, to)) = line_ahead(v, sel, 40.0) else {
        return Vec::new();
    };
    let side = (0.5 * (to.0 - from.0), 0.5 * (to.1 - from.1));
    sel.iter()
        .filter_map(|&q| {
            let a = v.lines.get(q)?.anchor;
            let mid = (0.5 * (from.0 + to.0), 0.5 * (from.1 + to.1));
            let ahead = (mid.0 - centre(v, sel)?.0, mid.1 - centre(v, sel)?.1);
            let first = (a.0 + 0.5 * ahead.0 + side.0, a.1 + 0.5 * ahead.1 + side.1);
            let second = (a.0 + ahead.0, a.1 + ahead.1);
            Some((
                q,
                Command::Orders(vec![Order::move_to(first), Order::move_to(second)]),
            ))
        })
        .collect()
}

/// Where `--rts-demo inspect` points: at the enemy squad nearest the
/// player's squads, at its soldier nearest them on the soldier model.
pub fn demo_inspect(v: &LiveView) -> Option<(f32, f32)> {
    let mine: Vec<usize> = (0..v.squads.len()).filter(|&q| selectable(v, q)).collect();
    let c = centre(v, &mine)?;
    let d2 = |p: (f32, f32)| (p.0 - c.0).powi(2) + (p.1 - c.1).powi(2);
    let pos = |q: usize| (v.frame.squads[q].0, v.frame.squads[q].1);
    let e = (0..v.squads.len())
        .filter(|&q| v.squads[q].side != v.player_side && v.frame.squads[q].2 > 0)
        .min_by(|&a, &b| d2(pos(a)).total_cmp(&d2(pos(b))))?;
    let soldiers = v.frame.soldiers.get(e).into_iter().flatten();
    Some(
        soldiers
            .filter(|s| s.state == FighterState::Standing)
            .map(|s| s.pos())
            .min_by(|&a, &b| d2(a).total_cmp(&d2(b)))
            .unwrap_or(pos(e)),
    )
}

/// The commands of a right-click or drag as given: deployments before the
/// battle starts, queued after each squad's moves with `queue`, else as
/// they are.
fn as_given(v: &LiveView, cmds: Vec<(usize, Command)>, queue: bool) -> Vec<(usize, Command)> {
    if v.deploying {
        return deployed(v, cmds);
    }
    if !queue {
        return cmds;
    }
    cmds.into_iter()
        .map(|(q, c)| match c {
            Command::Orders(o) => (q, Command::Orders(queued(v, q, &o))),
            c => (q, c),
        })
        .collect()
}

/// A stance key, for the selected squads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StanceKey {
    Fire(Fire),
    Spacing(Spacing),
    Pursuit(Pursuit),
}

impl StanceKey {
    /// The stance keys and their labels, in the order the panels show them.
    pub const ALL: [(StanceKey, &'static str); 9] = [
        (StanceKey::Fire(Fire::AtWill), "Fire at will"),
        (StanceKey::Fire(Fire::Hold), "Hold fire"),
        (StanceKey::Fire(Fire::Skirmish), "Skirmish"),
        (StanceKey::Spacing(Spacing::Tight), "Tight"),
        (StanceKey::Spacing(Spacing::Loose), "Loose"),
        (StanceKey::Spacing(Spacing::Own), "Own order"),
        (StanceKey::Pursuit(Pursuit::Kill), "Kill runners"),
        (StanceKey::Pursuit(Pursuit::Capture), "Capture"),
        (StanceKey::Pursuit(Pursuit::LetGo), "Let go"),
    ];

    fn apply(self, s: Stance) -> Stance {
        match self {
            StanceKey::Fire(fire) => Stance { fire, ..s },
            StanceKey::Spacing(spacing) => Stance { spacing, ..s },
            StanceKey::Pursuit(pursuit) => Stance { pursuit, ..s },
        }
    }

    /// Whether every squad of `sel` has this stance (and there is one).
    pub fn chosen(self, v: &LiveView, sel: &[usize]) -> bool {
        !sel.is_empty()
            && sel
                .iter()
                .all(|&q| v.stances.get(q).is_some_and(|&s| self.apply(s) == s))
    }
}

/// The commands that give squads `sel` stance `key`.
pub fn stance(v: &LiveView, sel: &[usize], key: StanceKey) -> Vec<(usize, Command)> {
    sel.iter()
        .filter(|&&q| selectable(v, q))
        .map(|&q| {
            let s = v.stances.get(q).copied().unwrap_or_default();
            (q, Command::Stance(key.apply(s)))
        })
        .collect()
}

/// What an armour is called, for the hints, by its armour class and what
/// it resists.
fn armour_name(k: &Kit) -> &'static str {
    let resists = |t: usize| k.taken[t] < 0.9;
    if k.ac >= 7.5 {
        "plate"
    } else if k.ac >= 4.5 && resists(1) {
        "mail"
    } else if k.ac >= 3.5 && resists(1) {
        "scale"
    } else if k.ac >= 0.5 && resists(2) {
        "padded coats"
    } else if k.ac < 0.5 && k.shield < 0.5 {
        "bare skin"
    } else if k.ac < 0.5 {
        "shields"
    } else {
        "armour"
    }
}

/// What a squad's weapons are called, for the hints: by their damage type,
/// reach and mount.
fn weapon_name(k: &Kit) -> &'static str {
    use sim_data::DamageType::{Blunt, Pierce, Slash};
    match k.damage_type {
        Pierce if k.mounted => "lances",
        Pierce if k.reach >= 2.5 => "spears",
        Pierce => "points",
        Slash if k.reach >= 1.0 => "swords",
        Slash => "knives",
        Blunt if k.quality > 0.5 => "maces",
        Blunt => "clubs",
    }
}

/// What armour does to a damage type, for the hints: ", resistant to
/// slashing", ", vulnerable to ...", or nothing.
fn resistance_words(k: &Kit, t: sim_data::DamageType) -> String {
    let taken = k.taken[t.index()];
    if taken < 0.9 {
        format!(", resistant to {}", t.word())
    } else if taken > 1.1 {
        format!(", vulnerable to {}", t.word())
    } else {
        String::new()
    }
}

/// A squad with `mine` (attack bonus `bonus` in melee, `shot_bonus` with
/// arrows) against one with `theirs` (`their_bonus`), in words: "strong",
/// "even" or "weak", and why in the combat rules' terms, e.g. "weak: your
/// swords hit their mail on 14+ (35%), resistant to slashing".
pub fn matchup_words(
    mine: &Kit,
    (bonus, shot_bonus): (f32, f32),
    theirs: &Kit,
    (their_bonus, their_shot): (f32, f32),
) -> String {
    let m = mine.against(theirs, bonus, shot_bonus);
    let t = theirs.against(mine, their_bonus, their_shot);
    let archer = mine.range > 0.0 && mine.arrows > 0;
    let give = m.melee_per_second.max(m.shots_per_second);
    let take = t.melee_per_second.max(t.shots_per_second);
    let mut r = give / take.max(1e-6);
    // Spears braced against riders, and riders running onto them.
    let spears = |k: &Kit| !k.mounted && k.reach >= 2.5;
    let mut why = None;
    if mine.mounted && spears(theirs) {
        r = r.min(0.7);
        why = Some("their spears brace against your riders".to_string());
    } else if theirs.mounted && spears(mine) {
        r = r.max(1.4);
        why = Some("your spears brace against their riders".to_string());
    }
    let verdict = if r >= 1.4 {
        "strong"
    } else if r <= 0.7 {
        "weak"
    } else {
        "even"
    };
    let why = why.unwrap_or_else(|| {
        let (weapon, needs, hit, t) = if archer {
            ("arrows", m.shot_needs, m.shot_hit, mine.shot_type)
        } else {
            (weapon_name(mine), m.needs, m.hit, mine.damage_type)
        };
        format!(
            "your {weapon} hit their {} on {needs}+ ({:.0}%){}",
            armour_name(theirs),
            100.0 * hit,
            resistance_words(theirs, t)
        )
    });
    format!("{verdict}: {why}")
}

/// The hint for hovering enemy squad `enemy`: a line per squad of `mine`
/// (all the player's fighting squads if none is selected), at most four.
pub fn hint(v: &LiveView, mine: &[usize], enemy: usize) -> Option<String> {
    let theirs = v.kits.get(enemy)?;
    if v.squads.get(enemy)?.side == v.player_side {
        return None;
    }
    let all: Vec<usize> = (0..v.squads.len()).filter(|&q| selectable(v, q)).collect();
    let mine = if mine.is_empty() { &all } else { mine };
    let theirs_bonus = *v.bonus.get(enemy)?;
    let lines: Vec<String> = mine
        .iter()
        .filter_map(|&q| Some((q, v.kits.get(q)?, *v.bonus.get(q)?)))
        .take(4)
        .map(|(q, k, b)| {
            let words = matchup_words(k, b, theirs, theirs_bonus);
            format!("{}: {words}", v.squads[q].name)
        })
        .collect();
    (!lines.is_empty()).then(|| format!("{}\n{}", v.squads[enemy].name, lines.join("\n")))
}

/// The soldier standing nearest field point `at`, within `within` metres:
/// their squad and place in it (the soldier model; none on the squad model).
pub fn soldier_at(v: &LiveView, at: (f32, f32), within: f32) -> Option<(usize, usize)> {
    let mut best = None;
    let mut best_d2 = within * within;
    for (q, soldiers) in v.frame.soldiers.iter().enumerate() {
        for (k, s) in soldiers.iter().enumerate() {
            if !matches!(s.state, FighterState::Standing | FighterState::Routing) {
                continue;
            }
            let (x, y) = s.pos();
            let d2 = (x - at.0).powi(2) + (y - at.1).powi(2);
            if d2 <= best_d2 {
                best_d2 = d2;
                best = Some((q, k));
            }
        }
    }
    best
}

/// Soldier `k` of squad `q` in the combat rules' terms, e.g. "a soldier of
/// swordsmen1: AC 17, +3 to hit, HP 6/8, 1d8+1 slashing".
pub fn soldier_words(v: &LiveView, q: usize, k: usize) -> Option<String> {
    let sheet = v.sheets.get(q)?.get(k)?;
    Some(format!(
        "a soldier of {}: {}",
        v.squads.get(q)?.name,
        sheet.words()
    ))
}

/// What the pointer at field point `at` inspects: the soldier under it (on
/// the soldier model; elsewhere the first of the squad under it still on
/// their feet), and for an enemy squad how the squads `mine` fare against
/// it ([`hint`]).
pub fn inspect(v: &LiveView, mine: &[usize], at: (f32, f32)) -> Option<String> {
    let squad = squad_at(v, at);
    let soldier = soldier_at(v, at, 1.0).or_else(|| {
        let q = squad?;
        let k = v.sheets.get(q)?.iter().position(|s| s.hp > 0)?;
        Some((q, k))
    });
    // The person first: "Mara, baker, 3 battles" (`price.rs`).
    let sheet = soldier.and_then(|(q, k)| {
        let sheet = soldier_words(v, q, k)?;
        Some(match v.fighter_words(q, k) {
            Some(who) => format!("{who}\n{sheet}"),
            None => sheet,
        })
    });
    let hint = squad
        .filter(|&e| v.squads[e].side != v.player_side)
        .and_then(|e| hint(v, mine, e));
    match (sheet, hint) {
        (Some(s), Some(h)) => Some(format!("{s}\n{h}")),
        (s, h) => s.or(h),
    }
}

/// The overlay's colour for a soldier: red when breaking, through yellow,
/// to green when steady; darker the more tired (linear RGB, 0..1).
pub fn condition_colour(c: Condition) -> [f32; 3] {
    let m = c.morale();
    let (r, g) = if m < 0.5 {
        (1.0, 2.0 * m)
    } else {
        (2.0 - 2.0 * m, 1.0)
    };
    let light = 1.0 - 0.6 * c.fatigue();
    [r * light, g * light, 0.1 * light]
}

/// Where the pointer is: on screen (pixels) and on the field (metres), if
/// it is over the field at all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pointer {
    pub screen: (f32, f32),
    pub field: Option<(f32, f32)>,
    pub shift: bool,
    /// Seconds, any clock.
    pub now: f64,
}

/// What a button held down is doing, for the clients to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    None,
    /// A selection box, corners on screen.
    Box((f32, f32), (f32, f32)),
    /// A line being laid out, ends on the field.
    Line((f32, f32), (f32, f32), bool),
}

/// A point on the field (metres) or on screen (pixels).
type Point = (f32, f32);

/// The mouse controls of a live battle: the selection, and a press in
/// progress.
#[derive(Clone, Debug, Default)]
pub struct Rts {
    pub selection: Selection,
    /// Where the left button went down, on screen.
    left: Option<(f32, f32)>,
    /// Where the right button went down: on the field and on screen, and
    /// whether it was the second click of a double-click.
    right: Option<(Point, Point, bool)>,
    /// The last right press (seconds, on screen), for a double-click.
    last_right: Option<(f64, (f32, f32))>,
}

fn far(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

impl Rts {
    /// The left button goes down: on the field, the battle takes it.
    pub fn left_down(&mut self, p: Pointer) -> bool {
        self.left = p.field.map(|_| p.screen);
        self.left.is_some()
    }

    /// The left button comes up: a click selects the squad under it, a box
    /// every squad of the player's whose centre `project` puts inside it.
    pub fn left_up(
        &mut self,
        v: &LiveView,
        p: Pointer,
        project: impl Fn((f32, f32)) -> Option<(f32, f32)>,
    ) {
        let Some(from) = self.left.take() else {
            return;
        };
        if far(from, p.screen) < DRAG_PX {
            let hit = p.field.and_then(|f| squad_at(v, f));
            self.selection.click(v, hit, p.shift);
            return;
        }
        let (x0, x1) = (from.0.min(p.screen.0), from.0.max(p.screen.0));
        let (y0, y1) = (from.1.min(p.screen.1), from.1.max(p.screen.1));
        let hits = (0..v.squads.len()).filter(|&q| {
            v.frame
                .squads
                .get(q)
                .and_then(|f| project((f.0, f.1)))
                .is_some_and(|(x, y)| (x0..=x1).contains(&x) && (y0..=y1).contains(&y))
        });
        self.selection.boxed(v, hits, p.shift);
    }

    /// The right button goes down: with squads selected and the pointer on
    /// the field, the battle takes it (and the camera must not).
    pub fn right_down(&mut self, v: &LiveView, p: Pointer) -> bool {
        self.selection.tidy(v);
        let (Some(field), false) = (p.field, self.selection.squads.is_empty()) else {
            self.right = None;
            return false;
        };
        let run = self
            .last_right
            .is_some_and(|(t, s)| p.now - t <= DOUBLE_CLICK && far(s, p.screen) < 2.0 * DRAG_PX);
        self.last_right = if run { None } else { Some((p.now, p.screen)) };
        self.right = Some((field, p.screen, run));
        true
    }

    /// Whether the battle holds the right button.
    pub fn holds_right(&self) -> bool {
        self.right.is_some()
    }

    /// The right button comes up: the commands it gives (queued with
    /// shift; deployments before the battle starts).
    pub fn right_up(&mut self, v: &LiveView, p: Pointer) -> Vec<(usize, Command)> {
        let Some((from, screen, run)) = self.right.take() else {
            return Vec::new();
        };
        let sel = &self.selection.squads;
        let cmds = match p.field {
            Some(to) if far(screen, p.screen) >= DRAG_PX => right_drag(v, sel, from, to, run),
            _ => right_click(v, sel, from, run),
        };
        as_given(v, cmds, p.shift)
    }

    /// What a held button is doing now.
    pub fn gesture(&self, p: Pointer) -> Gesture {
        if let Some(from) = self.left {
            if far(from, p.screen) >= DRAG_PX {
                return Gesture::Box(from, p.screen);
            }
        }
        if let (Some((from, screen, run)), Some(to)) = (self.right, p.field) {
            if far(screen, p.screen) >= DRAG_PX {
                return Gesture::Line(from, to, run);
            }
        }
        Gesture::None
    }

    /// The ghost of the line being laid out, if one is: with shift, where
    /// the queue ends; before the battle, where the squads would deploy.
    pub fn ghost(&self, v: &LiveView, p: Pointer) -> Vec<(usize, Preview)> {
        let Gesture::Line(from, to, run) = self.gesture(p) else {
            return Vec::new();
        };
        let sel = &self.selection.squads;
        if v.deploying {
            return deploy_ghost(v, sel, from, to);
        }
        if !p.shift {
            return ghost(v, sel, from, to, run);
        }
        as_given(v, right_drag(v, sel, from, to, run), true)
            .into_iter()
            .filter_map(|(q, c)| match c {
                Command::Orders(o) => v.preview(q, &o).map(|g| (q, g)),
                _ => None,
            })
            .collect()
    }

    /// A number key, `n` from 0: with Ctrl it sets the group, without it
    /// selects the group; on a double tap, where the camera should look.
    pub fn group_key(
        &mut self,
        v: &LiveView,
        n: usize,
        ctrl: bool,
        now: f64,
    ) -> Option<(f32, f32)> {
        if ctrl {
            self.selection.set_group(n);
            return None;
        }
        if self.selection.recall(v, n, now) {
            centre(v, &self.selection.squads)
        } else {
            None
        }
    }

    /// The enemy squad under the pointer, for the matchup hint.
    pub fn hover(&self, v: &LiveView, p: Pointer) -> Option<usize> {
        p.field
            .and_then(|f| squad_at(v, f))
            .filter(|&e| v.squads[e].side != v.player_side)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A live view of `lineup` on `model`, before the first step.
    fn view(model: BattleModel, lineup: &str) -> LiveView {
        let (field, specs) = parse_lineup(lineup).unwrap();
        let params = BattleParams {
            model,
            ..BattleParams::default()
        };
        Battle::new(params, field, specs, 1).live_view(0, (0.0, 0.0), (1.0, 0.0), 0)
    }

    fn at(v: &LiveView, q: usize) -> (f32, f32) {
        (v.frame.squads[q].0, v.frame.squads[q].1)
    }

    fn pointer(screen: (f32, f32), field: Option<(f32, f32)>, now: f64) -> Pointer {
        Pointer {
            screen,
            field,
            shift: false,
            now,
        }
    }

    const LINEUP: &str = "swordsmen*30,archers*20,cavalry*10 vs spearmen*30";

    #[test]
    fn clicks_select_your_own_squads() {
        for model in [BattleModel::Squads, BattleModel::Soldiers] {
            let v = view(model, LINEUP);
            let mut s = Selection::default();
            assert_eq!(squad_at(&v, at(&v, 1)), Some(1), "{model:?}");
            s.click(&v, squad_at(&v, at(&v, 0)), false);
            assert_eq!(s.squads, [0]);
            s.click(&v, Some(1), true);
            assert_eq!(s.squads, [0, 1]);
            s.click(&v, Some(0), true);
            assert_eq!(s.squads, [1]);
            // An enemy squad is not selected; empty ground clears.
            s.click(&v, Some(3), false);
            assert!(s.squads.is_empty());
            s.click(&v, Some(1), false);
            s.click(&v, squad_at(&v, (300.0, 390.0)), false);
            assert!(s.squads.is_empty());
        }
    }

    #[test]
    fn a_box_selects_the_squads_inside_it() {
        let v = view(BattleModel::Soldiers, LINEUP);
        let mut rts = Rts::default();
        // Screen = field, for the test.
        let project = |p: (f32, f32)| Some(p);
        assert!(rts.left_down(pointer((0.0, 0.0), Some((0.0, 0.0)), 0.0)));
        let up = pointer((600.0, 210.0), Some((600.0, 210.0)), 0.1);
        assert_eq!(rts.gesture(up), Gesture::Box((0.0, 0.0), (600.0, 210.0)));
        rts.left_up(&v, up, project);
        // The swordsmen and the archers stand in it; the enemy too, but is not
        // the player's.
        let mut got = rts.selection.squads.clone();
        got.sort();
        let want: Vec<usize> = (0..3).filter(|&q| at(&v, q).1 <= 210.0).collect();
        assert_eq!(got, want);
        // Shift adds.
        rts.left_down(pointer((0.0, 200.0), Some((0.0, 200.0)), 1.0));
        rts.left_up(
            &v,
            Pointer {
                shift: true,
                ..pointer((600.0, 400.0), Some((600.0, 400.0)), 1.1)
            },
            project,
        );
        let mut got = rts.selection.squads.clone();
        got.sort();
        assert_eq!(got, [0, 1, 2]);
    }

    #[test]
    fn control_groups_set_recall_and_jump() {
        let v = view(BattleModel::Squads, LINEUP);
        let mut rts = Rts::default();
        rts.selection.squads = vec![0, 2];
        assert_eq!(rts.group_key(&v, 3, true, 0.0), None);
        rts.selection.squads.clear();
        assert_eq!(rts.group_key(&v, 3, false, 1.0), None);
        assert_eq!(rts.selection.squads, [0, 2]);
        let c = rts.group_key(&v, 3, false, 1.2).unwrap();
        let want = centre(&v, &[0, 2]).unwrap();
        assert_eq!(c, want);
        // Slow taps do not jump; an empty group changes nothing.
        assert_eq!(rts.group_key(&v, 3, false, 3.0), None);
        rts.selection.squads = vec![1];
        assert_eq!(rts.group_key(&v, 5, false, 3.1), None);
        assert_eq!(rts.selection.squads, [1]);
    }

    #[test]
    fn right_clicks_move_attack_and_run() {
        let v = view(BattleModel::Soldiers, LINEUP);
        let mut rts = Rts::default();
        rts.selection.squads = vec![0, 1];
        // On the enemy: the swordsmen attack, the archers shoot.
        let e = at(&v, 3);
        assert!(rts.right_down(&v, pointer((5.0, 5.0), Some(e), 0.0)));
        assert!(rts.holds_right());
        let cmds = rts.right_up(&v, pointer((5.0, 5.0), Some(e), 0.05));
        assert_eq!(
            cmds,
            [
                (0, Command::Orders(vec![Order::Attack(3)])),
                (1, Command::Orders(vec![Order::Shoot(3)]))
            ]
        );
        // On ground, a second later: both move, keeping their places.
        let g = (150.0, 200.0);
        rts.right_down(&v, pointer((50.0, 50.0), Some(g), 1.0));
        let cmds = rts.right_up(&v, pointer((50.0, 50.0), Some(g), 1.05));
        let anchors: Vec<(f32, f32, bool)> = cmds
            .iter()
            .map(|c| match &c.1 {
                Command::Orders(o) => match o[..] {
                    [Order::MoveTo { anchor, run, .. }] => (anchor.0, anchor.1, run),
                    _ => panic!("{o:?}"),
                },
                _ => panic!(),
            })
            .collect();
        let (l0, l1) = (v.lines[0].anchor, v.lines[1].anchor);
        assert!(((anchors[0].0 - anchors[1].0) - (l0.0 - l1.0)).abs() < 1e-3);
        assert!(((anchors[0].0 + anchors[1].0) * 0.5 - g.0).abs() < 1e-3);
        assert!(!anchors[0].2);
        // Clicked again at once: they run.
        rts.right_down(&v, pointer((51.0, 50.0), Some(g), 1.2));
        let cmds = rts.right_up(&v, pointer((51.0, 50.0), Some(g), 1.25));
        assert!(cmds.iter().all(|c| matches!(
            &c.1,
            Command::Orders(o) if matches!(o[..], [Order::MoveTo { run: true, .. }])
        )));
        // One squad moves with its front's centre where clicked.
        rts.selection.squads = vec![2];
        rts.right_down(&v, pointer((300.0, 50.0), Some(g), 5.0));
        let cmds = rts.right_up(&v, pointer((300.0, 50.0), Some(g), 5.1));
        assert_eq!(cmds, [(2, Command::Orders(vec![Order::move_to(g)]))]);
        // Nothing selected: the right button is the camera's.
        rts.selection.squads.clear();
        assert!(!rts.right_down(&v, pointer((0.0, 0.0), Some(g), 9.0)));
        assert!(rts
            .right_up(&v, pointer((0.0, 0.0), Some(g), 9.1))
            .is_empty());
    }

    #[test]
    fn a_right_drag_lays_out_the_line_with_its_ghost() {
        let v = view(BattleModel::Soldiers, LINEUP);
        let mut rts = Rts::default();
        rts.selection.squads = vec![0, 1];
        let (from, to) = ((150.0, 100.0), (150.0, 220.0));
        rts.right_down(&v, pointer((0.0, 0.0), Some(from), 0.0));
        let held = pointer((0.0, 120.0), Some(to), 0.5);
        assert_eq!(rts.gesture(held), Gesture::Line(from, to, false));
        let ghost = rts.ghost(&v, held);
        assert_eq!(ghost.iter().map(|g| g.0).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(ghost[0].1.slots.len(), 30);
        assert!(ghost.iter().all(|g| g.1.eta > 0.0));
        let cmds = rts.right_up(&v, held);
        let want: Vec<(usize, Command)> = v
            .group_line(&[0, 1], from, to, false)
            .into_iter()
            .map(|(q, o)| (q, Command::Orders(vec![o])))
            .collect();
        assert_eq!(cmds, want);
    }

    /// Shift queues a move after the squad's moves, the path runs through
    /// them, and Backspace keeps only the order in force.
    #[test]
    fn shift_queues_waypoints_and_backspace_clears_them() {
        for model in [BattleModel::Squads, BattleModel::Soldiers] {
            let mut v = view(model, LINEUP);
            let mut rts = Rts::default();
            rts.selection.squads = vec![0];
            let a = (200.0, 150.0);
            let b = (260.0, 220.0);
            // A first move, as given.
            rts.right_down(&v, pointer((0.0, 0.0), Some(a), 0.0));
            let first = rts.right_up(&v, pointer((0.0, 0.0), Some(a), 0.05));
            let Command::Orders(o) = &first[0].1 else {
                panic!()
            };
            v.orders[0] = o.clone();
            // A second, queued with shift, and the ghost shows its end.
            rts.right_down(&v, pointer((0.0, 0.0), Some(b), 1.0));
            let shifted = Pointer {
                shift: true,
                ..pointer((0.0, 0.0), Some(b), 1.05)
            };
            let second = rts.right_up(&v, shifted);
            let Command::Orders(o) = &second[0].1 else {
                panic!()
            };
            assert_eq!(o.len(), 2, "{model:?}: {o:?}");
            assert!(matches!(o[1], Order::MoveTo { anchor, .. } if anchor == b));
            v.orders[0] = o.clone();
            assert_eq!(queued_path(&v, 0)[1..], [a, b]);
            // An attack queued after the moves; what followed them goes.
            v.orders[0].push(Order::Hold);
            let q = queued(&v, 0, &[Order::Attack(3)]);
            assert_eq!(q.len(), 3);
            assert_eq!(q[2], Order::Attack(3));
            // Backspace: the order in force alone.
            let cleared = clear_queue(&v, &[0]);
            assert_eq!(cleared, [(0, Command::Orders(vec![v.orders[0][0]]))]);
            // Nothing queued: nothing to draw or clear.
            v.orders[0].truncate(1);
            assert!(clear_queue(&v, &[0]).is_empty());
            v.orders[0].clear();
            assert!(queued_path(&v, 0).is_empty());
        }
    }

    /// Before the battle, a right-drag stands the squads in the player's
    /// half: points beyond it are pulled in, attacks are dropped.
    #[test]
    fn before_the_battle_orders_deploy_in_your_half() {
        let mut v = view(BattleModel::Soldiers, LINEUP);
        v.deploying = true;
        let (x0, x1, _, _) = v.deploy_zone(0);
        let mut rts = Rts::default();
        rts.selection.squads = vec![0, 1];
        rts.right_down(&v, pointer((0.0, 0.0), Some((100.0, 100.0)), 0.0));
        let up = pointer((300.0, 0.0), Some((x1 + 80.0, 300.0)), 0.1);
        assert!(rts.ghost(&v, up).iter().all(|g| g.1.eta == 0.0));
        let cmds = rts.right_up(&v, up);
        assert_eq!(cmds.len(), 2);
        for (_, c) in &cmds {
            let Command::Deploy { anchor, .. } = c else {
                panic!("{c:?}")
            };
            assert!((x0..=x1).contains(&anchor.0), "{anchor:?}");
        }
        // Right-clicking the enemy deploys nobody.
        let enemy = at(&v, 3);
        rts.right_down(&v, pointer((0.0, 0.0), Some(enemy), 5.0));
        assert!(rts
            .right_up(&v, pointer((0.0, 0.0), Some(enemy), 5.05))
            .is_empty());
    }

    #[test]
    fn stance_keys_set_the_selected_squads_stance() {
        let v = view(BattleModel::Soldiers, LINEUP);
        let key = StanceKey::Fire(Fire::Skirmish);
        assert!(!key.chosen(&v, &[1]) && StanceKey::Fire(Fire::AtWill).chosen(&v, &[0, 1]));
        let cmds = stance(&v, &[1, 3], key);
        assert_eq!(
            cmds,
            [(
                1,
                Command::Stance(Stance {
                    fire: Fire::Skirmish,
                    ..Stance::default()
                })
            )]
        );
    }

    #[test]
    fn matchup_hints_say_why() {
        let k = |n: &str| Kit::preset(n).unwrap();
        // Recruits of skill 0.5: proficiency +2, and the weapon's quality.
        let b = |n: &str| {
            let q = 2.0 + k(n).quality;
            (q, q - 4.0)
        };
        let words = |a: &str, t: &str| matchup_words(&k(a), b(a), &k(t), b(t));
        assert_eq!(
            words("swordsmen", "swordsmen"),
            "even: your swords hit their mail on 14+ (35%), resistant to slashing"
        );
        assert!(words("levy", "swordsmen").starts_with("weak: your clubs hit their mail on 15+"));
        assert_eq!(
            words("cavalry", "spearmen"),
            "weak: their spears brace against your riders"
        );
        assert!(words("spearmen", "cavalry").starts_with("strong"));
        assert!(
            words("archers", "levy").starts_with("strong: your arrows hit their bare skin on 12+"),
            "{}",
            words("archers", "levy")
        );
        let v = view(BattleModel::Squads, LINEUP);
        let h = hint(&v, &[0], 3).unwrap();
        assert!(
            h.starts_with("spearmen") && h.contains("swordsmen1: "),
            "{h}"
        );
        assert!(hint(&v, &[], 3).unwrap().lines().count() == 4);
        assert!(hint(&v, &[0], 1).is_none());
        // Pointing at a squad inspects its soldiers too, an enemy's with the hint.
        let own = inspect(&v, &[], at(&v, 0)).unwrap();
        assert!(
            own.starts_with("a soldier of swordsmen")
                && own.ends_with(": AC 17, +3 to hit, HP 8/8, 1d8+1 slashing"),
            "{own}"
        );
        let enemy = inspect(&v, &[0], at(&v, 3)).unwrap();
        assert!(
            enemy.lines().count() == 3 && enemy.contains("AC 13") && enemy.contains("on 10+"),
            "{enemy}"
        );
        // On the soldier model, the soldier under the pointer.
        let v = view(BattleModel::Soldiers, LINEUP);
        let (x, y) = v.frame.soldiers[3][5].pos();
        assert_eq!(soldier_at(&v, (x + 0.2, y), 1.0), Some((3, 5)));
        let words = inspect(&v, &[], (x, y)).unwrap();
        assert!(words.contains("1d6+1 piercing"), "{words}");
        // A battle of people names the soldier first.
        let mut v = v;
        v.people = v
            .squads
            .iter()
            .map(|q| {
                (0..q.start)
                    .map(|k| Fighter {
                        name: format!("Mara{k}"),
                        trade: "baker".into(),
                        battles: 3,
                        ..Default::default()
                    })
                    .collect()
            })
            .collect();
        let words = inspect(&v, &[], (x, y)).unwrap();
        assert!(
            words.starts_with("Mara5, baker, 3 battles\na soldier of"),
            "{words}"
        );
    }

    #[test]
    fn the_overlay_goes_from_red_to_green_and_darkens_with_fatigue() {
        let c = |m, f| condition_colour(Condition::new(m, f));
        assert_eq!(c(0.0, 0.0), [1.0, 0.0, 0.1]);
        assert_eq!(c(1.0, 0.0), [0.0, 1.0, 0.1]);
        let tired = c(1.0, 1.0);
        assert!((tired[1] - 0.4).abs() < 1e-6);
    }
}
