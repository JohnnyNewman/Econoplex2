//! The player's levers: a panel of buttons for the town the player governs.
//!
//! Buttons never change the simulation directly; they queue orders, which the
//! simulation applies at the start of its next day (see `sim_core::policy`).
//! Placing a building is a two-step order: pick the type, press Place, then click
//! the map where it should go (Esc cancels).

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use sim_core::policy::{Immigration, Order, OrderQueue, PlayerTown, School, War};
use sim_core::{Db, Towns};
use sim_data::Category;

/// Which product and domains the panel's pickers point at.
#[derive(Resource, Default)]
pub struct PolicyCursor {
    product: usize,
    school_domain: usize,
    charter_domain: usize,
    raid_target: usize,
    building: usize,
}

/// The building type waiting for a click on the map, if any.
#[derive(Resource, Default)]
pub struct Placing(pub Option<u32>);

#[derive(Component, Clone, Copy)]
pub enum Action {
    SubsidyProduct(i32),
    SubsidyAmount(i32),
    SchoolDomain(i32),
    SchoolSeats(i32),
    Immigration,
    CharterDomain(i32),
    CharterToggle,
    ArmyShare(i32),
    WarToggle,
    RaidTarget(i32),
    Raid,
    BuildType(i32),
    Place,
}

#[derive(Component, Clone, Copy, PartialEq)]
pub enum Field {
    Title,
    SubsidyProduct,
    SubsidyAmount,
    SchoolDomain,
    SchoolSeats,
    Immigration,
    CharterDomain,
    CharterState,
    ArmyShare,
    War,
    RaidTarget,
    BuildType,
    Commissions,
}

const BUTTON: Color = Color::srgb(0.22, 0.24, 0.22);
const BUTTON_HOVER: Color = Color::srgb(0.32, 0.36, 0.32);

/// Products a town can subsidize: everything made in a workshop, not buildings or land.
fn buildings(db: &Db) -> Vec<usize> {
    (0..db.content.products.len())
        .filter(|&p| db.category(p as u32) == Category::Building)
        .collect()
}

fn subsidizable(db: &Db) -> Vec<usize> {
    (0..db.content.products.len())
        .filter(|&p| db.category(p as u32) != Category::Building)
        .filter(|&p| db.content.products[p].opens.is_none())
        .collect()
}

pub fn spawn_policy_panel(mut commands: Commands) {
    let font = TextFont {
        font_size: FontSize::Px(13.0),
        ..default()
    };
    let text = |s: &str| (Text::new(s), font.clone());
    let button = |p: &mut ChildSpawnerCommands, label: &str, action: Action| {
        p.spawn((
            Button,
            Node {
                padding: UiRect::axes(Val::Px(6.0), Val::Px(1.0)),
                margin: UiRect::horizontal(Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(BUTTON),
            action,
        ))
        .with_children(|b| {
            b.spawn((Text::new(label), font.clone()));
        });
    };
    let value = |p: &mut ChildSpawnerCommands, field: Field, width: f32| {
        p.spawn((Node {
            width: Val::Px(width),
            justify_content: JustifyContent::Center,
            ..default()
        },))
            .with_children(|v| {
                v.spawn((Text::new(""), font.clone(), field));
            });
    };
    let row =
        |p: &mut ChildSpawnerCommands, label: &str, build: &dyn Fn(&mut ChildSpawnerCommands)| {
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                margin: UiRect::vertical(Val::Px(2.0)),
                ..default()
            })
            .with_children(|r| {
                r.spawn((Node {
                    width: Val::Px(84.0),
                    ..default()
                },))
                    .with_children(|l| {
                        l.spawn(text(label));
                    });
                build(r);
            });
        };

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(8.0),
                bottom: Val::Px(56.0),
                padding: UiRect::all(Val::Px(8.0)),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        ))
        .with_children(|p| {
            p.spawn((Text::new(""), font.clone(), Field::Title));
            row(p, "Subsidy", &|r| {
                button(r, "<", Action::SubsidyProduct(-1));
                value(r, Field::SubsidyProduct, 110.0);
                button(r, ">", Action::SubsidyProduct(1));
                button(r, "-", Action::SubsidyAmount(-1));
                value(r, Field::SubsidyAmount, 64.0);
                button(r, "+", Action::SubsidyAmount(1));
            });
            row(p, "School", &|r| {
                button(r, "<", Action::SchoolDomain(-1));
                value(r, Field::SchoolDomain, 110.0);
                button(r, ">", Action::SchoolDomain(1));
                button(r, "-", Action::SchoolSeats(-1));
                value(r, Field::SchoolSeats, 64.0);
                button(r, "+", Action::SchoolSeats(1));
            });
            row(p, "Charter", &|r| {
                button(r, "<", Action::CharterDomain(-1));
                value(r, Field::CharterDomain, 110.0);
                button(r, ">", Action::CharterDomain(1));
                button(r, "toggle", Action::CharterToggle);
                value(r, Field::CharterState, 44.0);
            });
            row(p, "Immigration", &|r| {
                button(r, "change", Action::Immigration);
                value(r, Field::Immigration, 90.0);
            });
            row(p, "Army", &|r| {
                button(r, "-", Action::ArmyShare(-1));
                value(r, Field::ArmyShare, 150.0);
                button(r, "+", Action::ArmyShare(1));
                button(r, "toggle", Action::WarToggle);
                value(r, Field::War, 90.0);
            });
            row(p, "Raid", &|r| {
                button(r, "<", Action::RaidTarget(-1));
                value(r, Field::RaidTarget, 190.0);
                button(r, ">", Action::RaidTarget(1));
                button(r, "march", Action::Raid);
            });
            row(p, "Build", &|r| {
                button(r, "<", Action::BuildType(-1));
                value(r, Field::BuildType, 110.0);
                button(r, ">", Action::BuildType(1));
                button(r, "place", Action::Place);
                value(r, Field::Commissions, 90.0);
            });
        });
}

/// Turn button presses into orders for the player's town.
#[allow(clippy::too_many_arguments)]
pub fn policy_buttons(
    mut buttons: Query<(&Interaction, &Action, &mut BackgroundColor), Changed<Interaction>>,
    mut cursor: ResMut<PolicyCursor>,
    mut queue: ResMut<OrderQueue>,
    mut placing: ResMut<Placing>,
    player: Res<PlayerTown>,
    towns: Res<Towns>,
    params: Res<sim_core::world::Params>,
    db: Res<Db>,
) {
    let Some(town) = player.0 else { return };
    let others: Vec<usize> = (0..towns.0.len()).filter(|&o| o != town as usize).collect();
    let kinds = buildings(&db);
    let policy = &towns.0[town as usize].policy;
    let products = subsidizable(&db);
    let nd = db.content.domains.len() as i32;
    let step = |i: usize, d: i32, n: i32| (i as i32 + d).rem_euclid(n.max(1)) as usize;
    for (interaction, action, mut color) in &mut buttons {
        match interaction {
            Interaction::Hovered => color.0 = BUTTON_HOVER,
            Interaction::None => color.0 = BUTTON,
            Interaction::Pressed => {
                color.0 = BUTTON_HOVER;
                let order = match *action {
                    Action::SubsidyProduct(d) => {
                        cursor.product = step(cursor.product, d, products.len() as i32);
                        None
                    }
                    Action::SubsidyAmount(d) => {
                        let product = products[cursor.product.min(products.len() - 1)];
                        let share = (policy.subsidy[product] + 0.1 * d as f32).max(0.0);
                        Some(Order::Subsidy {
                            town,
                            product: product as u32,
                            share: (share * 10.0).round() / 10.0,
                        })
                    }
                    Action::SchoolDomain(d) => {
                        cursor.school_domain = step(cursor.school_domain, d, nd);
                        // A running school switches subject with the picker.
                        policy.school.map(|s| Order::School {
                            town,
                            school: Some(School {
                                domain: cursor.school_domain as u32,
                                seats: s.seats,
                            }),
                        })
                    }
                    Action::SchoolSeats(d) => {
                        let seats = policy.school.map_or(0, |s| s.seats) as i32 + 10 * d;
                        Some(Order::School {
                            town,
                            school: (seats > 0).then_some(School {
                                domain: cursor.school_domain as u32,
                                seats: seats as u32,
                            }),
                        })
                    }
                    Action::CharterDomain(d) => {
                        cursor.charter_domain = step(cursor.charter_domain, d, nd);
                        None
                    }
                    Action::CharterToggle => Some(Order::Charter {
                        town,
                        domain: cursor.charter_domain as u32,
                        on: !policy.charters[cursor.charter_domain],
                    }),
                    Action::ArmyShare(d) => {
                        let now = policy.army_share.unwrap_or(params.military.soldier_share);
                        let share = ((now + 0.05 * d as f32) * 20.0).round() / 20.0;
                        Some(Order::Army {
                            town,
                            share: Some(share.max(0.0)),
                        })
                    }
                    Action::WarToggle => Some(Order::War {
                        town,
                        war: match policy.war {
                            War::Auto => War::Defend,
                            War::Defend => War::Auto,
                        },
                    }),
                    Action::RaidTarget(d) => {
                        cursor.raid_target = step(cursor.raid_target, d, others.len() as i32);
                        None
                    }
                    Action::Raid => others.get(cursor.raid_target).map(|&o| Order::Raid {
                        town,
                        target: o as u16,
                    }),
                    Action::BuildType(d) => {
                        cursor.building = step(cursor.building, d, kinds.len() as i32);
                        None
                    }
                    Action::Place => {
                        placing.0 = kinds.get(cursor.building).map(|&b| b as u32);
                        None
                    }
                    Action::Immigration => Some(Order::Immigration {
                        town,
                        policy: match policy.immigration {
                            Immigration::Open => Immigration::Encouraged,
                            Immigration::Encouraged => Immigration::Closed,
                            Immigration::Closed => Immigration::Open,
                        },
                    }),
                };
                queue.pending.extend(order);
            }
        }
    }
}

/// Show the player's town policy, including orders still waiting for the next day.
#[allow(clippy::too_many_arguments)]
pub fn update_policy_panel(
    mut fields: Query<(&mut Text, &Field)>,
    cursor: Res<PolicyCursor>,
    queue: Res<OrderQueue>,
    placing: Res<Placing>,
    player: Res<PlayerTown>,
    towns: Res<Towns>,
    params: Res<sim_core::world::Params>,
    db: Res<Db>,
) {
    let Some(t) = player.0 else {
        for (mut text, field) in &mut fields {
            if *field == Field::Title {
                text.0 = "No town to govern in this scenario".into();
            }
        }
        return;
    };
    let town = &towns.0[t as usize];
    let p = &town.policy;
    let products = subsidizable(&db);
    let product = products[cursor.product.min(products.len() - 1)];
    let waiting = queue.pending.len();
    let others: Vec<usize> = (0..towns.0.len()).filter(|&o| o != t as usize).collect();
    let kinds = buildings(&db);
    for (mut text, field) in &mut fields {
        text.0 = match field {
            Field::Title => format!(
                "Your town: {}   treasury {:.0}{}",
                town.name,
                town.treasury,
                if let Some(b) = placing.0 {
                    format!(
                        "\nClick the map to place the {} (Esc cancels)",
                        db.content.products[b as usize].name.to_lowercase()
                    )
                } else if waiting > 0 {
                    "   (orders take effect tomorrow)".into()
                } else {
                    String::new()
                }
            ),
            Field::ArmyShare => format!(
                "{:.0}% ({} soldiers)",
                p.army_share.unwrap_or(params.military.soldier_share) * 100.0,
                town.squad.len()
            ),
            Field::War => match (p.war, &town.raid) {
                (_, Some(r)) => format!("marching on {}", towns.0[r.target as usize].name),
                (War::Auto, None) => "raids freely".into(),
                (War::Defend, None) => "defends".into(),
            },
            Field::RaidTarget => match p.raid_order {
                Some(o) if town.squad.len() < params.military.min_raid_squad => format!(
                    "{} (needs {} soldiers)",
                    towns.0[o as usize].name, params.military.min_raid_squad
                ),
                Some(o) => format!("{} (ordered)", towns.0[o as usize].name),
                None => others
                    .get(cursor.raid_target)
                    .map_or(String::new(), |&o| towns.0[o].name.clone()),
            },
            Field::BuildType => kinds
                .get(cursor.building)
                .map_or(String::new(), |&b| db.content.products[b].name.clone()),
            Field::Commissions => match p.commissions.len() {
                0 => String::new(),
                n => format!("{n} ordered"),
            },
            Field::SubsidyProduct => db.content.products[product].name.clone(),
            Field::SubsidyAmount => format!("{:.0}%", p.subsidy[product] * 100.0),
            Field::SchoolDomain => match p.school {
                Some(s) if s.domain as usize != cursor.school_domain => format!(
                    "{} (now {})",
                    db.content.domains[cursor.school_domain], db.content.domains[s.domain as usize]
                ),
                _ => db.content.domains[cursor.school_domain].clone(),
            },
            Field::SchoolSeats => match p.school {
                Some(s) => format!("{} seats", s.seats),
                None => "closed".into(),
            },
            Field::CharterDomain => db.content.domains[cursor.charter_domain].clone(),
            Field::CharterState => if p.charters[cursor.charter_domain] {
                "on"
            } else {
                "off"
            }
            .into(),
            Field::Immigration => match p.immigration {
                Immigration::Open => "open",
                Immigration::Encouraged => "encouraged",
                Immigration::Closed => "closed",
            }
            .into(),
        };
    }
}

/// While a building is being placed, a left click on the map orders it there.
#[allow(clippy::too_many_arguments)]
pub fn place_building(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    buttons: Query<&Interaction, With<Button>>,
    mut placing: ResMut<Placing>,
    mut queue: ResMut<OrderQueue>,
    player: Res<PlayerTown>,
) {
    let (Some(building), Some(town)) = (placing.0, player.0) else {
        return;
    };
    if keys.just_pressed(KeyCode::Escape) {
        placing.0 = None;
        return;
    }
    if !mouse.just_pressed(MouseButton::Left) || buttons.iter().any(|i| *i != Interaction::None) {
        return;
    }
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let (cam, cam_tf) = *camera;
    let Ok(world) = cam.viewport_to_world_2d(cam_tf, cursor) else {
        return;
    };
    queue.pending.push(Order::Build {
        town,
        building,
        pos: Some((world.x, world.y)),
    });
    placing.0 = None;
}
