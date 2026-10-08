//! Social interaction as alignment on the ideological basis vectors of the mind.
//! Coworkers always interact; others meet random townspeople. Interactions are
//! local (team or town), never all pairs. The same encounters build or erode
//! trust (see `trust.rs`).

use super::streams;
use crate::models::Models;
use crate::store::AgentStore;
use crate::world::{Params, SimClock, SimSeed, Towns, WorkLog};
use bevy_ecs::prelude::*;
use rand::Rng;
use sim_data::dims::{mind, MIND_DIM};

const AXES: usize = MIND_DIM - mind::IDEOLOGY_START;

pub fn socialize(
    mut store: ResMut<AgentStore>,
    towns: Res<Towns>,
    models: Res<Models>,
    params: Res<Params>,
    clock: Res<SimClock>,
    seed: Res<SimSeed>,
    mut log: ResMut<WorkLog>,
) {
    let mut rng = seed.rng(clock.tick, streams::SOCIAL);
    let store = &mut *store;
    let tp = &params.trust;
    store.fade_trust(1.0 - tp.fade);
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
    // Closeness in outlook after talking: 1 for the same views, 0 for opposite ones.
    let closeness = |store: &AgentStore, a: usize, b: usize| {
        let mut d2 = 0.0;
        for k in mind::IDEOLOGY_START..MIND_DIM {
            d2 += (store.mind[a][k] - store.mind[b][k]).powi(2);
        }
        1.0 - d2.sqrt() / (AXES as f32).sqrt()
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
        let gain = tp.cowork_gain * if ev.success { 1.0 } else { tp.failure_factor };
        for (k, &a) in members.iter().enumerate() {
            for &b in &members[k + 1..] {
                if store.alive[a as usize] && store.alive[b as usize] {
                    store.bump_trust(a as usize, b as usize, gain);
                }
            }
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
                let (a, b) = (a as usize, b as usize);
                meet(store, a, b, &mut rng);
                if a != b && store.alive[a] && store.alive[b] {
                    let c = closeness(store, a, b);
                    store.bump_trust(a, b, tp.meet_gain * (2.0 * c - 1.0));
                }
            }
        }
    }
}
