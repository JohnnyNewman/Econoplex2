//! Loot the field (docs/design/war-ideas.md, idea 3): the weapons and armour
//! of those who die in a battle stay where they fell, and the side that
//! holds the field gathers them into its town's armory, where the market
//! counts them as stock. A share is too broken to use
//! (`military.loot_broken`). The war system moves the goods
//! (`systems/military/loot.rs`); this is what both clients and the CLI say.

use serde::{Deserialize, Serialize};

/// What the side holding the field gathered: each good's name and how many,
/// the most first. Empty when nobody held the field or nothing was left
/// whole.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Gathered {
    /// The side that gathered (0 the attackers, 1 the defenders).
    pub side: u8,
    pub goods: Vec<(String, u32)>,
}

impl Gathered {
    /// Count the goods `names` (one name per piece), the most first, then
    /// by name.
    pub fn of<'a>(side: u8, names: impl IntoIterator<Item = &'a str>) -> Self {
        let mut goods: Vec<(String, u32)> = Vec::new();
        for n in names {
            match goods.iter_mut().find(|g| g.0 == n) {
                Some(g) => g.1 += 1,
                None => goods.push((n.to_string(), 1)),
            }
        }
        goods.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Self { side, goods }
    }

    pub fn is_empty(&self) -> bool {
        self.goods.is_empty()
    }

    /// How many pieces in all.
    pub fn count(&self) -> u32 {
        self.goods.iter().map(|g| g.1).sum()
    }

    /// Another battle's gathering added to this one (a town's over a run).
    pub fn add(&mut self, other: &Gathered) {
        for (n, c) in &other.goods {
            match self.goods.iter_mut().find(|g| &g.0 == n) {
                Some(g) => g.1 += c,
                None => self.goods.push((n.clone(), *c)),
            }
        }
        self.goods
            .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    }

    /// "gathered: 12 iron spears, 4 mail shirts", or "gathered nothing".
    pub fn words(&self) -> String {
        if self.goods.is_empty() {
            return "gathered nothing".into();
        }
        let list: Vec<String> = self
            .goods
            .iter()
            .map(|(n, c)| format!("{c} {}", plural(n, *c)))
            .collect();
        format!("gathered: {}", list.join(", "))
    }
}

/// A good's name in lower case, in the plural for more than one: "Iron
/// spear (lvl 2)" gives "iron spears"; names of a mass ("Plate armour")
/// stay as they are.
fn plural(name: &str, n: u32) -> String {
    let bare = name.split(" (").next().unwrap_or(name);
    let lower = bare.to_lowercase();
    if n == 1 || lower.ends_with("armour") || lower.ends_with("mail") {
        return lower;
    }
    if lower.ends_with('s') || lower.ends_with('x') || lower.ends_with("sh") {
        format!("{lower}es")
    } else {
        format!("{lower}s")
    }
}

/// Whether piece `slot` of person `person` came through the battle whole
/// enough to use, a `broken` share not: the same for the same battle seed,
/// so a live battle can say what will be gathered before it is settled.
pub fn kept_whole(seed: u64, person: u32, slot: u8, broken: f32) -> bool {
    let mut h = seed ^ 0x6c6f_6f74_6669_656c;
    h = h.wrapping_add(u64::from(person).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    h = h.wrapping_add(u64::from(slot).wrapping_mul(0xbf58_476d_1ce4_e5b9));
    // splitmix64's finish.
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 31;
    let u = (h >> 40) as f32 / (1u64 << 24) as f32;
    u >= broken
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gathering_reads_as_the_idea_says() {
        let mut names = vec!["Iron spear (lvl 2)"; 12];
        names.extend(["Mail shirt"; 4]);
        let g = Gathered::of(0, names.iter().copied());
        assert_eq!(g.words(), "gathered: 12 iron spears, 4 mail shirts");
        assert_eq!(g.count(), 16);
        let one = Gathered::of(1, ["Plate armour", "War axe", "War axe"]);
        assert_eq!(one.words(), "gathered: 2 war axes, 1 plate armour");
        let mut all = g.clone();
        all.add(&one);
        assert_eq!(all.count(), 19);
        assert_eq!(Gathered::default().words(), "gathered nothing");
    }

    #[test]
    fn about_the_broken_share_is_left_and_the_same_each_time() {
        let n = 10_000u32;
        let kept = (0..n).filter(|&p| kept_whole(7, p, 0, 0.3)).count() as f32;
        assert!((kept / n as f32 - 0.7).abs() < 0.02, "{kept}");
        assert_eq!(kept_whole(7, 3, 1, 0.3), kept_whole(7, 3, 1, 0.3));
        assert!((0..100).all(|p| kept_whole(1, p, 0, 0.0)));
        assert!((0..100).all(|p| !kept_whole(1, p, 0, 1.0)));
    }
}
