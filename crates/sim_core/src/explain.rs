//! The "why" view: a rule-based explainer (docs/design/graphics-ui.md, section 4).
//!
//! [`explain`] looks at one town and returns its problems ("symptoms") as a tree
//! of [`Finding`]s: each finding says what is wrong, and its children say why,
//! walking the dependency graph of goods, jobs and money that the simulation
//! already keeps:
//! - **hunger:** food per resident against what the market aims for, and
//!   starving person-days, explained by the food goods that are short;
//! - **shortages:** goods far below the market's target stock. Each is
//!   explained by the recipes that make it:
//!   - no building of the right type, or all of them full;
//!   - the land it needs is missing or worn out;
//!   - an input that is short itself (followed recursively);
//!   - nobody in town works at the recipe, and what it pays against the
//!     town's typical pay;
//! - **a low treasury:** its biggest money flows out and in (from the ledger),
//!   and tribute paid to a ruler;
//! - **people leaving:** departures against arrivals since the last yearly
//!   report.
//!
//! It only reads the world, so it cannot change the simulation. The rules are
//! deliberately simple and their thresholds are constants here: a starting
//! point to refine, not a model of its own.

use crate::components::NaturalResource;
use crate::db::Db;
use crate::ledger::{Flow, Group};
use crate::store::AgentStore;
use crate::world::{Params, Towns};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

/// A good counts as short below this share of its target stock.
const SHORT: f32 = 0.5;
/// Buildings count as full above this share of their slots in use.
const FULL: f32 = 0.95;
/// How deep the explainer follows inputs.
const MAX_DEPTH: usize = 3;

/// One statement about a town, with the reasons behind it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub text: String,
    /// How bad it is, 0..1 (for sorting and colour).
    pub severity: f32,
    /// The building type involved, if any (product id), so a view can point
    /// at those buildings on the map.
    pub building: Option<String>,
    /// Why: the findings this one follows from.
    pub because: Vec<Finding>,
}

impl Finding {
    fn new(text: impl Into<String>, severity: f32) -> Self {
        Finding {
            text: text.into(),
            severity: severity.clamp(0.0, 1.0),
            building: None,
            because: Vec::new(),
        }
    }
}

/// The town's problems and their causes, worst first.
pub fn explain(world: &World, town: u16) -> Vec<Finding> {
    let towns = world.resource::<Towns>();
    let Some(t) = towns.0.get(town as usize) else {
        return Vec::new();
    };
    let ctx = Ctx {
        world,
        db: world.resource::<Db>(),
        params: &world.resource::<Params>().0,
        store: world.resource::<AgentStore>(),
        town,
        t,
    };
    let mut out = Vec::new();
    out.extend(ctx.hunger());
    out.extend(ctx.shortages());
    out.extend(ctx.treasury(towns));
    out.extend(ctx.leaving());
    out.sort_by(|a, b| b.severity.total_cmp(&a.severity));
    out
}

struct Ctx<'a> {
    world: &'a World,
    db: &'a Db,
    params: &'a crate::config::ModelParams,
    store: &'a AgentStore,
    town: u16,
    t: &'a crate::world::Town,
}

impl Ctx<'_> {
    fn pop(&self) -> f32 {
        self.t.residents.len().max(1) as f32
    }

    fn name(&self, p: u32) -> &str {
        &self.db.content.products[p as usize].name
    }

    /// The stock the market aims for (the default `StockTarget` rule).
    fn target(&self, p: u32) -> f32 {
        let mp = &self.params.market;
        let per_capita = if self.db.is_food[p as usize] {
            mp.food_target_per_capita
        } else {
            mp.target_per_capita
        };
        (per_capita * self.pop()).max(1.0) + self.t.policy.ordered(p as usize)
    }

    /// Stock as a share of the target.
    fn fill(&self, p: u32) -> f32 {
        self.t.stock[p as usize] / self.target(p)
    }

    fn hunger(&self) -> Option<Finding> {
        let per_capita = self.t.food_stock(self.db) / self.pop();
        let aim = self.params.market.food_target_per_capita;
        if per_capita >= aim * SHORT && self.t.starving == 0 {
            return None;
        }
        let mut f = Finding::new(
            format!(
                "Food is short: {per_capita:.1} per resident, the market aims for {aim:.1}{}",
                if self.t.starving > 0 {
                    format!("; {} person-days starving this year", self.t.starving)
                } else {
                    String::new()
                }
            ),
            1.0 - per_capita / aim + if self.t.starving > 0 { 0.3 } else { 0.0 },
        );
        // Which foods are short, and why.
        let mut foods: Vec<u32> = self.db.foods.iter().map(|(p, _)| *p).collect();
        foods.sort_by(|a, b| self.fill(*a).total_cmp(&self.fill(*b)));
        for p in foods.into_iter().take(2) {
            if self.fill(p) < SHORT {
                f.because.push(self.good(p, 1, &mut Vec::new()));
            }
        }
        Some(f)
    }

    fn shortages(&self) -> Vec<Finding> {
        // Goods (not food: that is the hunger finding) used as inputs or sold,
        // far below target, the scarcest first.
        let mut short: Vec<u32> = (0..self.t.stock.len() as u32)
            .filter(|&p| self.db.category(p) != sim_data::Category::Building)
            .filter(|&p| !self.db.is_food[p as usize] && self.db.is_input[p as usize])
            .filter(|&p| self.db.base_price[p as usize] > 0.0)
            .filter(|&p| self.fill(p) < SHORT * 0.4)
            .collect();
        short.sort_by(|a, b| self.fill(*a).total_cmp(&self.fill(*b)));
        let found: Vec<(u32, Finding)> = short
            .into_iter()
            .take(6)
            .map(|p| (p, self.good(p, 0, &mut Vec::new())))
            .collect();
        // A good already explained inside another one's chain (iron inside
        // steel) is not listed again on its own.
        let inside = |p: u32, f: &Finding| {
            let needle = format!("{} is short", self.name(p));
            f.because.iter().any(|c| contains(c, &needle))
        };
        found
            .iter()
            .filter(|(p, _)| !found.iter().any(|(q, f)| q != p && inside(*p, f)))
            .map(|(_, f)| f.clone())
            .take(3)
            .collect()
    }

    /// Why good `p` is short: the recipes that make it, and what stops them.
    fn good(&self, p: u32, depth: usize, seen: &mut Vec<u32>) -> Finding {
        seen.push(p);
        let fill = self.fill(p);
        let mut f = Finding::new(
            format!(
                "{} is short: {:.0} in stock, the market aims for {:.0} (price {:.2})",
                self.name(p),
                self.t.stock[p as usize],
                self.target(p),
                self.t.price[p as usize],
            ),
            1.0 - fill,
        );
        let recipes: Vec<usize> = (0..self.db.recipes.len())
            .filter(|&r| self.db.recipes[r].outputs.iter().any(|o| o.0 == p))
            .collect();
        if recipes.is_empty() {
            f.because.push(Finding::new(
                format!(
                    "Nobody can make {} here: it must come by caravan",
                    self.name(p)
                ),
                0.6,
            ));
        }
        for r in recipes.into_iter().take(2) {
            f.because.extend(self.recipe(r, depth, seen));
        }
        f
    }

    /// What stops recipe `r` from running.
    fn recipe(&self, r: usize, depth: usize, seen: &mut Vec<u32>) -> Vec<Finding> {
        let meta = &self.db.recipes[r];
        let mut out = Vec::new();
        if let Some(b) = meta.building {
            let id = self.db.content.products[b as usize].id.clone();
            let bname = self.name(b).to_string();
            let count = self.t.building_count(b);
            let load = self.t.building_load.get(b as usize).copied().unwrap_or(0.0);
            if count == 0 {
                let mut f = Finding::new(format!("There is no {bname} for {}", meta.name), 0.9);
                f.building = Some(id);
                out.push(f);
            } else if load > FULL {
                let mut f = Finding::new(
                    format!(
                        "The {count} {bname}(s) are full ({:.0}% of places in use)",
                        load * 100.0
                    ),
                    0.6,
                );
                f.building = Some(id);
                out.push(f);
            }
        }
        if let Some(k) = meta.nature {
            let (amount, capacity) = self
                .t
                .nature_sites
                .iter()
                .filter_map(|&e| self.world.get::<NaturalResource>(e))
                .filter(|n| n.kind == k)
                .fold((0.0, 0.0), |(a, c), n| (a + n.amount, c + n.capacity));
            let kind = &self.db.content.nature[k as usize].name;
            if capacity == 0.0 {
                out.push(Finding::new(
                    format!("The town has no {kind} for {}", meta.name),
                    0.8,
                ));
            } else if amount < capacity * 0.1 {
                out.push(Finding::new(
                    format!(
                        "The {kind} sites are worn out ({:.0}% left)",
                        100.0 * amount / capacity
                    ),
                    0.6,
                ));
            }
        }
        for &(input, _) in &meta.inputs {
            if self.fill(input) < SHORT && !seen.contains(&input) {
                if depth < MAX_DEPTH {
                    let mut f = self.good(input, depth + 1, seen);
                    f.text = format!("{} needs {}. {}", meta.name, self.name(input), f.text);
                    out.push(f);
                } else {
                    out.push(Finding::new(
                        format!(
                            "{} needs {}, which is short too",
                            meta.name,
                            self.name(input)
                        ),
                        0.5,
                    ));
                }
            }
        }
        let workers = (0..self.store.len())
            .filter(|&i| self.store.alive[i] && self.store.town[i] == self.town)
            // Runs take several days, so count everyone who did this trade
            // today or as their last job.
            .filter(|&i| {
                self.store.worked_today[i] == Some(r as u32)
                    || self.store.last_recipe[i] == Some(r as u32)
            })
            .count();
        if workers == 0 {
            let pay = self.t.pay_rate.get(r).copied().unwrap_or(0.0);
            let typical = typical_pay(&self.t.pay_rate);
            let mut f = Finding::new(format!("Nobody here works at {}", meta.name), 0.5);
            if pay < typical * 0.8 {
                f.because.push(Finding::new(
                    format!("It pays {pay:.2} a day, the town's typical work pays {typical:.2}"),
                    0.5,
                ));
            }
            out.push(f);
        }
        out
    }

    fn treasury(&self, towns: &Towns) -> Option<Finding> {
        let reserve = self.params.market.mint_below_per_capita * self.pop();
        if self.t.treasury >= reserve {
            return None;
        }
        let mut f = Finding::new(
            format!(
                "The treasury is low: {:.0}, a reserve would be {:.0}",
                self.t.treasury, reserve
            ),
            0.4 + 0.5 * (1.0 - self.t.treasury / reserve.max(1.0)),
        );
        let monthly = &self.t.ledger.monthly;
        let mut flows: Vec<(Flow, f32)> = Flow::ALL
            .iter()
            .map(|&fl| (fl, monthly[fl as usize]))
            .collect();
        flows.sort_by(|a, b| b.1.total_cmp(&a.1));
        let out_groups = [Group::Issued, Group::ExternalOut];
        let outs: Vec<String> = flows
            .iter()
            .filter(|(fl, v)| out_groups.contains(&fl.group()) && *v > 0.05)
            .take(3)
            .map(|(fl, v)| format!("{} {v:.1}", fl.name()))
            .collect();
        let ins: Vec<String> = flows
            .iter()
            .filter(|(fl, v)| !out_groups.contains(&fl.group()) && *v > 0.05)
            .take(3)
            .map(|(fl, v)| format!("{} {v:.1}", fl.name()))
            .collect();
        if !outs.is_empty() {
            f.because.push(Finding::new(
                format!(
                    "It pays out most for: {} (a day, last month)",
                    outs.join(", ")
                ),
                0.4,
            ));
        }
        f.because.push(Finding::new(
            if ins.is_empty() {
                "Hardly any money comes in".to_string()
            } else {
                format!("Its income: {} (a day, last month)", ins.join(", "))
            },
            0.4,
        ));
        if let Some(ruler) = self.t.ruler {
            f.because.push(Finding::new(
                format!(
                    "It pays tribute to {}",
                    towns
                        .0
                        .get(ruler as usize)
                        .map_or("its ruler", |r| r.name.as_str())
                ),
                0.6,
            ));
        }
        Some(f)
    }

    fn leaving(&self) -> Option<Finding> {
        let m = &self.t.moves;
        if m.left < 5 || m.left < 2 * m.arrived {
            return None;
        }
        Some(Finding::new(
            format!(
                "People are leaving: {} left and {} arrived since the last yearly report",
                m.left, m.arrived
            ),
            0.3 + (m.left as f32 / self.pop()).min(0.6),
        ))
    }
}

/// The median of the positive pay rates (what work typically pays here).
fn typical_pay(rates: &[f32]) -> f32 {
    let mut v: Vec<f32> = rates.iter().copied().filter(|&x| x > 0.0).collect();
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

/// Whether `f` or anything below it says `needle`.
fn contains(f: &Finding, needle: &str) -> bool {
    f.text.contains(needle) || f.because.iter().any(|c| contains(c, needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AssetPaths, Simulation};

    fn sim() -> Simulation {
        let paths = AssetPaths::discover();
        let (db, params, scenario) = crate::load(&paths).unwrap();
        Simulation::new(db, params, &scenario)
    }

    fn all_text(f: &[Finding]) -> String {
        f.iter()
            .map(|x| format!("{}\n{}", x.text, all_text(&x.because)))
            .collect()
    }

    #[test]
    fn a_town_without_a_bakery_and_without_bread_is_told_why() {
        let mut s = sim();
        for _ in 0..30 {
            s.step();
        }
        let db = s.world.resource::<Db>();
        let id = |name: &str| {
            db.content
                .products
                .iter()
                .position(|p| p.id == name)
                .unwrap() as u32
        };
        let (bread, bakery) = (id("bread"), id("bakery"));
        {
            let mut towns = s.world.resource_mut::<Towns>();
            let t = &mut towns.0[0];
            t.buildings.retain(|b| b.0 != bakery);
            // Empty every food store, so the town is hungry.
            for p in 0..t.stock.len() {
                t.stock[p] = 0.0;
            }
        }
        let found = explain(&s.world, 0);
        let text = all_text(&found);
        assert!(text.contains("Food is short"), "{text}");
        assert!(text.contains("Bread is short"), "{text}");
        assert!(text.contains("no Bakery"), "{text}");
        // The finding names the building type, so a view can point at it.
        fn has_building(f: &[Finding], id: &str) -> bool {
            f.iter()
                .any(|x| x.building.as_deref() == Some(id) || has_building(&x.because, id))
        }
        assert!(has_building(&found, "bakery"));
        let _ = bread;
    }

    #[test]
    fn explaining_does_not_change_the_simulation() {
        let (mut a, mut b) = (sim(), sim());
        for _ in 0..20 {
            a.step();
            for t in 0..a.world.resource::<Towns>().0.len() {
                let _ = explain(&a.world, t as u16);
            }
            b.step();
        }
        assert_eq!(a.state_hash(), b.state_hash());
    }
}
