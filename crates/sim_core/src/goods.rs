//! Where goods come from and where they go: the goods ledger.
//!
//! Step 2 of the economy redesign (items, time, needs). Every good that exists
//! is held somewhere:
//!
//! | Holder | Today's storage |
//! | --- | --- |
//! | a town's market | `Town::stock` |
//! | a workshop | `Firm::stock` |
//! | a person's larder | `AgentStore::larder` |
//! | a raid's loot on its way home | `Raid::loot` |
//!
//! Goods only appear or disappear through the flows below; moving between
//! holders (a sale, a caravan, plunder, a larder handed on at death) changes
//! no total. Each town counts the flows that happen in it, and
//! [`audit`] checks that, for every good, what is held equals what was seeded
//! and made minus what left. A good whose numbers do not add up is being made
//! or lost somewhere outside the rules.
//!
//! Household goods (`AgentStore::owned`) count as bought (they leave the
//! market for a home and wear out there), and tools and weapons are entities
//! as well as a count in the market (see `Flow::Equipped`); the next steps
//! turn both into proper held items.

use crate::store::AgentStore;
use crate::world::Towns;
use serde::{Deserialize, Serialize};

/// Ways goods enter or leave the economy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// In the town's stock when the world was made.
    Seeded,
    /// Output of a recipe (before any of it is kept, sold or delivered).
    Made,
    /// Eaten by someone.
    Eaten,
    /// Used up as a recipe input.
    Input,
    /// Bought by a household for its home (it wears out there).
    Bought,
    /// Delivered for a government order (public use).
    Procured,
    /// Became a building in the town.
    Built,
    /// Became new land (a cleared field, a planted woodlot).
    Opened,
    /// Struck coin: became money in the treasury.
    Minted,
    /// A tool taken from the market's count by the person who uses it.
    Equipped,
    /// Rotted in store.
    Spoiled,
}

impl Flow {
    pub const ALL: [Flow; 11] = [
        Flow::Seeded,
        Flow::Made,
        Flow::Eaten,
        Flow::Input,
        Flow::Bought,
        Flow::Procured,
        Flow::Built,
        Flow::Opened,
        Flow::Minted,
        Flow::Equipped,
        Flow::Spoiled,
    ];
    pub const COUNT: usize = Self::ALL.len();

    /// Whether the flow brings goods in (+1) or takes them out (-1).
    pub fn sign(self) -> f64 {
        match self {
            Flow::Seeded | Flow::Made => 1.0,
            _ => -1.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Flow::Seeded => "seeded",
            Flow::Made => "made",
            Flow::Eaten => "eaten",
            Flow::Input => "input",
            Flow::Bought => "bought",
            Flow::Procured => "procured",
            Flow::Built => "built",
            Flow::Opened => "opened",
            Flow::Minted => "minted",
            Flow::Equipped => "equipped",
            Flow::Spoiled => "spoiled",
        }
    }
}

/// A town's goods flows since the start, per good and flow (units).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoodsFlows {
    /// `[good * Flow::COUNT + flow]`.
    v: Vec<f64>,
}

impl GoodsFlows {
    /// Flows of a town that starts with `stock`.
    pub fn seeded(stock: &[f32]) -> Self {
        let mut f = GoodsFlows::default();
        for (p, &q) in stock.iter().enumerate() {
            if q != 0.0 {
                f.add(Flow::Seeded, p, q);
            }
        }
        f
    }

    pub fn add(&mut self, flow: Flow, good: usize, q: f32) {
        let i = good * Flow::COUNT + flow as usize;
        if self.v.len() <= i {
            self.v.resize((good + 1) * Flow::COUNT, 0.0);
        }
        self.v[i] += q as f64;
    }

    pub fn get(&self, flow: Flow, good: usize) -> f64 {
        self.v
            .get(good * Flow::COUNT + flow as usize)
            .copied()
            .unwrap_or(0.0)
    }

    /// In minus out for `good`: what should be held of it.
    pub fn net(&self, good: usize) -> f64 {
        Flow::ALL
            .iter()
            .map(|&f| f.sign() * self.get(f, good))
            .sum()
    }
}

/// What is held of each good, wherever it is.
pub fn census(store: &AgentStore, towns: &Towns, squads: &Squads, goods: usize) -> Vec<f64> {
    let mut held = vec![0.0f64; goods];
    for s in &squads.list {
        for &(p, q) in &s.stacks {
            if (p as usize) < goods {
                held[p as usize] += q as f64;
            }
        }
    }
    for t in &towns.0 {
        for (p, &q) in t.stock.iter().enumerate().take(goods) {
            held[p] += q as f64;
        }
        for f in &t.firms {
            for (p, &q) in f.stock.iter().enumerate().take(goods) {
                held[p] += q as f64;
            }
            for (p, &q) in f.inputs.iter().enumerate().take(goods) {
                held[p] += q as f64;
            }
        }
        for (p, &q) in t.granary.iter().enumerate().take(goods) {
            held[p] += q as f64;
        }
        if let Some(raid) = &t.raid {
            for (p, &q) in raid.loot.iter().enumerate().take(goods) {
                held[p] += q as f64;
            }
        }
    }
    for i in 0..store.len() {
        if !store.alive[i] {
            continue;
        }
        for &(p, q) in &store.larder[i] {
            if (p as usize) < goods {
                held[p as usize] += q as f64;
            }
        }
    }
    held
}

/// One good whose holdings do not match its flows.
#[derive(Debug, Clone)]
pub struct Mismatch {
    pub good: usize,
    pub held: f64,
    /// Seeded plus made minus everything that left.
    pub expected: f64,
}

impl Mismatch {
    /// Held minus expected: positive = goods appeared from nowhere.
    pub fn gap(&self) -> f64 {
        self.held - self.expected
    }
}

/// Goods whose holdings differ from their flows by more than `tolerance`
/// (relative to what passed through them, at least one unit).
pub fn audit(
    store: &AgentStore,
    towns: &Towns,
    squads: &Squads,
    goods: usize,
    tolerance: f64,
) -> Vec<Mismatch> {
    let held = census(store, towns, squads, goods);
    (0..goods)
        .filter_map(|p| {
            let expected: f64 = towns.0.iter().map(|t| t.flows.net(p)).sum();
            let through: f64 = towns
                .0
                .iter()
                .map(|t| t.flows.get(Flow::Seeded, p) + t.flows.get(Flow::Made, p))
                .sum();
            let gap = (held[p] - expected).abs();
            (gap > (tolerance * through).max(1.0)).then_some(Mismatch {
                good: p,
                held: held[p],
                expected,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flows_add_up() {
        let mut f = GoodsFlows::default();
        f.add(Flow::Seeded, 2, 10.0);
        f.add(Flow::Made, 2, 5.0);
        f.add(Flow::Eaten, 2, 3.0);
        f.add(Flow::Input, 0, 1.0);
        assert_eq!(f.net(2), 12.0);
        assert_eq!(f.net(0), -1.0);
        assert_eq!(f.net(7), 0.0);
    }
}

// ---------------------------------------------------------------- holders and items
//
// Step 2b of the economy redesign: every good has an owner. Tools and weapons
// (and later armour, shields, mounts and clothes) are items: ECS entities with
// a `Product` (good and quality), an `Item` (wear) and an `Owner`. Where an
// item is follows from its owner: an armory is in its town, a person's gear is
// with the person, a squad's kit is wherever the squad is.

use bevy_ecs::prelude::*;

/// A squad's index in [`Squads`].
pub type SquadId = u32;

/// Who holds goods.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Holder {
    /// A town's market stock.
    Market(u16),
    /// A town's granary (`granary.rs`).
    Granary(u16),
    /// A workshop: (town, firm index into `Db::firm_keys`).
    Workshop(u16, u32),
    /// A household, by its keeper's store index (`ladder::Homes`).
    Home(u32),
    /// A person's own gear and goods, by store index.
    Person(u32),
    /// A town's armory: its tools and weapons not in use.
    Armory(u16),
    /// A squad on the march (`Squads`).
    Squad(SquadId),
}

/// The owner of an item entity.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner(pub Holder);

/// Where something is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Place {
    Town(u16),
    /// On the road from one town to another, `progress` 0..1 of the way.
    Road {
        from: u16,
        to: u16,
        progress: f32,
    },
    /// In the field: a town's nature site, by its index in `Town::nature_sites`.
    Field {
        town: u16,
        site: u32,
    },
}

/// Where a person carries an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Slot {
    Tool,
    Weapon,
    Armour,
    Shield,
    Mount,
}

impl Slot {
    pub const ALL: [Slot; 5] = [
        Slot::Tool,
        Slot::Weapon,
        Slot::Armour,
        Slot::Shield,
        Slot::Mount,
    ];

    /// The slot a good goes in: its `equip.slot` in the content, else tools in
    /// the tool slot and weapons in the weapon slot.
    pub fn of(def: &sim_data::ProductDef) -> Option<Slot> {
        if let Some(s) = def.equip.as_ref().and_then(|e| e.slot.as_deref()) {
            return match s {
                "tool" => Some(Slot::Tool),
                "weapon" => Some(Slot::Weapon),
                "armour" => Some(Slot::Armour),
                "shield" => Some(Slot::Shield),
                "mount" => Some(Slot::Mount),
                _ => None,
            };
        }
        match def.category {
            sim_data::Category::Tool => Some(Slot::Tool),
            sim_data::Category::Weapon => Some(Slot::Weapon),
            _ => None,
        }
    }
}

/// Hours of use one unit of an item's `durability` stands for (a working day).
pub const HOURS_PER_USE: f32 = 8.0;

/// Wears `item` by `hours` of use (a day's work, a battle). Returns whether it
/// broke (no durability left).
pub fn wear(item: &mut crate::components::Item, hours: f32) -> bool {
    item.worn_hours += hours.max(0.0);
    while item.worn_hours >= HOURS_PER_USE && item.durability > 0 {
        item.worn_hours -= HOURS_PER_USE;
        item.durability -= 1;
    }
    item.durability == 0
}

/// The item person `i` has in `slot`.
pub fn gear(store: &AgentStore, i: usize, slot: Slot) -> Option<Entity> {
    match slot {
        Slot::Tool => store.equipped[i],
        Slot::Weapon => store.weapon[i],
        Slot::Armour => store.armour.get(i).copied().flatten(),
        Slot::Shield => store.shield.get(i).copied().flatten(),
        Slot::Mount => store.mount.get(i).copied().flatten(),
    }
}

fn gear_mut(store: &mut AgentStore, i: usize, slot: Slot) -> &mut Option<Entity> {
    let n = store.len();
    match slot {
        Slot::Tool => &mut store.equipped[i],
        Slot::Weapon => &mut store.weapon[i],
        Slot::Armour => {
            store.armour.resize(n, None);
            &mut store.armour[i]
        }
        Slot::Shield => {
            store.shield.resize(n, None);
            &mut store.shield[i]
        }
        Slot::Mount => {
            store.mount.resize(n, None);
            &mut store.mount[i]
        }
    }
}

/// Puts `item` in town `town`'s armory (it must already be out of any other
/// holder's hands).
pub fn to_armory(commands: &mut Commands, armory: &mut Vec<Entity>, town: u16, item: Entity) {
    commands
        .entity(item)
        .remove::<crate::components::EquippedBy>()
        .insert((crate::components::Stored, Owner(Holder::Armory(town))));
    armory.push(item);
}

/// Takes `item` out of an armory.
pub fn from_armory(armory: &mut Vec<Entity>, item: Entity) {
    if let Some(k) = armory.iter().position(|&e| e == item) {
        armory.swap_remove(k);
    }
}

/// Gives `item` to person `i` for `slot`.
pub fn to_person(
    commands: &mut Commands,
    store: &mut AgentStore,
    i: usize,
    slot: Slot,
    item: Entity,
) {
    *gear_mut(store, i, slot) = Some(item);
    commands
        .entity(item)
        .remove::<crate::components::Stored>()
        .insert((
            crate::components::EquippedBy(i as u32),
            Owner(Holder::Person(i as u32)),
        ));
}

/// Takes person `i`'s item in `slot` from them.
pub fn from_person(store: &mut AgentStore, i: usize, slot: Slot) -> Option<Entity> {
    gear_mut(store, i, slot).take()
}

/// Gives `item` to squad `id` as kit (carried on the march, on nobody's person).
pub fn to_squad(commands: &mut Commands, squads: &mut Squads, id: SquadId, item: Entity) {
    if let Some(s) = squads.list.get_mut(id as usize) {
        s.items.push(item);
    }
    commands
        .entity(item)
        .remove::<crate::components::Stored>()
        .remove::<crate::components::EquippedBy>()
        .insert(Owner(Holder::Squad(id)));
}

/// Takes `item` out of squad `id`'s kit.
pub fn from_squad(squads: &mut Squads, id: SquadId, item: Entity) {
    if let Some(s) = squads.list.get_mut(id as usize) {
        s.items.retain(|&e| e != item);
    }
}

/// A squad: people under arms who march, camp and fight together. A stub for
/// the war redesign, which raises squads and moves them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Squad {
    /// The town that raised it, and where it goes back to.
    pub home: u16,
    /// Store indices of its people.
    pub members: Vec<u32>,
    pub place: Place,
    /// Items it carries as kit (not on anyone's person).
    pub items: Vec<Entity>,
    /// Stacks in its packs and carts: rations, arrows, plunder (good, quantity).
    #[serde(default)]
    pub stacks: Vec<(u32, f32)>,
}

#[derive(Resource, Debug, Clone, Default, Serialize, Deserialize)]
pub struct Squads {
    pub list: Vec<Squad>,
}

impl Squads {
    /// The squad person `i` marches with, if any.
    pub fn squad_of(&self, i: usize) -> Option<SquadId> {
        self.list
            .iter()
            .position(|s| s.members.contains(&(i as u32)))
            .map(|k| k as SquadId)
    }
}

/// Where person `i` eats and keeps warm right now: their squad's stores while
/// they march with one, else their household (kept by `keeper`).
pub fn eats_from(squads: &Squads, i: usize, keeper: usize) -> Holder {
    match squads.squad_of(i) {
        Some(id) => Holder::Squad(id),
        None => Holder::Home(keeper as u32),
    }
}

/// Weight a squad member carries besides their own gear, kg.
pub const KG_PER_MEMBER: f32 = 25.0;

/// How much a holder can carry, kg; `None`: no limit (a building's stores).
/// A squad carries what its members can; the war redesign adds carts and mounts.
pub fn carry_capacity(holder: Holder, squads: &Squads) -> Option<f32> {
    match holder {
        Holder::Squad(id) => Some(
            squads
                .list
                .get(id as usize)
                .map_or(0.0, |s| s.members.len() as f32 * KG_PER_MEMBER),
        ),
        _ => None,
    }
}

// ---------------------------------------------------------------- stacks
//
// Step 2b-2: simple goods (grain, bread, arrows, coal) are stacks: a quantity
// of one good held by one holder. Each holder keeps its stacks where it always
// has: a town's market (`Town::stock`), granary (`Town::granary`), a workshop
// (`Firm::stock`), a person's larder (food) and household goods
// (`AgentStore::larder`, `owned`), and now a squad's packs (`Squad::stacks`).
// `move_stack` is the one way to move a quantity from one holder to another.

/// Quantity of `good` that `holder` holds.
pub fn held_by(
    holder: Holder,
    good: usize,
    towns: &Towns,
    store: &AgentStore,
    squads: &Squads,
    db: &crate::db::Db,
) -> f32 {
    let sparse = |v: &[(u32, f32)]| {
        v.iter()
            .filter(|e| e.0 as usize == good)
            .map(|e| e.1)
            .sum::<f32>()
    };
    match holder {
        Holder::Market(t) => towns.0[t as usize].stock.get(good).copied().unwrap_or(0.0),
        Holder::Granary(t) => towns.0[t as usize]
            .granary
            .get(good)
            .copied()
            .unwrap_or(0.0),
        Holder::Workshop(t, f) => towns.0[t as usize]
            .firms
            .get(f as usize)
            .map(|f| {
                f.stock.get(good).copied().unwrap_or(0.0)
                    + f.inputs.get(good).copied().unwrap_or(0.0)
            })
            .unwrap_or(0.0),
        Holder::Home(i) | Holder::Person(i) => {
            if db.is_food[good] {
                sparse(&store.larder[i as usize])
            } else {
                sparse(&store.owned[i as usize])
            }
        }
        Holder::Armory(_) => 0.0,
        Holder::Squad(id) => squads
            .list
            .get(id as usize)
            .map_or(0.0, |s| sparse(&s.stacks)),
    }
}

/// Weight a holder carries in stacks, kg.
pub fn load_kg(holder: Holder, squads: &Squads, db: &crate::db::Db) -> f32 {
    match holder {
        Holder::Squad(id) => squads.list.get(id as usize).map_or(0.0, |s| {
            s.stacks
                .iter()
                .map(|&(p, q)| q * db.content.products[p as usize].kg)
                .sum()
        }),
        _ => 0.0,
    }
}

fn add_sparse(v: &mut Vec<(u32, f32)>, good: usize, q: f32) {
    match v.iter_mut().find(|e| e.0 as usize == good) {
        Some(e) => e.1 += q,
        None if q > 0.0 => v.push((good as u32, q)),
        None => {}
    }
    v.retain(|e| e.1 > 1e-6);
}

fn add_to(
    holder: Holder,
    good: usize,
    q: f32,
    towns: &mut Towns,
    store: &mut AgentStore,
    squads: &mut Squads,
    db: &crate::db::Db,
) {
    let np = db.content.products.len();
    match holder {
        Holder::Market(t) => towns.0[t as usize].stock[good] += q,
        Holder::Granary(t) => {
            let g = &mut towns.0[t as usize].granary;
            g.resize(np, 0.0);
            g[good] += q;
        }
        Holder::Workshop(t, f) => {
            if let Some(f) = towns.0[t as usize].firms.get_mut(f as usize) {
                f.stock.resize(np, 0.0);
                f.stock[good] += q;
            }
        }
        Holder::Home(i) | Holder::Person(i) => {
            if db.is_food[good] {
                add_sparse(&mut store.larder[i as usize], good, q);
            } else {
                add_sparse(&mut store.owned[i as usize], good, q);
            }
        }
        Holder::Armory(_) => {}
        Holder::Squad(id) => {
            if let Some(s) = squads.list.get_mut(id as usize) {
                add_sparse(&mut s.stacks, good, q);
            }
        }
    }
}

/// Moves up to `q` of `good` from one holder to another: no more than `from`
/// holds, and no more than `to` can carry (`carry_capacity`). Returns what
/// moved. Moving changes no total, so the goods ledger records nothing.
#[allow(clippy::too_many_arguments)]
pub fn move_stack(
    from: Holder,
    to: Holder,
    good: usize,
    q: f32,
    towns: &mut Towns,
    store: &mut AgentStore,
    squads: &mut Squads,
    db: &crate::db::Db,
) -> f32 {
    if matches!(from, Holder::Armory(_)) || matches!(to, Holder::Armory(_)) || from == to {
        return 0.0;
    }
    let have = held_by(from, good, towns, store, squads, db);
    let mut q = q.min(have).max(0.0);
    if let Some(cap) = carry_capacity(to, squads) {
        let kg = db.content.products[good].kg.max(1e-6);
        q = q.min(((cap - load_kg(to, squads, db)) / kg).max(0.0));
    }
    if q <= 0.0 {
        return 0.0;
    }
    add_to(from, good, -q, towns, store, squads, db);
    add_to(to, good, q, towns, store, squads, db);
    q
}

/// A meal from a squad's packs for someone marching with it: the most
/// filling food first. Returns (good, food value) eaten.
pub fn squad_meal(squads: &mut Squads, id: SquadId, foods: &[(u32, f32)]) -> Option<(u32, f32)> {
    let s = squads.list.get_mut(id as usize)?;
    crate::decision::subsistence::take_meal(&mut s.stacks, foods)
}

#[cfg(test)]
mod item_tests {
    use super::*;
    use crate::components::Item;

    #[test]
    fn an_item_wears_by_the_hour() {
        let mut it = Item {
            durability: 2,
            worn_hours: 0.0,
        };
        assert!(!wear(&mut it, 6.0));
        assert_eq!(it.durability, 2);
        assert!(!wear(&mut it, 4.0));
        assert_eq!(it.durability, 1);
        assert!(wear(&mut it, 8.0));
    }

    #[test]
    fn a_squad_carries_by_its_members() {
        let mut squads = Squads::default();
        assert_eq!(carry_capacity(Holder::Market(0), &squads), None);
        squads.list.push(Squad {
            home: 0,
            members: vec![3, 4],
            place: Place::Town(0),
            items: Vec::new(),
            stacks: Vec::new(),
        });
        assert_eq!(
            carry_capacity(Holder::Squad(0), &squads),
            Some(2.0 * KG_PER_MEMBER)
        );
        assert_eq!(eats_from(&squads, 4, 9), Holder::Squad(0));
        assert_eq!(eats_from(&squads, 5, 9), Holder::Home(9));
    }
}
