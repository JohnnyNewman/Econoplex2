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
