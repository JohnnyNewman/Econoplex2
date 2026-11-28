//! Town walls (`sim_core::walls`) in 3D: each town's ring as its snapshot
//! has it, a palisade of stakes or a stone curtain with merlons, gates
//! between posts or gatehouse towers, battered pieces lower, a breach
//! rubble (`buildings::spawn_walls`). While a battle at the town is shown
//! (a replay or the one the player fights), its wall is drawn as that
//! battle leaves it, so a breach opens when the battle batters it down.

use crate::battles::BattleScene;
use crate::buildings::{spawn_walls, Kit, WallSpec};
use crate::live::LiveSim;
use crate::outline::Outlines;
use crate::{ground, World3d, SCALE};
use bevy::prelude::*;
use sim_core::battle::ReplayWall;

pub struct Walls3dPlugin;

impl Plugin for Walls3dPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DrawnWalls>()
            .add_systems(Update, draw_walls);
    }
}

/// Per town: the wall as last drawn, and its mesh.
#[derive(Resource, Default)]
struct DrawnWalls(Vec<(Vec<WallSpec>, Option<Entity>)>);

/// A share of hit points, in tenths: a piece is drawn again only when it
/// has lost a tenth more.
fn tenths(share: f32) -> f32 {
    (share.clamp(0.0, 1.0) * 10.0).ceil() / 10.0
}

/// A battle's wall on the map (`on_map`: field metres to map units), at
/// hit points `hp`.
fn battle_specs(
    walls: &[ReplayWall],
    hp: &[f32],
    on_map: impl Fn(f32, f32) -> (f32, f32),
    to_3d: &impl Fn((f32, f32)) -> Vec3,
) -> Vec<WallSpec> {
    walls
        .iter()
        .enumerate()
        .filter(|(_, w)| w.max > 0.0)
        .map(|(k, w)| {
            let hp = hp.get(k).copied().unwrap_or(w.max);
            WallSpec {
                a: to_3d(on_map(w.a.0, w.a.1)),
                b: to_3d(on_map(w.b.0, w.b.1)),
                // Thicker than a metre: stone (`assets/content/47_walls.ron`).
                stone: 2.0 * w.half_width > 1.0,
                gate: w.gate,
                standing: tenths(hp / w.max),
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn draw_walls(
    mut commands: Commands,
    live: Res<LiveSim>,
    world: Res<World3d>,
    kit: Option<ResMut<Kit>>,
    outlines: Option<Res<Outlines>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut drawn: ResMut<DrawnWalls>,
    scenes: Query<&BattleScene>,
    time: Res<Time>,
) {
    let (Some(mut kit), Some(outlines)) = (kit, outlines) else {
        return;
    };
    let t = &world.terrain;
    let to_3d = |m: (f32, f32)| Vec3::new(m.0 / SCALE, ground(t, m), -m.1 / SCALE);
    let snap = live.snapshot.clone();
    let view = live.live_battle();
    let now = time.elapsed_secs();
    drawn.0.resize_with(snap.towns.len(), Default::default);
    for (ti, town) in snap.towns.iter().enumerate() {
        // A battle shown at the town draws its wall as the battle has it.
        let fought = view
            .as_ref()
            .filter(|v| v.town as usize == ti && !v.walls.is_empty())
            .map(|v| {
                let geo = crate::battles::Geo::of(v);
                battle_specs(&v.walls, &v.frame.walls, |x, y| geo.on_map(x, y), &to_3d)
            })
            .or_else(|| {
                scenes
                    .iter()
                    .filter(|s| s.replay().town as usize == ti && !s.replay().walls.is_empty())
                    .max_by(|a, b| a.started().total_cmp(&b.started()))
                    .map(|s| {
                        let r = s.replay();
                        let hp = r
                            .around(s.playback(now))
                            .map_or(Vec::new(), |(a, _, _)| a.walls.clone());
                        let on_map = |x: f32, y: f32| {
                            sim_core::battle::field_to_map(r.at, r.facing, r.field, (x, y))
                        };
                        battle_specs(&r.walls, &hp, on_map, &to_3d)
                    })
            });
        let specs = fought.unwrap_or_else(|| {
            town.layout
                .walls
                .iter()
                .filter_map(|q| {
                    let def = q
                        .built
                        .and_then(|b| live.wall_defs.get(b as usize).copied().flatten())?;
                    Some(WallSpec {
                        a: to_3d(q.a),
                        b: to_3d(q.b),
                        stone: def.thickness > 1.0,
                        gate: q.gate,
                        standing: tenths(q.hp / def.hp.max(1e-3)),
                    })
                })
                .collect()
        });
        let (was, entity) = &mut drawn.0[ti];
        if *was == specs {
            continue;
        }
        if let Some(e) = entity.take() {
            commands.entity(e).despawn();
        }
        *entity = spawn_walls(&mut commands, &mut kit, &mut meshes, &outlines, &specs);
        *was = specs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_piece_is_drawn_again_only_a_tenth_later() {
        assert_eq!(tenths(1.0), 1.0);
        assert_eq!(tenths(0.95), 1.0);
        assert_eq!(tenths(0.85), 0.9);
        assert_eq!(tenths(0.0), 0.0);
        let walls = [ReplayWall {
            a: (0.0, 0.0),
            b: (10.0, 0.0),
            half_width: 0.75,
            gate: true,
            max: 900.0,
        }];
        let specs = battle_specs(&walls, &[0.0], |x, y| (x, y), &|m| Vec3::new(m.0, 0.0, m.1));
        assert_eq!(specs.len(), 1);
        assert!(specs[0].stone && specs[0].gate);
        assert_eq!(specs[0].standing, 0.0, "a breach");
    }
}
