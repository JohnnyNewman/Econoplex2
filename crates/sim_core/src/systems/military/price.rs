//! Battles' members as people, and what losing them costs their towns
//! (`battle::price`, docs/design/war-ideas.md idea 2).

use super::*;
use crate::battle::Fighter;

/// Days in a week, for what a fighter's work makes in one.
const WEEK: f32 = 7.0;

/// Person `i` as a battle's member: name, trade, battles fought (this one
/// included: `store.battles` counts those before it) and the week's work
/// their town would lose with them.
pub fn fighter(store: &AgentStore, db: &Db, towns: &Towns, params: &Params, i: u32) -> Fighter {
    let k = i as usize;
    let recipe = store.last_recipe[k].map(|r| r as usize);
    let trade = recipe
        .and_then(|r| db.content.skills.get(db.recipes[r].skill_idx as usize))
        .map_or_else(
            || "soldier".to_string(),
            |s| {
                if s.worker.is_empty() {
                    s.name.to_lowercase()
                } else {
                    s.worker.clone()
                }
            },
        );
    let weekly = match (recipe, towns.0.get(store.town[k] as usize)) {
        (Some(r), Some(town)) => WEEK * day_in_loaves(town, db, params, r),
        _ => 0.0,
    };
    Fighter {
        id: i,
        name: crate::names::given(i, store.female[k]).to_string(),
        trade,
        battles: store.battles[k].saturating_add(1),
        weekly,
    }
}

/// What a worker-day at recipe `r` makes in `town`, in loaves of bread: the
/// value one run adds (outputs less inputs at the town's prices) at a
/// typical worker's success rate, over the days one worker needs for it and
/// the price of bread there. Without bread in the content, in coin.
pub fn day_in_loaves(town: &crate::world::Town, db: &Db, params: &Params, r: usize) -> f32 {
    let m = &db.recipes[r];
    let value = |list: &[(u32, f32)]| -> f32 {
        list.iter()
            .map(|&(p, q)| town.price.get(p as usize).copied().unwrap_or(0.0) * q)
            .sum()
    };
    let added = (value(&m.outputs) - value(&m.inputs)).max(0.0)
        * params.decision.valuation.typical_success
        / m.duration.max(1.0);
    let bread = db
        .content
        .products
        .iter()
        .position(|p| p.id == "bread")
        .and_then(|b| town.price.get(b).copied())
        .filter(|&p| p > 0.0)
        .unwrap_or(1.0);
    added / bread
}

/// Each squad's people as fighters (`groups`: each squad's side and people).
pub(super) fn fighters(
    store: &AgentStore,
    db: &Db,
    towns: &Towns,
    params: &Params,
    groups: &[(u8, Vec<u32>)],
) -> Vec<Vec<Fighter>> {
    groups
        .iter()
        .map(|(_, people)| {
            people
                .iter()
                .map(|&i| fighter(store, db, towns, params, i))
                .collect()
        })
        .collect()
}

/// Everyone who fought one more battle behind them.
pub(super) fn count_battle(store: &mut AgentStore, groups: &[(u8, Vec<u32>)]) {
    for (_, people) in groups {
        for &i in people {
            let b = &mut store.battles[i as usize];
            *b = b.saturating_add(1);
        }
    }
}

/// What a battle cost, for the battle log (`BattleLog::costs`).
pub(super) fn cost(
    b: &crate::battle::Battle,
    people: &[Vec<Fighter>],
    day: u64,
    attacker: usize,
    town: usize,
) -> crate::battle::Cost {
    let mut fought = [0u32; 2];
    for (q, members) in people.iter().enumerate() {
        if let Some(s) = b.side(q) {
            fought[(s as usize).min(1)] += members.len() as u32;
        }
    }
    crate::battle::Cost {
        day,
        attacker: attacker as u16,
        town: town as u16,
        winner: b.report().winner,
        fought,
        losses: b.losses(people),
    }
}

/// People for a battle staged from a line-up in a running world (the
/// clients' `--battle`), so it can be watched as a real one: the defenders
/// (side 1) are adults of town `home`, the attackers of the nearest other
/// town (or `home` too, with only one), in turn, each squad's members from
/// where the last squad's left off. Returns them, and the attackers' town.
pub fn stage_fighters(
    world: &World,
    specs: &[crate::battle::SquadSpec],
    home: usize,
) -> (Vec<Vec<Fighter>>, Option<u16>) {
    let (Some(store), Some(towns), Some(db), Some(params)) = (
        world.get_resource::<AgentStore>(),
        world.get_resource::<Towns>(),
        world.get_resource::<Db>(),
        world.get_resource::<Params>(),
    ) else {
        return (Vec::new(), None);
    };
    let Some(here) = towns.0.get(home) else {
        return (Vec::new(), None);
    };
    let away = (0..towns.0.len())
        .filter(|&t| t != home)
        .min_by(|&a, &b| {
            let d = |t: usize| {
                let p = towns.0[t].pos;
                (p.0 - here.pos.0).powi(2) + (p.1 - here.pos.1).powi(2)
            };
            d(a).total_cmp(&d(b))
        })
        .unwrap_or(home);
    let adults = |t: usize| -> Vec<u32> {
        towns.0[t]
            .residents
            .iter()
            .copied()
            .filter(|&i| {
                let k = i as usize;
                store.alive[k] && store.activity[k] != Activity::Child
            })
            .collect()
    };
    let pools = [adults(away), adults(home)];
    let mut next = [0usize; 2];
    let people = specs
        .iter()
        .map(|spec| {
            let s = (spec.side as usize).min(1);
            let pool = &pools[s];
            if pool.is_empty() {
                return Vec::new();
            }
            (0..spec.members.len())
                .map(|_| {
                    let i = pool[next[s] % pool.len()];
                    next[s] += 1;
                    fighter(store, db, towns, params, i)
                })
                .collect()
        })
        .collect();
    (people, Some(away as u16))
}
