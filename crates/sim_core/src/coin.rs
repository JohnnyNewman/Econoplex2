//! Money as whole numbers.
//!
//! Every holding (a person's wealth, a town's treasury, a firm's cash) is a
//! [`Coin`]: a count of thousandths of a coin. Whole numbers add up exactly in
//! any order, so money is conserved to the last unit and two runs can never
//! drift apart by rounding.
//!
//! Prices and rates stay fractional. A payment is worked out as a fraction,
//! turned into a `Coin` once with [`Coin::from_coins`], and that same amount
//! is taken from the payer and given to the payee.

use serde::{Deserialize, Serialize};
use std::iter::Sum;
use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// Units in one coin.
pub const UNITS: i64 = 1000;

/// An amount of money, in thousandths of a coin.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Coin(pub i64);

impl Coin {
    pub const ZERO: Coin = Coin(0);

    /// The nearest amount to `coins` (not a number: nothing).
    pub fn from_coins(coins: f32) -> Coin {
        Coin((coins as f64 * UNITS as f64).round() as i64)
    }

    /// The amount in coins, for formulas and display.
    pub fn coins(self) -> f32 {
        (self.0 as f64 / UNITS as f64) as f32
    }

    /// The amount in coins at full precision, for totals.
    pub fn coins_f64(self) -> f64 {
        self.0 as f64 / UNITS as f64
    }

    pub fn min(self, other: Coin) -> Coin {
        Coin(self.0.min(other.0))
    }

    pub fn max(self, other: Coin) -> Coin {
        Coin(self.0.max(other.0))
    }

    /// `self` split into `parts` equal shares and what is left over, so that
    /// `share * parts + rest == self`.
    pub fn split(self, parts: usize) -> (Coin, Coin) {
        if parts == 0 {
            return (Coin::ZERO, self);
        }
        let share = self.0.div_euclid(parts as i64);
        (Coin(share), Coin(self.0 - share * parts as i64))
    }
}

impl Add for Coin {
    type Output = Coin;
    fn add(self, o: Coin) -> Coin {
        Coin(self.0 + o.0)
    }
}

impl Sub for Coin {
    type Output = Coin;
    fn sub(self, o: Coin) -> Coin {
        Coin(self.0 - o.0)
    }
}

impl Neg for Coin {
    type Output = Coin;
    fn neg(self) -> Coin {
        Coin(-self.0)
    }
}

impl AddAssign for Coin {
    fn add_assign(&mut self, o: Coin) {
        self.0 += o.0;
    }
}

impl SubAssign for Coin {
    fn sub_assign(&mut self, o: Coin) {
        self.0 -= o.0;
    }
}

impl Sum for Coin {
    fn sum<I: Iterator<Item = Coin>>(iter: I) -> Coin {
        Coin(iter.map(|c| c.0).sum())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_round_to_the_nearest_unit_and_read_back() {
        assert_eq!(Coin::from_coins(0.3), Coin(300));
        assert_eq!(Coin::from_coins(-1.2504), Coin(-1250));
        assert_eq!(Coin::from_coins(f32::NAN), Coin::ZERO);
        assert_eq!(Coin(1500).coins(), 1.5);
    }

    #[test]
    fn a_split_gives_back_exactly_what_went_in() {
        let (share, rest) = Coin(1000).split(3);
        assert_eq!((share, rest), (Coin(333), Coin(1)));
        assert_eq!(Coin(share.0 * 3) + rest, Coin(1000));
        assert_eq!(Coin(7).split(0), (Coin::ZERO, Coin(7)));
    }
}
