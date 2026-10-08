//! Yearly statistics: population, specialization, diversity and economic complexity.

use crate::db::Db;
use crate::store::AgentStore;
use crate::world::{SimClock, Towns, WorkLog};
use bevy_ecs::prelude::*;
use sim_data::dims::{mind, MIND_DIM};
use sim_data::Category;

#[derive(Debug, Clone, Default)]
pub struct TownStats {
    pub name: String,
    pub population: usize,
    pub adults: usize,
    pub food_per_capita: f32,
    /// Products made last year.
    pub diversity: usize,
    /// Products with revealed comparative advantage >= 1.
    pub rca_products: usize,
    /// Economic complexity index (method of reflections, standardized across towns).
    pub eci: f32,
    /// Mean specialization of adults: 1 - normalized entropy of their skill profile.
    pub specialization: f32,
    /// Mean best-skill proficiency of adults.
    pub mean_top_skill: f32,
    pub masters: usize,
    pub guild_members: usize,
    /// Mean of the ideological mind axes.
    pub ideology: [f32; MIND_DIM - mind::IDEOLOGY_START],
    pub ideology_spread: f32,
    /// Longswords in the armory or carried by soldiers, by level 1, 2, 3.
    pub swords: [u32; 3],
    pub tools_in_use: usize,
    pub treasury: f32,
    pub buildings: usize,
    pub soldiers: usize,
    pub war: crate::world::WarCounters,
    pub moves: crate::world::MoveCounters,
    /// Mean wealth of residents.
    pub mean_wealth: f32,
}

#[derive(Debug, Clone, Default)]
pub struct YearStats {
    pub year: u64,
    pub population: usize,
    pub success_rate: f32,
    /// Agents' wealth plus treasuries; constant when the money loop is closed.
    pub total_money: f64,
    pub towns: Vec<TownStats>,
}

#[derive(Resource, Default)]
pub struct Metrics {
    pub years: Vec<YearStats>,
    last_attempts: u64,
    last_successes: u64,
}

impl Metrics {
    pub fn latest(&self) -> Option<&YearStats> {
        self.years.last()
    }
}

/// 1 - normalized entropy of a nonnegative profile (0 = generalist, 1 = one skill only).
pub fn specialization(profile: &[f32]) -> f32 {
    let total: f32 = profile.iter().sum();
    if total <= 1e-6 || profile.len() < 2 {
        return 0.0;
    }
    let h: f32 = profile
        .iter()
        .filter(|&&x| x > 0.0)
        .map(|&x| {
            let p = x / total;
            -p * p.ln()
        })
        .sum();
    1.0 - h / (profile.len() as f32).ln()
}

/// ECI by the method of reflections on the binary town x product RCA matrix.
/// Returns (rca matrix, eci per town).
pub fn economic_complexity(x: &[Vec<f32>]) -> (Vec<Vec<bool>>, Vec<f32>) {
    let nt = x.len();
    let np = if nt == 0 { 0 } else { x[0].len() };
    let total: f32 = x.iter().flatten().sum();
    let town_tot: Vec<f32> = x.iter().map(|r| r.iter().sum()).collect();
    let prod_tot: Vec<f32> = (0..np).map(|p| x.iter().map(|r| r[p]).sum()).collect();
    let m: Vec<Vec<bool>> = (0..nt)
        .map(|t| {
            (0..np)
                .map(|p| {
                    if x[t][p] <= 0.0 || town_tot[t] <= 0.0 || prod_tot[p] <= 0.0 {
                        return false;
                    }
                    (x[t][p] / town_tot[t]) / (prod_tot[p] / total) >= 1.0
                })
                .collect()
        })
        .collect();
    let kc0: Vec<f32> = m
        .iter()
        .map(|r| r.iter().filter(|b| **b).count() as f32)
        .collect();
    let kp0: Vec<f32> = (0..np)
        .map(|p| m.iter().filter(|r| r[p]).count() as f32)
        .collect();
    // kp1: average diversity of the towns making product p; kc2: average kp1 over a town's products.
    let kp1: Vec<f32> = (0..np)
        .map(|p| {
            if kp0[p] > 0.0 {
                (0..nt).filter(|&t| m[t][p]).map(|t| kc0[t]).sum::<f32>() / kp0[p]
            } else {
                0.0
            }
        })
        .collect();
    let kc2: Vec<f32> = (0..nt)
        .map(|t| {
            if kc0[t] > 0.0 {
                (0..np).filter(|&p| m[t][p]).map(|p| kp1[p]).sum::<f32>() / kc0[t]
            } else {
                0.0
            }
        })
        .collect();
    // Combine diversity with the reflection so towns with equal kc2 still rank by breadth.
    let raw: Vec<f32> = (0..nt).map(|t| kc2[t] * kc0[t].max(1.0).ln_1p()).collect();
    let mean = raw.iter().sum::<f32>() / nt.max(1) as f32;
    let sd = (raw.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / nt.max(1) as f32).sqrt();
    let eci = raw
        .iter()
        .map(|v| if sd > 1e-6 { (v - mean) / sd } else { 0.0 })
        .collect();
    (m, eci)
}

#[allow(clippy::too_many_arguments)]
pub fn record(
    store: Res<AgentStore>,
    mut towns: ResMut<Towns>,
    db: Res<Db>,
    clock: Res<SimClock>,
    log: Res<WorkLog>,
    params: Res<crate::world::Params>,
    mut metrics: ResMut<Metrics>,
    weapons: Query<&crate::components::Product>,
) {
    if clock.tick == 0 || clock.day_of_year() != 0 {
        return;
    }
    let dpy = clock.days_per_year;
    for t in towns.0.iter_mut() {
        t.produced_last_year = std::mem::replace(
            &mut t.produced_this_year,
            vec![0.0; t.produced_last_year.len()],
        );
    }

    // Production value matrix (towns x non-building products).
    let products: Vec<usize> = (0..db.content.products.len())
        .filter(|&p| db.category(p as u32) != Category::Building)
        .collect();
    let x: Vec<Vec<f32>> = towns
        .0
        .iter()
        .map(|t| {
            products
                .iter()
                .map(|&p| t.produced_last_year[p] * db.base_price[p])
                .collect()
        })
        .collect();
    let (m, eci) = economic_complexity(&x);
    let sword_ids: Vec<Option<u32>> = ["longsword_1", "longsword_2", "longsword_3"]
        .iter()
        .map(|s| db.content.product(s))
        .collect();

    let mut ys = YearStats {
        year: clock.year(),
        ..Default::default()
    };
    let attempts = log.attempts - metrics.last_attempts;
    ys.success_rate = (log.successes - metrics.last_successes) as f32 / attempts.max(1) as f32;
    metrics.last_attempts = log.attempts;
    metrics.last_successes = log.successes;

    for (ti, town) in towns.0.iter().enumerate() {
        let mut s = TownStats {
            name: town.name.clone(),
            population: town.residents.len(),
            ..Default::default()
        };
        s.food_per_capita = town.food_stock(&db) / s.population.max(1) as f32;
        s.diversity = products
            .iter()
            .filter(|&&p| town.produced_last_year[p] > 0.0)
            .count();
        s.rca_products = m[ti].iter().filter(|b| **b).count();
        s.eci = eci[ti];
        let mut ideo_sum = [0.0; MIND_DIM - mind::IDEOLOGY_START];
        let mut spec_sum = 0.0;
        let mut top_sum = 0.0;
        for &i in &town.residents {
            let i = i as usize;
            for (k, v) in ideo_sum.iter_mut().enumerate() {
                *v += store.mind[i][mind::IDEOLOGY_START + k];
            }
            if let Some(g) = store.guild[i] {
                s.guild_members += 1;
                if g.role == crate::store::Role::Master {
                    s.masters += 1;
                }
            }
            if store.equipped[i].is_some() {
                s.tools_in_use += 1;
            }
            if store.age_years(i, dpy) < params.life.adult_age {
                continue;
            }
            s.adults += 1;
            let profile = db.skill_profile(&store.cap[i]);
            spec_sum += specialization(&profile);
            top_sum += profile.iter().copied().fold(0.0, f32::max);
        }
        let n = s.population.max(1) as f32;
        s.treasury = town.treasury;
        s.buildings = town.buildings.len();
        s.soldiers = town.squad.len();
        s.war = town.war;
        s.moves = town.moves;
        s.mean_wealth = town
            .residents
            .iter()
            .map(|&i| store.wealth[i as usize])
            .sum::<f32>()
            / n;
        s.ideology = ideo_sum.map(|v| v / n);
        let mut spread = 0.0;
        for &i in &town.residents {
            for k in 0..s.ideology.len() {
                spread +=
                    (store.mind[i as usize][mind::IDEOLOGY_START + k] - s.ideology[k]).powi(2);
            }
        }
        s.ideology_spread = (spread / n).sqrt();
        s.specialization = spec_sum / s.adults.max(1) as f32;
        s.mean_top_skill = top_sum / s.adults.max(1) as f32;
        let carried = town.squad.iter().filter_map(|&s| store.weapon[s as usize]);
        for e in town.armory.iter().copied().chain(carried) {
            if let Ok(p) = weapons.get(e) {
                for (lvl, id) in sword_ids.iter().enumerate() {
                    if *id == Some(p.def) {
                        s.swords[lvl] += 1;
                    }
                }
            }
        }
        ys.population += s.population;
        ys.towns.push(s);
    }
    ys.total_money = crate::world::total_money(&store, &towns);
    metrics.years.push(ys);
    for t in towns.0.iter_mut() {
        t.war = Default::default();
        t.moves = Default::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specialization_bounds() {
        assert!(specialization(&[1.0, 1.0, 1.0, 1.0]) < 1e-5);
        assert!((specialization(&[1.0, 0.0, 0.0, 0.0]) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn diverse_town_with_rare_products_ranks_highest() {
        // Town 0 makes everything, town 1 only common goods.
        let x = vec![
            vec![10.0, 10.0, 10.0, 10.0],
            vec![30.0, 0.0, 0.0, 0.0],
            vec![10.0, 20.0, 0.0, 0.0],
        ];
        let (_, eci) = economic_complexity(&x);
        assert!(eci[0] > eci[1]);
    }
}
