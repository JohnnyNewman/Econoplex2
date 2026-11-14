//! Battles played back where they were fought.
//!
//! The simulation fights a raid's battle in one go (`sim_core::battle`) and
//! records where every squad stood every few seconds (`Replay`). This plugin
//! plays each new replay back on the field outside the defending town, fast:
//! a figure per fighter in a block formation around its squad's position,
//! attackers in red, defenders in blue, riders larger. Fighters who fall lie
//! down where they were; a routing squad turns and runs. A scene stays a few
//! seconds after the battle ends, then clears.
//!
//! A battle the player's own town fights is live instead: the simulation holds
//! its clock while the battle runs in real time, the figures follow it, and a
//! panel lists the player's squads with orders (hold, attack, shoot, withdraw,
//! or their own judgment), a button to look at the field and one to fight it
//! out at once.

use crate::live::LiveSim;
use crate::toon::{toon, ToonMaterial};
use crate::{ground, World3d, SCALE};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use sim_core::battle::{Order, Replay, ReplaySquad, SquadState};

pub struct BattlesPlugin;

impl Plugin for BattlesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Looks>()
            .init_resource::<LiveScene>()
            .add_systems(Startup, stage_battle)
            .add_systems(Update, (start_battles, play_battles, show_live).chain())
            .add_systems(EguiPrimaryContextPass, battle_panel);
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

impl Looks {
    /// The figure mesh, with the side colours made on first use.
    fn ready(
        &mut self,
        meshes: &mut Assets<Mesh>,
        toons: &mut Assets<ToonMaterial>,
    ) -> Handle<Mesh> {
        if self.colours.is_empty() {
            self.colours = [
                Color::srgb(0.78, 0.22, 0.18),
                Color::srgb(0.20, 0.36, 0.78),
                Color::srgb(0.55, 0.52, 0.48),
            ]
            .into_iter()
            .map(|c| toons.add(toon(c)))
            .collect();
        }
        self.mesh
            .get_or_insert_with(|| meshes.add(figure_mesh()))
            .clone()
    }
}

/// The figures of the battle the player is fighting, per squad.
#[derive(Resource, Default)]
struct LiveScene {
    figures: Vec<Vec<Entity>>,
    /// The town it is fought at, to tell one battle from the next.
    town: Option<u16>,
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
    let mesh = looks.ready(&mut meshes, &mut toons);
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

/// Where a battlefield lies: its centre on the map, the attackers' heading,
/// and its size in metres.
#[derive(Clone, Copy)]
struct Geo {
    at: (f32, f32),
    facing: (f32, f32),
    field: (f32, f32),
}

impl Geo {
    /// Where a point of the field (metres) lies on the map.
    fn on_map(&self, x: f32, y: f32) -> (f32, f32) {
        let (dx, dy) = (x - self.field.0 / 2.0, y - self.field.1 / 2.0);
        let (fx, fy) = self.facing;
        let k = METRE * SCALE;
        (
            self.at.0 + (fx * dx - fy * dy) * k,
            self.at.1 + (fy * dx + fx * dy) * k,
        )
    }

    /// The field's centre in 3D.
    fn centre(&self, t: &sim_core::terrain::Terrain) -> Vec3 {
        Vec3::new(self.at.0 / SCALE, ground(t, self.at), -self.at.1 / SCALE)
    }
}

type Figures<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Transform,
        &'static mut Visibility,
        &'static mut MeshMaterial3d<ToonMaterial>,
    ),
>;

/// Stands a squad's figures in block formation around (x, y) on the field:
/// the first `standing` upright, the rest lying where they fell; a routing
/// squad turns and greys, a fled one is gone.
#[allow(clippy::too_many_arguments)]
fn pose_squad(
    geo: &Geo,
    sq: &ReplaySquad,
    (x, y): (f32, f32),
    standing: usize,
    state: SquadState,
    entities: &[Entity],
    figures: &mut Figures,
    looks: &Looks,
    t: &sim_core::terrain::Terrain,
) {
    // The map's forward (from the attackers' side) as a 3D direction.
    let forward = Vec3::new(geo.facing.0, 0.0, -geo.facing.1).normalize_or(Vec3::X);
    let right = forward.cross(Vec3::Y);
    let centre = geo.on_map(x, y);
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
    for (m, &e) in entities.iter().enumerate() {
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

fn play_battles(
    mut commands: Commands,
    scenes: Query<(Entity, &BattleScene)>,
    mut figures: Figures,
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
        let geo = Geo {
            at: r.at,
            facing: r.facing,
            field: r.field,
        };
        for (q, sq) in r.squads.iter().enumerate() {
            let (Some(fa), Some(fb)) = (a.squads.get(q), b.squads.get(q)) else {
                continue;
            };
            let pos = (fa.0 + (fb.0 - fa.0) * u, fa.1 + (fb.1 - fa.1) * u);
            // Fighters fall one by one between the two frames.
            let standing = (fa.2 as f32 - (fa.2 as f32 - fb.2 as f32) * u).round() as usize;
            let state = if u < 0.5 { fa.3 } else { fb.3 };
            pose_squad(
                &geo,
                sq,
                pos,
                standing,
                state,
                &s.figures[q],
                &mut figures,
                &looks,
                t,
            );
        }
    }
}

/// Follows the battle the player is fighting: figures appear when it starts,
/// stand where its squads are, and clear when it is settled.
#[allow(clippy::too_many_arguments)]
fn show_live(
    mut commands: Commands,
    live: Res<LiveSim>,
    mut scene: ResMut<LiveScene>,
    mut looks: ResMut<Looks>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut toons: ResMut<Assets<ToonMaterial>>,
    mut figures: Figures,
    world: Res<World3d>,
) {
    let view = live.live_battle();
    let same = view.as_ref().map(|v| v.town) == scene.town
        && view
            .as_ref()
            .is_some_and(|v| v.squads.len() == scene.figures.len());
    if !same {
        for e in scene.figures.drain(..).flatten() {
            commands.entity(e).despawn();
        }
        scene.town = None;
    }
    let Some(v) = view else { return };
    if scene.figures.is_empty() {
        let mesh = looks.ready(&mut meshes, &mut toons);
        scene.figures = v
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
        scene.town = Some(v.town);
        // Spawned this frame: they can be posed from the next one.
        return;
    }
    let geo = Geo {
        at: v.at,
        facing: v.facing,
        field: v.field,
    };
    for (q, sq) in v.squads.iter().enumerate() {
        let Some(&(x, y, standing, state)) = v.frame.squads.get(q) else {
            continue;
        };
        pose_squad(
            &geo,
            sq,
            (x, y),
            standing as usize,
            state,
            &scene.figures[q],
            &mut figures,
            &looks,
            &world.terrain,
        );
    }
}

/// The battle panel: the player's squads, their state and orders.
fn battle_panel(
    mut contexts: EguiContexts,
    live: Res<LiveSim>,
    world: Res<World3d>,
    mut cameras: Query<&mut crate::camera::WorldCamera>,
) {
    let Some(v) = live.live_battle() else { return };
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let town = live
        .snapshot
        .towns
        .get(v.town as usize)
        .map_or("?", |t| t.name.as_str())
        .to_string();
    let title = if v.player_side == 0 {
        format!("Battle: you attack {town}")
    } else {
        format!("Battle: {town} is attacked")
    };
    egui::Window::new(title)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 40.0))
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            if v.finished {
                ui.label(match v.winner {
                    Some(w) if w == v.player_side => "Victory. The day is yours.",
                    Some(_) => "Defeat. Your squads fell back.",
                    None => "The fighting is over.",
                });
            } else {
                ui.label(format!("{:.0} min fought", v.frame.t / 60.0));
            }
            ui.horizontal(|ui| {
                if ui.button("Look at the field").clicked() {
                    let geo = Geo {
                        at: v.at,
                        facing: v.facing,
                        field: v.field,
                    };
                    for mut cam in &mut cameras {
                        cam.focus = geo.centre(&world.terrain);
                        cam.distance = cam.distance.min(45.0);
                    }
                }
                if !v.finished && ui.button("Fight it out").clicked() {
                    live.resolve_battle();
                }
            });
            ui.separator();
            let enemies: Vec<usize> = (0..v.squads.len())
                .filter(|&q| {
                    v.squads[q].side != v.player_side
                        && v.frame.squads.get(q).is_some_and(|f| {
                            f.2 > 0 && matches!(f.3, SquadState::Fighting | SquadState::Routing)
                        })
                })
                .collect();
            egui::Grid::new("squads").striped(true).show(ui, |ui| {
                for (q, sq) in v.squads.iter().enumerate() {
                    let Some(f) = v.frame.squads.get(q) else {
                        continue;
                    };
                    let mine = sq.side == v.player_side;
                    let side = if mine { "yours" } else { "enemy" };
                    ui.label(format!("{} ({side})", sq.name));
                    ui.label(format!("{}/{}", f.2, sq.start));
                    ui.label(format!("{:?}", f.3));
                    ui.label(format!(
                        "morale {:.0}%",
                        100.0 * v.morale.get(q).copied().unwrap_or(0.0)
                    ));
                    if mine && !v.finished && f.3 == SquadState::Fighting {
                        let order = v.orders.get(q).copied().unwrap_or(Order::Auto);
                        ui.horizontal(|ui| {
                            for (label, o) in [
                                ("Own judgment", Order::Auto),
                                ("Hold", Order::Hold),
                                ("Withdraw", Order::Withdraw),
                            ] {
                                if ui.selectable_label(order == o, label).clicked() {
                                    live.order_battle(q, o);
                                }
                            }
                            for &e in &enemies {
                                let name = &v.squads[e].name;
                                if ui
                                    .selectable_label(
                                        order == Order::Attack(e),
                                        format!("Attack {name}"),
                                    )
                                    .clicked()
                                {
                                    live.order_battle(q, Order::Attack(e));
                                }
                                if v.squads[q].name.to_lowercase().contains("bow")
                                    && ui
                                        .selectable_label(
                                            order == Order::Shoot(e),
                                            format!("Shoot {name}"),
                                        )
                                        .clicked()
                                {
                                    live.order_battle(q, Order::Shoot(e));
                                }
                            }
                        });
                    } else {
                        ui.label("");
                    }
                    ui.end_row();
                }
            });
        });
}
