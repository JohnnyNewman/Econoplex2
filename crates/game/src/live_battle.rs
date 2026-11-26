//! Live battles in the 2D client, as the 3D client fights them (live.rs and
//! battles.rs there; docs/design/client-parity.md, group 4).
//!
//! A battle the player's own town fights waits for the player
//! (`sim_core::systems::military::LiveBattles`). While it waits, the sim loop
//! (`main.rs::run_sim_tick`) holds the economy's clock and steps the battle in
//! real time instead, `BATTLE_SPEED` battle seconds a second, unless the game
//! is paused. Everything goes through the simulation's own calls
//! (`order_live_battle`, `step_live_battle`, `resolve_live_battle`,
//! `live_battle_view`); once the battle is over, the next economy step settles
//! it as usual.
//!
//! The squads are drawn on the map with battles.rs, from the battle's current
//! frame. A panel at the top lists every squad: its fighters left, state and
//! morale, and for the player's fighting squads the orders: own judgment,
//! hold, withdraw, or attack (archers also shoot) a named enemy squad. "Look
//! at the field" moves the camera there; "Fight it out" settles it at once.

use crate::ui_compat::Interaction;
use bevy::prelude::*;
use sim_core::battle::{LiveView, Order, SquadState};
use sim_core::systems::military as war;
use sim_core::Towns;

/// Battle seconds per real second (as the 3D client).
pub const BATTLE_SPEED: f32 = 6.0;

/// The live battle as the view last saw it, and what the player asked of it
/// since (applied by the sim loop before its next step).
#[derive(Resource, Default)]
pub struct LiveBattle {
    pub view: Option<LiveView>,
    pub orders: Vec<(usize, Order)>,
    pub resolve: bool,
    /// Battle seconds due and not yet stepped.
    due: f32,
}

/// Called by the sim loop on every fixed update before the economy: apply the
/// player's orders, step a waiting battle by the real time passed (unless
/// paused), and refresh the view. Returns whether a battle waits, in which
/// case the economy's clock holds.
pub fn run(world: &mut World, real_secs: f32, paused: bool) -> bool {
    let (orders, resolve) = {
        let mut live = world.resource_mut::<LiveBattle>();
        (
            std::mem::take(&mut live.orders),
            std::mem::take(&mut live.resolve),
        )
    };
    for (squad, order) in orders {
        war::order_live_battle(world, squad, order);
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

/// Draw the live battle's squads where they stand now.
pub fn draw_live_battle(mut gizmos: Gizmos, time: Res<Time>, live: Res<LiveBattle>) {
    let Some(v) = &live.view else { return };
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
        Color::srgba(1.0, 0.95, 0.5, 0.8),
    );
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

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub enum BattleAction {
    Order(usize, Order),
    Look,
    FightOut,
}

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
    p.spawn((
        crate::ui_compat::button(),
        Node {
            padding: UiRect::axes(Val::Px(6.0), Val::Px(1.0)),
            margin: UiRect::all(Val::Px(2.0)),
            ..default()
        },
        BackgroundColor(if chosen { CHOSEN } else { BUTTON }),
        action,
    ))
    .with_children(|b| {
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
            BackgroundColor(Color::srgba(0.05, 0.04, 0.02, 0.85)),
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
                    ..default()
                },
                SquadRows,
            ));
        });
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

/// The orders a squad can be given: its own judgment, hold, withdraw, and
/// attack each enemy on the field (archers, by name as in the 3D client, also
/// shoot).
pub fn choices(v: &LiveView, q: usize) -> Vec<(String, Order)> {
    let mut out = vec![
        ("Own judgment".to_string(), Order::Auto),
        ("Hold".to_string(), Order::Hold),
        ("Withdraw".to_string(), Order::Withdraw),
    ];
    let archer = v.squads[q].name.to_lowercase().contains("bow");
    for e in enemies(v) {
        let name = &v.squads[e].name;
        out.push((format!("Attack {name}"), Order::Attack(e)));
        if archer {
            out.push((format!("Shoot {name}"), Order::Shoot(e)));
        }
    }
    out
}

/// What the rows are built from; they are rebuilt only when this changes.
type Layout = (usize, Vec<Option<Vec<(String, Order)>>>, Vec<Order>);

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
    mut texts: Query<(&mut Text, &SquadText), Without<BattleTitle>>,
    mut layout: Local<Option<Layout>>,
) {
    let Some(v) = &live.view else {
        root.set_if_neq(Visibility::Hidden);
        if layout.is_some() {
            commands.entity(*rows).despawn_children();
            *layout = None;
        }
        return;
    };
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
                let now = v.orders.get(q).copied().unwrap_or(Order::Auto);
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                })
                .with_children(|r| {
                    for (label, o) in opts {
                        spawn_button(r, label, BattleAction::Order(q, *o), *o == now);
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
        let side = if sq.side == v.player_side {
            "yours"
        } else {
            "enemy"
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
    mut buttons: Query<(&Interaction, &BattleAction, &mut BackgroundColor), Changed<Interaction>>,
    mut live: ResMut<LiveBattle>,
    mut follow: ResMut<crate::view::Follow>,
    mut camera: Single<&mut Transform, With<Camera2d>>,
) {
    for (interaction, action, mut colour) in &mut buttons {
        match interaction {
            Interaction::Hovered => colour.0 = HOVER,
            Interaction::None => colour.0 = BUTTON,
            Interaction::Pressed => {
                colour.0 = HOVER;
                match *action {
                    BattleAction::Order(q, o) => live.orders.push((q, o)),
                    BattleAction::FightOut => live.resolve = true,
                    BattleAction::Look => {
                        if let Some(v) = &live.view {
                            follow.0 = false;
                            crate::view::look_at(&mut camera, v.at);
                            let z = camera.scale.x.min(0.8);
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
    use sim_core::battle::{ReplayFrame, ReplaySquad};

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
            orders: vec![Order::Auto; 3],
            morale: vec![1.0; 3],
            frame: ReplayFrame {
                t: 30.0,
                squads: vec![
                    (10.0, 10.0, 20, SquadState::Fighting),
                    (10.0, 30.0, 0, SquadState::Destroyed),
                    (80.0, 20.0, 15, SquadState::Fighting),
                ],
            },
            player_side: 0,
            finished: false,
            winner: None,
        }
    }

    #[test]
    fn squads_get_the_orders_the_3d_client_offers() {
        let v = view();
        assert_eq!(enemies(&v), vec![2]);
        let bow: Vec<Order> = choices(&v, 0).into_iter().map(|c| c.1).collect();
        assert_eq!(
            bow,
            vec![
                Order::Auto,
                Order::Hold,
                Order::Withdraw,
                Order::Attack(2),
                Order::Shoot(2)
            ]
        );
        assert!(orderable(&v, 0));
        // A destroyed squad and the enemy take no orders.
        assert!(!orderable(&v, 1) && !orderable(&v, 2));
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
        let t0 = v.frame.t;
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
            .orders
            .push((mine, Order::Hold));
        run(&mut s.world, 0.0, true);
        assert_eq!(
            s.world
                .resource::<LiveBattle>()
                .view
                .as_ref()
                .unwrap()
                .orders[mine],
            Order::Hold
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
