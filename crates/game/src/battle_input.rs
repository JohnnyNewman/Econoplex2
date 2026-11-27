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
//! - Space still pauses, and orders can be given while paused;
//! - shift with a right-click or drag queues the move after the squad's
//!   moves (the queued path is drawn with a marker at each waypoint), and
//!   Backspace drops the selected squads' queues;
//! - before the battle starts, the player's half is drawn as a zone and the
//!   same clicks and drags stand the squads there, until "Start battle".
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
    /// The deployment zone, and a line dragged in it (with
    /// `--shot-battle 0`, before the battle starts).
    Deploy,
    /// Every squad of yours with two waypoints queued, and their paths.
    Queue,
}

impl std::str::FromStr for Demo {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "ghost" => Ok(Self::Ghost),
            "overlay" => Ok(Self::Overlay),
            "deploy" => Ok(Self::Deploy),
            "queue" => Ok(Self::Queue),
            _ => Err(format!("--rts-demo {s}: ghost, overlay, deploy or queue")),
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
    if matches!(
        controls.demo,
        Some(Demo::Ghost | Demo::Deploy | Demo::Queue)
    ) {
        controls.rts.selection.squads = (0..v.squads.len())
            .filter(|&q| rts::selectable(&v, q))
            .collect();
    }
    // The demo's queue, given again whenever the squads have used it up.
    let used_up = controls
        .rts
        .selection
        .squads
        .iter()
        .all(|&q| v.orders.get(q).map_or(0, Vec::len) < 2);
    if controls.demo == Some(Demo::Queue) && !v.deploying && used_up {
        live.commands
            .extend(rts::demo_queue(&v, &controls.rts.selection.squads));
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
    if keys.just_pressed(KeyCode::Backspace) && !controls.rts.selection.squads.is_empty() {
        keys.clear_just_pressed(KeyCode::Backspace);
        commands.extend(rts::clear_queue(&v, &controls.rts.selection.squads));
    }
    // The demos are shown from where they happen: the zone, or the paths.
    let look = look.or_else(|| match controls.demo? {
        Demo::Deploy if v.deploying => {
            let sel = &controls.rts.selection.squads;
            let (from, to) = rts::line_ahead(&v, sel, 25.0)?;
            let c = rts::centre(&v, sel)?;
            Some((
                0.5 * (c.0 + 0.5 * (from.0 + to.0)),
                0.5 * (c.1 + 0.5 * (from.1 + to.1)),
            ))
        }
        Demo::Queue => rts::centre(&v, &controls.rts.selection.squads),
        _ => None,
    });
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
    if c.demo == Some(Demo::Deploy) && v.deploying {
        let sel = &c.rts.selection.squads;
        if let Some((from, to)) = rts::line_ahead(v, sel, 25.0) {
            return (
                vec![Gesture::Line(from, to, false)],
                rts::deploy_ghost(v, sel, from, to),
            );
        }
    }
    let Some(p) = c.pointer else {
        return (Vec::new(), Vec::new());
    };
    (vec![c.rts.gesture(p)], c.rts.ghost(v, p))
}

const SELECTED: Color = Color::srgb(1.0, 0.95, 0.35);
const ZONE: Color = Color::srgba(0.45, 0.9, 0.5, 0.7);
const PATH: Color = Color::srgba(1.0, 0.75, 0.3, 0.85);
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
    let px = camera.1.compute_transform().scale.x;
    // Before the battle: the player's half, outlined and hatched.
    if v.deploying {
        let (x0, x1, y0, y1) = v.deploy_zone(v.player_side);
        let c = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
        for k in 0..4 {
            gizmos.line_2d(at(c[k]), at(c[(k + 1) % 4]), ZONE);
        }
        let step = 25.0;
        let mut s = 0.0;
        while s < (x1 - x0) + (y1 - y0) {
            // A hatch line where x + y is constant, clipped to the zone.
            let a = (x0 + s.min(x1 - x0), y0 + (s - (x1 - x0)).max(0.0));
            let b = (x0 + (s - (y1 - y0)).max(0.0), y0 + s.min(y1 - y0));
            gizmos.line_2d(at(a), at(b), ZONE.with_alpha(0.2));
            s += step;
        }
    }
    // Queued paths of the player's squads, a marker at each waypoint;
    // brighter for the selected.
    for q in (0..v.squads.len()).filter(|&q| rts::selectable(v, q)) {
        let path = rts::queued_path(v, q);
        let colour = if controls.rts.selection.squads.contains(&q) {
            PATH
        } else {
            PATH.with_alpha(0.35)
        };
        for w in path.windows(2) {
            gizmos.line_2d(at(w[0]), at(w[1]), colour);
        }
        let m = (1.5 * crate::battles::SCALE).max(7.0 * px);
        for &p in path.iter().skip(1) {
            let c = at(p);
            for (dx, dy) in [(m, 0.0), (0.0, m), (-m, 0.0), (0.0, -m)] {
                let (ex, ey) = (-dy, dx);
                gizmos.line_2d(c + Vec2::new(dx, dy), c + Vec2::new(ex, ey), colour);
            }
        }
    }
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

/// Words for the tip: where a ghost deploys or when it arrives (queued with
/// shift), else the hint.
pub fn tip_text(v: &LiveView, c: &BattleControls) -> Option<String> {
    let (_, ghosts) = ghost_of(v, c);
    if v.deploying && !ghosts.is_empty() {
        return Some("deploy here".into());
    }
    if let Some(eta) = ghosts.iter().map(|g| g.1.eta).reduce(f32::max) {
        let run = c
            .pointer
            .is_some_and(|p| matches!(c.rts.gesture(p), Gesture::Line(_, _, true)));
        let how = if run { "running" } else { "marching" };
        let queued = if c.pointer.is_some_and(|p| p.shift) {
            "queued: "
        } else {
            ""
        };
        return Some(format!("{queued}there in {eta:.0} s, {how}"));
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
