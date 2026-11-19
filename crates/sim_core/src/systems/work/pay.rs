//! Wages: posted pay rates and settling what each worker earned.

use super::*;

/// Share of the way the public pay rate moves toward each realised run of work.
const PAY_RATE_SMOOTHING: f32 = 0.05;

/// What a worker-day at recipe `r` would pay in this town at today's prices: the
/// workers' share of the value added (plus subsidy) of one run, over the days one
/// worker needs for it, at a typical worker's success rate.
pub fn posted_pay(
    town: &crate::world::Town,
    db: &Db,
    params: &crate::config::ModelParams,
    r: usize,
) -> f32 {
    let m = &db.recipes[r];
    let out: f32 = m
        .outputs
        .iter()
        .map(|&(p, q)| {
            let p = p as usize;
            (town.price[p] + town.policy.subsidy[p] * db.base_price[p]) * q
        })
        .sum();
    let inputs: f32 = m
        .inputs
        .iter()
        .map(|&(p, q)| town.price[p as usize] * q)
        .sum();
    let base = town.policy.wage_share.unwrap_or(params.work.wage_share);
    let factor = town.wage_factor.get(r).copied().unwrap_or(1.0);
    let share = (base * factor).min(1.0);
    share * (out - inputs).max(0.0) * params.decision.valuation.typical_success
        / m.duration.max(1.0)
}

/// Every step each town's public pay rate drifts toward the posted pay at today's
/// prices, over `decision.valuation.posted_pay_days`. Realised pay pulls much
/// harder (each finished step), so for jobs people do the rate is what they
/// earn; a job nobody has tried yet still shows what it would pay, instead of
/// nothing, and so can be tried.
pub fn post_pay(towns: &mut crate::world::Towns, db: &Db, params: &Params) {
    let days = params.decision.valuation.posted_pay_days;
    if days <= 0.0 {
        return;
    }
    let a = params.calendar().smoothing(days);
    for town in towns.0.iter_mut() {
        town.pay_rate.resize(db.recipes.len(), 0.0);
        for r in 0..db.recipes.len() {
            let posted = posted_pay(town, db, params, r);
            town.pay_rate[r] += (posted - town.pay_rate[r]) * a;
        }
    }
}

/// Pay the step's claims. When a town's treasury can't cover all wages, every claim
/// gets the same share of what it is owed, so pay still follows prices; subsidies are
/// paid the same way from what is left. Then income tax, the workers' pay, and the pay
/// expectations (public rate and each worker's own).
pub(super) fn settle_wages(
    claims: Vec<Claim>,
    towns: &mut Towns,
    store: &mut AgentStore,
    decision: &crate::decision::DecisionModels,
    apprentice_wage: f32,
    firms: crate::firms::FirmsParams,
) {
    let n_towns = towns.0.len();
    let reserve_days = match firms {
        crate::firms::FirmsParams::Workshops { reserve_days, .. } => reserve_days,
        crate::firms::FirmsParams::Treasury => 0.0,
    };
    let mut owed = vec![(0.0f32, 0.0f32); n_towns];
    // Wages a firm owes today, per town and firm.
    let mut firm_owed: Vec<Vec<f32>> = towns.0.iter().map(|t| vec![0.0; t.firms.len()]).collect();
    for c in &claims {
        match c.firm {
            Some(f) => firm_owed[c.town][f] += c.wage,
            None => owed[c.town].0 += c.wage,
        }
        owed[c.town].1 += c.subsidy;
    }
    // Each firm pays its claims at one ratio, out of its cash.
    let firm_ratio: Vec<Vec<f32>> = towns
        .0
        .iter()
        .zip(&firm_owed)
        .map(|(t, o)| {
            t.firms
                .iter()
                .zip(o)
                .map(|(f, &w)| {
                    if w > 0.0 {
                        (f.wage_budget(reserve_days) / w).min(1.0)
                    } else {
                        0.0
                    }
                })
                .collect()
        })
        .collect();
    let ratios: Vec<(f32, f32)> = towns
        .0
        .iter()
        .zip(&owed)
        .map(|(town, &(w, sub))| {
            let budget = town.treasury.coins().max(0.0);
            let rw = if w > 0.0 { (budget / w).min(1.0) } else { 0.0 };
            let left = (budget - w * rw).max(0.0);
            let rs = if sub > 0.0 {
                (left / sub).min(1.0)
            } else {
                0.0
            };
            (rw, rs)
        })
        .collect();
    for c in claims {
        let town = &mut towns.0[c.town];
        let (rw, rs) = ratios[c.town];
        let subsidy = c.subsidy * rs;
        let wage = match c.firm {
            Some(f) => {
                let wage = c.wage * firm_ratio[c.town][f];
                town.firms[f].cash -= crate::coin::Coin::from_coins(wage);
                town.firms[f].book(crate::firms::FirmFlow::Wages, wage);
                town.treasury -= crate::coin::Coin::from_coins(subsidy);
                wage
            }
            None => {
                let wage = c.wage * rw;
                town.treasury -= crate::coin::Coin::from_coins(wage);
                town.treasury -= crate::coin::Coin::from_coins(subsidy);
                town.ledger.add(crate::ledger::Flow::Wages, wage);
                wage
            }
        };
        town.ledger.add(crate::ledger::Flow::Subsidies, subsidy);
        let pay = wage + subsidy;
        // Income tax: the treasury keeps its share of every wage.
        let tax = pay * town.policy.income_tax;
        town.treasury += crate::coin::Coin::from_coins(tax);
        town.ledger.add(crate::ledger::Flow::IncomeTax, tax);
        // What the workers share, to the unit: whatever rounding leaves over
        // after each got their part stays with (or comes from) the treasury.
        let mut to_share = crate::coin::Coin::from_coins(wage)
            + crate::coin::Coin::from_coins(subsidy)
            - crate::coin::Coin::from_coins(tax);
        // Shares: 1 per regular worker, `apprentice_wage` per apprentice.
        let shares = c.workers.len() as f32 - c.trainees.len() as f32 * (1.0 - apprentice_wage);
        let each = (pay - tax) / shares.max(1e-6);
        let per_day = each / c.days;
        town.index_year.worker_days += shares * c.days;
        // Each run moves the public rate a step toward what it paid.
        let smoothing = 1.0 - (1.0 - PAY_RATE_SMOOTHING).powi(c.runs.max(1) as i32);
        let rate = &mut town.pay_rate[c.recipe as usize];
        *rate += smoothing * (per_day - *rate);
        for &w in &c.workers {
            let wi = w as usize;
            let own = if c.trainees.contains(&w) {
                each * apprentice_wage
            } else {
                each
            };
            let own_coin = crate::coin::Coin::from_coins(own);
            store.wealth[wi] += own_coin;
            to_share -= own_coin;
            let curiosity = store.mind[wi][sim_data::dims::mind::CURIOSITY];
            crate::decision::expectations::remember(
                &mut store.pay_expect[wi],
                c.recipe,
                own / c.days,
                c.days,
                curiosity,
                &*decision.expectations,
            );
        }
        town.treasury += to_share;
    }
}
