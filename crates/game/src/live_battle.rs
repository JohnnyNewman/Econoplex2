//! Live battles in the 2D client, as the 3D client fights them (live.rs and
//! battles.rs there; docs/design/client-parity.md, group 4).
//!
//! A battle the player's own town fights waits for the player
//! (`sim_core::systems::military::LiveBattles`). While it waits, the sim loop
//! (`main.rs::run_sim_tick`) holds the economy's clock and steps the battle in
//! real time instead, `BATTLE_SPEED` battle seconds a second, unless the game
//! is paused. Everything goes through the simulation's own calls
//! (`command_live_battle`, `step_live_battle`, `resolve_live_battle`,
//! `live_battle_view`); once the battle is over, the next economy step settles
//! it as usual.
//!
//! The squads are drawn on the map with battles.rs, from the battle's current
//! frame. Live battles fight on `battle.live_model` (the soldier model by
//! default): zoomed in, every soldier is drawn as a dot where they stand,
//! as the 3D client stands a figure for each. A panel at the top lists every
//! squad: its fighters left, state and morale, and for the player's fighting
//! squads the orders: own judgment, hold, withdraw, or attack (archers also
//! shoot) a named enemy squad. On the soldier model a squad can also go
//! round a named enemy squad's side (`LiveView::flank`: moves, then attack)
//! and re-form where it stands to a front of so many files
//! (`LiveView::reform`); the squad model has no front or sides, so those
//! buttons are not shown. Every button sends a list of orders, as the mouse
//! controls of an RTS will (`sim_core::battle::Command`; design note
//! battle-per-soldier.md, "Battle controls").
//! "Look at the field" moves the camera there; "Fight it out" settles it at
//! once. Under the title, the squads selected with the mouse
//! (battle_input.rs) and buttons for their stance: fire at will, hold fire
//! or skirmish; tight, loose or their own order; and whether they kill,
//! capture or let go enemies who run. While the battle waits, the summary,
//! economy and policy panels fold away (`fold_panels`), so nothing lies under
//! the battle panel.
//!
//! Before the battle's first step the player deploys: the title says so,
//! the right-drag line stands squads in their half (battle_input.rs), and
//! "Start battle" begins it. Squads with waypoints queued offer "Clear
//! queue" (or Backspace).

use crate::battle_input::BattleControls;
use crate::ui_compat::Interaction;
use crate::view::WorldCamera;
use bevy::prelude::*;
use sim_core::battle::rts::{self, StanceKey};
use sim_core::battle::{Command, LiveView, Order, SquadState};
use sim_core::systems::military as war;
use sim_core::Towns;

/// Battle seconds per real second (as the 3D client).
pub const BATTLE_SPEED: f32 = 6.0;

/// Below this camera zoom (map units per pixel) a live battle on the soldier
/// model shows its soldiers instead of a block per squad: 1 m of field is
/// then over a pixel.
pub const SOLDIERS_BELOW: f32 = 0.3;
/// The zoom "Look at the field" sets when the soldiers can be shown.
pub const LOOK_ZOOM: f32 = 0.15;

/// The live battle as the view last saw it, and what the player asked of it
/// since (applied by the sim loop before its next step).
#[derive(Resource, Default)]
pub struct LiveBattle {
    pub view: Option<LiveView>,
    /// What the player told which squad.
    pub commands: Vec<(usize, Command)>,
    pub resolve: bool,
    /// The player ended the deployment.
    pub start: bool,
    /// Battle seconds due and not yet stepped.
    due: f32,
}

/// Called by the sim loop on every fixed update before the economy: apply the
/// player's orders, step a waiting battle by the real time passed (unless
/// paused), and refresh the view. Returns whether a battle waits, in which
/// case the economy's clock holds.
pub fn run(world: &mut World, real_secs: f32, paused: bool) -> bool {
    let (commands, resolve, start) = {
        let mut live = world.resource_mut::<LiveBattle>();
        (
            std::mem::take(&mut live.commands),
            std::mem::take(&mut live.resolve),
            std::mem::take(&mut live.start),
        )
    };
    if start {
        war::start_live_battle(world);
    }
    for (squad, command) in commands {
        if let Err(e) = war::command_live_battle(world, squad, command) {
            warn!("battle command to squad {squad}: {e}");
        }
    }
    if resolve {
        war::resolve_live_battle(world);
    }
    let waiting = war::live_battle_waiting(world);
    if waiting && !paused {
        let steps = {
            let mut live = world.resource_mut::<LiveBattle>();
            live.due += real_secs * BATTLE_SPEED;
            let n = live.due.floor();
            live.due -= n;
            // A slow frame does not make the battle jump ahead.
            n.min(BATTLE_SPEED)
        };
        if steps >= 1.0 {
            war::step_live_battle(world, steps);
        }
    } else {
        world.resource_mut::<LiveBattle>().due = 0.0;
    }
    let view = war::live_battle_view(world);
    world.resource_mut::<LiveBattle>().view = view;
    waiting
}

/// Draw the live battle where it stands now: a block per squad, or zoomed
/// in on the soldier model, every soldier.
pub fn draw_live_battle(
    mut gizmos: Gizmos,
    time: Res<Time>,
    live: Res<LiveBattle>,
    camera: Single<&Transform, With<WorldCamera>>,
) {
    let Some(v) = &live.view else { return };
    let zoom = camera.scale.x;
    let outline = Color::srgba(1.0, 0.95, 0.5, 0.8);
    if shows_soldiers(v, zoom) {
        crate::battles::draw_soldiers(
            &mut gizmos,
            time.elapsed_secs(),
            v.at,
            v.facing,
            v.field,
            &v.squads,
            &v.frame.soldiers,
            outline,
            zoom,
        );
        return;
    }
    crate::battles::draw_field(
        &mut gizmos,
        time.elapsed_secs(),
        v.at,
        v.facing,
        v.field,
        &v.squads,
        |k| {
            v.frame
                .squads
                .get(k)
                .map(|f| (Vec2::new(f.0, f.1), f.2, f.3))
        },
        outline,
    );
}

/// Where on the map to look at a live battle: the centre of the soldiers
/// still on the field on the soldier model, else the field's centre.
pub fn look_point(v: &LiveView) -> (f32, f32) {
    let to_map = crate::battles::field_to_map(v.at, v.facing, v.field);
    let on: Vec<Vec2> = v
        .frame
        .soldiers
        .iter()
        .flatten()
        .filter(|m| m.state != sim_core::battle::FighterState::Fled)
        .map(|m| {
            let (x, y) = m.pos();
            to_map(Vec2::new(x, y))
        })
        .collect();
    if on.is_empty() {
        return v.at;
    }
    let c = on.iter().sum::<Vec2>() / on.len() as f32;
    (c.x, c.y)
}

/// Whether a live battle is drawn soldier by soldier at camera zoom `zoom`.
pub fn shows_soldiers(v: &LiveView, zoom: f32) -> bool {
    v.on_soldiers() && zoom < SOLDIERS_BELOW
}

#[derive(Component)]
pub struct BattleRoot;

/// The panel's title and status line.
#[derive(Component)]
pub struct BattleTitle;

/// A squad's row text: name, fighters left, state, morale.
#[derive(Component)]
pub struct SquadText(usize);

/// Where the per-squad rows go (rebuilt when the choices change).
#[derive(Component)]
pub struct SquadRows;

/// Where the selection and its stance buttons go.
#[derive(Component)]
pub struct StanceRows;

#[derive(Component, Clone, Debug, PartialEq)]
pub enum BattleAction {
    /// A squad's orders, carried out in turn.
    Orders(usize, Vec<Order>),
    /// A stance for the selected squads.
    Stance(StanceKey),
    /// End the deployment.
    Start,
    /// Drop the selected squads' queued waypoints.
    ClearQueue,
    Look,
    FightOut,
}

/// A button for what the squad does now (its order, or its formation).
#[derive(Component)]
pub struct Chosen;

const BUTTON: Color = Color::srgb(0.22, 0.24, 0.22);
const CHOSEN: Color = Color::srgb(0.45, 0.38, 0.15);
const HOVER: Color = Color::srgb(0.32, 0.36, 0.32);

fn font(px: f32) -> TextFont {
    TextFont {
        font_size: FontSize::Px(px),
        ..default()
    }
}

fn spawn_button(p: &mut ChildSpawnerCommands, label: &str, action: BattleAction, chosen: bool) {
    let mut button = p.spawn((
        crate::ui_compat::button(),
        Node {
            padding: UiRect::axes(Val::Px(6.0), Val::Px(1.0)),
            margin: UiRect::all(Val::Px(2.0)),
            ..default()
        },
        BackgroundColor(if chosen { CHOSEN } else { BUTTON }),
        action,
    ));
    if chosen {
        button.insert(Chosen);
    }
    button.with_children(|b| {
        b.spawn((Text::new(label), font(12.0)));
    });
}

pub fn spawn_battle_panel(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(8.0),
                left: Val::Percent(30.0),
                width: Val::Percent(40.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.05, 0.04, 0.02)),
            GlobalZIndex(10),
            Visibility::Hidden,
            BattleRoot,
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(""),
                font(14.0),
                TextColor(Color::srgb(1.0, 0.92, 0.7)),
                BattleTitle,
            ));
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                ..default()
            })
            .with_children(|r| {
                spawn_button(r, "Look at the field", BattleAction::Look, false);
                spawn_button(r, "Fight it out", BattleAction::FightOut, false);
            });
            p.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    margin: UiRect::vertical(Val::Px(4.0)),
                    ..default()
                },
                StanceRows,
            ));
            p.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
                SquadRows,
            ));
        });
}

/// While a battle waits, the summary, economy and policy panels fold away
/// (`hud::FoldsInBattle`), so nothing is drawn under the battle panel; they
/// come back when it is settled.
pub fn fold_panels(
    live: Res<LiveBattle>,
    mut panels: Query<&mut Node, With<crate::hud::FoldsInBattle>>,
) {
    let display = if live.view.is_some() {
        Display::None
    } else {
        Display::Flex
    };
    for mut n in &mut panels {
        if n.display != display {
            n.display = display;
        }
    }
}

/// Enemy squads still on the field (targets for attack and shoot orders).
pub fn enemies(v: &LiveView) -> Vec<usize> {
    (0..v.squads.len())
        .filter(|&q| {
            v.squads[q].side != v.player_side
                && v.frame.squads.get(q).is_some_and(|f| {
                    f.2 > 0 && matches!(f.3, SquadState::Fighting | SquadState::Routing)
                })
        })
        .collect()
}

/// Whether squad `q` is the player's and can take orders now.
fn orderable(v: &LiveView, q: usize) -> bool {
    !v.finished
        && v.squads[q].side == v.player_side
        && v.frame
            .squads
            .get(q)
            .is_some_and(|f| f.3 == SquadState::Fighting)
}

/// The orders a squad can be given, each a list carried out in turn: its own
/// judgment, hold, withdraw, and attack each enemy on the field (archers, by
/// name as in the 3D client, also shoot). On the soldier model, also go
/// round each enemy's side, and re-form where it stands to each of the
/// panels' fronts (`LiveView::FRONTS`).
pub fn choices(v: &LiveView, q: usize) -> Vec<(String, Vec<Order>)> {
    let mut out = vec![
        ("Own judgment".to_string(), vec![Order::Auto]),
        ("Hold".to_string(), vec![Order::Hold]),
        ("Withdraw".to_string(), vec![Order::Withdraw]),
    ];
    let archer = v.squads[q].name.to_lowercase().contains("bow");
    for e in enemies(v) {
        let name = &v.squads[e].name;
        out.push((format!("Attack {name}"), vec![Order::Attack(e)]));
        if v.on_soldiers() {
            out.push((format!("Flank {name}"), v.flank(q, e)));
        }
        if archer {
            out.push((format!("Shoot {name}"), vec![Order::Shoot(e)]));
        }
    }
    if v.on_soldiers() {
        for (name, files) in LiveView::FRONTS {
            out.push((name.to_string(), v.reform(q, files)));
        }
    }
    out
}

/// Whether squad `q` follows `plan` now: a single order in force and
/// nothing after it. Moves (going round, re-forming) are never shown chosen.
pub fn chosen(v: &LiveView, q: usize, plan: &[Order]) -> bool {
    let left = v.orders.get(q).map_or(0, Vec::len);
    matches!(plan, [o] if left <= 1 && v.order_now(q) == *o && !matches!(o, Order::MoveTo { .. }))
}

/// What the selection rows are built from: the selection, its stance marks,
/// whether the battle waits to start, and whether any selected squad has a
/// queue.
type StanceLayout = (Vec<usize>, Vec<bool>, bool, bool);

/// What the rows are built from; they are rebuilt only when this changes.
type Layout = (
    usize,
    Vec<Option<Vec<(String, Vec<Order>)>>>,
    Vec<Vec<Order>>,
);

/// Show, fill and hide the panel; rebuild its squad rows when the choices or
/// the orders given change.
#[allow(clippy::too_many_arguments)]
pub fn update_battle_panel(
    mut commands: Commands,
    live: Res<LiveBattle>,
    towns: Res<Towns>,
    mut root: Single<&mut Visibility, With<BattleRoot>>,
    mut title: Single<&mut Text, (With<BattleTitle>, Without<SquadText>)>,
    rows: Single<Entity, With<SquadRows>>,
    stance_rows: Single<Entity, With<StanceRows>>,
    mut texts: Query<(&mut Text, &SquadText), Without<BattleTitle>>,
    mut layout: Local<Option<Layout>>,
    controls: Res<BattleControls>,
    mut stance_layout: Local<Option<StanceLayout>>,
) {
    let Some(v) = &live.view else {
        root.set_if_neq(Visibility::Hidden);
        if layout.is_some() {
            commands.entity(*rows).despawn_children();
            commands.entity(*stance_rows).despawn_children();
            *layout = None;
            *stance_layout = None;
        }
        return;
    };
    // The selection and its stance.
    let sel = &controls.rts.selection.squads;
    let marks: Vec<bool> = StanceKey::ALL
        .iter()
        .map(|(k, _)| k.chosen(v, sel))
        .collect();
    let queued = sel
        .iter()
        .any(|&q| v.orders.get(q).is_some_and(|o| o.len() > 1));
    let want_stance = (sel.clone(), marks, v.deploying, queued);
    if stance_layout.as_ref() != Some(&want_stance) {
        commands.entity(*stance_rows).despawn_children();
        commands.entity(*stance_rows).with_children(|p| {
            if v.deploying {
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|r| {
                    spawn_button(r, "Start battle", BattleAction::Start, true);
                    r.spawn((
                        Text::new(" Deploy: right-click or right-drag your squads into the green zone"),
                        font(12.0),
                    ));
                });
            }
            if sel.is_empty() {
                p.spawn((
                    Text::new("Select squads with the mouse: click, shift-click or drag a box (F1: controls)"),
                    font(12.0),
                ));
                return;
            }
            let names: Vec<&str> = sel.iter().map(|&q| v.squads[q].name.as_str()).collect();
            p.spawn((Text::new(format!("Selected: {}", names.join(", "))), font(12.0)));
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                ..default()
            })
            .with_children(|r| {
                for (&(key, label), &on) in StanceKey::ALL.iter().zip(&want_stance.1) {
                    spawn_button(r, label, BattleAction::Stance(key), on);
                }
                if queued {
                    spawn_button(r, "Clear queue", BattleAction::ClearQueue, false);
                }
            });
        });
        *stance_layout = Some(want_stance);
    }
    root.set_if_neq(Visibility::Inherited);
    let town = towns
        .0
        .get(v.town as usize)
        .map_or("?", |t| t.name.as_str());
    let head = if v.player_side == 0 {
        format!("Battle: you attack {town}")
    } else {
        format!("Battle: {town} is attacked")
    };
    let status = if v.finished {
        match v.winner {
            Some(w) if w == v.player_side => "Victory. The day is yours.".to_string(),
            Some(_) => "Defeat. Your squads fell back.".to_string(),
            None => "The fighting is over.".to_string(),
        }
    } else if v.deploying {
        "Deploy your squads, then start the battle (the economy waits)".to_string()
    } else {
        format!(
            "{:.0} min fought (the economy waits; Space pauses the battle)",
            v.frame.t / 60.0
        )
    };
    let want = format!("{head}\n{status}");
    if title.0 != want {
        title.0 = want;
    }
    let shape: Layout = (
        v.squads.len(),
        (0..v.squads.len())
            .map(|q| orderable(v, q).then(|| choices(v, q)))
            .collect(),
        v.orders.clone(),
    );
    if layout.as_ref() != Some(&shape) {
        commands.entity(*rows).despawn_children();
        commands.entity(*rows).with_children(|p| {
            for (q, opts) in shape.1.iter().enumerate() {
                p.spawn((Text::new(""), font(13.0), SquadText(q)));
                let Some(opts) = opts else { continue };
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                })
                .with_children(|r| {
                    for (label, o) in opts {
                        let on = chosen(v, q, o);
                        spawn_button(r, label, BattleAction::Orders(q, o.clone()), on);
                    }
                });
            }
        });
        *layout = Some(shape);
    }
    for (mut text, row) in &mut texts {
        let q = row.0;
        let (Some(sq), Some(f)) = (v.squads.get(q), v.frame.squads.get(q)) else {
            continue;
        };
        let side = if sq.side != v.player_side {
            "enemy"
        } else if sel.contains(&q) {
            "yours, selected"
        } else {
            "yours"
        };
        let want = format!(
            "{} ({side})   {}/{}   {:?}   morale {:.0}%",
            sq.name,
            f.2,
            sq.start,
            f.3,
            100.0 * v.morale.get(q).copied().unwrap_or(0.0)
        );
        if text.0 != want {
            text.0 = want;
        }
    }
}

/// The panel's buttons: orders go to the sim loop; "Look at the field" centres
/// and zooms the camera on it; "Fight it out" settles the battle at once.
pub fn battle_buttons(
    mut buttons: Query<
        (
            &Interaction,
            &BattleAction,
            &mut BackgroundColor,
            Has<Chosen>,
        ),
        Changed<Interaction>,
    >,
    mut live: ResMut<LiveBattle>,
    mut follow: ResMut<crate::view::Follow>,
    mut camera: Single<&mut Transform, With<WorldCamera>>,
    controls: Res<BattleControls>,
) {
    for (interaction, action, mut colour, chosen) in &mut buttons {
        match interaction {
            Interaction::Hovered => colour.0 = HOVER,
            // The squad's order and formation stay marked.
            Interaction::None => colour.0 = if chosen { CHOSEN } else { BUTTON },
            Interaction::Pressed => {
                colour.0 = HOVER;
                match action {
                    BattleAction::Orders(q, plan) => {
                        live.commands.push((*q, Command::Orders(plan.clone())))
                    }
                    BattleAction::Stance(key) => {
                        if let Some(v) = &live.view {
                            let cmds = rts::stance(v, &controls.rts.selection.squads, *key);
                            live.commands.extend(cmds);
                        }
                    }
                    BattleAction::Start => live.start = true,
                    BattleAction::ClearQueue => {
                        if let Some(v) = &live.view {
                            let cmds = rts::clear_queue(v, &controls.rts.selection.squads);
                            live.commands.extend(cmds);
                        }
                    }
                    BattleAction::FightOut => live.resolve = true,
                    BattleAction::Look => {
                        if let Some(v) = &live.view {
                            follow.0 = false;
                            crate::view::look_at(&mut camera, look_point(v));
                            let most = if v.on_soldiers() { LOOK_ZOOM } else { 0.8 };
                            let z = camera.scale.x.min(most);
                            camera.scale = Vec3::new(z, z, 1.0);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::battle::{Kit, ReplayFrame, ReplaySquad, SquadLine, Stance};

    fn view() -> LiveView {
        let squad = |name: &str, side| ReplaySquad {
            name: name.into(),
            side,
            mounted: false,
            start: 20,
        };
        LiveView {
            town: 0,
            at: (0.0, 0.0),
            facing: (1.0, 0.0),
            field: (100.0, 50.0),
            squads: vec![squad("Bowmen", 0), squad("Spears", 0), squad("Raiders", 1)],
            orders: vec![Vec::new(); 3],
            lines: vec![
                SquadLine {
                    anchor: (10.0, 10.0),
                    facing: (1.0, 0.0),
                    width: 5.0,
                    spacing: 1.0,
                };
                3
            ],
            stances: vec![Stance::default(); 3],
            paces: vec![(1.4, 3.0); 3],
            kits: vec![Kit::preset("archers").unwrap(); 3],
            wall_x: None,
            nav: None,
            morale: vec![1.0; 3],
            condition: Vec::new(),
            frame: ReplayFrame {
                t: 30.0,
                squads: vec![
                    (10.0, 10.0, 20, SquadState::Fighting),
                    (10.0, 30.0, 0, SquadState::Destroyed),
                    (80.0, 20.0, 15, SquadState::Fighting),
                ],
                soldiers: Vec::new(),
            },
            player_side: 0,
            deploying: false,
            finished: false,
            winner: None,
        }
    }

    #[test]
    fn squads_get_the_orders_the_3d_client_offers() {
        let v = view();
        assert_eq!(enemies(&v), vec![2]);
        let bow: Vec<Vec<Order>> = choices(&v, 0).into_iter().map(|c| c.1).collect();
        assert_eq!(
            bow,
            [
                Order::Auto,
                Order::Hold,
                Order::Withdraw,
                Order::Attack(2),
                Order::Shoot(2)
            ]
            .map(|o| vec![o])
        );
        assert!(orderable(&v, 0));
        // A destroyed squad and the enemy take no orders.
        assert!(!orderable(&v, 1) && !orderable(&v, 2));
        assert!(chosen(&v, 0, &[Order::Auto]) && !chosen(&v, 0, &[Order::Hold]));
    }

    /// On the soldier model a squad can also go round each enemy's side and
    /// re-form where it stands, as in the 3D client: going round is moves
    /// along the enemy's side and then the attack; re-forming is a move to
    /// where it stands, that many files wide, and then on as before.
    #[test]
    fn on_soldiers_squads_also_go_round_and_re_form() {
        let mut v = view();
        v.frame.soldiers = vec![Vec::new(); 3];
        v.orders[1] = vec![Order::Hold];
        let opts = choices(&v, 1);
        let labels: Vec<&str> = opts.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Own judgment",
                "Hold",
                "Withdraw",
                "Attack Raiders",
                "Flank Raiders",
                "Column",
                "Deep",
                "Line",
                "Wide"
            ]
        );
        let flank = &opts[4].1;
        assert!(matches!(
            flank.as_slice(),
            [.., Order::MoveTo { .. }, Order::Attack(2)]
        ));
        assert_eq!(
            opts[5].1,
            vec![
                Order::MoveTo {
                    anchor: (10.0, 10.0),
                    facing: Some((1.0, 0.0)),
                    width: Some(4.0),
                    run: false
                },
                Order::Hold
            ]
        );
        assert!(chosen(&v, 1, &[Order::Hold]) && !chosen(&v, 1, &opts[5].1));
    }

    /// A live battle on the soldier model lists every soldier, and zoomed in
    /// the map draws one dot for each who has not left the field, on the field.
    #[test]
    fn zoomed_in_a_soldier_battle_is_drawn_soldier_by_soldier() {
        use sim_core::battle::{parse_lineup, Battle, BattleModel, BattleParams, FighterState};
        let (field, specs) = parse_lineup("swordsmen*30 vs spearmen*30").unwrap();
        let params = BattleParams {
            model: BattleModel::Soldiers,
            ..BattleParams::default()
        };
        let mut b = Battle::new(params, field, specs, 1);
        for _ in 0..120 {
            b.step();
        }
        let v = b.live_view(0, (500.0, 200.0), (0.0, 1.0), 0);
        let men = &v.frame.soldiers;
        assert_eq!(men.len(), 2);
        assert!(men.iter().all(|s| s.len() == 30));
        assert!(shows_soldiers(&v, 0.1) && !shows_soldiers(&v, 1.0));
        let dots = crate::battles::soldier_dots(0.0, v.at, v.facing, v.field, &v.squads, men);
        let on_field = men
            .iter()
            .flatten()
            .filter(|m| m.state != FighterState::Fled)
            .count();
        assert_eq!(dots.len(), on_field);
        // The field on the map: centred at `at`, its length along `facing` (+y).
        let half = Vec2::new(v.field.1, v.field.0) * 0.5 * crate::battles::SCALE;
        for d in &dots {
            let o = (d.at - Vec2::new(500.0, 200.0)).abs();
            assert!(
                o.x <= half.x + 1e-3 && o.y <= half.y + 1e-3,
                "{:?} off the field",
                d.at
            );
        }
        // Each squad's block stands at the centre of its soldiers in the line.
        for (q, men) in men.iter().enumerate() {
            let up: Vec<Vec2> = men
                .iter()
                .filter(|m| m.state == FighterState::Standing)
                .map(|m| Vec2::from(m.pos()))
                .collect();
            if up.is_empty() {
                continue;
            }
            let c = up.iter().sum::<Vec2>() / up.len() as f32;
            let f = v.frame.squads[q];
            assert!(
                (c - Vec2::new(f.0, f.1)).length() < 1.0,
                "squad {q}: {c} vs {f:?}"
            );
        }
        // The squad model has no soldiers to show.
        let (field, specs) = parse_lineup("swordsmen*30 vs spearmen*30").unwrap();
        let v = Battle::new(BattleParams::default(), field, specs, 1).live_view(
            0,
            (0.0, 0.0),
            (1.0, 0.0),
            0,
        );
        assert!(!v.on_soldiers() && !shows_soldiers(&v, 0.1));
    }

    /// The scenario's player raids a neighbour: the battle waits, the
    /// economy's clock holds while the loop steps it, and the player's order
    /// and "fight it out" go through the simulation's own calls.
    #[test]
    fn the_loop_holds_the_clock_and_steps_a_waiting_battle() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let mut s =
            sim_core::Simulation::from_paths(&sim_core::AssetPaths::from_root(&root)).unwrap();
        s.world.insert_resource(war::LiveBattles(true));
        s.world.insert_resource(LiveBattle::default());
        let player = s
            .world
            .resource::<sim_core::policy::PlayerTown>()
            .0
            .expect("the scenario has a player town") as usize;
        let (me, them) = {
            let towns = s.world.resource::<Towns>();
            let other = (0..towns.0.len()).find(|&t| t != player).unwrap();
            (towns.0[player].name.clone(), towns.0[other].name.clone())
        };
        s.order(&format!("army {me} 0.15")).unwrap();
        for _ in 0..4 {
            s.step();
        }
        s.order(&format!("raid {me} {them}")).unwrap();
        let mut waited = false;
        for _ in 0..20 {
            s.step();
            if run(&mut s.world, 0.0, false) {
                waited = true;
                break;
            }
        }
        assert!(waited, "the player's raid never reached a battle");
        let v = s.world.resource::<LiveBattle>().view.clone().unwrap();
        // Watched battles fight on the soldier model; auto-resolve stays on squads.
        assert!(
            v.on_soldiers(),
            "the live battle is not on the soldier model"
        );
        let t0 = v.frame.t;
        // It waits for the deployment until "Start battle".
        assert!(v.deploying);
        assert!(run(&mut s.world, 1.0, false));
        let view =
            |s: &sim_core::Simulation| s.world.resource::<LiveBattle>().view.clone().unwrap();
        assert_eq!(view(&s).frame.t, t0, "it stepped before the start");
        s.world.resource_mut::<LiveBattle>().start = true;
        run(&mut s.world, 0.0, false);
        assert!(!view(&s).deploying);
        // A second of real time steps the battle by BATTLE_SPEED seconds;
        // paused, it holds.
        assert!(run(&mut s.world, 1.0, false));
        let t1 = s
            .world
            .resource::<LiveBattle>()
            .view
            .as_ref()
            .unwrap()
            .frame
            .t;
        assert!(t1 > t0, "the battle stepped: {t0} -> {t1}");
        assert!(run(&mut s.world, 1.0, true));
        let t2 = s
            .world
            .resource::<LiveBattle>()
            .view
            .as_ref()
            .unwrap()
            .frame
            .t;
        assert_eq!(t1, t2, "paused, the battle holds");
        // The game's loop runs no economy step while the battle waits, even
        // at the fastest speed with steps due.
        s.world.insert_resource(crate::SimControl {
            paused: false,
            speed: crate::SPEEDS.len() - 1,
            due: 50.0,
        });
        let tick = s.world.resource::<sim_core::SimClock>().tick;
        let step = s.world.resource::<sim_core::SimClock>().step;
        crate::run_sim_tick(&mut s.world);
        let clock = s.world.resource::<sim_core::SimClock>();
        assert_eq!((clock.tick, clock.step), (tick, step), "the clock held");
        assert_eq!(s.world.resource::<crate::SimControl>().due, 0.0);
        let mine = v
            .squads
            .iter()
            .position(|q| q.side == v.player_side)
            .unwrap();
        s.world
            .resource_mut::<LiveBattle>()
            .commands
            .push((mine, Command::Orders(vec![Order::Hold])));
        run(&mut s.world, 0.0, true);
        assert_eq!(
            s.world
                .resource::<LiveBattle>()
                .view
                .as_ref()
                .unwrap()
                .orders[mine],
            [Order::Hold]
        );
        s.world.resource_mut::<LiveBattle>().resolve = true;
        assert!(!run(&mut s.world, 0.0, false), "fought out at once");
        assert!(
            s.world
                .resource::<LiveBattle>()
                .view
                .as_ref()
                .unwrap()
                .finished
        );
    }
}
