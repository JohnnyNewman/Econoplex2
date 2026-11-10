//! M5: data panels with egui (immediate-mode UI; graphics-ui.md, section 4).
//!
//! - **Town:** pick a town; population, treasury, ruler, its buildings, and its
//!   market (stock and price per good). "Look at" moves the camera there.
//! - **Govern:** the town's levers (taxes, wages, public spending, immigration,
//!   minting, war), raids and building commissions. Every button sends an order
//!   in the CLI's text syntax; the simulation applies it at its next tick, and
//!   the panel shows the new value once the next snapshot arrives.
//! - **Events:** the event log, newest first.
//!
//! The panels read only the snapshot (`LiveSim`) and act only through orders,
//! like the 2D game, the CLI and the AI governors.

use crate::camera::{UiCapture, WorldCamera};
use crate::live::LiveSim;
use crate::SCALE;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};

pub struct PanelsPlugin;

impl Plugin for PanelsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            .init_resource::<Panels>()
            .init_resource::<UiCapture>()
            .add_systems(EguiPrimaryContextPass, panels);
    }
}

/// What the panels have selected.
#[derive(Resource, Default)]
struct Panels {
    town: usize,
    build: usize,
    raid: usize,
    /// Orders sent, newest first, with the tick they were sent at.
    sent: Vec<(u64, String)>,
    errors: Vec<String>,
}

fn panels(
    mut contexts: EguiContexts,
    live: Res<LiveSim>,
    mut state: ResMut<Panels>,
    mut capture: ResMut<UiCapture>,
    mut cams: Query<&mut WorldCamera>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let snap = live.snapshot.clone();
    if snap.towns.is_empty() {
        return Ok(());
    }
    state.town = state.town.min(snap.towns.len() - 1);
    state.errors.extend(live.take_errors());
    let ti = state.town;
    let town = &snap.towns[ti];
    let send = |state: &mut Panels, text: String| {
        live.order_text(text.clone());
        state.sent.insert(0, (snap.tick, text));
        state.sent.truncate(8);
    };

    // ---------------------------------------------------------------- Town
    egui::Window::new("Town")
        .default_pos([12.0, 300.0])
        .default_width(260.0)
        .show(ctx, |ui| {
            egui::ComboBox::from_id_salt("town")
                .selected_text(&town.name)
                .show_ui(ui, |ui| {
                    for (i, t) in snap.towns.iter().enumerate() {
                        ui.selectable_value(&mut state.town, i, &t.name);
                    }
                });
            if ui.button("Look at").clicked() {
                for mut cam in &mut cams {
                    cam.focus = Vec3::new(town.pos.0 / SCALE, cam.focus.y, -town.pos.1 / SCALE);
                }
            }
            ui.separator();
            egui::Grid::new("stats").num_columns(2).show(ui, |ui| {
                ui.label("people");
                ui.label(town.population.to_string());
                ui.end_row();
                ui.label("treasury");
                ui.label(format!("{:.0}", town.treasury));
                ui.end_row();
                ui.label("rules itself");
                ui.label(match town.ruler {
                    Some(r) => format!("no, pays tribute to {}", snap.towns[r as usize].name),
                    None => "yes".into(),
                });
                ui.end_row();
            });
            ui.collapsing(format!("Buildings ({})", town.buildings.len()), |ui| {
                let mut kinds: Vec<(String, usize)> = Vec::new();
                for b in &town.buildings {
                    match kinds.iter_mut().find(|(k, _)| *k == b.kind) {
                        Some((_, n)) => *n += 1,
                        None => kinds.push((b.kind.clone(), 1)),
                    }
                }
                kinds.sort();
                for (k, n) in kinds {
                    ui.label(format!("{n} × {k}"));
                }
            });
            ui.collapsing("Market", |ui| {
                let mut goods = town.market.clone();
                goods.sort_by(|a, b| b.1.total_cmp(&a.1));
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        egui::Grid::new("market")
                            .striped(true)
                            .num_columns(3)
                            .show(ui, |ui| {
                                ui.strong("good");
                                ui.strong("stock");
                                ui.strong("price");
                                ui.end_row();
                                for (p, stock, price) in goods {
                                    ui.label(
                                        live.product_names
                                            .get(p as usize)
                                            .map_or("?", |s| s.as_str()),
                                    );
                                    ui.label(format!("{stock:.0}"));
                                    ui.label(format!("{price:.2}"));
                                    ui.end_row();
                                }
                            });
                    });
            });
        });

    // ---------------------------------------------------------------- Govern
    let name = town.name.clone();
    let pol = town.policy.clone();
    egui::Window::new(format!("Govern {name}"))
        .default_pos([1250.0, 60.0])
        .default_width(300.0)
        .show(ctx, |ui| {
            ui.label("Orders take effect at the next simulated day.");
            ui.separator();
            // A lever: its value and − / + buttons that order a new value.
            let lever = |ui: &mut egui::Ui,
                         state: &mut Panels,
                         label: &str,
                         value: f32,
                         step: f32,
                         cmd: &dyn Fn(f32) -> String| {
                ui.horizontal(|ui| {
                    ui.label(format!("{label:<16}"));
                    if ui.small_button("−").clicked() {
                        send(state, cmd((value - step).max(0.0)));
                    }
                    ui.monospace(format!("{value:>5.2}"));
                    if ui.small_button("+").clicked() {
                        send(state, cmd(value + step));
                    }
                });
            };
            lever(
                ui,
                &mut state,
                "wealth tax / yr",
                pol.wealth_tax,
                0.25,
                &|v| format!("tax {name} wealth {v:.2}"),
            );
            lever(ui, &mut state, "income tax", pol.income_tax, 0.05, &|v| {
                format!("tax {name} income {v:.2}")
            });
            lever(ui, &mut state, "wage share", pol.wage_share, 0.05, &|v| {
                format!("wages {name} {v:.2}")
            });
            lever(
                ui,
                &mut state,
                "public spending",
                pol.public_spending,
                0.5,
                &|v| format!("spending {name} {v:.2}"),
            );
            ui.separator();
            let choice = |ui: &mut egui::Ui,
                          state: &mut Panels,
                          label: &str,
                          now: &str,
                          options: &[&str],
                          cmd: &str| {
                ui.horizontal(|ui| {
                    ui.label(format!("{label:<16}"));
                    for o in options {
                        if ui.selectable_label(now == *o, *o).clicked() && now != *o {
                            send(state, format!("{cmd} {name} {o}"));
                        }
                    }
                });
            };
            choice(
                ui,
                &mut state,
                "immigration",
                &pol.immigration,
                &["open", "encouraged", "closed"],
                "immigration",
            );
            choice(
                ui,
                &mut state,
                "minting",
                &pol.minting,
                &["auto", "on", "off"],
                "mint",
            );
            choice(ui, &mut state, "war", &pol.war, &["auto", "defend"], "war");
            ui.separator();
            // Raids.
            let others: Vec<usize> = (0..snap.towns.len()).filter(|&o| o != ti).collect();
            state.raid = state.raid.min(others.len().saturating_sub(1));
            ui.horizontal(|ui| {
                ui.label("raid");
                if let Some(&target) = others.get(state.raid) {
                    egui::ComboBox::from_id_salt("raid")
                        .selected_text(&snap.towns[target].name)
                        .show_ui(ui, |ui| {
                            for (k, &o) in others.iter().enumerate() {
                                ui.selectable_value(&mut state.raid, k, &snap.towns[o].name);
                            }
                        });
                    if ui.button("Raid").clicked() {
                        send(
                            &mut state,
                            format!("raid {name} {}", snap.towns[target].name),
                        );
                    }
                }
            });
            // Building commissions.
            let kinds = &live.building_kinds;
            state.build = state.build.min(kinds.len().saturating_sub(1));
            ui.horizontal(|ui| {
                ui.label("build");
                if let Some((id, label)) = kinds.get(state.build) {
                    egui::ComboBox::from_id_salt("build")
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            for (k, (_, l)) in kinds.iter().enumerate() {
                                ui.selectable_value(&mut state.build, k, l);
                            }
                        });
                    if ui.button("Commission").clicked() {
                        send(&mut state, format!("build {name} {id}"));
                    }
                }
            });
            ui.label(format!("commissions waiting: {}", pol.commissions));
            if !state.sent.is_empty() {
                ui.separator();
                ui.label("sent:");
                for (tick, text) in &state.sent {
                    ui.small(format!("day {tick}: {text}"));
                }
            }
            if !state.errors.is_empty() {
                ui.separator();
                for e in state.errors.iter().rev().take(3) {
                    ui.colored_label(egui::Color32::LIGHT_RED, e);
                }
            }
        });

    // ---------------------------------------------------------------- Events
    egui::Window::new("Events")
        .default_pos([1250.0, 620.0])
        .default_width(330.0)
        .default_open(false)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .show(ui, |ui| {
                    for e in &live.recent {
                        ui.small(format!("day {}: {}", e.tick, e.text));
                    }
                });
        });

    capture.pointer = ctx.egui_wants_pointer_input() || ctx.is_pointer_over_egui();
    capture.keyboard = ctx.egui_wants_keyboard_input();
    Ok(())
}
