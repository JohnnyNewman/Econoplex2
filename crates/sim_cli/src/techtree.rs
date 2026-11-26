//! `--tech-tree`: the content's technology tree as a Mermaid flowchart.
//!
//! Products are grouped by era; an arrow runs from each input to what a recipe
//! makes from it. Natural resources feed the raw materials they yield, and a dashed
//! arrow from a workplace you have to build first marks what is made there. Wood,
//! planks and charcoal feed so many recipes that their arrows are left out (the
//! chart says so).

use sim_core::Db;
use sim_data::Category;
use std::collections::BTreeSet;
use std::fmt::Write;

/// Inputs too common to draw.
const COMMON: [&str; 3] = ["wood", "plank", "charcoal"];

fn node_id(id: &str) -> String {
    format!("p_{id}")
}

pub fn mermaid(db: &Db) -> String {
    let c = &db.content;
    let mut out = String::from("flowchart LR\n");
    let n_eras = c.eras.len().max(1);
    // Products by era, as nodes. Buildings get a double frame, coin a circle.
    for e in 0..n_eras {
        let name = c.eras.get(e).map_or("", String::as_str);
        let _ = writeln!(out, "  subgraph era{e}[\"{e} {name}\"]");
        for (i, p) in c.products.iter().enumerate() {
            if p.era as usize != e.min(255) || p.opens.is_some() {
                continue;
            }
            let shape = if p.category == Category::Building {
                format!("[[\"{}\"]]", p.name)
            } else if db.coin[i] > 0.0 {
                format!("((\"{}\"))", p.name)
            } else if p.category == Category::Raw {
                format!("([\"{}\"])", p.name)
            } else {
                format!("[\"{}\"]", p.name)
            };
            let _ = writeln!(out, "    {}{shape}", node_id(&p.id));
        }
        let _ = writeln!(out, "  end");
    }
    // Natural resources.
    let _ = writeln!(out, "  subgraph land[\"Natural resources\"]");
    for n in &c.nature {
        let _ = writeln!(out, "    n_{}{{{{\"{}\"}}}}", n.id, n.name);
    }
    let _ = writeln!(out, "  end");

    let mut edges = BTreeSet::new();
    let mut dashed = BTreeSet::new();
    for m in &db.recipes {
        for &(o, _) in &m.outputs {
            let out_id = &c.products[o as usize].id;
            if c.products[o as usize].opens.is_some() {
                continue;
            }
            if let Some(k) = m.nature {
                edges.insert((format!("n_{}", c.nature[k as usize].id), node_id(out_id)));
            }
            for &(p, _) in &m.inputs {
                let in_id = &c.products[p as usize].id;
                if !COMMON.contains(&in_id.as_str()) {
                    edges.insert((node_id(in_id), node_id(out_id)));
                }
            }
            // Starter buildings host so much that their arrows would hide the rest.
            if let Some(b) = m.building.filter(|&b| !c.products[b as usize].starter) {
                dashed.insert((node_id(&c.products[b as usize].id), node_id(out_id)));
            }
        }
    }
    for (a, b) in &edges {
        let _ = writeln!(out, "  {a} --> {b}");
    }
    for (a, b) in &dashed {
        let _ = writeln!(out, "  {a} -.-> {b}");
    }
    let _ = writeln!(
        out,
        "  note[\"Wood, planks and charcoal feed many recipes, and the starter workshops, mills, bakeries, furnaces and smithies host most of them; those arrows are left out\"]"
    );
    for e in 0..n_eras {
        let fill = ["#e8f0d8", "#efe3d3", "#f3dcc4", "#dde3ea", "#e6dcef"][e % 5];
        let _ = writeln!(out, "  style era{e} fill:{fill},stroke:#888");
    }
    let _ = writeln!(out, "  style land fill:#e3efe6,stroke:#888");
    out
}

/// `--tech-graph`: the knowledge graph of recipe-skills as a Mermaid flowchart
/// (docs/design/inventions.md). Recipes are grouped by the domain of their skill; an
/// arrow runs from each recipe-skill to the ones that require it. A requirement with
/// alternatives goes through a small "or" node. Recipes that need nothing are the start
/// of the graph and drawn round.
pub fn knowledge_mermaid(db: &Db) -> String {
    let c = &db.content;
    let rid = |id: &str| format!("r_{id}");
    let mut out = String::from("flowchart LR\n");
    for (d, domain) in c.domains.iter().enumerate() {
        let recipes: Vec<_> = c
            .recipes
            .iter()
            .filter(|r| {
                c.skill(&r.skill)
                    .is_some_and(|s| c.skills[s as usize].domain == *domain)
            })
            .collect();
        if recipes.is_empty() {
            continue;
        }
        let _ = writeln!(out, "  subgraph d{d}[\"{domain}\"]");
        for r in recipes {
            let label = if r.level > 1 {
                format!("{} {}", r.name, r.level)
            } else {
                r.name.clone()
            };
            if r.requires.is_empty() {
                let _ = writeln!(out, "    {}([\"{label}\"])", rid(&r.id));
            } else {
                let _ = writeln!(out, "    {}[\"{label}\"]", rid(&r.id));
            }
        }
        let _ = writeln!(out, "  end");
    }
    for r in &c.recipes {
        for (k, req) in r.requires.iter().enumerate() {
            let opts = req.options();
            if opts.len() == 1 {
                let _ = writeln!(out, "  {} --> {}", rid(&opts[0]), rid(&r.id));
            } else {
                let or = format!("or_{}_{k}", r.id);
                let _ = writeln!(out, "  {or}((or))");
                for o in opts {
                    let _ = writeln!(out, "  {} -.-> {or}", rid(o));
                }
                let _ = writeln!(out, "  {or} --> {}", rid(&r.id));
            }
        }
    }
    for d in 0..c.domains.len() {
        let fill = ["#e8f0d8", "#efe3d3", "#f3dcc4", "#dde3ea", "#e6dcef"][d % 5];
        let _ = writeln!(out, "  style d{d} fill:{fill},stroke:#888");
    }
    out
}
