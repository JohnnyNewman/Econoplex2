//! Battles a player watches and commands while the simulation waits.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_live_battle(
    raid: &Raid,
    attacker: usize,
    towns: &Towns,
    store: &AgentStore,
    db: &Db,
    params: &Params,
    fight: &CapVec,
    gear_defs: &Query<&Product, Without<Stored>>,
    terrain: &crate::terrain::Terrain,
    rng: &mut ChaCha8Rng,
    player: u16,
) -> Option<LiveBattle> {
    let mp = &params.military;
    let loyalty = params.trust.loyalty;
    let d = raid.target as usize;
    let attack = squad_power(store, &raid.soldiers, fight, loyalty);
    let defense = total_defense(store, towns, d, fight, mp, loyalty);
    let away: Vec<u32> = towns.0[d]
        .raid
        .as_ref()
        .map_or(Vec::new(), |r| r.soldiers.clone());
    let mut defenders: Vec<u32> = towns.0[d]
        .squad
        .iter()
        .copied()
        .filter(|s| !away.contains(s))
        .collect();
    let militia = muster_militia(store, &towns.0[d], &defenders, mp);
    let seed = rng.random::<u64>();
    // Fought in the town, among its buildings and houses.
    let (field, at, facing) = crate::layout::battle_field(towns, d, attacker, db, terrain);
    let Setup::Battle(battle, groups) = setup_field_battle(
        store,
        db,
        params,
        params.battle.live_model,
        field,
        fight,
        gear_defs,
        &raid.soldiers,
        &defenders,
        &militia,
        seed,
    ) else {
        return None;
    };
    defenders.extend(militia);
    Some(LiveBattle {
        battle: *battle,
        groups,
        attacker,
        target: d,
        defenders,
        attack,
        defense,
        at,
        facing,
        player_side: if player as usize == attacker { 0 } else { 1 },
    })
}

/// The battle waiting for the player, as the game shows it.
pub fn live_battle_view(world: &World) -> Option<crate::battle::LiveView> {
    let live = world.get_resource::<PendingBattle>()?.0.as_ref()?;
    Some(
        live.battle
            .live_view(live.target as u16, live.at, live.facing, live.player_side),
    )
}

/// Advances the waiting battle by `seconds` (whole steps).
pub fn step_live_battle(world: &mut World, seconds: f32) {
    let Some(mut pending) = world.get_resource_mut::<PendingBattle>() else {
        return;
    };
    let Some(live) = pending.0.as_mut() else {
        return;
    };
    let steps = (seconds / live.battle.params().dt.max(1e-3))
        .round()
        .max(1.0) as usize;
    for _ in 0..steps {
        live.battle.step();
    }
}

/// What the player tells one of their squads in the waiting battle: orders
/// carried out in turn, a stance, or (before it starts) where to deploy
/// (`Battle::command`). An error says why it was not done.
pub fn command_live_battle(
    world: &mut World,
    squad: usize,
    command: crate::battle::Command,
) -> Result<(), String> {
    let mut pending = world
        .get_resource_mut::<PendingBattle>()
        .ok_or("no battle is waiting")?;
    let live = pending.0.as_mut().ok_or("no battle is waiting")?;
    if live.battle.side(squad) != Some(live.player_side) {
        return Err("not the player's squad".into());
    }
    live.battle.command(squad, command)
}

/// Fights the waiting battle to its end at once (auto-resolve).
pub fn resolve_live_battle(world: &mut World) {
    let Some(mut pending) = world.get_resource_mut::<PendingBattle>() else {
        return;
    };
    if let Some(live) = pending.0.as_mut() {
        while !live.battle.finished() {
            live.battle.step();
        }
    }
}

/// Whether a battle waits for the player (the game holds the clock until it
/// is over).
pub fn live_battle_waiting(world: &World) -> bool {
    world
        .get_resource::<PendingBattle>()
        .and_then(|p| p.0.as_ref())
        .is_some_and(|b| !b.battle.finished())
}
