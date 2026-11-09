//! Migration: households move to towns that suit them better.
//!
//! Every few weeks some idle adults weigh the towns they know about (see
//! `decision::knowledge`): through friends and family there, the traders who pass
//! between the towns, or having lived there. The household (the agent, an idle
//! partner and their young children) decides as a unit: each member values each
//! town with the migration component (`decision::migration`), the household adds
//! the values up, and it moves to the best town if that beats home by `min_gain`
//! per member. Estimates of little-known towns are noisy. Movers carry their
//! skills, wealth and tools, so skills follow demand between towns, and they
//! remember their old home.

use super::streams;
use crate::components::*;
use crate::db::Db;
use crate::policy::Immigration;
use crate::store::{Activity, AgentStore};
use crate::world::{Params, SimClock, SimSeed, Towns};
use bevy_ecs::prelude::*;
use rand::Rng;
use sim_data::dims::*;

/// Move one agent to another town: new home, new guild, the walk over there.
pub fn relocate(
    commands: &mut Commands,
    store: &mut AgentStore,
    i: usize,
    to: u16,
    pos: (f32, f32),
    jitter: (f32, f32),
) {
    store.town[i] = to;
    commands.entity(store.entity[i]).insert(TownId(to));
    // Guilds are per town; organize enrolls the agent in the new town's guild.
    if let Some(g) = store.guild[i].take() {
        commands.entity(g.membership).despawn();
    }
    store.target[i] = (pos.0 + jitter.0, pos.1 + jitter.1);
}

#[allow(clippy::too_many_arguments)]
pub fn migrate(
    mut commands: Commands,
    mut store: ResMut<AgentStore>,
    mut towns: ResMut<Towns>,
    db: Res<Db>,
    params: Res<Params>,
    clock: Res<SimClock>,
    seed: Res<SimSeed>,
    terrain: Res<crate::terrain::Terrain>,
    decision: Res<crate::decision::DecisionModels>,
) {
    use crate::decision::migration::{MoverView, TownView};
    let mp = &params.migration;
    if mp.interval == 0 || !clock.tick.is_multiple_of(mp.interval as u64) || towns.0.len() < 2 {
        return;
    }
    let mut rng = seed.rng(clock.tick, streams::MIGRATE);
    let store = &mut *store;
    let dpy = clock.days_per_year;
    let n_towns = towns.0.len();

    // --- what each town offers everyone
    let food_pc: Vec<f32> = towns
        .0
        .iter()
        .map(|t| t.food_stock(&db) / t.residents.len().max(1) as f32)
        .collect();
    let danger: Vec<f32> = towns
        .0
        .iter()
        .map(|t| {
            if t.war.attacked > t.war.defended {
                1.0
            } else {
                0.0
            }
        })
        .collect();
    let median_wealth: Vec<f32> = towns
        .0
        .iter()
        .map(|t| {
            let mut w: Vec<f32> = t
                .residents
                .iter()
                .map(|&i| store.wealth[i as usize])
                .collect();
            if w.is_empty() {
                return 1.0;
            }
            let mid = w.len() / 2;
            *w.select_nth_unstable_by(mid, f32::total_cmp).1
        })
        .collect();
    let safety: Vec<f32> = towns
        .0
        .iter()
        .map(|t| {
            crate::decision::migration::safety(
                t.war.attacked > t.war.defended,
                t.war.fallen + t.war.captives_lost,
                t.residents.len(),
                t.squad.len(),
                mp.guard_share,
            )
        })
        .collect();
    // Goods shipped between each pair of towns per day (both ways): what traders
    // tell people about other towns.
    let trade_ties: Vec<Vec<f32>> = (0..n_towns)
        .map(|a| {
            (0..n_towns)
                .map(|b| {
                    let x = |p: usize, q: usize| {
                        towns.0[p].ledger.exports.get(q).copied().unwrap_or(0.0)
                    };
                    x(a, b) + x(b, a)
                })
                .collect()
        })
        .collect();
    // Children of each agent (for households and family ties).
    let mut children: std::collections::BTreeMap<u32, Vec<u32>> = Default::default();
    for c in 0..store.len() {
        if let (true, Some((a, b))) = (store.alive[c], store.parents[c]) {
            children.entry(a).or_default().push(c as u32);
            children.entry(b).or_default().push(c as u32);
        }
    }
    // Memories of past homes fade.
    let fade = params.decision.knowledge.fade(mp.interval as f32);
    for mem in store.lived_in.iter_mut() {
        mem.iter_mut().for_each(|e| e.1 *= fade);
        mem.retain(|e| e.1 > 0.01);
    }
    let horizon = params.decision.valuation.horizon_weight();
    let u = &*decision.utility;
    let np = db.content.products.len();
    let mean_price: Vec<f32> = (0..np)
        .map(|p| towns.0.iter().map(|t| t.price[p]).sum::<f32>() / n_towns as f32)
        .collect();
    // The journey is as long as the road, not the crow's flight.
    let dist = |a: usize, b: usize, _: &Towns| terrain.road(a, b);

    let free = |s: &AgentStore, j: usize| {
        s.alive[j] && matches!(s.activity[j], Activity::Idle | Activity::Resting)
    };
    let mut moved = vec![false; store.len()];
    for i in 0..store.len() {
        if moved[i]
            || !free(store, i)
            || store.age_years(i, dpy) > mp.max_age
            || rng.random::<f32>() >= mp.consider
        {
            continue;
        }
        let home = store.town[i] as usize;
        // A partner who is busy, or lives elsewhere, keeps the household at home.
        let partner = store.partner[i].map(|p| p as usize);
        if let Some(p) = partner {
            if store.alive[p] && (!free(store, p) || store.town[p] as usize != home) {
                continue;
            }
        }

        // --- the household: the agent, an idle partner at home, their young children
        let mut household = vec![i];
        if let Some(p) = partner.filter(|&p| store.alive[p]) {
            household.push(p);
        }
        for parent in [Some(i), partner].into_iter().flatten() {
            for &c in children.get(&(parent as u32)).map_or(&[][..], |v| &v[..]) {
                let c = c as usize;
                if store.activity[c] == Activity::Child
                    && store.town[c] as usize == home
                    && !household.contains(&c)
                {
                    household.push(c);
                }
            }
        }
        let in_household = |j: usize| household.contains(&j);

        // --- what each member has in each town: trust, family, memories
        let ties_by_town = |m: usize| {
            let mut trust = vec![0.0f32; n_towns];
            for tie in &store.ties[m] {
                let o = tie.other as usize;
                if tie.value > 0.0 && tie.other != u32::MAX && store.alive[o] && !in_household(o) {
                    trust[store.town[o] as usize] += tie.value;
                }
            }
            let mut kin = vec![0u32; n_towns];
            let parents = store.parents[m].map_or(Vec::new(), |(a, b)| vec![a, b]);
            let kids = children.get(&(m as u32)).cloned().unwrap_or_default();
            for k in parents.into_iter().chain(kids) {
                let k = k as usize;
                if store.alive[k] && !in_household(k) {
                    kin[store.town[k] as usize] += 1;
                }
            }
            (0..n_towns)
                .map(|t| {
                    (
                        trust[t].min(1.0),
                        crate::decision::migration::network(trust[t], kin[t], mp.kin_tie),
                    )
                })
                .collect::<Vec<(f32, f32)>>()
        };
        let nets: Vec<Vec<(f32, f32)>> = household.iter().map(|&m| ties_by_town(m)).collect();

        // --- which towns the household knows, and how well (the adults' knowledge)
        let know = &params.decision.knowledge;
        let known: Vec<f32> = (0..n_towns)
            .map(|t| {
                if t == home {
                    return 1.0;
                }
                household
                    .iter()
                    .zip(&nets)
                    .filter(|(&m, _)| store.activity[m] != Activity::Child)
                    .map(|(&m, net)| {
                        let lived = store.lived_in[m]
                            .iter()
                            .find(|e| e.0 as usize == t)
                            .map_or(0.0, |e| e.1);
                        know.known(&crate::decision::knowledge::Sources {
                            network: net[t].1,
                            trade: trade_ties[home][t],
                            lived,
                        })
                    })
                    .fold(0.0, f32::max)
            })
            .collect();
        // Estimates of little-known towns are off: one error per town for the household.
        let mut hrng = seed.agent_rng(clock.tick, streams::MIGRATE, i);
        let errors: Vec<f32> = (0..n_towns)
            .map(|t| know.error(known[t], hrng.random_range(-1.0..1.0f32)))
            .collect();

        // --- what each town is worth to each member; the household sums them
        let value_of = |k: usize, t: usize, road: f32| {
            let m = household[k];
            let weights =
                crate::decision::utility::need_weights(&params.decision.utility, &store.mind[m]);
            let st = store.state[m];
            let me = MoverView {
                state: &st,
                wealth: store.wealth[m],
                median_wealth: median_wealth[home],
                weights: &weights,
            };
            let trade = store.last_recipe[m]
                .and_then(|r| db.recipes[r as usize].outputs.first())
                .map(|&(p, _)| p as usize);
            let err = 1.0 + errors[t];
            let view = TownView {
                food_per_capita: food_pc[t] * err,
                danger: danger[t],
                safety: safety[t],
                culture_fit: crate::decision::culture::fit(
                    &store.mind[m][mind::IDEOLOGY_START..],
                    &towns.0[t].culture,
                ),
                friends: nets[k][t].0,
                social: nets[k][t].1,
                sells: trade.map_or(0.0, |p| {
                    (towns.0[t].price[p] / mean_price[p].max(1e-6) - 1.0).clamp(-1.0, 2.0)
                }),
                rent: towns.0[t].rent,
                pay: err
                    * store.last_recipe[m].map_or(0.0, |r| {
                        towns.0[t].pay_rate.get(r as usize).copied().unwrap_or(0.0)
                    }),
                welcome: if towns.0[t].policy.immigration == Immigration::Encouraged {
                    params.policy.immigration_bonus
                } else {
                    0.0
                },
                road,
            };
            decision.migration.value(&me, &view, u, horizon)
        };
        let total = |t: usize, road: f32| -> f32 {
            (0..household.len())
                .map(|k| {
                    let w = if store.activity[household[k]] == Activity::Child {
                        mp.child_weight
                    } else {
                        1.0
                    };
                    w * value_of(k, t, road)
                })
                .sum()
        };
        let stay = total(home, 0.0);
        let mut best: Option<(usize, f32)> = None;
        for (t, &k) in known.iter().enumerate() {
            if t == home
                || towns.0[t].policy.immigration == Immigration::Closed
                || !know.considers(k)
            {
                continue;
            }
            let v = total(t, dist(home, t, &towns));
            if best.is_none_or(|b| v > b.1) {
                best = Some((t, v));
            }
        }
        let Some((to, v)) = best else { continue };
        // The household moves when the move is worth it per adult (children add
        // their own gains and losses, but don't count toward the threshold).
        let adults = household
            .iter()
            .filter(|&&m| store.activity[m] != Activity::Child)
            .count()
            .max(1);
        if (v - stay) / (adults as f32) < decision.migration.min_gain() {
            continue;
        }

        // --- the household moves together and remembers its old home
        for &j in &household {
            let mem = &mut store.lived_in[j];
            match mem.iter_mut().find(|e| e.0 as usize == home) {
                Some(e) => e.1 = 1.0,
                None => mem.push((home as u16, 1.0)),
            }
        }
        let pos = towns.0[to].pos;
        for &j in &household {
            let jitter = (
                rng.random_range(-40.0..40.0f32),
                rng.random_range(-40.0..40.0f32),
            );
            relocate(&mut commands, store, j, to as u16, pos, jitter);
            moved[j] = true;
        }
        let n = household.len() as u32;
        if towns.0[to].policy.immigration == Immigration::Encouraged {
            // Settlement grant for each newcomer, paid to the household.
            let grant = (params.policy.settlement_grant * n as f32)
                .clamp(0.0, towns.0[to].treasury.max(0.0));
            towns.0[to].treasury -= grant;
            towns.0[to]
                .ledger
                .add(crate::ledger::Flow::SettlementGrants, grant);
            store.wealth[i] += grant;
        }
        towns.0[home].moves.left += n;
        towns.0[to].moves.arrived += n;
    }
}
