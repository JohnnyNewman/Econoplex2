//! The win-chance forecast in the 2D client (docs/design/war-ideas.md,
//! idea 1), as the 3D client shows it (docs/design/client-parity.md): while
//! the player's battle is fought, a box at the lower left says, for the
//! orders as given and for charging, holding or withdrawing now, how likely
//! the player's side is to win and how many would die on each side, with a
//! 90% range that narrows as the runs come in.
//!
//! The sim loop (live_battle.rs) feeds `sim_core::battle::forecast::Forecaster`,
//! which runs the squad-model simulations on a thread of its own, off the
//! frame; the words are the simulation's, so both clients say the same.

use crate::live_battle::LiveBattle;
use bevy::prelude::*;
use sim_core::battle::forecast::{self, Forecaster};

pub struct ForecastPlugin;

impl Plugin for ForecastPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Forecast(Forecaster::new()))
            .add_systems(Startup, spawn_forecast)
            .add_systems(Update, update_forecast);
    }
}

/// The forecaster for the battle waiting for the player.
#[derive(Resource)]
pub struct Forecast(Forecaster);

/// The forecast's box.
#[derive(Component)]
struct ForecastText;

/// Called by the sim loop after each battle step (live_battle.rs): feed the
/// forecaster the battle as it stands, every `Forecaster::EVERY` battle
/// seconds. Nothing without the plugin (as in tests of the sim loop).
pub fn feed(world: &mut World) {
    if !world.contains_resource::<Forecast>() {
        return;
    }
    world.resource_scope(|world, mut f: Mut<Forecast>| {
        sim_core::systems::military::feed_forecast(world, &mut f.0);
    });
}

fn spawn_forecast(mut commands: Commands) {
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
            left: Val::Px(8.0),
            bottom: Val::Px(96.0),
            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
            ..default()
        },
        Visibility::Hidden,
        bevy::picking::Pickable::IGNORE,
        ForecastText,
    ));
}

fn update_forecast(
    live: Res<LiveBattle>,
    f: Res<Forecast>,
    mut text: Single<(&mut Text, &mut Visibility), With<ForecastText>>,
) {
    let (forecasts, _) = f.0.latest();
    let fighting = live.view.as_ref().is_some_and(|v| !v.deploying);
    let (words, shown) = &mut *text;
    if !fighting || forecasts.is_empty() {
        shown.set_if_neq(Visibility::Hidden);
        return;
    }
    let line = forecast::words(&forecasts);
    if words.0 != line {
        words.0 = line;
    }
    shown.set_if_neq(Visibility::Inherited);
}
