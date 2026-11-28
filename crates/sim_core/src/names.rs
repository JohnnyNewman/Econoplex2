//! People's given names, for the views (a battle names its soldiers:
//! `battle::Fighter`). A name follows from the person's index in the agent
//! store and their sex alone, so it needs no state, draws on no random
//! stream and is the same in every client and after a reload.

const WOMEN: [&str; 48] = [
    "Mara", "Agnes", "Edith", "Alys", "Joan", "Maud", "Isolde", "Wynn", "Hild", "Elsa", "Bryn",
    "Cora", "Della", "Ebba", "Freya", "Gwen", "Hanna", "Ida", "Juna", "Katla", "Lena", "Mira",
    "Nell", "Orla", "Petra", "Rhea", "Sigrid", "Tilde", "Una", "Vera", "Wren", "Yara", "Ada",
    "Beth", "Clara", "Dora", "Eda", "Fern", "Greta", "Hester", "Iris", "Jessa", "Kit", "Lise",
    "Meg", "Nora", "Olga", "Rose",
];

const MEN: [&str; 48] = [
    "Tom", "Aldric", "Bram", "Cole", "Dunstan", "Edwin", "Finn", "Godric", "Hal", "Ivo", "Jory",
    "Kell", "Leof", "Milo", "Ned", "Osric", "Piers", "Rolf", "Sven", "Tam", "Ulf", "Wat", "Alan",
    "Bert", "Cedric", "Dirk", "Egil", "Fulk", "Gil", "Hugh", "Ivar", "Jon", "Kurt", "Lars", "Mats",
    "Nico", "Odo", "Paul", "Ralf", "Simon", "Tobin", "Udo", "Walt", "Arne", "Bjorn", "Colm",
    "Davy", "Emil",
];

/// The given name of person `id` (agent store index).
pub fn given(id: u32, female: bool) -> &'static str {
    // A cheap mix (splitmix64's finaliser), so neighbours differ.
    let mut x = (id as u64).wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    let list: &[&str] = if female { &WOMEN } else { &MEN };
    list[(x % list.len() as u64) as usize]
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_are_fixed_and_varied() {
        assert_eq!(super::given(7, true), super::given(7, true));
        let n: std::collections::HashSet<_> = (0..200).map(|i| super::given(i, false)).collect();
        assert!(n.len() > 30, "{}", n.len());
    }
}
