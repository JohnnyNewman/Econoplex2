//! The price of a battle in the 3D client (docs/design/war-ideas.md, idea
//! 2), as the 2D client shows it (docs/design/client-parity.md):
//!
//! - pointing at a soldier of the live battle names them first in its tip
//!   (battle_input.rs, through `rts::inspect`): "Mara, baker, 3 battles";
//! - pointing at a soldier of a battle being played back does the same, in a
//!   tip of its own (on the squad model, the first of the squad under the
//!   pointer still in the fight);
//! - a ticker at the foot of the screen tells each side's losses so far by
//!   trade, and the week's work they would have done: "Ashford lost: 2
//!   smiths, 5 farmers, about 300 loaves a week". It follows the live
//!   battle, else the newest battle being played back.
//!
//! The words are the simulation's (`sim_core::battle::price`), so both
//! clients say the same.

use crate::battle_input::ray_ground;
use crate::battles::{BattleScene, Geo};
use crate::camera::UiCapture;
use crate::live::LiveSim;
use crate::{World3d, SCALE};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

pub struct BattlePricePlugin;

impl Plugin for BattlePricePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(EguiPrimaryContextPass, price_ui);
    }
}

/// The ticker, and the tip over a battle being played back.
#[allow(clippy::too_many_arguments)]
fn price_ui(
    mut contexts: EguiContexts,
    live: Res<LiveSim>,
    scenes: Query<&BattleScene>,
    time: Res<Time>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    ui: Res<UiCapture>,
    world: Res<World3d>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let towns = &live.snapshot.towns;
    let name = |t: u16| {
        towns
            .get(t as usize)
            .map_or_else(|| "?".to_string(), |t| t.name.clone())
    };
    let now = time.elapsed_secs();
    // Oldest first, as the scenes started.
    let mut playing: Vec<(&BattleScene, f32)> =
        scenes.iter().map(|s| (s, s.shown_at(now))).collect();
    playing.sort_by(|a, b| a.0.started().total_cmp(&b.0.started()));
    let battle = live.live_battle();
    let line = match &battle {
        Some(v) => v.ticker(&name),
        None => playing
            .last()
            .and_then(|(s, t)| s.replay().ticker(*t, &name)),
    };
    if let Some(line) = line {
        egui::Area::new(egui::Id::new("battle price"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -48.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.add(egui::Label::new(line).extend());
                });
            });
    }
    // The pointer over a battle being played back (the live one has its own
    // tip).
    if battle.is_some() || ui.pointer {
        return;
    }
    let Some(cursor) = windows.iter().next().and_then(|w| w.cursor_position()) else {
        return;
    };
    let Some((cam, tf)) = cameras.iter().find(|(c, _)| c.is_active) else {
        return;
    };
    let Ok(ray) = cam.viewport_to_world(tf, cursor) else {
        return;
    };
    let t = &world.terrain;
    let who = playing.iter().rev().find_map(|(s, bt)| {
        let r = s.replay();
        let geo = Geo {
            at: r.at,
            facing: r.facing,
            field: r.field,
        };
        let h0 = geo.point_3d(t, (r.field.0 / 2.0, r.field.1 / 2.0)).y;
        let g = ray_ground(ray, t, h0)?;
        let f = geo.field_at((g.x * SCALE, -g.z * SCALE));
        let inside = (0.0..=r.field.0).contains(&f.0) && (0.0..=r.field.1).contains(&f.1);
        if !inside {
            return None;
        }
        r.inspect(*bt, f)
    });
    let Some(who) = who else { return };
    egui::Area::new(egui::Id::new("replay tip"))
        .fixed_pos(egui::pos2(cursor.x + 16.0, cursor.y + 16.0))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.add(egui::Label::new(who).extend());
            });
        });
}
