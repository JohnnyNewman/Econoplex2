//! Team formation: who works together on which task today.

use super::*;

/// Split the people who chose the same job into teams. Each team forms around the
/// first person not yet placed, who brings the coworkers they trust most; the rest
/// fill up in the order they chose.
fn trusted_teams(
    store: &AgentStore,
    agents: &[u32],
    size: usize,
    place: &mut [u32],
) -> Vec<Vec<u32>> {
    const PLACED: u32 = u32::MAX - 1;
    for (k, &a) in agents.iter().enumerate() {
        place[a as usize] = k as u32;
    }
    let mut teams = Vec::new();
    let mut next = 0;
    for &lead in agents {
        if place[lead as usize] == PLACED {
            continue;
        }
        place[lead as usize] = PLACED;
        let mut team = vec![lead];
        let mut ties: Vec<_> = store.ties[lead as usize]
            .iter()
            .filter(|t| t.value > 0.0 && t.other != u32::MAX)
            .filter(|t| place[t.other as usize] < PLACED)
            .copied()
            .collect();
        ties.sort_by(|a, b| b.value.total_cmp(&a.value).then(a.other.cmp(&b.other)));
        for t in ties {
            if team.len() >= size {
                break;
            }
            place[t.other as usize] = PLACED;
            team.push(t.other);
        }
        while team.len() < size && next < agents.len() {
            let a = agents[next];
            next += 1;
            if place[a as usize] != PLACED {
                place[a as usize] = PLACED;
                team.push(a);
            }
        }
        teams.push(team);
    }
    for &a in agents {
        place[a as usize] = u32::MAX;
    }
    teams
}

#[allow(clippy::too_many_arguments)]
pub fn assign(
    mut commands: Commands,
    mut store: ResMut<AgentStore>,
    mut towns: ResMut<Towns>,
    db: Res<Db>,
    params: Res<Params>,
    decisions: Res<Decisions>,
    clock: Res<SimClock>,
    seed: Res<SimSeed>,
    sites: Query<(&mut NaturalResource, &Pos)>,
    mut tasks: Query<(Entity, &Task, &mut Participants, &Pos)>,
    mut log: ResMut<WorkLog>,
) {
    let mut rng = seed.rng(clock.steps(), streams::ASSIGN);
    let store = &mut *store;
    // Batches under way with room in their team, per (town, recipe), oldest
    // first: people who take up that job join them before new batches start,
    // so teams are as large as when everyone chose their work at once.
    // Each: (serial, task, free places).
    type Open = Vec<(u64, Entity, usize)>;
    let mut open: BTreeMap<(u16, u32), Open> = BTreeMap::new();
    for (e, task, parts, _) in tasks.iter() {
        let working = parts
            .workers
            .iter()
            .filter(|&&w| {
                store.alive[w as usize] && store.activity[w as usize] == Activity::Working(e)
            })
            .count();
        let room = db.recipes[task.recipe as usize]
            .max_team
            .saturating_sub(working);
        if room > 0 && task.progress < task.required {
            open.entry((task.town, task.recipe))
                .or_default()
                .push((task.serial, e, room));
        }
    }
    for list in open.values_mut() {
        list.sort_unstable();
    }
    // A team's task is one batch: a day's runs of a short recipe, or one run
    // of a long one (a working step's runs when a step does more than a day's
    // work).
    let days = params
        .calendar()
        .work_step_days(params.time.day_length.equinox_hours);

    // Group choices by (town, recipe), keeping agent order for determinism.
    let mut groups: BTreeMap<(u16, u32), Vec<u32>> = BTreeMap::new();
    for &(a, r) in &decisions.jobs {
        groups
            .entry((store.town[a as usize], r))
            .or_default()
            .push(a);
    }

    // Scarce inputs go to construction first (the town commissioned it), then to the
    // most profitable work, as if those teams outbid the others at the market.
    let mut groups: Vec<((u16, u32), Vec<u32>)> = groups.into_iter().collect();
    groups.sort_by(|((ta, ra), _), ((tb, rb), _)| {
        let pa = crate::systems::decide::profit_per_day(&db, &towns.0[*ta as usize], *ra as usize);
        let pb = crate::systems::decide::profit_per_day(&db, &towns.0[*tb as usize], *rb as usize);
        let build_a = db.recipes[*ra as usize].builds.is_some();
        let build_b = db.recipes[*rb as usize].builds.is_some();
        ta.cmp(tb)
            .then(build_b.cmp(&build_a))
            .then(pb.total_cmp(&pa))
            .then(ra.cmp(rb))
    });

    // Each town's nature sites by kind, in the town's order, with their amounts.
    // Amounts only go down while teams are placed, so a site too worn for one team
    // stays too worn for the next: a cursor per kind skips it for good instead of
    // scanning every site again (big towns have hundreds of fields).
    let nk = db.content.nature.len();
    // (site, amount left, position), per town and kind.
    type Site = (Entity, f32, (f32, f32));
    let mut nature: Vec<Vec<Vec<Site>>> = towns
        .0
        .iter()
        .map(|town| {
            let mut by_kind = vec![Vec::new(); nk];
            for &e in &town.nature_sites {
                if let Ok((n, p)) = sites.get(e) {
                    by_kind[n.kind as usize].push((e, n.amount, (p.x, p.y)));
                }
            }
            by_kind
        })
        .collect();
    let mut cursor = vec![vec![0usize; nk]; towns.0.len()];

    let mut pending: Vec<PendingTask> = Vec::new();
    let mut place = vec![u32::MAX; store.len()];
    for ((t, r), agents) in groups {
        let m = &db.recipes[r as usize];
        let town = &mut towns.0[t as usize];
        // Join the batches under way first.
        let mut agents = agents;
        if let Some(list) = open.get_mut(&(t, r)) {
            let mut rest = Vec::new();
            for a in agents {
                match list.iter_mut().find(|x| x.2 > 0) {
                    Some(slot) => {
                        slot.2 -= 1;
                        let e = slot.1;
                        let Ok((_, _, mut parts, pos)) = tasks.get_mut(e) else {
                            rest.push(a);
                            continue;
                        };
                        // Whoever left the team earlier is no longer in it.
                        parts.workers.retain(|&w| {
                            store.alive[w as usize]
                                && store.activity[w as usize] == Activity::Working(e)
                        });
                        parts.workers.push(a);
                        if decisions.apprentices.binary_search(&a).is_ok() {
                            parts.trainees.push(a);
                        }
                        store.activity[a as usize] = Activity::Working(e);
                        store.target[a as usize] = jitter((pos.x, pos.y), 14.0, &mut rng);
                    }
                    None => rest.push(a),
                }
            }
            agents = rest;
        }
        for team in &trusted_teams(store, &agents, m.max_team, &mut place) {
            // A team's batch covers several runs of a short recipe.
            let rate = (team.len() as f32).powf(params.work.work_exponent) * days;
            let mut batches = ((rate / m.duration).floor() as u32).max(1);
            let firms_on = params.market.firms.on();
            for &(p, q) in &m.inputs {
                let have = town.stock[p as usize]
                    + if firms_on {
                        town.firm_stock(p as usize)
                    } else {
                        0.0
                    };
                batches = batches.min((have / q).floor() as u32);
            }
            if batches == 0 {
                continue;
            }
            let out: f32 = m.outputs.iter().map(|o| o.1).sum();
            let mut pos = town.pos;
            if let Some(kind) = m.nature {
                // A batch's runs may take several sites in turn.
                let mut found = None;
                let mut runs = 0u32;
                let list = &mut nature[t as usize][kind as usize];
                let c = &mut cursor[t as usize][kind as usize];
                while let Some(site) = list.get_mut(*c) {
                    if site.1 >= out {
                        let here = (batches - runs).min((site.1 / out).floor() as u32);
                        site.1 -= out * here as f32;
                        runs += here;
                        found.get_or_insert(site.2);
                        if runs >= batches {
                            break;
                        }
                    }
                    *c += 1;
                }
                batches = runs;
                match found {
                    Some(p) if runs > 0 => pos = p,
                    _ => continue,
                }
            }
            if let Some(b) = m.building {
                match town.building(b) {
                    Some((_, p)) => pos = p,
                    None => continue,
                }
            }
            // With firms, inputs are bought: from the market's stock first, then
            // from other firms' stock, as many runs as the buyer (the firm, or the
            // treasury for public work) can pay.
            let buyer =
                (firms_on && !crate::firms::is_public(&db, m)).then(|| db.recipe_firm[r as usize]);
            if buyer.is_some_and(|f| town.firms[f].closed) {
                continue;
            }
            if firms_on {
                let per_batch: f32 = m
                    .inputs
                    .iter()
                    .map(|&(p, q)| town.price[p as usize] * q)
                    .sum();
                let cash = match buyer {
                    Some(f) => {
                        town.firms[f].cash.coins().max(0.0)
                            + params.market.firms.credit_room(&town.firms[f])
                    }
                    None => town.treasury.coins(),
                };
                if per_batch > 0.0 {
                    batches = batches.min((cash.max(0.0) / per_batch).floor() as u32);
                    if batches == 0 {
                        continue;
                    }
                }
            }
            // Inputs and nature flow in as the team works (work::produce).
            let mut input_quality: f32 = 1.0;
            for &(p, _) in &m.inputs {
                input_quality = input_quality.min(town.quality[p as usize]);
            }
            pending.push(PendingTask {
                recipe: r,
                town: t,
                workers: team.clone(),
                apprentices: Vec::new(),
                trainees: team
                    .iter()
                    .copied()
                    .filter(|w| decisions.apprentices.binary_search(w).is_ok())
                    .collect(),
                input_quality,
                pos,
            });
        }
    }

    // Children old enough follow a working parent as apprentices.
    let mut task_of = vec![usize::MAX; store.len()];
    for (k, p) in pending.iter().enumerate() {
        for &w in &p.workers {
            task_of[w as usize] = k;
        }
    }
    let per_year = params.time.months_per_year.max(1);
    for i in 0..store.len() {
        if !store.alive[i] || store.activity[i] != Activity::Child {
            continue;
        }
        if store.age_years(i, per_year) < params.life.apprentice_age {
            continue;
        }
        if let Some((a, b)) = store.parents[i] {
            let k = if task_of[a as usize] != usize::MAX {
                task_of[a as usize]
            } else {
                task_of[b as usize]
            };
            if k != usize::MAX {
                pending[k].apprentices.push(i as u32);
                store.target[i] = jitter(pending[k].pos, 10.0, &mut rng);
            }
        }
    }

    for p in pending {
        // The first team on a recipe takes up the runs left under way.
        let town = &mut towns.0[p.town as usize];
        town.wip.resize(db.recipes.len(), 0.0);
        let progress = std::mem::take(&mut town.wip[p.recipe as usize]);
        log.next_serial += 1;
        let e = commands
            .spawn((
                Task {
                    serial: log.next_serial,
                    recipe: p.recipe,
                    town: p.town,
                    // Runs done towards the next whole one (the progress bar).
                    progress,
                    required: 1.0,
                    input_quality: p.input_quality,
                    batches: 0,
                },
                TownId(p.town),
                Pos {
                    x: p.pos.0,
                    y: p.pos.1,
                },
            ))
            .id();
        for &w in &p.workers {
            store.activity[w as usize] = Activity::Working(e);
            store.target[w as usize] = jitter(p.pos, 14.0, &mut rng);
        }
        commands.entity(e).insert(Participants {
            workers: p.workers,
            apprentices: p.apprentices,
            trainees: p.trainees,
        });
    }
}
