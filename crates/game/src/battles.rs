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
//! A battle fought on the soldier model records every soldier in each frame.
//! Zoomed in (`live_battle::SOLDIERS_BELOW`), such a replay is drawn soldier
//! by soldier (`draw_soldiers`), each moving between frames as the 3D client
//! moves its figures.
//!
//! A battle the player's own town fights is live instead (live_battle.rs):
//! its squads are drawn here the same way, from the battle's current frame.
//! On the soldier model the frame puts each squad at its soldiers' centre,
//! and zoomed in the soldiers themselves are drawn.

use crate::view::WorldCamera;
use bevy::prelude::*;
use sim_core::battle::{BattleLog, FighterState, FrameSoldier, Replay, ReplaySquad, SquadState};
use sim_core::Towns;

/// Battle seconds shown per real second.
const PLAYBACK: f32 = 30.0;
/// Map units per metre of battlefield, as the sim lays a field over a town.
pub const SCALE: f32 = sim_core::battle::MAP_PER_METRE;
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

impl BattleScenes {
    /// The towns whose battles are being shown.
    pub fn towns(&self) -> impl Iterator<Item = u16> + '_ {
        self.scenes.iter().map(|s| s.replay.town)
    }
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

impl BattleScenes {
    /// How far (battle seconds) the newest battle being played back has got.
    pub fn newest_at(&self, now: f32) -> Option<f32> {
        self.scenes.last().map(|s| (now - s.start) * PLAYBACK)
    }

    /// Every battle being played back, oldest first, with how far it has
    /// got (battle seconds, held at its end while it lingers).
    pub fn playing(&self, now: f32) -> Vec<(&Replay, f32)> {
        self.scenes
            .iter()
            .map(|s| {
                let end = s.replay.frames.last().map_or(0.0, |f| f.t);
                (&s.replay, ((now - s.start) * PLAYBACK).min(end))
            })
            .collect()
    }
}

/// `--battle LINEUP`: fight it now on `battle.model` (models.ron, `--set`)
/// and put it in the battle log to play beside the player's town, from
/// shortly before the lines meet, as the 3D client stages it. Returns where
/// on the map the lines meet.
pub fn stage_battle(world: &mut World, lineup: &str) -> (f32, f32) {
    use sim_core::battle::{parse_lineup, Battle};
    let (mut field, mut specs) = parse_lineup(lineup).unwrap_or_else(|e| {
        eprintln!("error: --battle: {e}");
        std::process::exit(2);
    });
    let params = world.resource::<sim_core::world::Params>().battle.clone();
    let k = world
        .resource::<sim_core::policy::PlayerTown>()
        .0
        .map_or(0, |t| t as usize);
    let town = world
        .resource::<Towns>()
        .0
        .get(k)
        .map_or((0.0, 0.0), |t| t.pos);
    let size = (field.width, field.height);
    // A battle `in town` is fought in the player's own town, among its real
    // buildings and houses, where a raid on it would be; otherwise below
    // the town, clear of its houses.
    let in_town = !field.obstacles.is_empty();
    let (at, facing) = if in_town {
        ((town.0 - 60.0, town.1), (1.0, 0.0))
    } else {
        ((town.0, town.1 - 0.5 * size.1 * SCALE - 60.0), (1.0, 0.0))
    };
    if in_town {
        let layout = sim_core::layout::TownLayout::of(
            world.resource::<Towns>(),
            k,
            world.resource::<sim_core::Db>(),
            world.resource::<sim_core::terrain::Terrain>(),
        );
        let db = world.resource::<sim_core::Db>();
        let palisade = sim_core::walls::palisade(db);
        layout.stage(&mut field, &mut specs, at, facing, &db.wall, palisade, k);
    }
    // Its soldiers are the town's people, and their neighbours'.
    let (people, attacker) = sim_core::systems::military::stage_fighters(world, &specs, k);
    let mut battle = Battle::new(params, field, specs, 1);
    let walls = battle.replay_walls();
    let r = battle.record(5.0);
    info!(
        "--battle {lineup}: {:?} after {:.0} s",
        r.report.winner, r.report.seconds
    );
    let mut replay = Replay {
        town: k as u16,
        at,
        facing,
        field: size,
        squads: r.squads,
        frames: r.frames,
        winner: r.report.winner,
        people,
        attacker,
        lost: r.lost,
        walls,
        gathered: Default::default(),
    };
    let to_map = field_to_map(replay.at, replay.facing, replay.field);
    let meet = replay
        .skip_approach()
        .map_or(Vec2::new(at.0, at.1), |p| to_map(Vec2::from(p)));
    world.resource_mut::<BattleLog>().push(replay);
    (meet.x, meet.y)
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

/// Where a point of a battlefield (metres) lies on the map, for a field
/// centred at `at` whose +x points along `facing`, from the attackers to the
/// defenders.
pub fn field_to_map(
    at: (f32, f32),
    facing: (f32, f32),
    field: (f32, f32),
) -> impl Fn(Vec2) -> Vec2 {
    move |p: Vec2| Vec2::from(sim_core::battle::field_to_map(at, facing, field, p.into()))
}

/// The other way round: where a point of the map lies on the battlefield
/// (metres), for clicks on it.
pub fn map_to_field(
    at: (f32, f32),
    facing: (f32, f32),
    field: (f32, f32),
) -> impl Fn(Vec2) -> Vec2 {
    move |p: Vec2| Vec2::from(sim_core::battle::map_to_field(at, facing, field, p.into()))
}

/// Draw one battlefield: its outline, and a block per squad where `squad`
/// says it stands (metres on the field), sized by the fighters standing.
#[allow(clippy::too_many_arguments)]
pub fn draw_field(
    gizmos: &mut Gizmos,
    now: f32,
    at: (f32, f32),
    facing: (f32, f32),
    field: (f32, f32),
    squads: &[ReplaySquad],
    squad: impl Fn(usize) -> Option<(Vec2, u16, SquadState)>,
    outline: Color,
) {
    draw_outline(gizmos, at, facing, field, outline);
    let to_map = field_to_map(at, facing, field);
    for (k, sq) in squads.iter().enumerate() {
        let Some((p, standing, state)) = squad(k) else {
            continue;
        };
        let base = if sq.side == 0 { ATTACKER } else { DEFENDER };
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
        if sq.mounted {
            gizmos.rect_2d(
                Isometry2d::new(at, Rot2::IDENTITY),
                Vec2::splat(side + 4.0),
                colour,
            );
        }
    }
}

/// A battlefield's outline.
fn draw_outline(
    gizmos: &mut Gizmos,
    at: (f32, f32),
    facing: (f32, f32),
    field: (f32, f32),
    outline: Color,
) {
    let to_map = field_to_map(at, facing, field);
    let corners = [
        Vec2::ZERO,
        Vec2::new(field.0, 0.0),
        Vec2::new(field.0, field.1),
        Vec2::new(0.0, field.1),
    ]
    .map(to_map);
    for k in 0..4 {
        gizmos.line_2d(corners[k], corners[(k + 1) % 4], outline);
    }
}

/// One soldier as a dot on the map: where, how big (map units) and in what
/// colour. Routing soldiers are grey and blink, the fallen are dim, and those
/// who fled are not drawn.
pub struct Dot {
    pub at: Vec2,
    pub radius: f32,
    pub colour: Color,
    pub down: bool,
}

/// The dots for every soldier of a battle on the soldier model.
pub fn soldier_dots(
    now: f32,
    at: (f32, f32),
    facing: (f32, f32),
    field: (f32, f32),
    squads: &[ReplaySquad],
    soldiers: &[Vec<FrameSoldier>],
) -> Vec<Dot> {
    let to_map = field_to_map(at, facing, field);
    let mut out = Vec::new();
    for (sq, men) in squads.iter().zip(soldiers) {
        let base = if sq.side == 0 { ATTACKER } else { DEFENDER };
        // A man's body and a horse's, as the soldier model has them.
        let radius = if sq.mounted { 0.9 } else { 0.4 } * SCALE;
        for m in men {
            let (colour, down) = match m.state {
                FighterState::Standing => (base, false),
                FighterState::Routing => (
                    Color::srgb(0.7, 0.68, 0.62).with_alpha(0.5 + 0.5 * (now * 8.0).sin().abs()),
                    false,
                ),
                FighterState::Down => (base.with_alpha(0.3), true),
                FighterState::Fled => continue,
            };
            let (x, y) = m.pos();
            out.push(Dot {
                at: to_map(Vec2::new(x, y)),
                radius,
                colour,
                down,
            });
        }
    }
    out
}

/// Draw a battle's soldiers as dots, with the field's outline.
/// `px` is map units per screen pixel, so a dot is never smaller than a pixel.
#[allow(clippy::too_many_arguments)]
pub fn draw_soldiers(
    gizmos: &mut Gizmos,
    now: f32,
    at: (f32, f32),
    facing: (f32, f32),
    field: (f32, f32),
    squads: &[ReplaySquad],
    soldiers: &[Vec<FrameSoldier>],
    outline: Color,
    px: f32,
) {
    draw_outline(gizmos, at, facing, field, outline);
    for d in soldier_dots(now, at, facing, field, squads, soldiers) {
        let r = d.radius.max(px);
        if d.down {
            // The fallen as a small cross.
            gizmos.line_2d(d.at - Vec2::splat(r), d.at + Vec2::splat(r), d.colour);
            gizmos.line_2d(d.at + Vec2::new(-r, r), d.at + Vec2::new(r, -r), d.colour);
        } else {
            gizmos
                .circle_2d(Isometry2d::from_translation(d.at), r, d.colour)
                .resolution(6);
            gizmos
                .circle_2d(Isometry2d::from_translation(d.at), 0.5 * r, d.colour)
                .resolution(4);
        }
    }
}

/// Draw every scene, and write the outcome once a battle is over. Zoomed
/// in, a battle fought on the soldier model shows its soldiers.
pub fn draw_battles(
    mut gizmos: Gizmos,
    time: Res<Time>,
    scenes: Res<BattleScenes>,
    mut labels: Query<&mut Text2d>,
    camera: Single<&Transform, With<WorldCamera>>,
) {
    let now = time.elapsed_secs();
    let zoom = camera.scale.x;
    let outline = Color::srgba(1.0, 0.9, 0.6, 0.35);
    for s in &scenes.scenes {
        let r = &s.replay;
        let t = (now - s.start) * PLAYBACK;
        let over = (now - s.start) >= duration(r);
        let soldiers = if zoom < crate::live_battle::SOLDIERS_BELOW {
            r.soldiers_at(t)
        } else {
            Vec::new()
        };
        if !r.walls.is_empty() {
            let hp = r.around(t).map_or(Vec::new(), |(a, _, _)| a.walls.clone());
            crate::walls_view::draw_field_walls(
                &mut gizmos,
                &r.walls,
                &hp,
                field_to_map(r.at, r.facing, r.field),
            );
        }
        if soldiers.is_empty() {
            draw_field(
                &mut gizmos,
                now,
                r.at,
                r.facing,
                r.field,
                &r.squads,
                |k| squad_at(r, k, t),
                outline,
            );
        } else {
            draw_soldiers(
                &mut gizmos,
                now,
                r.at,
                r.facing,
                r.field,
                &r.squads,
                &soldiers,
                outline,
                zoom,
            );
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
                    soldiers: Vec::new(),
                    walls: Vec::new(),
                },
                ReplayFrame {
                    t: 10.0,
                    squads: vec![(10.0, 10.0, 12, SquadState::Routing)],
                    soldiers: Vec::new(),
                    walls: Vec::new(),
                },
            ],
            winner: Some(1),
            ..Default::default()
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
        assert!(r.soldiers_at(5.0).is_empty());
    }

    /// A battle recorded on the soldier model plays back soldier by
    /// soldier: one dot for each who has not left the field, moving between
    /// frames.
    #[test]
    fn a_soldier_replay_is_drawn_soldier_by_soldier() {
        use sim_core::battle::{parse_lineup, Battle, BattleModel, BattleParams};
        let (field, specs) = parse_lineup("swordsmen*30 vs spearmen*20").unwrap();
        let params = BattleParams {
            model: BattleModel::Soldiers,
            ..BattleParams::default()
        };
        let (_, squads, frames) = Battle::new(params, field.clone(), specs, 1).run_recorded(5.0);
        let r = Replay {
            town: 0,
            at: (0.0, 0.0),
            facing: (1.0, 0.0),
            field: (field.width, field.height),
            squads,
            frames,
            winner: None,
            ..Default::default()
        };
        let t = 0.5 * (r.frames[3].t + r.frames[4].t);
        let men = r.soldiers_at(t);
        assert_eq!(men.iter().map(Vec::len).collect::<Vec<_>>(), [30, 20]);
        let dots = soldier_dots(0.0, r.at, r.facing, r.field, &r.squads, &men);
        let on = men
            .iter()
            .flatten()
            .filter(|m| m.state != FighterState::Fled);
        assert_eq!(dots.len(), on.count());
        // The first soldier's dot lies between where the frames have them.
        let to_map = field_to_map(r.at, r.facing, r.field);
        let at = |f: &sim_core::battle::ReplayFrame| {
            let (x, y) = f.soldiers[0][0].pos();
            to_map(Vec2::new(x, y))
        };
        let (a, b) = (at(&r.frames[3]), at(&r.frames[4]));
        assert!((dots[0].at - (a + b) * 0.5).length() <= 0.1 * SCALE);
    }
}
