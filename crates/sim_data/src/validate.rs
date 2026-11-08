//! Content validation: broken references, cycles, unreachable products.

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

pub fn validate(c: &Content) -> Report {
    let mut r = Report::default();

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
