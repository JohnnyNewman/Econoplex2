//! The price of a battle in the 2D client (docs/design/war-ideas.md, idea
//! 2), as the 3D client shows it (docs/design/client-parity.md):
//!
//! - pointing at a soldier of the live battle names them first in its tip
//!   (battle_input.rs, through `rts::inspect`): "Mara, baker, 3 battles";
//! - pointing at a soldier of a battle being played back does the same, in a
//!   tip of its own (on the squad model, the first of the squad under the
//!   pointer still in the fight);
//! - a ticker at the bottom of the screen tells each side's losses so far
//!   by trade, and the week's work they would have done: "Ashford lost: 2
//!   smiths, 5 farmers, about 300 loaves a week". It follows the live
//!   battle, else the newest battle being played back.
//!
//! The words are the simulation's (`sim_core::battle::price`), so both
//! clients say the same.

use crate::battles::{map_to_field, BattleScenes};
use crate::live_battle::LiveBattle;
use crate::view::WorldCamera;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use sim_core::Towns;

/// The ticker's box, across the bottom of the screen.
#[derive(Component)]
pub struct PriceTicker;

/// The ticker's words.
#[derive(Component)]
pub struct PriceText;

/// The tip by the pointer over a battle being played back.
#[derive(Component)]
pub struct ReplayTip;

const TEXT: Color = Color::srgb(1.0, 0.95, 0.8);
const BACK: Color = Color::srgba(0.0, 0.0, 0.0, 0.7);

pub fn spawn_price(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(40.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Visibility::Hidden,
            bevy::picking::Pickable::IGNORE,
            PriceTicker,
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(TEXT),
                BackgroundColor(BACK),
                Node {
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                    ..default()
                },
                bevy::picking::Pickable::IGNORE,
                PriceText,
            ));
        });
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(13.0),
            ..default()
        },
        TextColor(TEXT),
        BackgroundColor(BACK),
        Node {
            position_type: PositionType::Absolute,
            padding: UiRect::all(Val::Px(4.0)),
            ..default()
        },
        Visibility::Hidden,
        bevy::picking::Pickable::IGNORE,
        ReplayTip,
    ));
}

/// Refresh the ticker and the replay tip.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_price(
    live: Res<LiveBattle>,
    scenes: Res<BattleScenes>,
    time: Res<Time>,
    towns: Res<Towns>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform), With<WorldCamera>>,
    mut ticker: Single<&mut Visibility, With<PriceTicker>>,
    mut words: Single<&mut Text, (With<PriceText>, Without<ReplayTip>)>,
    mut tip: Single<
        (&mut Text, &mut Node, &mut Visibility),
        (With<ReplayTip>, Without<PriceTicker>),
    >,
) {
    let name = |t: u16| {
        towns
            .0
            .get(t as usize)
            .map_or_else(|| "?".to_string(), |t| t.name.clone())
    };
    let now = time.elapsed_secs();
    let playing = scenes.playing(now);
    let line = match &live.view {
        Some(v) => v.ticker(&name),
        None => playing.last().and_then(|(r, t)| r.ticker(*t, &name)),
    };
    match line {
        Some(line) => {
            if words.0 != line {
                words.0 = line;
            }
            ticker.set_if_neq(Visibility::Inherited);
        }
        None => {
            ticker.set_if_neq(Visibility::Hidden);
        }
    }
    // The pointer over a battle being played back (the live one has its own
    // tip).
    let (cam, cam_tf) = *camera;
    let cursor = window.cursor_position();
    let who = cursor
        .filter(|_| live.view.is_none())
        .and_then(|c| Some((c, cam.viewport_to_world_2d(cam_tf, c).ok()?)))
        .and_then(|(c, world)| {
            playing.iter().rev().find_map(|(r, t)| {
                let f = map_to_field(r.at, r.facing, r.field)(world);
                let inside = (0.0..=r.field.0).contains(&f.x) && (0.0..=r.field.1).contains(&f.y);
                inside
                    .then(|| r.inspect(*t, (f.x, f.y)))
                    .flatten()
                    .map(|w| (c, w))
            })
        });
    let (text, node, vis) = &mut *tip;
    match who {
        Some((c, w)) => {
            node.left = Val::Px(c.x + 16.0);
            node.top = Val::Px(c.y + 16.0);
            if text.0 != w {
                text.0 = w;
            }
            vis.set_if_neq(Visibility::Inherited);
        }
        None => {
            vis.set_if_neq(Visibility::Hidden);
        }
    }
}
