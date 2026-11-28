//! Loot the field (docs/design/war-ideas.md, idea 3): the weapons, armour
//! and shields of those who die in a field battle stay where they fell. The
//! side that holds the field afterwards gathers them into its town's
//! armory, as prisoners' kit already goes (`take_prisoners`), where the
//! market counts them as stock; a `military.loot_broken` share is too
//! broken to use and is left. Mounts are not gathered: a dead rider's horse
//! goes home as before. With nobody holding the field, everyone's kit goes
//! home with them, as before.

use super::*;
use crate::battle::{kept_whole, Battle, Gathered};
use crate::goods::Slot;

/// The slots gathered from the dead.
const LOOT_SLOTS: [Slot; 3] = [Slot::Weapon, Slot::Armour, Slot::Shield];

/// A piece of kit left on the field by someone who died, and whether it
/// is whole enough to use.
pub(super) struct Left {
    person: u32,
    slot: Slot,
    def: u32,
    whole: bool,
}

/// The kit the dead of a finished battle left on the field, for the side
/// holding it; nothing when no side does. `def_of` names a carried item's
/// good.
pub(super) fn left_on_field(
    b: &Battle,
    groups: &[(u8, Vec<u32>)],
    store: &AgentStore,
    def_of: &dyn Fn(Entity) -> Option<u32>,
    broken: f32,
) -> Vec<Left> {
    if !b.finished() || b.report().winner.is_none() {
        return Vec::new();
    }
    let seed = b.seed();
    let mut left = Vec::new();
    for (g, (_, people)) in groups.iter().enumerate() {
        for (k, &i) in people.iter().enumerate() {
            if !b.dies(g, k) {
                continue;
            }
            for (n, slot) in LOOT_SLOTS.into_iter().enumerate() {
                let Some(def) = crate::goods::gear(store, i as usize, slot).and_then(def_of) else {
                    continue;
                };
                left.push(Left {
                    person: i,
                    slot,
                    def,
                    whole: kept_whole(seed, i, n as u8, broken),
                });
            }
        }
    }
    left
}

/// What the side holding the field gathers of `left`, by name.
pub(super) fn gathered(b: &Battle, left: &[Left], db: &Db) -> Gathered {
    let side = b.report().winner.unwrap_or(0);
    Gathered::of(
        side,
        left.iter()
            .filter(|l| l.whole)
            .map(|l| db.content.products[l.def as usize].name.as_str()),
    )
}

/// Gathers `left` for the side holding the field: the whole pieces go to
/// its town's armory (the raiders' or the defenders'), the broken are
/// thrown away. Returns how many pieces it gathered.
pub(super) fn gather(
    commands: &mut Commands,
    towns: &mut Towns,
    store: &mut AgentStore,
    winner_town: usize,
    left: &[Left],
) -> u32 {
    let mut n = 0;
    for l in left {
        let Some(e) = crate::goods::from_person(store, l.person as usize, l.slot) else {
            continue;
        };
        if l.whole {
            crate::goods::to_armory(
                commands,
                &mut towns.0[winner_town].armory,
                winner_town as u16,
                e,
            );
            n += 1;
        } else {
            commands.entity(e).despawn();
        }
    }
    n
}

/// The town holding the field of a finished raid's battle: the raiders'
/// (`attacker`) or the defenders' (`target`).
pub(super) fn holder(b: &Battle, attacker: usize, target: usize) -> Option<usize> {
    match b.report().winner? {
        0 => Some(attacker),
        _ => Some(target),
    }
}
