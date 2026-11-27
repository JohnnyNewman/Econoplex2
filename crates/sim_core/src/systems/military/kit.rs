//! Kit in store: how much a town keeps, and wear on what fighters carry.

use super::*;

/// Pieces of a kit good a town keeps when its government sets no stockpile
/// for it: one per soldier it means to keep plus one per adult of the levy
/// reserve (`military.levy_reserve` of its residents), shared among the goods
/// that fill the same slot.
pub fn kit_reserve(
    db: &Db,
    town: &crate::world::Town,
    params: &crate::config::ModelParams,
    p: usize,
) -> f32 {
    let mp = &params.military;
    let def = &db.content.products[p];
    let share = town.policy.army_share.unwrap_or(mp.soldier_share);
    let people = town.residents.len() as f32;
    // Enough for the soldiers it means to keep, or to match the largest army
    // next door, whichever is more.
    let soldiers = (share * people)
        .max(town.squad.len() as f32)
        .max(town.threat);
    let slot = crate::goods::Slot::of(def);
    let rivals = db
        .content
        .products
        .iter()
        .filter(|q| q.category == Category::Weapon && crate::goods::Slot::of(q) == slot)
        .count()
        .max(1);
    (soldiers + mp.levy_reserve * people) / rivals as f32
}

/// The slots a soldier fills from the armory.
pub(super) const KIT_SLOTS: [crate::goods::Slot; 4] = [
    crate::goods::Slot::Weapon,
    crate::goods::Slot::Armour,
    crate::goods::Slot::Shield,
    crate::goods::Slot::Mount,
];

/// How much a piece of kit is worth to a soldier, before quality: weapons by
/// their tier; armour and shields by the armour class they add (a point is
/// worth a tenth) and the damage types armour resists (a tenth each, less a
/// tenth per type it is vulnerable to); a mount by itself.
pub(super) fn kit_value(def: &sim_data::ProductDef) -> f32 {
    let Some(f) = &def.fight else {
        return def.tier.max(1) as f32;
    };
    match crate::goods::Slot::of(def) {
        Some(crate::goods::Slot::Weapon) | None => def.tier.max(1) as f32,
        _ => {
            0.1 * (f.ac as f32 + f.shield as f32)
                + 0.1 * (f.resist.len() as f32 - f.vulnerable.len() as f32)
                + if f.mounted { 1.0 } else { 0.0 }
        }
    }
}

/// Wears the kit of everyone who fought by `hours`; broken pieces are gone.
pub(super) fn wear_kit(
    commands: &mut Commands,
    store: &mut AgentStore,
    kit: &mut Query<&mut crate::components::Item, Without<Stored>>,
    fought: &[u32],
    hours: f32,
) {
    for &s in fought {
        let i = s as usize;
        for slot in KIT_SLOTS {
            let Some(e) = crate::goods::gear(store, i, slot) else {
                continue;
            };
            let Ok(mut item) = kit.get_mut(e) else {
                continue;
            };
            if crate::goods::wear(&mut item, hours) {
                crate::goods::from_person(store, i, slot);
                commands.entity(e).despawn();
                if slot == crate::goods::Slot::Weapon {
                    store.weapon_power[i] = 0.0;
                }
            }
        }
    }
}
