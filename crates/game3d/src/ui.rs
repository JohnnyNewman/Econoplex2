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
//! - **Product space** (overlay `V`, fourth stop): the selected town's skills
//!   on a circle grouped by domain, linked by similarity, lit where the town
//!   produced or its people know the skill, sized by its residents' level.
//! - **Culture** (overlay `V`, third stop): a legend with the town's culture.
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
            .init_resource::<UiHidden>()
            .add_systems(Update, hide_ui)
            .add_systems(EguiPrimaryContextPass, panels);
    }
}

/// `H` (or `--no-ui`) hides all panels and text, to look at the world alone.
#[derive(Resource, Default)]
pub struct UiHidden(pub bool);

/// Toggle with `H`; hide or show the text overlays (the egui panels check the
/// flag themselves).
fn hide_ui(
    keys: Res<ButtonInput<KeyCode>>,
    mut hidden: ResMut<UiHidden>,
    mut roots: Query<&mut Visibility, (With<Node>, Without<ChildOf>)>,
) {
    if keys.just_pressed(KeyCode::KeyH) {
        hidden.0 = !hidden.0;
    }
    let want = if hidden.0 {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for mut v in &mut roots {
        v.set_if_neq(want);
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

#[allow(clippy::too_many_arguments)]
fn panels(
    mut contexts: EguiContexts,
    overlay: Res<crate::overlay::Overlay>,
    space: Res<Space>,
    live: Res<LiveSim>,
    mut state: ResMut<Panels>,
    mut capture: ResMut<UiCapture>,
    mut highlight: ResMut<WhyHighlight>,
    mut cams: Query<&mut WorldCamera>,
    hidden: Res<UiHidden>,
    mut selected: ResMut<crate::inspect::Selected>,
) -> Result {
    if hidden.0 {
        return Ok(());
    }
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
            ui.collapsing("Weather", |ui| {
                let (env, _, _, _) = live.env();
                let local = env.at(ti);
                let w = local.weather;
                egui::Grid::new("weather").num_columns(2).show(ui, |ui| {
                    ui.label("time");
                    ui.label(format!(
                        "{:02.0}:00, {:?}, {}",
                        env.hour.floor(),
                        env.phase,
                        season_name(env.season)
                    ));
                    ui.end_row();
                    ui.label("temperature");
                    ui.label(format!(
                        "{:.0} °C (today's mean {:.0} °C)",
                        w.temperature, local.day_mean_c
                    ));
                    ui.end_row();
                    ui.label("sky");
                    ui.label(sky_words(w.cloud, w.precip, w.snow_fraction()));
                    ui.end_row();
                    ui.label("wind");
                    ui.label(format!("{:.0} m/s", w.wind_speed()));
                    ui.end_row();
                    ui.label("ground");
                    ui.label(format!(
                        "snow {:.0}%, wet {:.0}%",
                        local.ground.snow * 100.0,
                        local.ground.wet * 100.0
                    ));
                    ui.end_row();
                });
            });
            ui.collapsing("Food and stores", |ui| {
                egui::Grid::new("stores").num_columns(2).show(ui, |ui| {
                    ui.label("food in stock");
                    ui.label(format!("{:.0} days", town.food_days));
                    ui.end_row();
                    ui.label("granary");
                    ui.label(if town.granary_days > 0.0 {
                        format!(
                            "{:.0} food (aims for {:.0} days)",
                            town.granary_food, town.granary_days
                        )
                    } else {
                        "none".into()
                    });
                    ui.end_row();
                    ui.label("seed grain");
                    ui.label(if town.seed_reserve {
                        "held back"
                    } else {
                        "free to eat"
                    });
                    ui.end_row();
                    ui.label("starving this year");
                    ui.label(format!("{} person-days", town.starving));
                    ui.end_row();
                    ui.label("cold so far");
                    ui.label(format!("{} person-days", town.cold_days));
                    ui.end_row();
                });
                if !town.armory.is_empty() {
                    ui.label("armory:");
                    for &(p, n) in &town.armory {
                        ui.label(format!(
                            "  {n} × {}",
                            live.product_names
                                .get(p as usize)
                                .map_or("?", |s| s.as_str())
                        ));
                    }
                }
            });
            if !town.needs.is_empty() {
                ui.collapsing("Needs", |ui| {
                    for (k, need) in sim_data::Need::ALL.iter().enumerate() {
                        let v = town.needs.get(k).copied().unwrap_or(0.0);
                        ui.horizontal(|ui| {
                            ui.add_sized([90.0, 16.0], egui::Label::new(format!("{need:?}")));
                            ui.add(
                                egui::ProgressBar::new(v.clamp(0.0, 1.0))
                                    .desired_width(140.0)
                                    .text(format!("{:.0}%", v * 100.0)),
                            );
                        });
                    }
                });
            }
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

    // ---------------------------------------------------------------- Overlays
    match *overlay {
        crate::overlay::Overlay::ProductSpace => {
            let skills = snap
                .skills
                .as_ref()
                .filter(|(t, _)| *t as usize == ti)
                .map(|(_, s)| s);
            egui::Window::new(format!("Product space: {}", town.name))
                .default_pos([430.0, 300.0])
                .show(ctx, |ui| product_space(ui, &space.0, skills));
        }
        crate::overlay::Overlay::Culture => {
            egui::Window::new(format!("Culture: {}", town.name))
                .default_pos([430.0, 300.0])
                .show(ctx, |ui| culture_legend(ui, &town.culture));
        }
        _ => {}
    }

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

    person_window(ctx, &live, &snap, &mut selected);

    capture.pointer = ctx.egui_wants_pointer_input() || ctx.is_pointer_over_egui();
    capture.keyboard = ctx.egui_wants_keyboard_input();
    Ok(())
}

/// The "Person" panel: the selected person as the simulation describes them
/// (inspect.rs). Household members are buttons that select them.
fn person_window(
    ctx: &egui::Context,
    live: &LiveSim,
    snap: &sim_core::view::ViewSnapshot,
    selected: &mut crate::inspect::Selected,
) {
    let (Some(id), info) = live.person() else {
        return;
    };
    let mut open = true;
    let mut pick: Option<Option<u32>> = None;
    egui::Window::new("Person")
        .id(egui::Id::new("person"))
        .open(&mut open)
        .default_pos([12.0, 120.0])
        .default_width(260.0)
        .show(ctx, |ui| {
            let Some(p) = info else {
                ui.label(format!("#{id}: no longer alive, or being looked up..."));
                return;
            };
            let town = snap
                .towns
                .get(p.town as usize)
                .map_or("?", |t| t.name.as_str());
            ui.heading(format!(
                "#{} · {} · {:.0} years · {}",
                p.id,
                if p.female { "woman" } else { "man" },
                p.age_years,
                town
            ));
            let doing = match &p.job {
                Some(job) if p.activity == "working" => format!("works: {job}"),
                Some(job) => format!("{} (last worked: {job})", p.activity),
                None => p.activity.clone(),
            };
            ui.label(doing);
            ui.label(format!("wealth {:.1}", p.wealth));
            if !p.needs.is_empty() {
                ui.separator();
                ui.strong("Needs");
                egui::Grid::new("person-needs")
                    .num_columns(2)
                    .show(ui, |ui| {
                        for (need, v) in &p.needs {
                            ui.label(need);
                            ui.add(
                                egui::ProgressBar::new(v.clamp(0.0, 1.0))
                                    .desired_width(140.0)
                                    .text(format!("{:.0} %", v * 100.0)),
                            );
                            ui.end_row();
                        }
                    });
            }
            let list = |ui: &mut egui::Ui, title: &str, items: &[(String, f32)]| {
                if items.is_empty() {
                    return;
                }
                ui.separator();
                ui.strong(title);
                for (name, q) in items {
                    ui.label(format!("{q:.1} {name}"));
                }
            };
            list(ui, "Larder", &p.larder);
            list(ui, "Household goods", &p.owned);
            if !p.gear.is_empty() {
                ui.separator();
                ui.strong("Equipment");
                for (slot, item) in &p.gear {
                    ui.label(format!("{slot}: {item}"));
                }
            }
            ui.separator();
            ui.strong("Household");
            let mut member = |ui: &mut egui::Ui, label: &str, who: u32| {
                ui.horizontal(|ui| {
                    ui.label(label);
                    if ui.small_button(format!("#{who}")).clicked() {
                        pick = Some(Some(who));
                    }
                });
            };
            if let Some(partner) = p.partner {
                member(ui, "partner", partner);
            }
            if let Some((a, b)) = p.parents {
                member(ui, "parent", a);
                member(ui, "parent", b);
            }
            for &c in &p.children {
                member(ui, "child", c);
            }
            if p.partner.is_none() && p.parents.is_none() && p.children.is_empty() {
                ui.label("lives alone");
            }
        });
    if !open {
        pick = Some(None);
    }
    if let Some(who) = pick {
        selected.0 = who;
        live.inspect(who);
    }
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

/// The product space's fixed part (from the simulation's content, at start).
#[derive(Resource, Default)]
pub struct Space(pub sim_core::view::ProductSpace);

/// A domain's colour: evenly spaced hues, as the crowd's workers.
fn domain_colour(d: u16, n: usize, strong: bool) -> egui::Color32 {
    let hue = d as f32 / n.max(1) as f32;
    let (s, v) = if strong { (0.6, 0.95) } else { (0.15, 0.55) };
    egui::ecolor::Hsva::new(hue, s, v, 1.0).into()
}

/// The skill graph (as in the 2D game): skills on a circle grouped by domain,
/// links between similar skills, nodes lit where the town has reached them and
/// sized by its residents' mean level.
fn product_space(
    ui: &mut egui::Ui,
    space: &sim_core::view::ProductSpace,
    town: Option<&sim_core::view::TownSkills>,
) {
    let n = space.skills.len();
    let nd = space.domains.len();
    let (resp, painter) = ui.allocate_painter(egui::vec2(470.0, 440.0), egui::Sense::hover());
    let c = resp.rect.center();
    let r = 150.0;
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&s| (space.domain[s], s));
    let mut pos = vec![c; n];
    for (k, &s) in order.iter().enumerate() {
        let a = k as f32 / n.max(1) as f32 * std::f32::consts::TAU;
        pos[s] = c + r * egui::vec2(a.cos(), a.sin());
    }
    let lit = |s: usize| town.is_some_and(|t| t.lit.get(s).copied().unwrap_or(false));
    let level = |s: usize| {
        town.and_then(|t| t.level.get(s).copied())
            .unwrap_or(0.0)
            .clamp(0.0, 1.5)
            / 1.5
    };
    for &(a, b, sim) in &space.edges {
        let (a, b) = (a as usize, b as usize);
        let fade = if lit(a) && lit(b) { 1.0 } else { 0.2 };
        let alpha = ((sim - 0.1) * 1.6 * fade).clamp(0.0, 1.0);
        painter.line_segment(
            [pos[a], pos[b]],
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha((alpha * 200.0) as u8)),
        );
    }
    for (s, &at) in pos.iter().enumerate() {
        let on = lit(s);
        let radius = if on { 3.5 + 9.0 * level(s) } else { 3.0 };
        painter.circle_filled(at, radius, domain_colour(space.domain[s], nd, on));
        if on {
            // The name outside the circle, on the node's side.
            let dir = (at - c).normalized();
            let align = if dir.x >= 0.0 {
                egui::Align2::LEFT_CENTER
            } else {
                egui::Align2::RIGHT_CENTER
            };
            painter.text(
                at + dir * (radius + 4.0),
                align,
                &space.skills[s],
                egui::FontId::proportional(10.0),
                egui::Color32::from_gray(225),
            );
        }
    }
    // Hover: the nearest skill's name and the town's level in it.
    if let Some(h) = resp.hover_pos() {
        if let Some(s) = (0..n).min_by(|&a, &b| pos[a].distance(h).total_cmp(&pos[b].distance(h))) {
            if pos[s].distance(h) < 12.0 {
                let text = format!(
                    "{} ({}): level {:.2}",
                    space.skills[s],
                    space
                        .domains
                        .get(space.domain[s] as usize)
                        .map_or("", |d| d.as_str()),
                    town.and_then(|t| t.level.get(s).copied()).unwrap_or(0.0)
                );
                painter.text(
                    h + egui::vec2(12.0, -12.0),
                    egui::Align2::LEFT_BOTTOM,
                    text,
                    egui::FontId::proportional(12.0),
                    egui::Color32::WHITE,
                );
            }
        }
    }
    if town.is_none() {
        ui.label("Waiting for the simulation's answer (the next simulated day)...");
    }
    ui.horizontal_wrapped(|ui| {
        for (d, name) in space.domains.iter().enumerate() {
            ui.colored_label(domain_colour(d as u16, nd, true), name);
        }
    });
    ui.small("Lit: the town made something with the skill last year, or its people know it. Size: its residents' mean level.");
}

/// The culture overlay's legend: the town's three axes and its colour.
fn culture_legend(ui: &mut egui::Ui, culture: &[f32]) {
    let axes = [
        ("tradition", "red"),
        ("faith", "green"),
        ("liberty", "blue"),
    ];
    for (k, (axis, channel)) in axes.iter().enumerate() {
        let v = culture.get(k).copied().unwrap_or(0.5);
        ui.horizontal(|ui| {
            ui.label(format!("{axis} ({channel})"));
            ui.add(
                egui::ProgressBar::new(v)
                    .desired_width(160.0)
                    .text(format!("{v:.2}")),
            );
        });
    }
    let c = crate::overlay::culture_colour(culture);
    let srgb = Color::linear_rgb(c[0], c[1], c[2]).to_srgba();
    let swatch = egui::Color32::from_rgb(
        (srgb.red * 255.0) as u8,
        (srgb.green * 255.0) as u8,
        (srgb.blue * 255.0) as u8,
    );
    ui.horizontal(|ui| {
        ui.label("the town's colour:");
        let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 16.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 3.0, swatch);
    });
    ui.small("0.5 is neutral (grey). People are coloured by their own beliefs, so newcomers and minorities stand out.");
}

/// The season's name for a fraction of the year (0 = midwinter).
fn season_name(season: f32) -> &'static str {
    match ((season + 0.125).fract() * 4.0) as u32 {
        0 => "winter",
        1 => "spring",
        2 => "summer",
        _ => "autumn",
    }
}

/// A few words for the sky.
fn sky_words(cloud: f32, precip: f32, snow: f32) -> String {
    if precip > 0.05 {
        let what = if snow > 0.5 { "snow" } else { "rain" };
        let how = if precip > 0.6 { "heavy " } else { "" };
        format!("{how}{what}")
    } else if cloud > 0.7 {
        "overcast".into()
    } else if cloud > 0.3 {
        "cloudy".into()
    } else {
        "clear".into()
    }
}
