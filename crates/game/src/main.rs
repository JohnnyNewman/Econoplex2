//! Econoplex: a simple 2D view of the economic simulation.
//!
//! The simulation lives in the same ECS world as the view. It advances one day per
//! `FixedUpdate` step; the view attaches sprites to simulation entities and animates
//! agents toward where they work.
//!
//! Controls: Space pause · 1-5 speed · WASD/arrows pan · mouse wheel zoom ·
//! left click select agent · Esc deselect · P product space overlay

mod hud;
mod view;

use bevy::prelude::*;
use sim_core::AssetPaths;

/// Speed steps in simulated days per second.
pub const SPEEDS: [f64; 5] = [1.0, 4.0, 12.0, 30.0, 60.0];

#[derive(Resource)]
pub struct SimControl {
    pub paused: bool,
    pub speed: usize,
}

#[derive(Resource, Default)]
pub struct Selection(pub Option<u32>);

fn main() {
    let paths = AssetPaths::discover();
    let (db, params, scenario) = match sim_core::load(&paths) {
        Ok(x) => x,
        Err(e) => {
            eprintln!(
                "failed to load Econoplex assets from {}: {e}",
                paths.content.display()
            );
            std::process::exit(1);
        }
    };

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Econoplex".into(),
            ..default()
        }),
        ..default()
    }));

    sim_core::world::setup(app.world_mut(), db, params, &scenario);
    app.add_schedule(sim_core::build_schedule());

    app.insert_resource(Time::<Fixed>::from_hz(SPEEDS[1]))
        .insert_resource(SimControl {
            paused: false,
            speed: 1,
        })
        .insert_resource(Selection::default())
        .insert_resource(view::Overlay::default())
        .insert_resource(ClearColor(Color::srgb(0.08, 0.09, 0.08)))
        .add_systems(
            Startup,
            (
                view::spawn_camera,
                view::spawn_static,
                view::spawn_skill_labels,
                hud::spawn_hud,
            ),
        )
        .add_systems(FixedUpdate, run_sim_tick)
        .add_systems(
            Update,
            (
                controls,
                view::camera_controls,
                view::attach_agent_sprites,
                view::attach_building_sprites,
                view::attach_nature_sprites,
                view::animate_agents,
                view::draw_world,
                view::update_skill_labels,
                view::select_agent,
                hud::update_hud,
                hud::update_inspector,
                hud::update_legend,
            ),
        );
    app.run();
}

/// Advance the simulation by one day.
fn run_sim_tick(world: &mut World) {
    if world.resource::<SimControl>().paused {
        return;
    }
    world.run_schedule(sim_core::SimTick);
}

fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut control: ResMut<SimControl>,
    mut fixed: ResMut<Time<Fixed>>,
    mut selection: ResMut<Selection>,
    mut overlay: ResMut<view::Overlay>,
) {
    if keys.just_pressed(KeyCode::Space) {
        control.paused = !control.paused;
    }
    let digits = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
    ];
    for (i, k) in digits.iter().enumerate() {
        if keys.just_pressed(*k) {
            control.speed = i;
            fixed.set_timestep_hz(SPEEDS[i]);
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        selection.0 = None;
    }
    if keys.just_pressed(KeyCode::KeyP) {
        overlay.product_space = !overlay.product_space;
    }
}
