//! Precomputed lookup tables derived from content: skill vectors, recipe metadata,
//! domain directions. Built once at startup (or on content reload).

use bevy_ecs::prelude::Resource;
use sim_data::dims::*;
use sim_data::embed::{self, EmbedMethod, Embedding};
use sim_data::{Category, Content};

#[derive(Debug, Clone)]
pub struct RecipeMeta {
    pub name: String,
    pub skill_idx: u32,
    /// Latent skill vector (unit length).
    pub skill: CapVec,
    pub difficulty: f32,
    /// `RecipeDef::quality_bonus`: quality as if the team were this much more skilled.
    pub quality_bonus: f32,
    /// `RecipeDef::level`.
    pub level: u32,
    pub domain: u32,
    /// Physical demand weights per physique dimension.
    pub phys: PhysVec,
    pub phys_total: f32,
    pub inputs: Vec<(u32, f32)>,
    pub outputs: Vec<(u32, f32)>,
    pub nature: Option<u32>,
    pub building: Option<u32>,
    pub duration: f32,
    pub max_team: usize,
    /// Hunger one run's outputs can eventually remove, counting outputs that
    /// feed into food recipes (grain and flour lead to bread).
    pub food_out: f32,
    /// Construction recipes: the building this recipe puts up.
    pub builds: Option<u32>,
}

#[derive(Resource)]
pub struct Db {
    pub content: Content,
    /// Per recipe, the recipe-skills it requires: every entry must be met, by any
    /// one of its recipes (`RecipeDef::requires`, by index).
    pub requires: Vec<Vec<Vec<u32>>>,
    /// Per recipe, the other recipes whose skill points the same way (cosine
    /// of the skill vectors above zero), closest first: what working at it can
    /// lead a person to think of (`knowledge::invent`).
    pub related: Vec<Vec<(u32, f32)>>,
    /// Per recipe, its era: the latest era among what it makes or builds and
    /// the eras of what it requires (the earliest way to meet each requirement).
    pub recipe_era: Vec<u8>,
    /// Groups of two or more recipes that make exactly the same goods (several
    /// ways of making them), by recipe index.
    pub same_outputs: Vec<Vec<u32>>,
    pub embedding: Embedding,
    /// Length of every capability vector in this game.
    pub cap_dim: usize,
    pub skill_vecs: Vec<CapVec>,
    pub recipes: Vec<RecipeMeta>,
    /// Unit direction per domain: normalized sum of the domain's skill vectors.
    pub domain_dirs: Vec<CapVec>,
    pub domain_skills: Vec<Vec<u32>>,
    /// (product, hunger removed) for all foods, best first.
    pub foods: Vec<(u32, f32)>,
    pub base_price: Vec<f32>,
    pub is_food: Vec<bool>,
    /// Products some recipe takes as an input.
    pub is_input: Vec<bool>,
    /// The firms recipes belong to (see `firms`), and each recipe's firm index.
    pub firm_keys: Vec<crate::firms::FirmKey>,
    pub recipe_firm: Vec<usize>,
    /// Land products: the nature kind each one opens.
    pub opens: Vec<Option<u32>>,
    /// Houses: (people each holds, how well it shelters them).
    pub house: Vec<Option<(u32, f32)>>,
    /// Face value of coin products (0 for everything else).
    pub coin: Vec<f32>,
    /// Per recipe: work done outdoors (harvesting nature, putting up a
    /// building, opening land), which needs daylight.
    pub outdoor: Vec<bool>,
}

impl Db {
    /// Build lookup tables and the skill embedding. `dims` is the capability
    /// dimension; `None` sizes it from the content ([`cap_dim_for`]).
    pub fn build(content: Content, decay: f32, method: &str, dims: Option<usize>) -> Db {
        let method = match method {
            "Closure" => EmbedMethod::Closure,
            "Nmf" => EmbedMethod::Nmf,
            _ => EmbedMethod::Auto,
        };
        let cap_dim = dims
            .unwrap_or_else(|| cap_dim_for(content.skills.len()))
            .max(1);
        let embedding = embed::embed(&content, cap_dim, decay, method);
        let skill_vecs: Vec<CapVec> = embedding
            .vectors
            .iter()
            .map(|v| {
                let mut a = vec![0.0; cap_dim];
                let n = v.len().min(cap_dim);
                a[..n].copy_from_slice(&v[..n]);
                a
            })
            .collect();

        let nd = content.domains.len();
        let mut domain_dirs = vec![vec![0.0; cap_dim]; nd];
        let mut domain_skills = vec![Vec::new(); nd];
        for (i, s) in content.skills.iter().enumerate() {
            let d = content.domain(&s.domain).unwrap() as usize;
            domain_skills[d].push(i as u32);
            for (x, s) in domain_dirs[d].iter_mut().zip(&skill_vecs[i]) {
                *x += s;
            }
        }
        for d in &mut domain_dirs {
            let n = norm(d).max(1e-9);
            d.iter_mut().for_each(|x| *x /= n);
        }

        let food_potential = food_potential(&content);
        let recipes: Vec<RecipeMeta> = content
            .recipes
            .iter()
            .map(|r| {
                let si = content.skill(&r.skill).unwrap();
                let mut phys = [0.0; PHYS_DIM];
                for (name, w) in &r.physique {
                    phys[phys_index(name).unwrap()] += w;
                }
                let outputs: Vec<(u32, f32)> = r
                    .outputs
                    .iter()
                    .map(|(p, q)| (content.product(p).unwrap(), *q as f32))
                    .collect();
                let food_out = outputs
                    .iter()
                    .map(|(p, q)| food_potential[*p as usize] * q)
                    .sum();
                RecipeMeta {
                    name: r.name.clone(),
                    skill_idx: si,
                    skill: skill_vecs[si as usize].clone(),
                    difficulty: r.difficulty,
                    quality_bonus: r.quality_bonus,
                    level: r.level,
                    domain: content.domain(&content.skills[si as usize].domain).unwrap(),
                    phys,
                    phys_total: phys.iter().sum(),
                    inputs: r
                        .inputs
                        .iter()
                        .map(|(p, q)| (content.product(p).unwrap(), *q as f32))
                        .collect(),
                    outputs: outputs.clone(),
                    nature: r.nature.as_ref().map(|n| content.nature_kind(n).unwrap()),
                    building: r.building.as_ref().map(|b| content.product(b).unwrap()),
                    duration: r.duration,
                    max_team: r.max_team.max(1) as usize,
                    food_out,
                    builds: outputs
                        .iter()
                        .map(|o| o.0)
                        .find(|&p| content.products[p as usize].category == Category::Building),
                }
            })
            .collect();

        let mut foods: Vec<(u32, f32)> = content
            .products
            .iter()
            .enumerate()
            .filter(|(_, p)| p.food > 0.0)
            .map(|(i, p)| (i as u32, p.food))
            .collect();
        foods.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let base_price = content.products.iter().map(|p| p.base_price).collect();
        let is_food = content.products.iter().map(|p| p.food > 0.0).collect();
        let opens = content
            .products
            .iter()
            .map(|p| p.opens.as_ref().and_then(|n| content.nature_kind(n)))
            .collect();

        let coin = content.products.iter().map(|p| p.coin).collect();
        let house = content
            .products
            .iter()
            .map(|p| p.house.map(|h| (h.residents, h.quality)))
            .collect();
        let mut firm_keys: Vec<crate::firms::FirmKey> =
            recipes.iter().map(crate::firms::key_of).collect();
        firm_keys.sort_unstable();
        firm_keys.dedup();
        let recipe_firm: Vec<usize> = recipes
            .iter()
            .map(|m| firm_keys.binary_search(&crate::firms::key_of(m)).unwrap())
            .collect();
        let outdoor = recipes
            .iter()
            .map(|m| {
                m.nature.is_some()
                    || m.outputs.iter().any(|&(p, _)| {
                        let def = &content.products[p as usize];
                        def.category == Category::Building || def.opens.is_some()
                    })
            })
            .collect();
        let mut is_input: Vec<bool> = vec![false; content.products.len()];
        for m in &recipes {
            for &(p, _) in &m.inputs {
                is_input[p as usize] = true;
            }
        }
        // The tech graph by index: per recipe, its `requires` entries, each a list
        // of recipes of which any one is enough (docs/design/inventions.md).
        let requires: Vec<Vec<Vec<u32>>> = content
            .recipes
            .iter()
            .map(|r| {
                r.requires
                    .iter()
                    .map(|req| {
                        req.options()
                            .iter()
                            .filter_map(|id| content.recipe(id))
                            .collect()
                    })
                    .collect()
            })
            .collect();
        let related = (0..recipes.len())
            .map(|a| {
                let mut near: Vec<(u32, f32)> = (0..recipes.len())
                    .filter(|&b| b != a)
                    .map(|b| (b as u32, dot(&recipes[a].skill, &recipes[b].skill)))
                    .filter(|&(_, c)| c > 0.0)
                    .collect();
                near.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
                near
            })
            .collect();
        let recipe_era = {
            let own: Vec<u8> = recipes
                .iter()
                .map(|m| {
                    m.outputs
                        .iter()
                        .map(|&(p, _)| p)
                        .chain(m.builds)
                        .map(|p| content.products[p as usize].era)
                        .max()
                        .unwrap_or(0)
                })
                .collect();
            // The requires links have no loops (validation), so this settles
            // within as many rounds as there are recipes.
            let mut era = own.clone();
            for _ in 0..recipes.len() {
                let next: Vec<u8> = (0..recipes.len())
                    .map(|r| {
                        requires[r]
                            .iter()
                            .map(|req| req.iter().map(|&o| era[o as usize]).min().unwrap_or(0))
                            .fold(own[r], u8::max)
                    })
                    .collect();
                if next == era {
                    break;
                }
                era = next;
            }
            era
        };
        let same_outputs: Vec<Vec<u32>> = {
            let mut by: std::collections::BTreeMap<Vec<(u32, u32)>, Vec<u32>> = Default::default();
            for (r, m) in recipes.iter().enumerate() {
                if m.outputs.is_empty() {
                    continue;
                }
                let mut key: Vec<(u32, u32)> =
                    m.outputs.iter().map(|&(p, q)| (p, q.to_bits())).collect();
                key.sort();
                by.entry(key).or_default().push(r as u32);
            }
            by.into_values().filter(|g| g.len() > 1).collect()
        };
        Db {
            cap_dim,
            related,
            same_outputs,
            recipe_era,
            coin,
            requires,
            house,
            content,
            embedding,
            skill_vecs,
            recipes,
            domain_dirs,
            domain_skills,
            foods,
            base_price,
            is_food,
            is_input,
            firm_keys,
            recipe_firm,
            opens,
            outdoor,
        }
    }

    /// Regroup recipes into firms (see `firms::Grouping`).
    pub fn group_firms(&mut self, g: crate::firms::Grouping) {
        let mut keys: Vec<crate::firms::FirmKey> = self
            .recipes
            .iter()
            .map(|m| crate::firms::key_grouped(m, g))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        self.recipe_firm = self
            .recipes
            .iter()
            .map(|m| {
                keys.binary_search(&crate::firms::key_grouped(m, g))
                    .unwrap()
            })
            .collect();
        self.firm_keys = keys;
    }

    pub fn category(&self, product: u32) -> Category {
        self.content.products[product as usize].category
    }

    /// Proficiency of a capability vector in each skill, clamped at zero.
    pub fn skill_profile(&self, cap: &CapVec) -> Vec<f32> {
        self.skill_vecs
            .iter()
            .map(|s| dot(cap, s).max(0.0))
            .collect()
    }

    /// Best skill proficiency within a domain.
    pub fn domain_proficiency(&self, cap: &CapVec, domain: usize) -> f32 {
        self.domain_skills[domain]
            .iter()
            .map(|&s| dot(cap, &self.skill_vecs[s as usize]))
            .fold(0.0, f32::max)
    }
}

/// Hunger each product can eventually remove: its own food value, or the food its
/// best downstream recipe makes per unit of input (shared evenly across inputs).
pub fn food_potential(content: &Content) -> Vec<f32> {
    let mut pot: Vec<f32> = content.products.iter().map(|p| p.food).collect();
    // Production chains are short DAGs; a few relaxation passes reach the fixed point.
    for _ in 0..content.recipes.len().min(16) {
        let mut changed = false;
        for r in &content.recipes {
            let out: f32 = r
                .outputs
                .iter()
                .map(|(p, q)| pot[content.product(p).unwrap() as usize] * *q as f32)
                .sum();
            let inputs: f32 = r.inputs.iter().map(|(_, q)| *q as f32).sum();
            if inputs <= 0.0 {
                continue;
            }
            for (p, _) in &r.inputs {
                let i = content.product(p).unwrap() as usize;
                if out / inputs > pot[i] + 1e-6 {
                    pot[i] = out / inputs;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    pot
}
