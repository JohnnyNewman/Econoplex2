//! Organizations: guild membership and roles from proficiency; tools from the armory.

use crate::components::*;
use crate::db::Db;
use crate::store::{Activity, AgentStore, GuildLink, Role};
use crate::world::{Params, SimClock, Towns};
use bevy_ecs::prelude::*;

pub fn organize(
    mut commands: Commands,
    mut store: ResMut<AgentStore>,
    mut towns: ResMut<Towns>,
    db: Res<Db>,
    params: Res<Params>,
    clock: Res<SimClock>,
    items: Query<&Product, With<Stored>>,
) {
    let op = &params.organize;
    if op.interval == 0 || !clock.tick.is_multiple_of(op.interval as u64) {
        return;
    }
    let dpy = clock.days_per_year;
    let store = &mut *store;
    let nd = db.domain_dirs.len();

    // Tools in each town's armory, by domain. The armory also holds weapons and
    // goods nobody equips; scanning it per agent made this quadratic in big towns.
    let mut tools: Vec<Vec<Vec<(Entity, u32, f32)>>> = towns
        .0
        .iter()
        .map(|town| {
            let mut by_domain = vec![Vec::new(); nd];
            for &e in &town.armory {
                let Ok(prod) = items.get(e) else { continue };
                let Some(eq) = &db.content.products[prod.def as usize].equip else {
                    continue;
                };
                if let Some(d) = db.content.domain(&eq.domain) {
                    by_domain[d as usize].push((e, prod.def, prod.quality));
                }
            }
            by_domain
        })
        .collect();

    for i in 0..store.len() {
        if !store.alive[i] {
            continue;
        }
        let t = store.town[i] as usize;
        let age = store.age_years(i, dpy);

        // --- desired guild and role
        let desired: Option<(u32, Role)> = if age < params.life.adult_age {
            // Apprentices join a parent's guild.
            store.parents[i]
                .and_then(|(a, b)| store.guild[a as usize].or(store.guild[b as usize]))
                .filter(|_| age >= params.life.apprentice_age)
                .map(|g| (g.domain, Role::Apprentice))
        } else {
            let (best, prof) = (0..nd)
                .map(|d| (d as u32, db.domain_proficiency(&store.cap[i], d)))
                .fold((0, f32::MIN), |acc, x| if x.1 > acc.1 { x } else { acc });
            if prof >= op.master_threshold {
                Some((best, Role::Master))
            } else if prof >= op.guild_threshold {
                Some((best, Role::Member))
            } else {
                None
            }
        };

        let current = store.guild[i].map(|g| (g.domain, g.role));
        if current != desired {
            if let Some(old) = store.guild[i].take() {
                commands.entity(old.membership).despawn();
            }
            if let Some((domain, role)) = desired {
                let org = towns.0[t].guilds[domain as usize];
                let membership = commands
                    .spawn(Membership {
                        agent: i as u32,
                        org,
                        role,
                        since_day: clock.tick,
                    })
                    .id();
                store.guild[i] = Some(GuildLink {
                    org,
                    membership,
                    domain,
                    role,
                });
            }
        }

        // --- tools: an idle guild member without a tool takes the best matching one
        if store.equipped[i].is_some() || store.activity[i] != Activity::Idle {
            continue;
        }
        let Some(g) = store.guild[i] else { continue };
        let town = &mut towns.0[t];
        let shelf = &mut tools[t][g.domain as usize];
        let mut best: Option<(usize, f32)> = None;
        for (k, &(_, def, quality)) in shelf.iter().enumerate() {
            let cost = town.price[def as usize] * (0.5 + quality);
            if cost <= store.wealth[i] + params.market.tool_credit
                && best.is_none_or(|b| quality > b.1)
            {
                best = Some((k, quality));
            }
        }
        if let Some((k, _)) = best {
            let (e, _, _) = shelf.remove(k);
            let at = town.armory.iter().position(|&x| x == e).unwrap();
            town.armory.swap_remove(at);
            let prod = items.get(e).unwrap();
            // Tools are bought from the town, on credit if need be; wages repay it.
            let cost = town.price[prod.def as usize] * (0.5 + prod.quality);
            store.wealth[i] -= cost;
            town.treasury += cost;
            let bonus = db.content.products[prod.def as usize]
                .equip
                .as_ref()
                .unwrap()
                .bonus
                * (0.5 + prod.quality);
            town.stock[prod.def as usize] = (town.stock[prod.def as usize] - 1.0).max(0.0);
            store.equipped[i] = Some(e);
            let dir = db.domain_dirs[g.domain as usize];
            store.equip_vec[i] = std::array::from_fn(|k| dir[k] * bonus);
            commands
                .entity(e)
                .remove::<Stored>()
                .insert(EquippedBy(i as u32));
        }
    }
}
