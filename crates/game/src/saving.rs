//! Quicksave (F5) and quickload (F9).
//!
//! A save holds only the simulation (see `sim_core::save`). Loading despawns every
//! simulation entity and the view entities drawn from the world, restores the
//! saved simulation into the app's world, and rebuilds the view: agents, buildings
//! and nature get their sprites from the usual attach systems, town names and the
//! terrain are spawned again. The camera, HUD, selection and speed stay as they are.

use crate::view::WorldView;
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use sim_core::components::*;
use sim_core::Db;
use std::path::PathBuf;

/// A short message shown in the HUD for a few seconds.
#[derive(Resource, Default)]
pub struct Toast {
    pub text: String,
    pub until: f64,
}

impl Toast {
    pub fn show(&mut self, now: f64, text: impl Into<String>) {
        self.text = text.into();
        self.until = now + 4.0;
    }

    pub fn current(&self, now: f64) -> Option<&str> {
        (now < self.until).then_some(self.text.as_str())
    }
}

pub fn quicksave_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join("saves")
        .join("quicksave.ron")
}

/// F5 saves, F9 loads the quicksave.
pub fn quicksave_keys(world: &mut World) {
    let (save, load) = {
        let keys = world.resource::<ButtonInput<KeyCode>>();
        (
            keys.just_pressed(KeyCode::F5),
            keys.just_pressed(KeyCode::F9),
        )
    };
    let path = quicksave_path();
    let message = if save {
        match save_game(world, &path) {
            Ok(()) => format!(
                "Saved day {} to {}",
                world.resource::<sim_core::SimClock>().tick,
                path.display()
            ),
            Err(e) => format!("Save failed: {e}"),
        }
    } else if load {
        match load_game(world, &path) {
            Ok(day) => format!("Loaded day {day}"),
            Err(e) => format!("Load failed: {e}"),
        }
    } else {
        return;
    };
    info!("{message}");
    let now = world.resource::<Time>().elapsed_secs_f64();
    world.resource_mut::<Toast>().show(now, message);
}

pub fn save_game(world: &World, path: &std::path::Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    sim_core::save::save_to_file(world, path)
}

/// Load the save at `path` and rebuild the view. Returns the loaded day. Nothing
/// changes if the save cannot be read or does not fit the game's content.
pub fn load_game(world: &mut World, path: &std::path::Path) -> Result<u64, String> {
    let file = sim_core::save::read_file(path)?;
    let content = world.resource::<Db>().content.clone();
    file.check(&content)?;
    let day = file.day();
    replace_simulation(world, file, content)?;
    world
        .run_system_once(crate::view::spawn_static)
        .map_err(|e| e.to_string())?;
    world
        .run_system_once(crate::terrain_view::spawn_terrain)
        .map_err(|e| e.to_string())?;
    Ok(day)
}

/// Despawn the running simulation (and the view drawn from it) and restore `file`.
pub fn replace_simulation(
    world: &mut World,
    file: sim_core::save::SaveFile,
    content: sim_data::Content,
) -> Result<(), String> {
    let old: Vec<Entity> = world
        .query_filtered::<Entity, Or<(
            With<Agent>,
            With<Pos>,
            With<TownId>,
            With<Product>,
            With<Item>,
            With<NaturalResource>,
            With<Organization>,
            With<Membership>,
            With<Task>,
            With<Participants>,
            With<WorldView>,
        )>>()
        .iter(world)
        .collect();
    for e in old {
        world.despawn(e);
    }
    sim_core::save::restore(world, file, content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::AssetPaths;

    fn world() -> World {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let (db, params, mut scenario) = sim_core::load(&AssetPaths::from_root(&root)).unwrap();
        for t in &mut scenario.towns {
            t.population = 40;
        }
        let mut world = World::new();
        sim_core::world::setup(&mut world, db, params, &scenario);
        world.add_schedule(sim_core::build_schedule());
        world
    }

    #[test]
    fn quickload_replaces_the_running_simulation() {
        let mut w = world();
        for _ in 0..150 {
            w.run_schedule(sim_core::SimTick);
        }
        let text = sim_core::save::to_string(&w).unwrap();
        let hash = sim_core::state_hash(&w);
        let entities = w.query::<&Agent>().iter(&w).count();
        // A view entity that must go, and one (a HUD panel) that must stay.
        w.spawn((WorldView, Transform::default()));
        let hud = w.spawn(Node::default()).id();

        // Play on, then load the save made earlier.
        for _ in 0..40 {
            w.run_schedule(sim_core::SimTick);
        }
        let file = sim_core::save::from_str(&text).unwrap();
        let content = w.resource::<Db>().content.clone();
        replace_simulation(&mut w, file, content).unwrap();

        assert_eq!(sim_core::state_hash(&w), hash);
        assert_eq!(w.query::<&Agent>().iter(&w).count(), entities);
        assert_eq!(w.query::<&WorldView>().iter(&w).count(), 0);
        assert!(w.get_entity(hud).is_ok());
        // Every agent's store entry points at a live agent entity.
        let store = w.resource::<sim_core::AgentStore>();
        for i in 0..store.len() {
            if store.alive[i] {
                assert_eq!(w.get::<Agent>(store.entity[i]).unwrap().idx, i as u32);
            }
        }
        // And the loaded game keeps running.
        for _ in 0..40 {
            w.run_schedule(sim_core::SimTick);
        }
    }
}
