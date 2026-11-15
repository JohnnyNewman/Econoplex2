//! Battles played back on the 2D map, as the 3D client plays them on its
//! field (docs/design/client-parity.md).
//!
//! The simulation fights a raid's battle in one go and records where every
//! squad stood every few seconds (`sim_core::battle::BattleLog`). Each new
//! battle is played back fast beside the defending town: a block per squad,
//! sized by the fighters still standing, attackers red and defenders blue,
//! riders with a frame around them. A routing squad blinks, one that fled or
//! was destroyed fades. The scene stays a few seconds after the end, with the
//! outcome written above the field, then clears.
//!
//! Live battles with orders (the player's own) are fought in the 3D client.

use bevy::prelude::*;
use sim_core::battle::{BattleLog, Replay, SquadState};
use sim_core::Towns;

/// Battle seconds shown per real second.
const PLAYBACK: f32 = 30.0;
/// Map units per metre of battlefield.
const SCALE: f32 = 0.35;
/// Real seconds a finished battle stays on the map.
const LINGER: f32 = 4.0;

const ATTACKER: Color = Color::srgb(0.9, 0.25, 0.2);
const DEFENDER: Color = Color::srgb(0.3, 0.55, 1.0);

/// Battles being shown, and how many of all fought battles have been taken.
#[derive(Resource, Default)]
pub struct BattleScenes {
    seen: usize,
    scenes: Vec<Scene>,
}

struct Scene {
    replay: Replay,
    /// Real time the playback started.
    start: f32,
    label: Entity,
}

/// Start new battles and clear finished ones.
pub fn start_battles(
    mut commands: Commands,
    time: Res<Time>,
    log: Option<Res<BattleLog>>,
    towns: Res<Towns>,
    mut scenes: ResMut<BattleScenes>,
) {
    let now = time.elapsed_secs();
    scenes.scenes.retain(|s| {
        let keep = now - s.start < duration(&s.replay) + LINGER;
        if !keep {
            commands.entity(s.label).despawn();
        }
        keep
    });
    let Some(log) = log else { return };
    if log.total == scenes.seen {
        return;
    }
    for replay in log.since(scenes.seen) {
        let town = towns
            .0
            .get(replay.town as usize)
            .map_or("a town", |t| t.name.as_str());
        let (w, h) = (replay.field.0 * SCALE, replay.field.1 * SCALE);
        let label = commands
            .spawn((
                Text2d::new(format!("Battle at {town}")),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.92, 0.7)),
                // Below the field, clear of the town's name above it.
                Transform::from_xyz(replay.at.0, replay.at.1 - 0.5 * w.max(h) - 14.0, 7.0),
            ))
            .id();
        scenes.scenes.push(Scene {
            replay,
            start: now,
            label,
        });
    }
    scenes.seen = log.total;
}

/// How long a replay lasts, battle seconds.
fn duration(r: &Replay) -> f32 {
    r.frames.last().map_or(0.0, |f| f.t) / PLAYBACK
}

/// Where squad `k` stands at battle time `t` (metres on the field), how many
/// stand, and its state: the frames before and after, blended.
pub fn squad_at(r: &Replay, k: usize, t: f32) -> Option<(Vec2, u16, SquadState)> {
    let next = r.frames.iter().position(|f| f.t >= t);
    let (a, b) = match next {
        Some(0) => (0, 0),
        Some(n) => (n - 1, n),
        None => (r.frames.len().checked_sub(1)?, r.frames.len() - 1),
    };
    let (fa, fb) = (&r.frames[a], &r.frames[b]);
    let (sa, sb) = (fa.squads.get(k)?, fb.squads.get(k)?);
    let u = if fb.t > fa.t {
        ((t - fa.t) / (fb.t - fa.t)).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let pos = Vec2::new(sa.0, sa.1).lerp(Vec2::new(sb.0, sb.1), u);
    let state = if u < 1.0 { sa.3 } else { sb.3 };
    Some((pos, if u < 0.5 { sa.2 } else { sb.2 }, state))
}

/// Draw every scene, and write the outcome once a battle is over.
pub fn draw_battles(
    mut gizmos: Gizmos,
    time: Res<Time>,
    scenes: Res<BattleScenes>,
    mut labels: Query<&mut Text2d>,
) {
    let now = time.elapsed_secs();
    for s in &scenes.scenes {
        let r = &s.replay;
        let t = (now - s.start) * PLAYBACK;
        let over = (now - s.start) >= duration(r);
        let centre = Vec2::new(r.at.0, r.at.1);
        // The field's +x points from the attackers to the defenders.
        let x = Vec2::new(r.facing.0, r.facing.1).normalize_or(Vec2::X);
        let y = x.perp();
        let to_map =
            |p: Vec2| centre + (x * (p.x - 0.5 * r.field.0) + y * (p.y - 0.5 * r.field.1)) * SCALE;
        let corners = [
            Vec2::ZERO,
            Vec2::new(r.field.0, 0.0),
            Vec2::new(r.field.0, r.field.1),
            Vec2::new(0.0, r.field.1),
        ]
        .map(to_map);
        for k in 0..4 {
            gizmos.line_2d(
                corners[k],
                corners[(k + 1) % 4],
                Color::srgba(1.0, 0.9, 0.6, 0.35),
            );
        }
        for (k, squad) in r.squads.iter().enumerate() {
            let Some((p, standing, state)) = squad_at(r, k, t) else {
                continue;
            };
            let base = if squad.side == 0 { ATTACKER } else { DEFENDER };
            let alpha = match state {
                SquadState::Fighting => 1.0,
                // Routing squads blink.
                SquadState::Routing => 0.4 + 0.6 * (now * 8.0).sin().abs(),
                SquadState::Fled | SquadState::Destroyed => 0.2,
            };
            let colour = base.with_alpha(alpha);
            let side = 6.0 + (standing as f32).sqrt() * 2.5;
            let at = to_map(p);
            gizmos.rect_2d(
                Isometry2d::new(at, Rot2::IDENTITY),
                Vec2::splat(side),
                colour,
            );
            // A filled look: a smaller block inside.
            if standing > 0 {
                gizmos.rect_2d(
                    Isometry2d::new(at, Rot2::IDENTITY),
                    Vec2::splat(side * 0.5),
                    colour,
                );
            }
            if squad.mounted {
                gizmos.rect_2d(
                    Isometry2d::new(at, Rot2::IDENTITY),
                    Vec2::splat(side + 4.0),
                    colour,
                );
            }
        }
        if over {
            if let Ok(mut text) = labels.get_mut(s.label) {
                let outcome = match r.winner {
                    Some(0) => "the attackers win",
                    Some(_) => "the defenders hold",
                    None => "undecided",
                };
                if !text.0.contains(outcome) {
                    text.0 = format!("{}: {outcome}", text.0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::battle::{ReplayFrame, ReplaySquad};

    fn replay() -> Replay {
        Replay {
            town: 0,
            at: (0.0, 0.0),
            facing: (1.0, 0.0),
            field: (100.0, 50.0),
            squads: vec![ReplaySquad {
                name: "a".into(),
                side: 0,
                mounted: false,
                start: 20,
            }],
            frames: vec![
                ReplayFrame {
                    t: 0.0,
                    squads: vec![(0.0, 10.0, 20, SquadState::Fighting)],
                },
                ReplayFrame {
                    t: 10.0,
                    squads: vec![(10.0, 10.0, 12, SquadState::Routing)],
                },
            ],
            winner: Some(1),
        }
    }

    #[test]
    fn squads_move_between_frames_and_stop_at_the_end() {
        let r = replay();
        let (p, n, s) = squad_at(&r, 0, 5.0).unwrap();
        assert!((p.x - 5.0).abs() < 1e-4 && n == 12 && s == SquadState::Fighting);
        let (p, _, s) = squad_at(&r, 0, 99.0).unwrap();
        assert!((p.x - 10.0).abs() < 1e-4 && s == SquadState::Routing);
        assert!(squad_at(&r, 3, 1.0).is_none());
    }
}
