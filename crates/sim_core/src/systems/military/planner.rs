//! Whether a raid is worth making: forecasting the fight and the haul.

use super::*;

/// How many times a planner simulates a raid's fight to forecast it.
const FORECASTS: u32 = 3;

/// The raid most worth making, if any. For each town a road leads to, the
/// fight is forecast by simulating it a few times against the soldiers at
/// home and the militia. A raid is worth it when the chance of
/// winning times the haul (what the survivors could carry, the goods worth most
/// for their weight first, plus `loot_share` of the treasury and, if the win
/// would conquer, a first tribute) beats the cost: wages and rations for the
/// trip there and back, and a year's pay for each soldier expected to die.
#[allow(clippy::too_many_arguments)]
pub(super) fn plan_raid(
    t: usize,
    towns: &Towns,
    store: &AgentStore,
    db: &Db,
    params: &Params,
    fight: &CapVec,
    gear_defs: &Query<&Product, Without<Stored>>,
    terrain: &crate::terrain::Terrain,
    rng: &mut ChaCha8Rng,
) -> Option<usize> {
    let mp = &params.military;
    let loyalty = params.trust.loyalty;
    let town = &towns.0[t];
    let n = town.squad.len() as f32;
    if n <= 0.0 {
        return None;
    }
    let attack = squad_power(store, &town.squad, fight, loyalty);
    // Food value costs, at home: the cheapest food per unit of hunger it stills.
    let food_cost = db
        .foods
        .iter()
        .filter(|&&(_, v)| v > 0.0)
        .map(|&(p, v)| town.price[p as usize] / v)
        .fold(f32::INFINITY, f32::min);
    let food_cost = if food_cost.is_finite() {
        food_cost
    } else {
        0.0
    };
    let year = params.life.days_per_year.max(1) as f32;
    let mut best: Option<(usize, f32)> = None;
    for (o, other) in towns.0.iter().enumerate() {
        let ruled =
            other.ruler == Some(t as u16) || (town.ruler.is_some() && other.ruler == town.ruler);
        let road = terrain.road(t, o);
        if o == t || ruled || !road.is_finite() {
            continue;
        }
        let away: &[u32] = other.raid.as_ref().map_or(&[], |r| &r.soldiers);
        let defenders: Vec<u32> = other
            .squad
            .iter()
            .copied()
            .filter(|s| !away.contains(s))
            .collect();
        let militia = muster_militia(store, other, &defenders, mp);
        let (mut wins, mut dead, mut standing) = (0.0f32, 0.0f32, 0.0f32);
        for _ in 0..FORECASTS {
            let seed = rng.random::<u64>();
            match setup_field_battle(
                store,
                db,
                params,
                fight,
                gear_defs,
                &town.squad,
                &defenders,
                &militia,
                seed,
            ) {
                Setup::Decided(won) => {
                    if won {
                        wins += 1.0;
                        standing += n;
                    }
                }
                Setup::Battle(mut b, groups) => {
                    let r = b.run();
                    if r.winner == Some(0) {
                        wins += 1.0;
                    }
                    for (g, (side, _)) in groups.iter().enumerate() {
                        if *side == 0 {
                            dead += r.squads[g].dead as f32;
                            standing += r.squads[g].standing as f32;
                        }
                    }
                }
            }
        }
        let k = FORECASTS as f32;
        let (p, dead, standing) = (wins / k, dead / k, standing / k);
        let defense = total_defense(store, towns, o, fight, mp, loyalty);
        let tribute = if attack >= mp.conquest_margin * defense {
            mp.tribute_share * other.treasury.coins().max(0.0)
        } else {
            0.0
        };
        let haul = carry_value(other, db, standing * crate::goods::KG_PER_MEMBER)
            + mp.loot_share * other.treasury.coins().max(0.0)
            + tribute;
        let trip = 2.0 * road / mp.march_speed.max(1.0);
        let cost = n * trip * (mp.soldier_pay + params.needs.hunger_per_day * food_cost)
            + dead * mp.soldier_pay * year;
        let net = p * haul - cost;
        if net > 0.0 && best.is_none_or(|b| net > b.1) {
            best = Some((o, net));
        }
    }
    best.map(|b| b.0)
}

/// What `kg` of a town's goods are worth at its prices, taking the goods worth
/// most for their weight first (as raiders carry them off).
fn carry_value(town: &crate::world::Town, db: &Db, kg: f32) -> f32 {
    let mut goods: Vec<usize> = (0..db.content.products.len())
        .filter(|&p| {
            let cat = db.category(p as u32);
            cat != Category::Building && !cat.is_item() && town.stock[p] >= 1.0
        })
        .collect();
    let per_kg = |p: usize| town.price[p] / db.content.products[p].kg.max(0.01);
    goods.sort_by(|&a, &b| per_kg(b).total_cmp(&per_kg(a)).then(a.cmp(&b)));
    let (mut room, mut value) = (kg.max(0.0), 0.0);
    for p in goods {
        if room <= 0.0 {
            break;
        }
        let w = db.content.products[p].kg.max(0.01);
        let q = town.stock[p].floor().min((room / w).floor());
        value += q * town.price[p];
        room -= q * w;
    }
    value
}
