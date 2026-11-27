//! Content validation: broken references, cycles, unreachable products, recipe-skills
//! that cannot be learned.

use crate::{dims, Category, Content};

#[derive(Debug, Default)]
pub struct Report {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Which products (including buildings) can be made from natural resources: the
/// fixpoint of every recipe whose inputs can be made and whose building can be built.
/// `nature` limits the natural resources available (all of them when `None`).
pub fn reachable_with(c: &Content, nature: Option<&[bool]>) -> Vec<bool> {
    let mut have: Vec<bool> = c.products.iter().map(|p| p.starter).collect();
    loop {
        let mut changed = false;
        for rec in &c.recipes {
            let inputs = rec
                .inputs
                .iter()
                .all(|(p, _)| c.product(p).is_some_and(|i| have[i as usize]));
            let building = rec
                .building
                .as_ref()
                .is_none_or(|b| c.product(b).is_some_and(|i| have[i as usize]));
            let land = match (&rec.nature, nature) {
                (Some(n), Some(avail)) => c.nature_kind(n).is_some_and(|k| avail[k as usize]),
                _ => true,
            };
            if inputs && building && land {
                for (p, _) in &rec.outputs {
                    if let Some(i) = c.product(p) {
                        if !have[i as usize] {
                            have[i as usize] = true;
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed {
            return have;
        }
    }
}

pub fn reachable(c: &Content) -> Vec<bool> {
    reachable_with(c, None)
}

/// Which recipe-skills can be learned at all: the fixpoint from the recipes with no
/// `requires`, adding every recipe whose requirements are each met by a learnable one.
/// A loop in `requires` leaves its members unlearnable. Unknown ids never count as met.
pub fn learnable(c: &Content) -> Vec<bool> {
    let mut known = vec![false; c.recipes.len()];
    loop {
        let mut changed = false;
        for (i, rec) in c.recipes.iter().enumerate() {
            if known[i] {
                continue;
            }
            let met = rec.requires.iter().all(|req| {
                req.options()
                    .iter()
                    .any(|id| c.recipe(id).is_some_and(|j| known[j as usize]))
            });
            if met {
                known[i] = true;
                changed = true;
            }
        }
        if !changed {
            return known;
        }
    }
}

pub fn validate(c: &Content) -> Report {
    let mut r = Report::default();

    for n in &c.nutrition {
        match c.products.iter().find(|p| p.id == n.food) {
            Some(p) if p.food > 0.0 => {}
            _ => r
                .errors
                .push(format!("nutrition for `{}`, which is not a food", n.food)),
        }
    }

    for k in &c.keeping {
        if c.products.iter().all(|p| p.id != k.product) {
            r.errors
                .push(format!("keeping for unknown product `{}`", k.product));
        }
        if k.days <= 0.0 {
            r.errors
                .push(format!("keeping for `{}`: days must be above 0", k.product));
        }
    }

    for p in &c.products {
        if let Some(h) = &p.house {
            if p.category != Category::Building {
                r.errors
                    .push(format!("house `{}` must be a Building", p.id));
            }
            if h.residents == 0 || !(0.0..=1.0).contains(&h.quality) {
                r.errors.push(format!(
                    "house `{}` needs residents above 0 and quality in 0..1",
                    p.id
                ));
            }
        }
    }

    for s in &c.skills {
        for p in &s.prereqs {
            if c.skill(p).is_none() {
                r.errors
                    .push(format!("skill `{}` has unknown prerequisite `{p}`", s.id));
            }
        }
    }
    if let Some(cycle) = find_skill_cycle(c) {
        r.errors.push(format!(
            "skill prerequisites form a cycle: {}",
            cycle.join(" -> ")
        ));
    }

    for p in &c.products {
        if let Some(n) = &p.opens {
            if c.nature_kind(n).is_none() {
                r.errors
                    .push(format!("product `{}` opens unknown nature `{n}`", p.id));
            }
            if p.category != Category::Building {
                r.errors.push(format!(
                    "product `{}` opens land but is not a building",
                    p.id
                ));
            }
        }
        if let Some(f) = &p.fight {
            let resist = f.armour.shares();
            let shares = [f.close_penalty, resist[0], resist[1], resist[2], f.shield];
            let amounts = [
                f.reach,
                f.damage,
                f.strike_rate,
                f.range,
                f.shot_rate,
                f.shot_damage,
            ];
            if shares.iter().any(|&x| !(0.0..1.0).contains(&x))
                || amounts.iter().any(|&x| !(x >= 0.0 && x.is_finite()))
            {
                r.errors.push(format!(
                    "product `{}` has fight values out of range (shares 0..1, amounts at least 0)",
                    p.id
                ));
            }
            if f.damage > 0.0 && f.damage_type.is_none() {
                r.errors.push(format!(
                    "product `{}` has melee damage but no damage_type (Pierce, Slash or Blunt)",
                    p.id
                ));
            }
            if f.range > 0.0 && (f.shot_rate <= 0.0 || f.arrows == 0) {
                r.errors.push(format!(
                    "product `{}` shoots but has no shot rate or arrows",
                    p.id
                ));
            }
        }
        if let Some(e) = &p.equip {
            if c.domain(&e.domain).is_none() {
                r.errors.push(format!(
                    "product `{}` equips unknown domain `{}`",
                    p.id, e.domain
                ));
            }
        }
    }

    for rec in &c.recipes {
        if c.skill(&rec.skill).is_none() {
            r.errors.push(format!(
                "recipe `{}` uses unknown skill `{}`",
                rec.id, rec.skill
            ));
        }
        for (p, _) in rec.inputs.iter().chain(rec.outputs.iter()) {
            if c.product(p).is_none() {
                r.errors.push(format!(
                    "recipe `{}` references unknown product `{p}`",
                    rec.id
                ));
            }
        }
        if let Some(n) = &rec.nature {
            if c.nature_kind(n).is_none() {
                r.errors
                    .push(format!("recipe `{}` needs unknown nature `{n}`", rec.id));
            }
        }
        if let Some(b) = &rec.building {
            match c.product(b) {
                None => r
                    .errors
                    .push(format!("recipe `{}` needs unknown building `{b}`", rec.id)),
                Some(i) if c.products[i as usize].category != Category::Building => r
                    .errors
                    .push(format!("recipe `{}`: `{b}` is not a building", rec.id)),
                _ => {}
            }
        }
        for (name, _) in &rec.physique {
            if dims::phys_index(name).is_none() {
                r.errors.push(format!(
                    "recipe `{}` uses unknown physique `{name}`",
                    rec.id
                ));
            }
        }
        let builds = rec.outputs.iter().filter(|(p, _)| {
            c.product(p)
                .is_some_and(|i| c.products[i as usize].category == Category::Building)
        });
        if builds.count() > 0 && (rec.outputs.len() != 1 || rec.outputs[0].1 != 1) {
            r.errors.push(format!(
                "recipe `{}`: a construction recipe must output exactly one building",
                rec.id
            ));
        }
        if rec.outputs.is_empty() {
            r.errors.push(format!("recipe `{}` has no outputs", rec.id));
        }
        if rec.level == 0 {
            r.errors.push(format!(
                "recipe `{}` has level 0 (levels start at 1)",
                rec.id
            ));
        }
        for req in &rec.requires {
            if req.options().is_empty() {
                r.errors
                    .push(format!("recipe `{}` has an empty `requires` entry", rec.id));
            }
            for id in req.options() {
                if c.recipe(id).is_none() {
                    r.errors.push(format!(
                        "recipe `{}` requires unknown recipe `{id}`",
                        rec.id
                    ));
                } else if *id == rec.id {
                    r.errors
                        .push(format!("recipe `{}` requires itself", rec.id));
                }
            }
        }
    }
    if r.errors.is_empty() {
        let known = learnable(c);
        for (i, rec) in c.recipes.iter().enumerate() {
            if !known[i] {
                r.errors.push(format!(
                    "recipe `{}` cannot be learned: its `requires` never get met from the \
                     recipes that need nothing (a loop?)",
                    rec.id
                ));
            }
        }
    }

    for p in &c.products {
        if !c.eras.is_empty() && p.era as usize >= c.eras.len() {
            r.errors.push(format!(
                "product `{}` is in era {}, but only {} eras are named",
                p.id,
                p.era,
                c.eras.len()
            ));
        }
    }

    // Reachability: which products and buildings can be made starting from natural
    // resources only? A recipe counts once its inputs can be made and its building
    // can be built.
    if r.errors.is_empty() {
        let have = reachable(c);
        for (i, p) in c.products.iter().enumerate() {
            if !have[i] {
                r.errors.push(format!(
                    "{} `{}` cannot be produced from natural resources",
                    if p.category == Category::Building {
                        "building"
                    } else {
                        "product"
                    },
                    p.id
                ));
            }
        }
    }
    for (i, s) in c.skills.iter().enumerate() {
        let used = c.recipes.iter().any(|r| r.skill == s.id)
            || c.skills.iter().any(|o| o.prereqs.contains(&s.id));
        if !used {
            r.warnings.push(format!(
                "skill `{}` is not used by any recipe or skill (#{i})",
                s.id
            ));
        }
    }
    r
}

fn find_skill_cycle(c: &Content) -> Option<Vec<String>> {
    // 0 = unvisited, 1 = on stack, 2 = done
    let n = c.skills.len();
    let mut mark = vec![0u8; n];
    let mut stack = Vec::new();
    fn dfs(c: &Content, i: usize, mark: &mut [u8], stack: &mut Vec<usize>) -> Option<Vec<String>> {
        mark[i] = 1;
        stack.push(i);
        for p in c.prereqs(i) {
            if mark[p] == 1 {
                let start = stack.iter().position(|&x| x == p).unwrap();
                let mut cyc: Vec<String> = stack[start..]
                    .iter()
                    .map(|&x| c.skills[x].id.clone())
                    .collect();
                cyc.push(c.skills[p].id.clone());
                return Some(cyc);
            }
            if mark[p] == 0 {
                if let Some(cyc) = dfs(c, p, mark, stack) {
                    return Some(cyc);
                }
            }
        }
        stack.pop();
        mark[i] = 2;
        None
    }
    for i in 0..n {
        if mark[i] == 0 {
            if let Some(cyc) = dfs(c, i, &mut mark, &mut stack) {
                return Some(cyc);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::{Content, ContentFile, Requirement};

    fn content(recipes: &str) -> Result<Content, crate::LoadError> {
        let text = format!(
            r#"(
                skills: [(id: "craft", name: "Craft", domain: "craft")],
                products: [(id: "thing", name: "Thing", category: Material, base_price: 1.0)],
                recipes: [{recipes}],
            )"#
        );
        let file: ContentFile = ron::from_str(&text).expect("test content parses");
        Content::from_files(vec![file])
    }

    fn recipe(id: &str, requires: &str) -> String {
        format!(
            r#"(id: "{id}", name: "{id}", skill: "craft", difficulty: 0.1, outputs: [("thing", 1)],
                duration: 1.0, requires: [{requires}]),"#
        )
    }

    #[test]
    fn requires_parses_single_ids_and_alternatives() {
        let c = content(
            &[
                recipe("a", ""),
                recipe("b", ""),
                recipe("c", r#""a", ["a", "b"]"#),
            ]
            .concat(),
        )
        .expect("valid");
        let r = &c.recipes[c.recipe("c").unwrap() as usize];
        assert_eq!(r.level, 1);
        assert_eq!(
            r.requires,
            vec![
                Requirement::One("a".into()),
                Requirement::AnyOf(vec!["a".into(), "b".into()])
            ]
        );
    }

    #[test]
    fn an_alternative_route_is_enough() {
        // `c` needs `a` or `d`; `d` is in a loop with `e`, but `a` can be learned.
        let c = content(
            &[
                recipe("a", ""),
                recipe("c", r#"["a", "d"]"#),
                recipe("d", r#""e""#),
                recipe("e", r#""d""#),
            ]
            .concat(),
        );
        let err = c.expect_err("the loop is invalid").to_string();
        assert!(err.contains("`d` cannot be learned"), "{err}");
        assert!(err.contains("`e` cannot be learned"), "{err}");
        assert!(!err.contains("`c` cannot be learned"), "{err}");
    }

    #[test]
    fn unknown_and_self_requirements_are_errors() {
        let err = content(&[recipe("a", r#""nope""#), recipe("b", r#""b""#)].concat())
            .expect_err("invalid")
            .to_string();
        assert!(err.contains("requires unknown recipe `nope`"), "{err}");
        assert!(err.contains("`b` requires itself"), "{err}");
    }

    #[test]
    fn an_override_without_requires_keeps_the_earlier_ones() {
        let file = |recipes: String| -> ContentFile {
            ron::from_str(&format!(
                r#"(
                    skills: [(id: "craft", name: "Craft", domain: "craft")],
                    products: [(id: "thing", name: "Thing", category: Material, base_price: 1.0)],
                    recipes: [{recipes}],
                )"#
            ))
            .expect("test content parses")
        };
        let c = Content::from_files(vec![
            file([recipe("a", ""), recipe("b", r#""a""#)].concat()),
            file(recipe("b", "")),
        ])
        .expect("valid");
        let b = &c.recipes[c.recipe("b").unwrap() as usize];
        assert_eq!(b.requires, vec![Requirement::One("a".into())]);
    }

    #[test]
    fn shipped_content_has_a_learnable_tech_graph() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/content");
        let c = Content::load_dir(&dir).expect("shipped content is valid");
        assert!(super::learnable(&c).iter().all(|&k| k));
        assert!(c.recipes.iter().any(|r| r.requires.is_empty()));
        assert!(c.recipes.iter().any(|r| !r.requires.is_empty()));
    }
}
