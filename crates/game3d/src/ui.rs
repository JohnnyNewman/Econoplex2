//! M5: data panels with egui (immediate-mode UI; graphics-ui.md, section 4).
//!
//! - **Town:** pick a town; population, treasury, ruler, its buildings, and its
//!   market (stock and price per good). "Look at" moves the camera there.
//! - **Govern:** the town's levers (taxes, wages, public spending, immigration,
//!   minting, war), raids and building commissions. Every button sends an order
//!   in the CLI's text syntax; the simulation applies it at its next tick, and
//!   the panel shows the new value once the next snapshot arrives.
//! - **Why** (M7): the selected town's problems and their causes, as a tree
//!   (from `sim_core::explain`); building types it names are ringed on the map.
//! - **Events:** the event log, newest first.
//! - **History:** monthly line charts per town (population, treasury, food per
//!   resident, mean wealth, bread price, soldiers, buildings), the selected town
//!   drawn bold.
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
            .init_resource::<WhyHighlight>()
            .add_systems(EguiPrimaryContextPass, panels);
    }
}

/// What the panels have selected.
#[derive(Resource, Default)]
struct Panels {
    town: usize,
    build: usize,
    raid: usize,
    /// Which history series the chart shows.
    metric: usize,
    /// Orders sent, newest first, with the tick they were sent at.
    sent: Vec<(u64, String)>,
    errors: Vec<String>,
}

fn panels(
    mut contexts: EguiContexts,
    live: Res<LiveSim>,
    mut state: ResMut<Panels>,
    mut capture: ResMut<UiCapture>,
    mut highlight: ResMut<WhyHighlight>,
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
        .default_pos([820.0, 860.0])
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

    // ---------------------------------------------------------------- Why
    // Ask the simulation to explain the selected town after every tick.
    live.explain(Some(ti as u16));
    highlight.town = ti;
    highlight.kinds.clear();
    egui::Window::new(format!("Why? {}", town.name))
        .default_pos([430.0, 60.0])
        .default_width(420.0)
        .show(ctx, |ui| match &snap.why {
            Some((t, found)) if *t as usize == ti => {
                if found.is_empty() {
                    ui.label("Nothing stands out: food, goods, treasury and people look fine.");
                }
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for (k, f) in found.iter().enumerate() {
                            finding(ui, f, k, &mut highlight.kinds);
                        }
                    });
                ui.small("Rule-based: what is short, what stops it, where the money goes.");
            }
            _ => {
                ui.label("Explaining… (the answer comes with the next simulated day)");
            }
        });

    // ---------------------------------------------------------------- History
    egui::Window::new("History")
        .default_pos([1220.0, 560.0])
        .default_width(380.0)
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (k, (label, _)) in METRICS.iter().enumerate() {
                    ui.selectable_value(&mut state.metric, k, *label);
                }
            });
            chart(
                ui,
                &live.history,
                snap.towns.len(),
                ti,
                METRICS[state.metric].1,
            );
            ui.small(format!(
                "{} (bold), monthly; {} samples",
                snap.towns[ti].name,
                live.history.len()
            ));
        });

    capture.pointer = ctx.egui_wants_pointer_input() || ctx.is_pointer_over_egui();
    capture.keyboard = ctx.egui_wants_keyboard_input();
    Ok(())
}

/// The history series a chart can show.
type Metric = fn(&sim_core::history::Sample) -> f32;
const METRICS: [(&str, Metric); 7] = [
    ("people", |s| s.population as f32),
    ("treasury", |s| s.treasury),
    ("food / resident", |s| s.food_per_capita),
    ("mean wealth", |s| s.mean_wealth),
    ("bread price", |s| s.bread_price),
    ("soldiers", |s| s.soldiers as f32),
    ("buildings", |s| s.buildings as f32),
];

/// A line per town, drawn with egui's painter: x is time, y the metric, both
/// scaled to the data's range.
fn chart(
    ui: &mut egui::Ui,
    samples: &[sim_core::history::Sample],
    towns: usize,
    selected: usize,
    metric: Metric,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(360.0, 170.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, egui::Color32::from_black_alpha(90));
    if samples.len() < 2 {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "the first samples come after a month",
            egui::FontId::proportional(13.0),
            egui::Color32::GRAY,
        );
        return;
    }
    let (t0, t1) = samples
        .iter()
        .fold((u64::MAX, 0), |(a, b), s| (a.min(s.tick), b.max(s.tick)));
    let (v0, v1) = samples.iter().fold((f32::MAX, f32::MIN), |(a, b), s| {
        let v = metric(s);
        (a.min(v), b.max(v))
    });
    let (v0, v1) = if v1 - v0 < 1e-3 {
        (v0 - 1.0, v1 + 1.0)
    } else {
        (v0, v1)
    };
    let inner = rect.shrink(8.0);
    let at = |tick: u64, v: f32| {
        egui::pos2(
            inner.left() + inner.width() * (tick - t0) as f32 / (t1 - t0).max(1) as f32,
            inner.bottom() - inner.height() * (v - v0) / (v1 - v0),
        )
    };
    for town in 0..towns {
        let points: Vec<egui::Pos2> = samples
            .iter()
            .filter(|s| s.town as usize == town)
            .map(|s| at(s.tick, metric(s)))
            .collect();
        let hue = town as f32 / towns.max(1) as f32;
        let color: egui::Color32 = egui::ecolor::Hsva::new(hue, 0.6, 0.95, 1.0).into();
        let width = if town == selected { 2.6 } else { 1.0 };
        let color = if town == selected {
            color
        } else {
            color.gamma_multiply(0.6)
        };
        painter.add(egui::Shape::line(points, egui::Stroke::new(width, color)));
    }
    let label = |pos: egui::Pos2, align, text: String| {
        painter.text(
            pos,
            align,
            text,
            egui::FontId::monospace(11.0),
            egui::Color32::LIGHT_GRAY,
        );
    };
    label(inner.left_top(), egui::Align2::LEFT_TOP, format!("{v1:.1}"));
    label(
        inner.left_bottom(),
        egui::Align2::LEFT_BOTTOM,
        format!("{v0:.1}"),
    );
    label(
        inner.right_bottom(),
        egui::Align2::RIGHT_BOTTOM,
        format!("day {t1}"),
    );
}

/// The building types the "why" view names, for the map (overlay.rs rings them).
#[derive(Resource, Default)]
pub struct WhyHighlight {
    pub town: usize,
    pub kinds: Vec<String>,
}

/// One finding and, folded, its causes. Worse findings are redder.
fn finding(ui: &mut egui::Ui, f: &sim_core::explain::Finding, id: usize, kinds: &mut Vec<String>) {
    if let Some(k) = &f.building {
        if !kinds.contains(k) {
            kinds.push(k.clone());
        }
    }
    let warm = egui::Color32::from_rgb(
        200 + (55.0 * f.severity) as u8,
        (220.0 - 120.0 * f.severity) as u8,
        (180.0 - 120.0 * f.severity) as u8,
    );
    let text = egui::RichText::new(&f.text).color(warm);
    if f.because.is_empty() {
        ui.label(text);
    } else {
        egui::CollapsingHeader::new(text)
            .id_salt((id, &f.text))
            .default_open(f.severity > 0.6)
            .show(ui, |ui| {
                for (k, c) in f.because.iter().enumerate() {
                    finding(ui, c, id * 31 + k + 1, kinds);
                }
            });
    }
}
