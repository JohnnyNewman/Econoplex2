//! Battles played back where they were fought.
//!
//! The simulation fights a raid's battle in one go (`sim_core::battle`) and
//! records where every squad stood every few seconds (`Replay`). This plugin
//! plays each new replay back on the field outside the defending town, fast:
//! a figure per fighter in a block formation around its squad's position,
//! attackers in red, defenders in blue, riders larger. Fighters who fall lie
//! down where they were; a routing squad turns and runs. A scene stays a few
//! seconds after the battle ends, then clears.

use crate::live::LiveSim;
use crate::toon::{toon, ToonMaterial};
use crate::{ground, World3d, SCALE};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use sim_core::battle::{Replay, SquadState};

pub struct BattlesPlugin;

impl Plugin for BattlesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Looks>()
            .add_systems(Startup, stage_battle)
            .add_systems(Update, (start_battles, play_battles).chain());
    }
}

/// Battle seconds shown per real second.
const PLAYBACK: f32 = 30.0;
/// 3D units per metre of battlefield (a figure is about 0.75 units tall), and
/// between figures in a formation.
const METRE: f32 = 0.08;
const SPACING: f32 = 0.45;
/// Real seconds a scene stays after its battle ends.
const LINGER: f32 = 6.0;
/// Battles shown at once; older ones clear when more start.
const AT_ONCE: usize = 3;

#[derive(Resource, Default)]
struct Looks {
    mesh: Option<Handle<Mesh>>,
    /// Attackers, defenders, and a routing squad's colour.
    colours: Vec<Handle<ToonMaterial>>,
}

#[derive(Component)]
struct BattleScene {
    replay: Replay,
    started: f32,
    /// Figures per squad, in formation order.
    figures: Vec<Vec<Entity>>,
}

fn figure_mesh() -> Mesh {
    let (pos, nrm) = crate::crowd::figure();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
}

fn start_battles(
    mut commands: Commands,
    mut live: ResMut<LiveSim>,
    mut looks: ResMut<Looks>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut toons: ResMut<Assets<ToonMaterial>>,
    scenes: Query<(Entity, &BattleScene)>,
    time: Res<Time>,
) {
    let new = std::mem::take(&mut live.battles);
    if new.is_empty() {
        return;
    }
    let mesh = looks
        .mesh
        .get_or_insert_with(|| meshes.add(figure_mesh()))
        .clone();
    if looks.colours.is_empty() {
        looks.colours = [
            Color::srgb(0.78, 0.22, 0.18),
            Color::srgb(0.20, 0.36, 0.78),
            Color::srgb(0.55, 0.52, 0.48),
        ]
        .into_iter()
        .map(|c| toons.add(toon(c)))
        .collect();
    }
    // Make room: the oldest scenes go first.
    let mut old: Vec<(f32, Entity, Vec<Entity>)> = scenes
        .iter()
        .map(|(e, s)| (s.started, e, s.figures.concat()))
        .collect();
    old.sort_by(|a, b| a.0.total_cmp(&b.0));
    let room = AT_ONCE.saturating_sub(new.len().min(AT_ONCE));
    let excess = old.len().saturating_sub(room);
    for (_, e, figures) in old.into_iter().take(excess) {
        for f in figures {
            commands.entity(f).despawn();
        }
        commands.entity(e).despawn();
    }
    let skip = new.len().saturating_sub(AT_ONCE);
    for replay in new.into_iter().skip(skip) {
        let figures = replay
            .squads
            .iter()
            .map(|sq| {
                let colour = looks.colours[(sq.side as usize).min(1)].clone();
                (0..sq.start)
                    .map(|_| {
                        commands
                            .spawn((
                                Mesh3d(mesh.clone()),
                                MeshMaterial3d(colour.clone()),
                                Transform::default(),
                                Visibility::Hidden,
                            ))
                            .id()
                    })
                    .collect()
            })
            .collect();
        commands.spawn(BattleScene {
            replay,
            started: time.elapsed_secs(),
            figures,
        });
    }
}

/// `--battle LINEUP` (as in `econoplex-sim --battle`): fight it now and play
/// it beside the focus town, from shortly before the lines meet.
fn stage_battle(args: Res<crate::Args>, world: Res<World3d>, mut live: ResMut<LiveSim>) {
    use sim_core::battle::{parse_lineup, Battle, BattleParams};
    let Some(lineup) = &args.battle else { return };
    let (field, specs) = match parse_lineup(lineup) {
        Ok(x) => x,
        Err(e) => {
            warn!("--battle: {e}");
            return;
        }
    };
    let size = (field.width, field.height);
    let (report, squads, mut frames) =
        Battle::new(BattleParams::default(), field, specs, args.seed).run_recorded(5.0);
    // Skip the long approach: start a little before two enemies first meet.
    let meet = frames.iter().position(|f| {
        f.squads.iter().enumerate().any(|(a, p)| {
            f.squads.iter().enumerate().any(|(b, q)| {
                squads[a].side != squads[b].side && (p.0 - q.0).hypot(p.1 - q.1) < 60.0
            })
        })
    });
    if let Some(k) = meet {
        let from = k.saturating_sub(3);
        let t0 = frames[from].t;
        frames.drain(..from);
        for f in &mut frames {
            f.t -= t0;
        }
    }
    let town = world
        .scenario
        .towns
        .get(args.focus_town)
        .or(world.scenario.towns.first())
        .map_or((0.0, 0.0), |t| t.position);
    info!(
        "--battle {lineup}: {:?} after {:.0} s",
        report.winner, report.seconds
    );
    live.battles.push(Replay {
        town: args.focus_town as u16,
        at: (town.0, town.1 - 300.0),
        facing: (1.0, 0.0),
        field: size,
        squads,
        frames,
        winner: report.winner,
    });
}

/// Where a point of the field (metres) lies on the map.
fn on_map(r: &Replay, x: f32, y: f32) -> (f32, f32) {
    let (dx, dy) = (x - r.field.0 / 2.0, y - r.field.1 / 2.0);
    let (fx, fy) = r.facing;
    let k = METRE * SCALE;
    (
        r.at.0 + (fx * dx - fy * dy) * k,
        r.at.1 + (fy * dx + fx * dy) * k,
    )
}

fn play_battles(
    mut commands: Commands,
    scenes: Query<(Entity, &BattleScene)>,
    mut figures: Query<(
        &mut Transform,
        &mut Visibility,
        &mut MeshMaterial3d<ToonMaterial>,
    )>,
    looks: Res<Looks>,
    world: Res<World3d>,
    time: Res<Time>,
) {
    let t = &world.terrain;
    let now = time.elapsed_secs();
    for (scene, s) in &scenes {
        let r = &s.replay;
        let Some(last) = r.frames.last() else {
            continue;
        };
        let bt = (now - s.started) * PLAYBACK;
        if bt > last.t + LINGER * PLAYBACK {
            for f in s.figures.iter().flatten() {
                commands.entity(*f).despawn();
            }
            commands.entity(scene).despawn();
            continue;
        }
        // The two frames around this moment.
        let k = r.frames.partition_point(|f| f.t <= bt).max(1);
        let (a, b) = (
            &r.frames[k - 1],
            r.frames.get(k).unwrap_or(&r.frames[k - 1]),
        );
        let u = if b.t > a.t {
            ((bt - a.t) / (b.t - a.t)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        // The map's forward (from the attackers' side) as a 3D direction.
        let forward = Vec3::new(r.facing.0, 0.0, -r.facing.1).normalize_or(Vec3::X);
        let right = forward.cross(Vec3::Y);
        for (q, sq) in r.squads.iter().enumerate() {
            let (Some(fa), Some(fb)) = (a.squads.get(q), b.squads.get(q)) else {
                continue;
            };
            let (x, y) = (fa.0 + (fb.0 - fa.0) * u, fa.1 + (fb.1 - fa.1) * u);
            // Fighters fall one by one between the two frames.
            let standing = (fa.2 as f32 - (fa.2 as f32 - fb.2 as f32) * u).round() as usize;
            let state = if u < 0.5 { fa.3 } else { fb.3 };
            let centre = on_map(r, x, y);
            let gone = matches!(state, SquadState::Fled);
            let running = matches!(state, SquadState::Routing);
            // Attackers face the defenders and back; a routing squad turns.
            let mut face = if sq.side == 0 { forward } else { -forward };
            if running {
                face = -face;
            }
            let cols = (sq.start as f32).sqrt().ceil().max(1.0) as usize;
            let size = if sq.mounted { 1.35 } else { 1.0 };
            let spacing = SPACING * size;
            for (m, &e) in s.figures[q].iter().enumerate() {
                let Ok((mut tf, mut vis, mut mat)) = figures.get_mut(e) else {
                    continue;
                };
                if gone && m < standing {
                    *vis = Visibility::Hidden;
                    continue;
                }
                let (row, col) = ((m / cols) as f32, (m % cols) as f32);
                let off = right * (col - cols as f32 / 2.0) * spacing - face * row * spacing;
                let base = Vec3::new(centre.0 / SCALE, 0.0, -centre.1 / SCALE) + off;
                let map = (base.x * SCALE, -base.z * SCALE);
                let mut p = base.with_y(ground(t, map));
                let fallen = m >= standing;
                let rot = if fallen {
                    // Lying where they fell.
                    p.y += 0.05;
                    Quat::from_rotation_arc(Vec3::Y, face) * Quat::from_rotation_y(0.3 * m as f32)
                } else {
                    Quat::from_rotation_arc(Vec3::Z, face)
                };
                *tf = Transform::from_translation(p)
                    .with_rotation(rot)
                    .with_scale(Vec3::splat(size));
                *vis = Visibility::Inherited;
                let colour = if running && !fallen {
                    looks.colours[2].clone()
                } else {
                    looks.colours[(sq.side as usize).min(1)].clone()
                };
                if mat.0 != colour {
                    mat.0 = colour;
                }
            }
        }
    }
}
