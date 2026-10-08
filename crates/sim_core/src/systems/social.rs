//! Social interaction as alignment on the ideological basis vectors of the mind.
//! Coworkers always interact; others meet random townspeople. Interactions are
//! local (team or town), never all pairs.

use super::streams;
use crate::models::Models;
use crate::store::AgentStore;
use crate::world::{SimClock, SimSeed, Towns, WorkLog};
use bevy_ecs::prelude::*;
use rand::Rng;
use sim_data::dims::{mind, MIND_DIM};

const AXES: usize = MIND_DIM - mind::IDEOLOGY_START;

pub fn socialize(
    mut store: ResMut<AgentStore>,
    towns: Res<Towns>,
    models: Res<Models>,
    clock: Res<SimClock>,
    seed: Res<SimSeed>,
    mut log: ResMut<WorkLog>,
) {
    let mut rng = seed.rng(clock.tick, streams::SOCIAL);
    let store = &mut *store;
    let mut noise = [0.0f32; 2 * AXES];
    let mut meet =
        |store: &mut AgentStore, a: usize, b: usize, rng: &mut rand_chacha::ChaCha8Rng| {
            if a == b || !store.alive[a] || !store.alive[b] {
                return;
            }
            noise
                .iter_mut()
                .for_each(|x| *x = rng.random_range(-1.0..1.0));
            let (mut ma, mut mb) = (store.mind[a], store.mind[b]);
            models.social.interact(&mut ma, &mut mb, &noise);
            store.mind[a] = ma;
            store.mind[b] = mb;
        };

    // Coworkers (consecutive pairs within each finished team, including apprentices).
    let events = std::mem::take(&mut log.events);
    for ev in &events {
        let members: Vec<u32> = ev
            .workers
            .iter()
            .chain(ev.apprentices.iter())
            .copied()
            .collect();
        for pair in members.windows(2) {
            meet(store, pair[0] as usize, pair[1] as usize, &mut rng);
        }
    }

    // Random contacts within the town.
    let p = models.social.daily_contacts();
    for town in &towns.0 {
        let n = town.residents.len();
        if n < 2 {
            continue;
        }
        for &a in &town.residents {
            if rng.random::<f32>() < p {
                let b = town.residents[rng.random_range(0..n)];
                meet(store, a as usize, b as usize, &mut rng);
            }
        }
    }
}
