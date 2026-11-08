//! Money accounting: every payment into or out of a town treasury, by kind.
//!
//! Read with a credit-first eye, the town is the issuer of its money. What it pays
//! out (wages, subsidies, fees, soldiers' pay, public spending, grants) puts money
//! into people's hands: **issued**. What it takes back by right (the wealth tax,
//! estates without heirs) is **taxed**. What people hand back by choice, buying food
//! and tools at the town market, is **spent**. Trade, tribute and plunder move money
//! between towns. Coin struck at a mint is new money: **minted**. Apart from that
//! the total stays constant; the ledger only labels the flows.
//!
//! The ledger keeps today's sums, a daily average over about a month (for the HUD),
//! and yearly totals (for metrics). It also tracks the value of goods each town ships
//! to every other one, which the game draws as flows on the map.

use crate::world::{SimClock, Towns};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Flow {
    // Issued: the treasury pays people.
    Wages,
    Subsidies,
    SchoolFees,
    SoldierPay,
    PublicSpending,
    SettlementGrants,
    // Taxed: people pay the treasury by right.
    WealthTax,
    Estates,
    // Spent: people buy from the town market.
    FoodSales,
    ToolSales,
    // Between towns.
    TradeSales,
    TradePurchases,
    TributeReceived,
    TributePaid,
    LootTaken,
    LootLost,
    // New coin struck at the town's mint.
    Minted,
}

pub const FLOWS: usize = 17;

/// How the HUD groups flows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Issued,
    Taxed,
    Spent,
    /// Money coming in from other towns.
    ExternalIn,
    /// Money going out to other towns.
    ExternalOut,
    /// New money from the mint.
    Minted,
}

impl Flow {
    pub const ALL: [Flow; FLOWS] = [
        Flow::Wages,
        Flow::Subsidies,
        Flow::SchoolFees,
        Flow::SoldierPay,
        Flow::PublicSpending,
        Flow::SettlementGrants,
        Flow::WealthTax,
        Flow::Estates,
        Flow::FoodSales,
        Flow::ToolSales,
        Flow::TradeSales,
        Flow::TradePurchases,
        Flow::TributeReceived,
        Flow::TributePaid,
        Flow::LootTaken,
        Flow::LootLost,
        Flow::Minted,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Flow::Wages => "wages",
            Flow::Subsidies => "subsidies",
            Flow::SchoolFees => "school fees",
            Flow::SoldierPay => "soldiers' pay",
            Flow::PublicSpending => "public spending",
            Flow::SettlementGrants => "settlement grants",
            Flow::WealthTax => "wealth tax",
            Flow::Estates => "estates",
            Flow::FoodSales => "food",
            Flow::ToolSales => "tools",
            Flow::TradeSales => "trade sales",
            Flow::TradePurchases => "trade purchases",
            Flow::TributeReceived => "tribute received",
            Flow::TributePaid => "tribute paid",
            Flow::LootTaken => "loot taken",
            Flow::LootLost => "loot lost",
            Flow::Minted => "minted",
        }
    }

    pub fn group(self) -> Group {
        use Flow::*;
        match self {
            Wages | Subsidies | SchoolFees | SoldierPay | PublicSpending | SettlementGrants => {
                Group::Issued
            }
            WealthTax | Estates => Group::Taxed,
            FoodSales | ToolSales => Group::Spent,
            TradeSales | TributeReceived | LootTaken => Group::ExternalIn,
            TradePurchases | TributePaid | LootLost => Group::ExternalOut,
            Minted => Group::Minted,
        }
    }
}

/// Days the monthly average reaches back (time constant of the moving average).
pub const MONTH_DAYS: f32 = 30.0;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ledger {
    pub today: [f32; FLOWS],
    /// Daily average over about the last month.
    pub monthly: [f32; FLOWS],
    pub this_year: [f32; FLOWS],
    pub last_year: [f32; FLOWS],
    /// Since the start of the game.
    #[serde(default)]
    pub total: [f64; FLOWS],
    /// Value of goods shipped to each town (at the buyer's price): today, and the
    /// daily average over about a month.
    pub exports_today: Vec<f32>,
    pub exports: Vec<f32>,
}

impl Ledger {
    pub fn new(towns: usize) -> Self {
        Ledger {
            exports_today: vec![0.0; towns],
            exports: vec![0.0; towns],
            ..Default::default()
        }
    }

    #[inline]
    pub fn add(&mut self, flow: Flow, amount: f32) {
        self.today[flow as usize] += amount;
    }

    pub fn export(&mut self, to: usize, value: f32) {
        if self.exports_today.len() <= to {
            self.exports_today.resize(to + 1, 0.0);
        }
        self.exports_today[to] += value;
    }

    /// Sum of a group: daily average over the last month.
    pub fn monthly_group(&self, g: Group) -> f32 {
        sum_group(&self.monthly, g)
    }

    pub fn last_year_group(&self, g: Group) -> f32 {
        sum_group(&self.last_year, g)
    }

    fn end_day(&mut self) {
        let a = 1.0 / MONTH_DAYS;
        for k in 0..FLOWS {
            self.monthly[k] += a * (self.today[k] - self.monthly[k]);
            self.this_year[k] += self.today[k];
            self.total[k] += self.today[k] as f64;
        }
        self.today = [0.0; FLOWS];
        self.exports.resize(self.exports_today.len(), 0.0);
        for (m, t) in self.exports.iter_mut().zip(self.exports_today.iter_mut()) {
            *m += a * (*t - *m);
            *t = 0.0;
        }
    }

    fn end_year(&mut self) {
        self.last_year = std::mem::replace(&mut self.this_year, [0.0; FLOWS]);
    }
}

pub fn sum_group(v: &[f32; FLOWS], g: Group) -> f32 {
    Flow::ALL
        .iter()
        .filter(|f| f.group() == g)
        .map(|f| v[*f as usize])
        .sum()
}

/// Close the day's books (and the year's at year end). Runs before metrics.
pub fn close_day(mut towns: ResMut<Towns>, clock: Res<SimClock>) {
    let year_end = clock.tick > 0 && clock.day_of_year() == 0;
    for t in towns.0.iter_mut() {
        t.ledger.end_day();
        if year_end {
            t.ledger.end_year();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_flow_is_listed_once_in_order() {
        for (k, f) in Flow::ALL.iter().enumerate() {
            assert_eq!(*f as usize, k);
        }
    }

    #[test]
    fn the_monthly_average_approaches_a_steady_flow() {
        let mut l = Ledger::new(2);
        for _ in 0..300 {
            l.add(Flow::Wages, 10.0);
            l.export(1, 4.0);
            l.end_day();
        }
        assert!((l.monthly[Flow::Wages as usize] - 10.0).abs() < 0.01);
        assert!((l.exports[1] - 4.0).abs() < 0.01);
        assert_eq!(l.monthly_group(Group::Issued), l.monthly[0]);
        l.end_year();
        assert_eq!(l.last_year[Flow::Wages as usize], 3000.0);
        assert_eq!(l.this_year[Flow::Wages as usize], 0.0);
    }
}
