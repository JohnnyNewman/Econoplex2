//! Town walls (`sim_core::walls`) on the map: each town's ring as it stands
//! (a palisade brown, a stone wall grey, gates lighter, a piece the darker
//! red the more battered it is, a gap never built dotted), and on a
//! battlefield the wall as the battle leaves it, a breach a broken red line.

use crate::battles::BattleScenes;
use crate::live_battle::LiveBattle;
use bevy::prelude::*;
use sim_core::battle::ReplayWall;
use sim_core::{Db, Towns};

const PALISADE: Color = Color::srgb(0.55, 0.36, 0.18);
const STONE: Color = Color::srgb(0.68, 0.67, 0.62);
const GATE: Color = Color::srgb(0.9, 0.75, 0.4);
const BATTERED: Color = Color::srgb(0.75, 0.12, 0.08);
const GAP: Color = Color::srgba(0.9, 0.85, 0.7, 0.35);

/// How a piece of wall stands, for drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Standing {
    /// Never built.
    Gap,
    /// Battered down.
    Breach,
    /// Up, with this share of its hit points left.
    Up(f32),
}

impl Standing {
    pub fn of(hp: f32, max: f32) -> Self {
        if max <= 0.0 {
            Standing::Gap
        } else if hp <= 0.0 {
            Standing::Breach
        } else {
            Standing::Up((hp / max).clamp(0.0, 1.0))
        }
    }
}

/// The colour a standing piece is drawn in.
pub fn piece_colour(stone: bool, gate: bool, share: f32) -> Color {
    let base = if gate {
        GATE
    } else if stone {
        STONE
    } else {
        PALISADE
    };
    base.mix(&BATTERED, (1.0 - share) * 0.8)
}

/// One piece from `a` to `b` (map units), `width` map units thick.
pub fn draw_piece(
    gizmos: &mut Gizmos,
    a: Vec2,
    b: Vec2,
    width: f32,
    stone: bool,
    gate: bool,
    standing: Standing,
) {
    let along = b - a;
    let len = along.length();
    if len < 1e-3 {
        return;
    }
    let across = along.perp() / len;
    match standing {
        Standing::Gap => {
            // Dotted.
            let n = (len / 3.0).ceil() as usize;
            for k in (0..n).step_by(2) {
                let (u, v) = (k as f32 / n as f32, ((k + 1) as f32 / n as f32).min(1.0));
                gizmos.line_2d(a + along * u, a + along * v, GAP);
            }
        }
        Standing::Breach => {
            // Rubble: the two ends, and broken bits between.
            for (u, v) in [(0.0, 0.12), (0.3, 0.36), (0.6, 0.66), (0.88, 1.0)] {
                for s in [-0.5, 0.5] {
                    let o = across * (s * width);
                    gizmos.line_2d(a + along * u + o, a + along * v + o, BATTERED);
                }
            }
        }
        Standing::Up(share) => {
            let colour = piece_colour(stone, gate, share);
            let lines = if stone { 4 } else { 3 };
            for k in 0..lines {
                let s = k as f32 / (lines - 1) as f32 - 0.5;
                let o = across * (s * width);
                gizmos.line_2d(a + o, b + o, colour);
            }
            if gate {
                // A gate's posts.
                for p in [a, b] {
                    gizmos.line_2d(p - across * width, p + across * width, colour);
                }
            }
        }
    }
}

/// Every town's wall, but at a town where a battle is being shown (that
/// battle draws its own).
pub fn draw_town_walls(
    mut gizmos: Gizmos,
    towns: Res<Towns>,
    db: Res<Db>,
    scenes: Res<BattleScenes>,
    live: Res<LiveBattle>,
) {
    let busy = |ti: usize| {
        scenes.towns().any(|t| t as usize == ti)
            || live
                .view
                .as_ref()
                .is_some_and(|v| v.town as usize == ti && !v.walls.is_empty())
    };
    for (ti, town) in towns.0.iter().enumerate() {
        if town.walls.pieces.is_empty() || busy(ti) {
            continue;
        }
        for q in &town.walls.pieces {
            let def = q.built.and_then(|b| db.wall[b as usize]);
            let standing = Standing::of(q.hp, def.map_or(0.0, |d| d.hp));
            let width = def.map_or(1.0, |d| 2.0 + d.thickness);
            draw_piece(
                &mut gizmos,
                Vec2::new(q.a.0, q.a.1),
                Vec2::new(q.b.0, q.b.1),
                width,
                def.is_some_and(|d| d.thickness > 1.0),
                q.gate,
                standing,
            );
        }
    }
}

/// A battlefield's wall, field metres mapped to the map by `to_map`, at
/// the hit points `hp` (in the walls' order).
pub fn draw_field_walls(
    gizmos: &mut Gizmos,
    walls: &[ReplayWall],
    hp: &[f32],
    to_map: impl Fn(Vec2) -> Vec2,
) {
    for (k, w) in walls.iter().enumerate() {
        let hp = hp.get(k).copied().unwrap_or(w.max);
        draw_piece(
            gizmos,
            to_map(Vec2::from(w.a)),
            to_map(Vec2::from(w.b)),
            2.0 + 2.0 * w.half_width,
            w.half_width > 0.5,
            w.gate,
            Standing::of(hp, w.max),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_piece_stands_as_its_hit_points_say() {
        assert_eq!(Standing::of(0.0, 0.0), Standing::Gap);
        assert_eq!(Standing::of(0.0, 400.0), Standing::Breach);
        assert_eq!(Standing::of(100.0, 400.0), Standing::Up(0.25));
        // The more battered, the redder.
        let whole = piece_colour(false, false, 1.0).to_srgba();
        let worn = piece_colour(false, false, 0.2).to_srgba();
        assert!(worn.red > whole.red && worn.green < whole.green);
    }
}
