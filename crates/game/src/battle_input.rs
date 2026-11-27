//! RTS controls for the live battle in the 2D client, on the shared logic in
//! `sim_core::battle::rts` (the 3D client's battles.rs uses the same), so
//! both clients behave alike (docs/design/client-parity.md, group 4).
//!
//! While a live battle waits:
//!
//! - left-click a squad of yours to select it, shift-click to add or remove
//!   one, drag a box to select several, click empty ground to select none;
//! - right-click an enemy squad to attack it (archers shoot), right-click
//!   ground to move there, right-drag to lay out the line (its ghost and
//!   arrival time follow the pointer), double-click to run;
//! - Ctrl+1..9 make the selection a group, 1..9 select it, twice to look
//!   at it (the number keys do not change the game speed meanwhile);
//! - Z fire at will, X hold fire, C skirmish; V tight, B loose, N own order;
//! - O colours the soldiers by morale (red breaking, green steady), darker
//!   the more tired;
//! - hovering an enemy squad says how your squads fare against it;
//! - Space still pauses, and orders can be given while paused.
//!
//! The mouse and keys are read in `PreUpdate`, after picking: a press the
//! battle takes is cleared, so the agent selection and the speed keys never
//! see it. Presses over a panel stay the panel's.

use crate::battles::{field_to_map, map_to_field};
use crate::live_battle::LiveBattle;
use crate::view::WorldCamera;
use bevy::picking::hover::HoverMap;
use bevy::picking::pointer::PointerId;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use sim_core::battle::rts::{self, Gesture, Pointer, Rts, StanceKey};
use sim_core::battle::{Fire, LiveView, Spacing};

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
    /// The pointer as last read.
    pointer: Option<Pointer>,
    /// The matchup hint for the enemy squad under the pointer.
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

/// The stance keys.
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

/// Read the mouse and keys for the live battle, and give its commands.
#[allow(clippy::too_many_arguments)]
pub fn battle_input(
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Single<(&Camera, &GlobalTransform, &mut Transform), With<WorldCamera>>,
    hover: Res<HoverMap>,
    nodes: Query<(), With<Node>>,
    time: Res<Time<Real>>,
    mut live: ResMut<LiveBattle>,
    mut controls: ResMut<BattleControls>,
    mut follow: ResMut<crate::view::Follow>,
) {
    let Some(v) = live.view.clone() else {
        controls.rts.selection.squads.clear();
        controls.pointer = None;
        controls.hint = None;
        return;
    };
    if let Some(Demo::Ghost) = controls.demo {
        controls.rts.selection.squads = (0..v.squads.len())
            .filter(|&q| rts::selectable(&v, q))
            .collect();
    }
    controls.overlay |= controls.demo == Some(Demo::Overlay);
    controls.rts.selection.tidy(&v);
    let (cam, cam_tf, _) = &*camera;
    let over_ui = hover
        .get(&PointerId::Mouse)
        .is_some_and(|m| m.keys().any(|e| nodes.contains(*e)));
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let to_field = map_to_field(v.at, v.facing, v.field);
    let p = window.cursor_position().map(|c| Pointer {
        screen: (c.x, c.y),
        field: cam
            .viewport_to_world_2d(cam_tf, c)
            .ok()
            .filter(|_| !over_ui)
            .map(&to_field)
            .filter(|f| (0.0..=v.field.0).contains(&f.x) && (0.0..=v.field.1).contains(&f.y))
            .map(|f| (f.x, f.y)),
        shift,
        now: time.elapsed_secs_f64(),
    });
    let mut commands = Vec::new();
    if let Some(p) = p {
        if mouse.just_pressed(MouseButton::Left) && controls.rts.left_down(p) {
            mouse.clear_just_pressed(MouseButton::Left);
        }
        if mouse.just_released(MouseButton::Left) {
            let to_map = field_to_map(v.at, v.facing, v.field);
            let project = |f: (f32, f32)| {
                cam.world_to_viewport(cam_tf, to_map(Vec2::new(f.0, f.1)).extend(0.0))
                    .ok()
                    .map(|s| (s.x, s.y))
            };
            controls.rts.left_up(&v, p, project);
        }
        if mouse.just_pressed(MouseButton::Right) && controls.rts.right_down(&v, p) {
            mouse.clear_just_pressed(MouseButton::Right);
        }
        if mouse.just_released(MouseButton::Right) {
            commands.extend(controls.rts.right_up(&v, p));
        }
    }
    // Keys: groups, stances, the overlay.
    let mut look = None;
    for (n, k) in DIGITS.iter().enumerate() {
        if keys.just_pressed(*k) {
            keys.clear_just_pressed(*k);
            let now = time.elapsed_secs_f64();
            look = controls.rts.group_key(&v, n, ctrl, now).or(look);
        }
    }
    if !controls.rts.selection.squads.is_empty() {
        for (k, key) in STANCE_KEYS {
            if keys.just_pressed(k) {
                keys.clear_just_pressed(k);
                commands.extend(rts::stance(&v, &controls.rts.selection.squads, key));
            }
        }
    }
    if keys.just_pressed(KeyCode::KeyO) {
        keys.clear_just_pressed(KeyCode::KeyO);
        controls.overlay = !controls.overlay;
    }
    if keys.just_pressed(KeyCode::Escape) {
        controls.rts.selection.squads.clear();
    }
    if let Some(f) = look {
        follow.0 = false;
        let m = field_to_map(v.at, v.facing, v.field)(Vec2::new(f.0, f.1));
        crate::view::look_at(&mut camera.2, (m.x, m.y));
    }
    live.commands.extend(commands);
    controls.hint = p.and_then(|p| {
        let e = controls.rts.hover(&v, p)?;
        rts::hint(&v, &controls.rts.selection.squads, e)
    });
    controls.pointer = p;
}

/// The ghost being dragged (or shown by `--rts-demo ghost`): each squad's
/// slots, and the line's two ends, on the field.
fn ghost_of(
    v: &LiveView,
    c: &BattleControls,
) -> (Vec<rts::Gesture>, Vec<(usize, sim_core::battle::Preview)>) {
    if c.demo == Some(Demo::Ghost) {
        let sel = &c.rts.selection.squads;
        if let Some((from, to)) = rts::line_ahead(v, sel, 60.0) {
            return (
                vec![Gesture::Line(from, to, false)],
                rts::ghost(v, sel, from, to, false),
            );
        }
    }
    let Some(p) = c.pointer else {
        return (Vec::new(), Vec::new());
    };
    (vec![c.rts.gesture(p)], c.rts.ghost(v, p))
}

const SELECTED: Color = Color::srgb(1.0, 0.95, 0.35);
const GHOST: Color = Color::srgba(0.75, 0.95, 1.0, 0.8);

/// Draw the selection, the box or the ghost line, and the overlay.
pub fn draw_battle_controls(
    mut gizmos: Gizmos,
    live: Res<LiveBattle>,
    controls: Res<BattleControls>,
    camera: Single<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    let Some(v) = &live.view else { return };
    let to_map = field_to_map(v.at, v.facing, v.field);
    let at = |p: (f32, f32)| to_map(Vec2::new(p.0, p.1));
    for &q in &controls.rts.selection.squads {
        if let Some(c) = rts::outline(v, q) {
            for k in 0..4 {
                gizmos.line_2d(at(c[k]), at(c[(k + 1) % 4]), SELECTED);
            }
        }
    }
    let (gestures, ghosts) = ghost_of(v, &controls);
    for g in gestures {
        match g {
            Gesture::Box(a, b) => {
                let (cam, tf) = *camera;
                let w = |s: (f32, f32)| cam.viewport_to_world_2d(tf, Vec2::new(s.0, s.1)).ok();
                if let (Some(a), Some(b)) = (w(a), w(b)) {
                    let corners = [a, Vec2::new(b.x, a.y), b, Vec2::new(a.x, b.y)];
                    for k in 0..4 {
                        gizmos.line_2d(corners[k], corners[(k + 1) % 4], SELECTED);
                    }
                }
            }
            Gesture::Line(a, b, _) => gizmos.line_2d(at(a), at(b), GHOST),
            Gesture::None => {}
        }
    }
    let r = 0.45 * crate::battles::SCALE;
    for (_, g) in &ghosts {
        for &s in &g.slots {
            gizmos
                .circle_2d(Isometry2d::from_translation(at(s)), r, GHOST)
                .resolution(6);
        }
    }
    // The overlay: each soldier on their feet a disc in the colour of their
    // morale, darker the more tired, at least a few pixels wide.
    let px = camera.1.compute_transform().scale.x;
    let disc = (1.1 * r).max(3.5 * px);
    if controls.overlay && v.on_soldiers() {
        for (men, cond) in v.frame.soldiers.iter().zip(&v.condition) {
            for (m, c) in men.iter().zip(cond) {
                if !matches!(
                    m.state,
                    sim_core::battle::FighterState::Standing
                        | sim_core::battle::FighterState::Routing
                ) {
                    continue;
                }
                let [r_, g_, b_] = rts::condition_colour(*c);
                let p = at(m.pos());
                // Rings close together, to read as a filled disc.
                for k in 1..=4 {
                    gizmos
                        .circle_2d(
                            Isometry2d::from_translation(p),
                            disc * k as f32 / 4.0,
                            Color::linear_rgb(r_, g_, b_),
                        )
                        .resolution(8);
                }
            }
        }
    }
}

/// What the overlay's colours mean, at the foot of the screen while it is on.
#[derive(Component)]
pub struct OverlayLegend;

pub fn spawn_overlay_legend(mut commands: Commands) {
    let font = TextFont {
        font_size: FontSize::Px(13.0),
        ..default()
    };
    let rgb = |m: f32, f: f32| {
        let [r, g, b] = rts::condition_colour(sim_core::battle::Condition::new(m, f));
        TextColor(Color::linear_rgb(r, g, b))
    };
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(8.0),
                left: Val::Percent(30.0),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                column_gap: Val::Px(10.0),
                flex_direction: FlexDirection::Row,
                ..default()
            },
            BackgroundColor(Color::srgb(0.05, 0.04, 0.02)),
            GlobalZIndex(10),
            Visibility::Hidden,
            bevy::picking::Pickable::IGNORE,
            OverlayLegend,
        ))
        .with_children(|p| {
            p.spawn((Text::new("Overlay (O):"), font.clone()));
            p.spawn((Text::new("## breaking"), font.clone(), rgb(0.05, 0.0)));
            p.spawn((Text::new("## wavering"), font.clone(), rgb(0.5, 0.0)));
            p.spawn((Text::new("## steady"), font.clone(), rgb(1.0, 0.0)));
            p.spawn((
                Text::new("## steady, worn out"),
                font.clone(),
                rgb(1.0, 1.0),
            ));
        });
}

/// Shows the legend while the overlay is on in a live battle.
pub fn update_overlay_legend(
    live: Res<LiveBattle>,
    controls: Res<BattleControls>,
    mut legend: Single<&mut Visibility, With<OverlayLegend>>,
) {
    let on = controls.overlay && live.view.as_ref().is_some_and(|v| v.on_soldiers());
    legend.set_if_neq(if on {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
}

/// The text by the pointer: the matchup hint, or the ghost's arrival time.
#[derive(Component)]
pub struct ControlsTip;

pub fn spawn_controls_tip(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(13.0),
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.95, 0.8)),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        Node {
            position_type: PositionType::Absolute,
            padding: UiRect::all(Val::Px(4.0)),
            ..default()
        },
        Visibility::Hidden,
        bevy::picking::Pickable::IGNORE,
        ControlsTip,
    ));
}

/// Words for the tip: the arrival time of a ghost, else the hint.
pub fn tip_text(v: &LiveView, c: &BattleControls) -> Option<String> {
    let (_, ghosts) = ghost_of(v, c);
    if let Some(eta) = ghosts.iter().map(|g| g.1.eta).reduce(f32::max) {
        let run = c
            .pointer
            .is_some_and(|p| matches!(c.rts.gesture(p), Gesture::Line(_, _, true)));
        let how = if run { "running" } else { "marching" };
        return Some(format!("there in {eta:.0} s, {how}"));
    }
    c.hint.clone()
}

pub fn update_controls_tip(
    live: Res<LiveBattle>,
    controls: Res<BattleControls>,
    mut tip: Single<(&mut Text, &mut Node, &mut Visibility), With<ControlsTip>>,
    window: Single<&Window, With<PrimaryWindow>>,
) {
    let (text, node, vis) = &mut *tip;
    let words = live.view.as_ref().and_then(|v| tip_text(v, &controls));
    let Some(words) = words else {
        vis.set_if_neq(Visibility::Hidden);
        return;
    };
    let at = window
        .cursor_position()
        .or_else(|| controls.demo.map(|_| Vec2::new(0.55, 0.55) * window.size()))
        .unwrap_or_default();
    node.left = Val::Px(at.x + 16.0);
    node.top = Val::Px(at.y + 16.0);
    if text.0 != words {
        text.0 = words;
    }
    vis.set_if_neq(Visibility::Inherited);
}
