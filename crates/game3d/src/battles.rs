//! Battles played back where they were fought.
//!
//! The simulation fights a raid's battle in one go (`sim_core::battle`) and
//! records where every squad stood every few seconds (`Replay`). This plugin
//! plays each new replay back on the field outside the defending town, fast:
//! a figure per fighter in a block formation around its squad's position,
//! attackers in red, defenders in blue, riders larger. Fighters who fall lie
//! down where they were; a routing squad turns and runs. A scene stays a few
//! seconds after the battle ends, then clears. A battle fought on the soldier
//! model records every soldier in each frame: then each figure walks from
//! where its soldier stood in one frame to where they stood in the next.
//!
//! A battle the player's own town fights is live instead: the simulation holds
//! its clock while the battle runs in real time, the figures follow it, and a
//! panel lists the player's squads with orders (hold, attack, shoot, withdraw,
//! or their own judgment), a button to look at the field and one to fight it
//! out at once. Live battles fight on `battle.live_model`, the soldier model
//! by default: then each figure stands where its soldier stands, facing their
//! way, so the squads take the shape their soldiers give them, and the panel
//! also offers to flank an enemy squad and to set a squad's formation.
//!
//! Squads are also selected and ordered with the mouse (battle_input.rs);
//! the panel then marks the selected squads and offers buttons for their
//! stance (fire at will, hold fire, skirmish; tight, loose, own order) and
//! for what they do with enemies who run (kill, capture, let go). With the
//! overlay on (`O`) the figures take their soldier's morale and fatigue as
//! colour instead of their side's.

use crate::battle_input::BattleControls;
use crate::live::LiveSim;
use crate::toon::{toon, ToonMaterial};
use crate::{ground, World3d, SCALE};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use sim_core::battle::rts::{self, StanceKey};
use sim_core::battle::{
    Command, Condition, FighterState, FrameSoldier, LiveView, Order, Replay, ReplaySquad,
    SquadState,
};

pub struct BattlesPlugin;

impl Plugin for BattlesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Looks>()
            .init_resource::<LiveScene>()
            .init_resource::<Staged>()
            .add_systems(Update, (stage_battle, start_battles, play_battles, show_live).chain())
            .add_systems(EguiPrimaryContextPass, battle_panel);
    }
}

/// Battle seconds shown per real second.
const PLAYBACK: f32 = 30.0;
/// 3D units per metre of battlefield, as the sim lays a field over a town (a
/// figure is about 0.75 units tall), and between figures in a formation.
const METRE: f32 = sim_core::battle::MAP_PER_METRE / SCALE;
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
    /// The overlay's colours, by morale (`MORALE_STEPS`) then fatigue
    /// (`FATIGUE_STEPS`).
    overlay: Vec<Handle<ToonMaterial>>,
}

const MORALE_STEPS: usize = 6;
const FATIGUE_STEPS: usize = 3;

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
            self.overlay = (0..MORALE_STEPS * FATIGUE_STEPS)
                .map(|k| {
                    let m = (k / FATIGUE_STEPS) as f32 / (MORALE_STEPS - 1) as f32;
                    let f = (k % FATIGUE_STEPS) as f32 / (FATIGUE_STEPS - 1) as f32;
                    let [r, g, b] = rts::condition_colour(Condition::new(m, f));
                    toons.add(toon(Color::linear_rgb(r, g, b)))
                })
                .collect();
        }
        self.mesh
            .get_or_insert_with(|| meshes.add(figure_mesh()))
            .clone()
    }

    /// The overlay's colour nearest a soldier's condition.
    fn condition(&self, c: Condition) -> Handle<ToonMaterial> {
        let m = (c.morale() * (MORALE_STEPS - 1) as f32).round() as usize;
        let f = (c.fatigue() * (FATIGUE_STEPS - 1) as f32).round() as usize;
        self.overlay[m.min(MORALE_STEPS - 1) * FATIGUE_STEPS + f.min(FATIGUE_STEPS - 1)].clone()
    }
}

/// The figures of the battle the player is fighting, per squad.
#[derive(Resource, Default)]
struct LiveScene {
    figures: Vec<Vec<Entity>>,
    /// The town it is fought at, to tell one battle from the next.
    town: Option<u16>,
}

/// Where on the map the lines of the `--battle` line-up meet, once staged.
#[derive(Resource, Default)]
pub(crate) struct Staged(pub Option<(f32, f32)>);

#[derive(Component)]
pub(crate) struct BattleScene {
    replay: Replay,
    started: f32,
    /// Figures per squad, in formation order.
    figures: Vec<Vec<Entity>>,
}

impl BattleScene {
    /// How far (battle seconds) its playback has got at time `now`.
    pub(crate) fn playback(&self, now: f32) -> f32 {
        (now - self.started) * PLAYBACK
    }
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
        let first = replay.frames.first();
        let figures = replay
            .squads
            .iter()
            .enumerate()
            .map(|(q, sq)| {
                let colour = looks.colours[(sq.side as usize).min(1)].clone();
                // A figure per soldier on the soldier model, else per fighter.
                let n = first
                    .and_then(|f| f.soldiers.get(q))
                    .map_or(sq.start as usize, Vec::len);
                (0..n)
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
/// it beside the focus town, from shortly before the lines meet. A line-up
/// `in town` is fought in the focus town itself, among its real buildings
/// and houses (from the first snapshot), where a raid on it would be, as the
/// 2D client stages it.
fn stage_battle(
    args: Res<crate::Args>,
    world: Res<World3d>,
    mut live: ResMut<LiveSim>,
    mut staged: ResMut<Staged>,
    mut done: Local<bool>,
) {
    use sim_core::battle::{parse_lineup, Battle};
    if *done {
        return;
    }
    let Some(lineup) = &args.battle else {
        *done = true;
        return;
    };
    let (mut field, specs) = match parse_lineup(lineup) {
        Ok(x) => x,
        Err(e) => {
            warn!("--battle: {e}");
            *done = true;
            return;
        }
    };
    let in_town = !field.obstacles.is_empty();
    let layout = live
        .snapshot
        .towns
        .get(args.focus_town)
        .map(|t| t.layout.clone());
    if in_town && layout.is_none() {
        return;
    }
    *done = true;
    let size = (field.width, field.height);
    let town = world
        .scenario
        .towns
        .get(args.focus_town)
        .or(world.scenario.towns.first())
        .map_or((0.0, 0.0), |t| t.position);
    let (at, facing) = if in_town {
        ((town.0 - 60.0, town.1), (1.0, 0.0))
    } else {
        ((town.0, town.1 - 300.0), (1.0, 0.0))
    };
    if let (true, Some(layout)) = (in_town, &layout) {
        field.obstacles = layout.obstacles_on(at, facing, size);
    }
    // On `battle.model` from models.ron (`--preset`, `--set`).
    let (report, squads, frames) =
        Battle::new(args.battle_params.clone(), field, specs, args.seed).run_recorded(5.0);
    info!(
        "--battle {lineup}: {:?} after {:.0} s",
        report.winner, report.seconds
    );
    let mut replay = Replay {
        town: args.focus_town as u16,
        at,
        facing,
        field: size,
        squads,
        frames,
        winner: report.winner,
    };
    let geo = Geo {
        at: replay.at,
        facing: replay.facing,
        field: replay.field,
    };
    staged.0 = replay.skip_approach().map(|(x, y)| geo.on_map(x, y));
    live.battles.push(replay);
}

/// Where a battlefield lies: its centre on the map, the attackers' heading,
/// and its size in metres.
#[derive(Clone, Copy)]
pub(crate) struct Geo {
    pub at: (f32, f32),
    pub facing: (f32, f32),
    pub field: (f32, f32),
}

impl Geo {
    /// A live battle's field.
    pub(crate) fn of(v: &LiveView) -> Self {
        Self {
            at: v.at,
            facing: v.facing,
            field: v.field,
        }
    }

    /// Where a point of the map lies on the field (metres), for clicks.
    pub(crate) fn field_at(&self, m: (f32, f32)) -> (f32, f32) {
        sim_core::battle::map_to_field(self.at, self.facing, self.field, m)
    }

    /// A point of the field in 3D, on the ground.
    pub(crate) fn point_3d(&self, t: &sim_core::terrain::Terrain, (x, y): (f32, f32)) -> Vec3 {
        let m = self.on_map(x, y);
        Vec3::new(m.0 / SCALE, ground(t, m), -m.1 / SCALE)
    }

    /// Where a point of the field (metres) lies on the map.
    pub(crate) fn on_map(&self, x: f32, y: f32) -> (f32, f32) {
        sim_core::battle::field_to_map(self.at, self.facing, self.field, (x, y))
    }

    /// A direction on the field as a 3D direction (unit, level).
    fn dir_3d(&self, (dx, dy): (f32, f32)) -> Vec3 {
        let (fx, fy) = self.facing;
        let (mx, my) = (fx * dx - fy * dy, fy * dx + fx * dy);
        Vec3::new(mx, 0.0, -my).normalize_or(Vec3::X)
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

/// Where a soldier stands in 3D (before the ground's height), which way
/// they face, and whether they lie fallen; `None` for one who left the field.
fn soldier_pose(geo: &Geo, m: &FrameSoldier) -> Option<(Vec3, Vec3, bool)> {
    if m.state == FighterState::Fled {
        return None;
    }
    let (fx, fy) = m.pos();
    let (x, y) = geo.on_map(fx, fy);
    // A runner turns to face where they run, as the model turns them.
    let face = geo.dir_3d(m.facing());
    Some((
        Vec3::new(x / SCALE, 0.0, -y / SCALE),
        face,
        m.state == FighterState::Down,
    ))
}

/// Stands each figure of a squad on the soldier model where its soldier
/// stands: upright and facing their way, lying where they fell, grey when
/// they run, gone when they left the field. With `overlay`, those still on
/// their feet take their condition's colour.
#[allow(clippy::too_many_arguments)]
fn pose_soldiers(
    geo: &Geo,
    sq: &ReplaySquad,
    men: &[FrameSoldier],
    overlay: Option<&[Condition]>,
    entities: &[Entity],
    figures: &mut Figures,
    looks: &Looks,
    t: &sim_core::terrain::Terrain,
) {
    let size = if sq.mounted { 1.35 } else { 1.0 };
    for (k, &e) in entities.iter().enumerate() {
        let Ok((mut tf, mut vis, mut mat)) = figures.get_mut(e) else {
            continue;
        };
        let Some((base, face, fallen)) = men.get(k).and_then(|m| soldier_pose(geo, m)) else {
            *vis = Visibility::Hidden;
            continue;
        };
        let mut p = base.with_y(ground(t, (base.x * SCALE, -base.z * SCALE)));
        let rot = if fallen {
            p.y += 0.05;
            Quat::from_rotation_arc(Vec3::Y, face) * Quat::from_rotation_y(0.3 * k as f32)
        } else {
            Quat::from_rotation_arc(Vec3::Z, face)
        };
        *tf = Transform::from_translation(p)
            .with_rotation(rot)
            .with_scale(Vec3::splat(size));
        *vis = Visibility::Inherited;
        let running = men[k].state == FighterState::Routing;
        let condition = overlay.and_then(|c| c.get(k)).filter(|_| !fallen);
        let colour = if let Some(&c) = condition {
            looks.condition(c)
        } else if running {
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
        // On the soldier model, every soldier between the two frames.
        let men = r.soldiers_at(bt);
        for (q, sq) in r.squads.iter().enumerate() {
            if let Some(men) = men.get(q) {
                pose_soldiers(&geo, sq, men, None, &s.figures[q], &mut figures, &looks, t);
                continue;
            }
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
    controls: Res<BattleControls>,
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
            .enumerate()
            .map(|(q, sq)| {
                let colour = looks.colours[(sq.side as usize).min(1)].clone();
                // A figure per soldier on the soldier model, else per fighter.
                let n = v.frame.soldiers.get(q).map_or(sq.start as usize, Vec::len);
                (0..n)
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
        if let Some(men) = v.frame.soldiers.get(q) {
            let overlay = controls.overlay.then(|| v.condition.get(q)).flatten();
            pose_soldiers(
                &geo,
                sq,
                men,
                overlay.map(Vec::as_slice),
                &scene.figures[q],
                &mut figures,
                &looks,
                &world.terrain,
            );
            continue;
        }
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

/// Points the camera at a live battle, from no further than `distance`: at
/// the centre of the soldiers still on the field on the soldier model, else
/// at the field's centre.
pub(crate) fn look_at_field(
    cam: &mut crate::camera::WorldCamera,
    v: &sim_core::battle::LiveView,
    t: &sim_core::terrain::Terrain,
    distance: f32,
) {
    let geo = Geo {
        at: v.at,
        facing: v.facing,
        field: v.field,
    };
    let on: Vec<(f32, f32)> = v
        .frame
        .soldiers
        .iter()
        .flatten()
        .filter(|m| m.state != FighterState::Fled)
        .map(|m| {
            let (x, y) = m.pos();
            geo.on_map(x, y)
        })
        .collect();
    cam.focus = if on.is_empty() {
        geo.centre(t)
    } else {
        let n = on.len() as f32;
        let at = (
            on.iter().map(|p| p.0).sum::<f32>() / n,
            on.iter().map(|p| p.1).sum::<f32>() / n,
        );
        Vec3::new(at.0 / SCALE, ground(t, at), -at.1 / SCALE)
    };
    cam.distance = cam.distance.min(distance);
}

/// The battle panel: the player's squads, their state and orders.
fn battle_panel(
    mut contexts: EguiContexts,
    live: Res<LiveSim>,
    world: Res<World3d>,
    mut cameras: Query<&mut crate::camera::WorldCamera>,
    controls: Res<BattleControls>,
) {
    let Some(v) = live.live_battle() else { return };
    let sel = &controls.rts.selection.squads;
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
            } else if v.deploying {
                ui.label("Deploy your squads: right-click or right-drag them into the green zone");
            } else {
                ui.label(format!("{:.0} min fought", v.frame.t / 60.0));
            }
            ui.horizontal(|ui| {
                if ui.button("Look at the field").clicked() {
                    for mut cam in &mut cameras {
                        look_at_field(&mut cam, &v, &world.terrain, 45.0);
                    }
                }
                if v.deploying && ui.button("Start battle").clicked() {
                    live.start_battle();
                }
                if !v.finished && ui.button("Fight it out").clicked() {
                    live.resolve_battle();
                }
            });
            ui.separator();
            // The selection and its stance.
            if sel.is_empty() {
                ui.label(
                    "Select squads with the mouse: click, shift-click or drag a box (F1: controls)",
                );
            } else if !v.finished {
                let names: Vec<&str> = sel.iter().map(|&q| v.squads[q].name.as_str()).collect();
                ui.label(format!("Selected: {}", names.join(", ")));
                ui.horizontal_wrapped(|ui| {
                    for &(key, label) in StanceKey::ALL.iter() {
                        if ui.selectable_label(key.chosen(&v, sel), label).clicked() {
                            for (q, c) in rts::stance(&v, sel, key) {
                                live.command_battle(q, c);
                            }
                        }
                    }
                    let queued = sel
                        .iter()
                        .any(|&q| v.orders.get(q).is_some_and(|o| o.len() > 1));
                    if queued && ui.button("Clear queue").clicked() {
                        for (q, c) in rts::clear_queue(&v, sel) {
                            live.command_battle(q, c);
                        }
                    }
                });
            }
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
                    let side = if !mine {
                        "enemy"
                    } else if sel.contains(&q) {
                        "yours, selected"
                    } else {
                        "yours"
                    };
                    ui.label(format!("{} ({side})", sq.name));
                    ui.label(format!("{}/{}", f.2, sq.start));
                    ui.label(format!("{:?}", f.3));
                    ui.label(format!(
                        "morale {:.0}%",
                        100.0 * v.morale.get(q).copied().unwrap_or(0.0)
                    ));
                    if mine && !v.finished && f.3 == SquadState::Fighting {
                        // A single order in force is shown chosen; moves
                        // (going round, re-forming) never are.
                        let single = v.orders.get(q).map_or(0, Vec::len) <= 1;
                        let order = v.order_now(q);
                        let on = |o: Order| single && order == o;
                        let send =
                            |orders: Vec<Order>| live.command_battle(q, Command::Orders(orders));
                        ui.horizontal(|ui| {
                            for (label, o) in [
                                ("Own judgment", Order::Auto),
                                ("Hold", Order::Hold),
                                ("Withdraw", Order::Withdraw),
                            ] {
                                if ui.selectable_label(on(o), label).clicked() {
                                    send(vec![o]);
                                }
                            }
                            for &e in &enemies {
                                let name = &v.squads[e].name;
                                if ui
                                    .selectable_label(
                                        on(Order::Attack(e)),
                                        format!("Attack {name}"),
                                    )
                                    .clicked()
                                {
                                    send(vec![Order::Attack(e)]);
                                }
                                // Only the soldier model has sides to go round:
                                // moves along the enemy's side, then the attack.
                                if v.on_soldiers()
                                    && ui
                                        .selectable_label(false, format!("Flank {name}"))
                                        .clicked()
                                {
                                    send(v.flank(q, e));
                                }
                                if v.squads[q].name.to_lowercase().contains("bow")
                                    && ui
                                        .selectable_label(
                                            on(Order::Shoot(e)),
                                            format!("Shoot {name}"),
                                        )
                                        .clicked()
                                {
                                    send(vec![Order::Shoot(e)]);
                                }
                            }
                            // Fronts, on the soldier model: re-form where it
                            // stands so many files wide, then carry on.
                            if v.on_soldiers() {
                                ui.separator();
                                let files =
                                    v.lines.get(q).map(|l| (l.width / l.spacing).round() as u16);
                                for (label, n) in LiveView::FRONTS {
                                    if ui.selectable_label(files == Some(n), label).clicked() {
                                        send(v.reform(q, n));
                                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn man(x: f32, y: f32, facing: (f32, f32), state: FighterState) -> FrameSoldier {
        FrameSoldier::new(x, y, facing, state)
    }

    /// A figure of a soldier battle stands where its soldier stands on the
    /// map, faces their way, and is gone once they leave the field.
    #[test]
    fn a_soldier_figure_stands_and_faces_as_its_soldier() {
        // The field's +x points north on the map (+y), which is 3D -z.
        let geo = Geo {
            at: (100.0, 50.0),
            facing: (0.0, 1.0),
            field: (200.0, 80.0),
        };
        let m = man(150.0, 40.0, (1.0, 0.0), FighterState::Standing);
        let (p, face, down) = soldier_pose(&geo, &m).unwrap();
        let (x, y) = geo.on_map(150.0, 40.0);
        assert!((p - Vec3::new(x / SCALE, 0.0, -y / SCALE)).length() < 1e-5);
        assert!((face - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5, "{face}");
        assert!(!down);
        // 50 m toward the defenders is 50 metres' worth of map units north.
        assert!((y - 50.0 - 50.0 * METRE * SCALE).abs() < 1e-3 && (x - 100.0).abs() < 1e-3);
        let fallen = man(150.0, 40.0, (1.0, 0.0), FighterState::Down);
        assert!(soldier_pose(&geo, &fallen).unwrap().2);
        let fled = man(0.0, 0.0, (1.0, 0.0), FighterState::Fled);
        assert!(soldier_pose(&geo, &fled).is_none());
    }

    /// A click on the map finds the field point drawn there.
    #[test]
    fn a_map_point_goes_back_to_its_field_point() {
        let geo = Geo {
            at: (100.0, 50.0),
            facing: (0.6, 0.8),
            field: (200.0, 80.0),
        };
        for (x, y) in [(0.0, 0.0), (150.0, 40.0), (37.0, 71.0)] {
            let back = geo.field_at(geo.on_map(x, y));
            assert!(
                (back.0 - x).abs() < 1e-3 && (back.1 - y).abs() < 1e-3,
                "{back:?}"
            );
        }
    }
}
