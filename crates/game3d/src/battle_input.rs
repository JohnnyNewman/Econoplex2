//! RTS controls for the live battle in the 3D client, on the shared logic in
//! `sim_core::battle::rts`, as the 2D client's battle_input.rs (the same
//! controls; docs/design/client-parity.md, group 4).
//!
//! While a live battle waits:
//!
//! - left-click a squad of yours to select it, shift-click to add or remove
//!   one, drag a box to select several, click empty ground to select none;
//! - right-click an enemy squad to attack it (archers shoot), right-click
//!   ground to move there, right-drag to lay out the line (its ghost and
//!   arrival time follow the pointer), double-click to run. With no squad
//!   selected, or off the field, right-drag turns the camera as always;
//! - Ctrl+1..9 make the selection a group, 1..9 select it, twice to look
//!   at it (the number keys do not switch shader views meanwhile);
//! - Z fire at will, X hold fire, C skirmish; V tight, B loose, N own order
//!   (with squads selected; otherwise the keys do what they always do);
//! - O colours the soldiers by morale (red breaking, green steady), darker
//!   the more tired;
//! - hovering an enemy squad says how your squads fare against it;
//! - Space still pauses, and orders can be given while paused;
//! - F1 lists every control.
//!
//! The mouse and keys are read in `PreUpdate`: a press the battle takes is
//! cleared, so the figure picker and the shader keys never see it, and the
//! camera is told not to turn while the battle holds the right button.

use crate::battles::Geo;
use crate::camera::{BattleHolds, UiCapture, WorldCamera};
use crate::live::LiveSim;
use crate::{World3d, SCALE};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use sim_core::battle::rts::{self, Gesture, Pointer, Rts, StanceKey};
use sim_core::battle::{Fire, LiveView, Preview, Spacing};

pub struct BattleInputPlugin;

impl Plugin for BattleInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_gizmo_group::<BattleGizmos>()
            .init_resource::<BattleControls>()
            .init_resource::<BattleHolds>()
            .init_resource::<ControlsHelp>()
            .add_systems(PreUpdate, battle_input.after(bevy::input::InputSystems))
            .add_systems(Startup, configure_gizmos)
            .add_systems(Update, draw_battle_controls)
            .add_systems(EguiPrimaryContextPass, (controls_tip, controls_help));
    }
}

/// The selection and the ghost, drawn over roofs and figures, so a line laid
/// out in a town still shows.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct BattleGizmos;

fn configure_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<BattleGizmos>();
    config.depth_bias = -1.0;
    config.line.width = 2.5;
}

/// What `--rts-demo` shows, for screenshots without a mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Demo {
    /// Every squad of yours selected, and the ghost of a line ahead of them.
    Ghost,
    /// The morale and fatigue overlay.
    Overlay,
}

impl std::str::FromStr for Demo {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "ghost" => Ok(Self::Ghost),
            "overlay" => Ok(Self::Overlay),
            _ => Err(format!("--rts-demo {s}: ghost or overlay")),
        }
    }
}

/// The battle controls' state.
#[derive(Resource, Default)]
pub struct BattleControls {
    pub rts: Rts,
    /// Soldiers coloured by morale and fatigue (`O`).
    pub overlay: bool,
    pub demo: Option<Demo>,
    pointer: Option<Pointer>,
    hint: Option<String>,
}

impl BattleControls {
    pub fn new(demo: Option<Demo>) -> Self {
        Self {
            demo,
            ..Default::default()
        }
    }
}

/// `F1`: the controls window.
#[derive(Resource, Default)]
struct ControlsHelp(bool);

const STANCE_KEYS: [(KeyCode, StanceKey); 6] = [
    (KeyCode::KeyZ, StanceKey::Fire(Fire::AtWill)),
    (KeyCode::KeyX, StanceKey::Fire(Fire::Hold)),
    (KeyCode::KeyC, StanceKey::Fire(Fire::Skirmish)),
    (KeyCode::KeyV, StanceKey::Spacing(Spacing::Tight)),
    (KeyCode::KeyB, StanceKey::Spacing(Spacing::Loose)),
    (KeyCode::KeyN, StanceKey::Spacing(Spacing::Own)),
];

const DIGITS: [KeyCode; 9] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];

/// Where a ray from the camera meets the ground: a few steps between the
/// ray and the terrain's height under it.
fn ray_ground(ray: Ray3d, t: &sim_core::terrain::Terrain, h0: f32) -> Option<Vec3> {
    let d = *ray.direction;
    if d.y > -1e-4 {
        return None;
    }
    let mut h = h0;
    let mut p = ray.origin;
    for _ in 0..5 {
        let s = (h - ray.origin.y) / d.y;
        if s < 0.0 {
            return None;
        }
        p = ray.origin + d * s;
        h = crate::ground(t, (p.x * SCALE, -p.z * SCALE));
    }
    Some(p.with_y(h))
}

/// Read the mouse and keys for the live battle, and give its commands.
#[allow(clippy::too_many_arguments)]
fn battle_input(
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut world_cams: Query<&mut WorldCamera>,
    ui: Res<UiCapture>,
    time: Res<Time<Real>>,
    live: Res<LiveSim>,
    world: Res<World3d>,
    mut controls: ResMut<BattleControls>,
    mut holds: ResMut<BattleHolds>,
    mut help: ResMut<ControlsHelp>,
) {
    if keys.just_pressed(KeyCode::F1) {
        help.0 = !help.0;
    }
    let Some(v) = live.live_battle() else {
        controls.rts.selection.squads.clear();
        controls.pointer = None;
        controls.hint = None;
        holds.0 = false;
        return;
    };
    let v = &*v;
    if controls.demo == Some(Demo::Ghost) {
        controls.rts.selection.squads = (0..v.squads.len())
            .filter(|&q| rts::selectable(v, q))
            .collect();
    }
    controls.overlay |= controls.demo == Some(Demo::Overlay);
    controls.rts.selection.tidy(v);
    let geo = Geo::of(v);
    let t = &world.terrain;
    let camera = cameras.iter().find(|(c, _)| c.is_active);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let cursor = windows.iter().next().and_then(|w| w.cursor_position());
    let h0 = geo.point_3d(t, (v.field.0 / 2.0, v.field.1 / 2.0)).y;
    let p = cursor.map(|c| Pointer {
        screen: (c.x, c.y),
        field: camera
            .filter(|_| !ui.pointer)
            .and_then(|(cam, tf)| cam.viewport_to_world(tf, c).ok())
            .and_then(|ray| ray_ground(ray, t, h0))
            .map(|g| geo.field_at((g.x * SCALE, -g.z * SCALE)))
            .filter(|f| (0.0..=v.field.0).contains(&f.0) && (0.0..=v.field.1).contains(&f.1)),
        shift,
        now: time.elapsed_secs_f64(),
    });
    let mut commands = Vec::new();
    if let Some(p) = p {
        if mouse.just_pressed(MouseButton::Left) && controls.rts.left_down(p) {
            mouse.clear_just_pressed(MouseButton::Left);
        }
        if mouse.just_released(MouseButton::Left) {
            let project = |f: (f32, f32)| {
                let (cam, tf) = camera?;
                cam.world_to_viewport(tf, geo.point_3d(t, f))
                    .ok()
                    .map(|s| (s.x, s.y))
            };
            controls.rts.left_up(v, p, project);
        }
        if mouse.just_pressed(MouseButton::Right) && controls.rts.right_down(v, p) {
            mouse.clear_just_pressed(MouseButton::Right);
        }
        if mouse.just_released(MouseButton::Right) {
            commands.extend(controls.rts.right_up(v, p));
        }
    }
    holds.0 = controls.rts.holds_right();
    if !ui.keyboard {
        let mut look = None;
        for (n, k) in DIGITS.iter().enumerate() {
            if keys.just_pressed(*k) {
                keys.clear_just_pressed(*k);
                let now = time.elapsed_secs_f64();
                look = controls.rts.group_key(v, n, ctrl, now).or(look);
            }
        }
        if !controls.rts.selection.squads.is_empty() {
            for (k, key) in STANCE_KEYS {
                if keys.just_pressed(k) {
                    keys.clear_just_pressed(k);
                    commands.extend(rts::stance(v, &controls.rts.selection.squads, key));
                }
            }
        }
        if keys.just_pressed(KeyCode::KeyO) {
            keys.clear_just_pressed(KeyCode::KeyO);
            controls.overlay = !controls.overlay;
        }
        if keys.just_pressed(KeyCode::Escape) && !controls.rts.selection.squads.is_empty() {
            keys.clear_just_pressed(KeyCode::Escape);
            controls.rts.selection.squads.clear();
        }
        // The demos are shown from above: the ghost between its squads and
        // its line, the overlay over every squad.
        let look = look.or_else(|| match controls.demo? {
            Demo::Ghost => {
                let sel = &controls.rts.selection.squads;
                let (from, to) = rts::line_ahead(v, sel, 60.0)?;
                let c = rts::centre(v, sel)?;
                let mid = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
                Some(((c.0 + mid.0) / 2.0, (c.1 + mid.1) / 2.0))
            }
            Demo::Overlay => {
                let all: Vec<usize> = (0..v.squads.len()).collect();
                rts::centre(v, &all)
            }
        });
        if let Some(f) = look {
            for mut cam in &mut world_cams {
                cam.focus = geo.point_3d(t, f);
            }
        }
    }
    for (q, c) in commands {
        live.command_battle(q, c);
    }
    controls.hint = p.and_then(|p| {
        let e = controls.rts.hover(v, p)?;
        rts::hint(v, &controls.rts.selection.squads, e)
    });
    controls.pointer = p;
}

/// The ghost being dragged (or shown by `--rts-demo ghost`): the gestures
/// to draw, and each squad's ghost.
fn ghost_of(v: &LiveView, c: &BattleControls) -> (Gesture, Vec<(usize, Preview)>) {
    if c.demo == Some(Demo::Ghost) {
        let sel = &c.rts.selection.squads;
        if let Some((from, to)) = rts::line_ahead(v, sel, 60.0) {
            return (
                Gesture::Line(from, to, false),
                rts::ghost(v, sel, from, to, false),
            );
        }
    }
    match c.pointer {
        Some(p) => (c.rts.gesture(p), c.rts.ghost(v, p)),
        None => (Gesture::None, Vec::new()),
    }
}

const SELECTED: Color = Color::srgb(1.0, 0.95, 0.35);
const GHOST: Color = Color::srgba(0.75, 0.95, 1.0, 0.9);

/// Draw the selected squads' blocks and the ghost line on the ground.
fn draw_battle_controls(
    mut gizmos: Gizmos<BattleGizmos>,
    live: Res<LiveSim>,
    world: Res<World3d>,
    controls: Res<BattleControls>,
) {
    let Some(v) = live.live_battle() else { return };
    let v = &*v;
    let geo = Geo::of(v);
    let t = &world.terrain;
    // Above the grass.
    let lift = Vec3::Y * 0.2;
    let at = |p: (f32, f32)| geo.point_3d(t, p) + lift;
    for &q in &controls.rts.selection.squads {
        if let Some(c) = rts::outline(v, q) {
            for k in 0..4 {
                gizmos.line(at(c[k]), at(c[(k + 1) % 4]), SELECTED);
            }
        }
    }
    let (gesture, ghosts) = ghost_of(v, &controls);
    if let Gesture::Line(a, b, _) = gesture {
        gizmos.line(at(a), at(b), GHOST);
    }
    let flat = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    for (_, g) in &ghosts {
        for &s in &g.slots {
            gizmos
                .circle(Isometry3d::new(at(s), flat), 0.06, GHOST)
                .resolution(6);
        }
    }
}

/// The selection box, and the text by the pointer: the matchup hint, or
/// the ghost's arrival time.
fn controls_tip(mut contexts: EguiContexts, live: Res<LiveSim>, controls: Res<BattleControls>) {
    let Some(v) = live.live_battle() else { return };
    let Ok(ctx) = contexts.ctx_mut() else { return };
    if controls.overlay && v.on_soldiers() {
        overlay_legend(ctx);
    }
    let (gesture, ghosts) = ghost_of(&v, &controls);
    if let Gesture::Box(a, b) = gesture {
        let rect = egui::Rect::from_two_pos(egui::pos2(a.0, a.1), egui::pos2(b.0, b.1));
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("battle box"),
        ))
        .rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 240, 90)),
            egui::StrokeKind::Middle,
        );
    }
    let words = if let Some(eta) = ghosts.iter().map(|g| g.1.eta).reduce(f32::max) {
        let run = matches!(gesture, Gesture::Line(_, _, true));
        let how = if run { "running" } else { "marching" };
        Some(format!("there in {eta:.0} s, {how}"))
    } else {
        controls.hint.clone()
    };
    let Some(words) = words else { return };
    let at = controls
        .pointer
        .map(|p| egui::pos2(p.screen.0 + 16.0, p.screen.1 + 16.0))
        .unwrap_or_else(|| ctx.content_rect().center());
    egui::Area::new(egui::Id::new("battle tip"))
        .fixed_pos(at)
        .order(egui::Order::Tooltip)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(words);
            });
        });
}

/// What the overlay's colours mean, at the foot of the screen.
fn overlay_legend(ctx: &egui::Context) {
    let colour = |m: f32, f: f32| {
        let [r, g, b] = rts::condition_colour(sim_core::battle::Condition::new(m, f));
        let c = Color::linear_rgb(r, g, b).to_srgba();
        egui::Color32::from_rgb(
            (c.red * 255.0) as u8,
            (c.green * 255.0) as u8,
            (c.blue * 255.0) as u8,
        )
    };
    egui::Area::new(egui::Id::new("overlay legend"))
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -8.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Overlay (O):");
                    for (words, m, f) in [
                        ("## breaking", 0.05, 0.0),
                        ("## wavering", 0.5, 0.0),
                        ("## steady", 1.0, 0.0),
                        ("## steady, worn out", 1.0, 1.0),
                    ] {
                        ui.colored_label(colour(m, f), words);
                    }
                });
            });
        });
}

/// Every control of the 3D client, battle controls included (`F1`).
pub const HELP: &str = "\
Camera: WASD pan, Q/E or right-drag rotate, middle-drag pan, wheel zoom, Home reset
Space pause, , . slower / faster, H hide panels, V overlays, click a figure to inspect it
Shaders: Tab picks which, 0-6 its debug view, M toon/PBR, F fog, R rim, O outlines, X tilt-shift
Time: [ ] change the hour, T run the clock, Y weather, U season

In a live battle:
click / shift-click / drag a box: select your squads; Esc: select none
right-click: move there, or attack the enemy squad under it (archers shoot)
right-drag: lay out the line (its ghost shows where, and the arrival time)
double right-click: run; right-drag with nothing selected still turns the camera
Ctrl+1-9: make a group; 1-9: select it, twice: look at it
Z fire at will, X hold fire, C skirmish; V tight, B loose, N own order
O: soldiers coloured by morale (red breaking, green steady), darker when tired
hover an enemy squad: how your squads fare against it
the battle panel: orders per squad, and the stance and pursuit of the selection
Space pauses the battle; orders still go in";

fn controls_help(mut contexts: EguiContexts, mut help: ResMut<ControlsHelp>) {
    if !help.0 {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let mut open = true;
    egui::Window::new("Controls (F1)")
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label(HELP);
        });
    help.0 = open;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ray meets the ground where the terrain is, and never looking up.
    #[test]
    fn a_ray_meets_the_ground() {
        let paths = sim_core::AssetPaths::discover();
        let (db, _, mut scenario) = sim_core::load(&paths).unwrap();
        scenario.seed = 23;
        scenario.generate = Some(sim_core::worldgen::GenSpec {
            towns: 3,
            ..Default::default()
        });
        let scenario = sim_core::worldgen::resolve(&scenario, &db);
        let t = sim_core::terrain::Terrain::build(&scenario);
        let (x, z) = (40.0, -35.0);
        let h = crate::ground(&t, (x * SCALE, -z * SCALE));
        let down = Ray3d::new(Vec3::new(x, h + 30.0, z), Dir3::NEG_Y);
        let p = ray_ground(down, &t, 0.0).unwrap();
        assert!((p - Vec3::new(x, h, z)).length() < 1e-3, "{p:?} vs {h}");
        let up = Ray3d::new(Vec3::new(x, h + 1.0, z), Dir3::Y);
        assert!(ray_ground(up, &t, h).is_none());
    }
}
