//! The win-chance forecast in the 3D client (docs/design/war-ideas.md,
//! idea 1), as the 2D client shows it (docs/design/client-parity.md): while
//! the player's battle is fought, a box at the lower left says, for the
//! orders as given and for charging, holding or withdrawing now, how likely
//! the player's side is to win and how many would die on each side, with a
//! 90% range that narrows as the runs come in.
//!
//! The sim thread (live.rs) feeds `sim_core::battle::forecast::Forecaster`,
//! which runs the squad-model simulations on a thread of its own, off the
//! frame; the words are the simulation's, so both clients say the same.

use crate::live::LiveSim;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use sim_core::battle::forecast;

pub struct ForecastPlugin;

impl Plugin for ForecastPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(EguiPrimaryContextPass, forecast_ui);
    }
}

fn forecast_ui(mut contexts: EguiContexts, live: Res<LiveSim>) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let Some(view) = live.live_battle().filter(|v| !v.deploying) else {
        return;
    };
    let (forecasts, _) = live.forecast();
    if forecasts.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::new("battle forecast"))
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(8.0, -96.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                let words = forecast::words_in(&forecasts, view.conditions.as_deref());
                ui.add(egui::Label::new(words).extend());
            });
        });
}
