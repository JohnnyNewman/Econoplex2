//! Workshops as firms (see docs/design/firms.md), behind `market.firms`.
//!
//! `Treasury` (the default) keeps today's rule: the town market buys every output
//! and pays the makers out of the treasury. `Workshops` makes each town's
//! workplaces of one kind a firm with its own cash and stock:
//! - a run's goods go into the firm's stock; the town market buys from firms only
//!   what it wants (up to its target stock), paying from the treasury;
//! - the firm buys its inputs from the market before a run, out of its cash;
//! - the firm pays its workers' wages (the same claim as before: the wage share of
//!   the value added, with apprentice wages and the market pay factor) out of its
//!   cash, pro rata when short;
//! - cash above a reserve of a few days' wages is paid out to the owners (the
//!   town's guild members of the firm's domain), less a profit tax to the
//!   treasury;
//! - the treasury gives each firm start-up cash and pays its own public employees
//!   (builders of public buildings, land clearers, minters) as before.
//!
//! A firm is identified by its key: the building its recipes need, else the
//! nature site they work, else the skill domain (a guild trade).

use serde::{Deserialize, Serialize};

/// `market.firms` in models.ron.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub enum FirmsParams {
    /// The town market buys every output and pays wages from the treasury.
    #[default]
    Treasury,
    Workshops {
        /// Start-up cash each firm gets from the treasury (if it has it).
        start_cash: f32,
        /// Cash a firm keeps, in days of its recent wage bill, before paying out.
        reserve_days: f32,
        /// Share of the cash above the reserve paid out each day.
        payout_rate: f32,
        /// Share of each payout the treasury takes as profit tax.
        profit_tax: f32,
        /// Days a firm may lack the cash of a fifth of `start_cash` before it
        /// raises new capital (a treasury grant, then its owners); with none, it
        /// closes until capital comes.
        #[serde(default = "close_days_default")]
        close_days: u32,
    },
}

fn close_days_default() -> u32 {
    30
}

impl FirmsParams {
    pub fn on(&self) -> bool {
        matches!(self, FirmsParams::Workshops { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FirmKey {
    Building(u32),
    Nature(u32),
    Guild(u32),
}

/// A firm's own accounts, by kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmFlow {
    /// Paid by the town market for goods.
    Sales,
    /// Paid by residents buying straight from the firm (food when the market is out).
    DirectSales,
    /// Paid to the market for inputs.
    Inputs,
    Wages,
    /// Paid out to owners, after profit tax.
    Dividends,
    ProfitTax,
    /// Start-up cash from the treasury.
    Grants,
    /// Capital put in by the owners.
    Capital,
    /// Loans received from the treasury, principal repaid, and interest paid.
    Loans,
    Repayments,
    Interest,
}

pub const FIRM_FLOWS: usize = 11;

impl FirmFlow {
    pub const ALL: [FirmFlow; FIRM_FLOWS] = [
        FirmFlow::Sales,
        FirmFlow::DirectSales,
        FirmFlow::Inputs,
        FirmFlow::Wages,
        FirmFlow::Dividends,
        FirmFlow::ProfitTax,
        FirmFlow::Grants,
        FirmFlow::Capital,
        FirmFlow::Loans,
        FirmFlow::Repayments,
        FirmFlow::Interest,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FirmFlow::Sales => "sales",
            FirmFlow::DirectSales => "direct sales",
            FirmFlow::Inputs => "inputs",
            FirmFlow::Wages => "wages",
            FirmFlow::Dividends => "dividends",
            FirmFlow::ProfitTax => "profit tax",
            FirmFlow::Grants => "grants",
            FirmFlow::Capital => "owners' capital",
            FirmFlow::Loans => "loans",
            FirmFlow::Repayments => "repayments",
            FirmFlow::Interest => "interest",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Firm {
    pub key: Option<FirmKey>,
    pub cash: f32,
    /// Unsold goods, by product.
    pub stock: Vec<f32>,
    /// Wages paid per day, averaged over about a month (for the reserve).
    pub wage_bill: f32,
    /// Whether it got its start-up cash.
    pub started: bool,
    pub this_year: [f32; FIRM_FLOWS],
    pub last_year: [f32; FIRM_FLOWS],
    /// Today's wages (rolled into `wage_bill` each day).
    pub wages_today: f32,
    /// Inputs bought per day, averaged over about a month, and today's.
    #[serde(default)]
    pub input_bill: f32,
    #[serde(default)]
    pub inputs_today: f32,
    /// Capital paid in (start-up grants, owners' capital): never paid out.
    #[serde(default)]
    pub capital: f32,
    /// Days in a row it has been short of cash, and whether it is closed.
    #[serde(default)]
    pub short_days: u32,
    #[serde(default)]
    pub closed: bool,
    /// Loans from the town's treasury still to repay.
    #[serde(default)]
    pub loans: Vec<Loan>,
}

/// A loan from the treasury to a firm, repaid in daily instalments with interest.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Loan {
    /// Principal still owed.
    pub left: f32,
    /// Yearly interest rate.
    pub rate: f32,
    pub days_left: u32,
}

impl Loan {
    /// Today's instalment: (interest, principal), for a year of `days_per_year`.
    pub fn due(&self, days_per_year: f32) -> (f32, f32) {
        let interest = self.left * self.rate.max(0.0) / days_per_year.max(1.0);
        let principal = self.left / self.days_left.max(1) as f32;
        (interest, principal)
    }
}

/// A firm's display name: its building, nature site or guild domain.
pub fn firm_name(db: &crate::db::Db, key: FirmKey) -> String {
    match key {
        FirmKey::Building(b) => db.content.products[b as usize].id.clone(),
        FirmKey::Nature(k) => db.content.nature[k as usize].id.clone(),
        FirmKey::Guild(d) => db.content.domains[d as usize].clone(),
    }
}

/// Outstanding principal over a town's firms.
pub fn outstanding(firms: &[Firm]) -> f32 {
    firms.iter().flat_map(|f| &f.loans).map(|l| l.left).sum()
}

impl Firm {
    pub fn book(&mut self, f: FirmFlow, amount: f32) {
        self.this_year[f as usize] += amount;
        if f == FirmFlow::Wages {
            self.wages_today += amount;
        }
        if f == FirmFlow::Inputs {
            self.inputs_today += amount;
        }
    }

    /// Cash it may pay out today: `rate` of what is above `reserve_days` of its
    /// wages and inputs.
    pub fn payout(&self, reserve_days: f32, rate: f32) -> f32 {
        let reserve = (reserve_days * (self.wage_bill + self.input_bill)).max(self.capital);
        ((self.cash - reserve) * rate.clamp(0.0, 1.0)).max(0.0)
    }

    /// Cash it can spend on wages: what is above `reserve_days` of its inputs (the
    /// working capital it keeps to buy the next inputs).
    pub fn wage_budget(&self, reserve_days: f32) -> f32 {
        (self.cash - reserve_days * self.input_bill).max(0.0)
    }

    /// Close the day's books.
    pub fn end_day(&mut self) {
        self.wage_bill += (self.wages_today - self.wage_bill) / 30.0;
        self.wages_today = 0.0;
        self.input_bill += (self.inputs_today - self.input_bill) / 30.0;
        self.inputs_today = 0.0;
    }

    pub fn end_year(&mut self) {
        self.last_year = std::mem::replace(&mut self.this_year, [0.0; FIRM_FLOWS]);
    }
}

/// A town's firms, one per firm key, with empty accounts.
pub fn new_firms(db: &crate::db::Db) -> Vec<Firm> {
    db.firm_keys
        .iter()
        .map(|&k| Firm {
            key: Some(k),
            stock: vec![0.0; db.content.products.len()],
            ..Default::default()
        })
        .collect()
}

/// Last year's totals over a town's firms, by flow.
pub fn last_year_totals(firms: &[Firm]) -> [f32; FIRM_FLOWS] {
    let mut t = [0.0; FIRM_FLOWS];
    for f in firms {
        for (k, v) in f.last_year.iter().enumerate() {
            t[k] += v;
        }
    }
    t
}

/// The firm a recipe's work belongs to.
pub fn key_of(r: &crate::db::RecipeMeta) -> FirmKey {
    match (r.building, r.nature) {
        (Some(b), _) => FirmKey::Building(b),
        (None, Some(k)) => FirmKey::Nature(k),
        (None, None) => FirmKey::Guild(r.domain),
    }
}

/// Whether a recipe's work is public (paid by the treasury even with firms):
/// buildings, opening land, striking coin.
pub fn is_public(db: &crate::db::Db, r: &crate::db::RecipeMeta) -> bool {
    r.builds.is_some()
        || r.outputs
            .iter()
            .any(|&(p, _)| db.coin[p as usize] > 0.0 || db.opens[p as usize].is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_firm_pays_out_only_above_its_reserve() {
        let mut f = Firm {
            cash: 100.0,
            wage_bill: 10.0,
            ..Default::default()
        };
        assert_eq!(f.payout(5.0, 0.5), 25.0);
        // Paid-in capital is never paid out.
        let idle = Firm {
            cash: 30.0,
            capital: 30.0,
            ..Default::default()
        };
        assert_eq!(idle.payout(5.0, 0.5), 0.0);
        // Working capital for inputs comes before wages.
        f.input_bill = 4.0;
        assert_eq!(f.wage_budget(5.0), 80.0);
        f.cash = 40.0;
        assert_eq!(f.payout(5.0, 0.5), 0.0);
        f.book(FirmFlow::Wages, 20.0);
        f.end_day();
        assert!((f.wage_bill - (10.0 + (20.0 - 10.0) / 30.0)).abs() < 1e-5);
    }

    #[test]
    fn a_loan_is_repaid_in_instalments_with_interest() {
        let mut l = Loan {
            left: 120.0,
            rate: 0.1,
            days_left: 120,
        };
        let (i, p) = l.due(120.0);
        assert!((i - 0.1).abs() < 1e-6 && (p - 1.0).abs() < 1e-6);
        let mut paid = 0.0;
        while l.days_left > 0 {
            let (i, p) = l.due(120.0);
            paid += i + p;
            l.left -= p;
            l.days_left -= 1;
        }
        assert!(l.left.abs() < 1e-3);
        assert!(paid > 120.0 && paid < 120.0 * 1.1);
    }
}
