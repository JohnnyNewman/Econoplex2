//! A world generated from the seed: towns, their land, rivers and hills.
//!
//! `generate` in scenario.ron (or `--generate N` in the CLI) replaces the
//! scenario's fixed towns and terrain with a generated world:
//! - **towns** are placed at random within the map, at least `min_distance`
//!   apart, with made-up names and `population` people each;
//! - **land:** each kind of nature site is spread by smooth regional noise
//!   (fertile plains, wooded valleys, mining hills), so neighbouring towns tend to
//!   share land and distant ones differ. Every town gets enough fields and forest
//!   to feed and build, and every kind of site exists somewhere;
//! - **buildings:** each town starts with the starter buildings (workshop, mill,
//!   bakery) and those its land calls for (a furnace where there is ore, a smithy
//!   where there is iron);
//! - **rivers** run across the map, each with a couple of fords, and hills come
//!   from the terrain's own noise.
//!
//! The same seed always gives the same world.

use crate::config::{RiverSpec, Scenario, TerrainSpec, TownSpec};
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

/// `generate` in scenario.ron.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GenSpec {
    pub towns: u32,
    /// Starting people per town, at least and at most.
    pub population: (u32, u32),
    /// Half the map's width and height.
    pub half_size: (f32, f32),
    pub min_distance: f32,
    pub rivers: u32,
    /// Typical number of sites of a kind a town gets where its land suits it.
    pub sites: f32,
}

impl Default for GenSpec {
    fn default() -> Self {
        GenSpec {
            towns: 5,
            population: (240, 340),
            half_size: (1000.0, 750.0),
            min_distance: 450.0,
            rivers: 1,
            sites: 3.0,
        }
    }
}

const PREFIX: [&str; 16] = [
    "Ash", "Bram", "Cold", "Dun", "Elm", "Fen", "Gil", "Har", "Iver", "Kel", "Lark", "Mor", "Ned",
    "Ost", "Rook", "Wyn",
];
const SUFFIX: [&str; 12] = [
    "ford", "wick", "stead", "holm", "mere", "by", "ton", "dale", "moor", "ridge", "brook", "field",
];

/// Cheap deterministic value noise in [0, 1] (same as the terrain's).
fn noise(seed: u64, x: f32, y: f32) -> f32 {
    let lattice = |i: i64, j: i64| {
        let mut h = seed
            ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        h ^= h >> 31;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 29;
        (h >> 40) as f32 / (1u64 << 24) as f32
    };
    let (i, j) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy) = (s(fx), s(fy));
    let top = lattice(i, j) * (1.0 - sx) + lattice(i + 1, j) * sx;
    let bottom = lattice(i, j + 1) * (1.0 - sx) + lattice(i + 1, j + 1) * sx;
    top * (1.0 - sy) + bottom * sy
}

/// Replace `scenario`'s towns and terrain with a world generated from its seed.
/// `nature` lists the content's nature kinds (id, plus how common they are: 1
/// for fields and forests, less for rare veins); `ore` names the kinds that call
/// for a furnace, `iron` the one that calls for a smithy.
pub fn generate(scenario: &mut Scenario, spec: &GenSpec, nature: &[(String, f32)]) {
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(scenario.seed ^ 0x57_0E1D);
    let (hx, hy) = spec.half_size;
    // --- towns: random places, at least min_distance apart (relaxed if crowded)
    let mut places: Vec<(f32, f32)> = Vec::new();
    let mut min_d = spec.min_distance;
    while places.len() < spec.towns as usize {
        let mut placed = false;
        for _ in 0..200 {
            let p = (
                rng.random_range(-0.8 * hx..0.8 * hx),
                rng.random_range(-0.8 * hy..0.8 * hy),
            );
            if places
                .iter()
                .all(|q| ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt() >= min_d)
            {
                places.push(p);
                placed = true;
                break;
            }
        }
        if !placed {
            min_d *= 0.9;
        }
    }
    // --- names, unique
    let mut names: Vec<String> = Vec::new();
    while names.len() < places.len() {
        let n = format!(
            "{}{}",
            PREFIX[rng.random_range(0..PREFIX.len())],
            SUFFIX[rng.random_range(0..SUFFIX.len())]
        );
        if !names.contains(&n) {
            names.push(n);
        }
    }
    // --- land: regional noise per kind of site
    let scale = 0.6 * hx.max(hy);
    let mut towns: Vec<TownSpec> = places
        .iter()
        .zip(&names)
        .map(|(&p, name)| {
            let nat: Vec<(String, u32)> = nature
                .iter()
                .enumerate()
                .filter_map(|(k, (id, common))| {
                    let v = noise(
                        scenario.seed.wrapping_add(k as u64 * 7919),
                        p.0 / scale,
                        p.1 / scale,
                    );
                    // Common land almost everywhere, rare veins only where the noise peaks.
                    let level =
                        (v - (1.0 - common.clamp(0.05, 1.0))).max(0.0) / common.clamp(0.05, 1.0);
                    let n = (spec.sites * 2.0 * level).round() as u32;
                    (n > 0).then(|| (id.clone(), n))
                })
                .collect();
            TownSpec {
                name: name.clone(),
                position: p,
                population: rng
                    .random_range(spec.population.0..=spec.population.1.max(spec.population.0)),
                nature: nat,
                buildings: Vec::new(),
                stock: Vec::new(),
                governor: None,
                culture: None,
            }
        })
        .collect();
    // Every town can feed and build: at least 3 fields and 2 forests.
    for t in towns.iter_mut() {
        for (kind, min) in [("field", 3), ("forest", 2)] {
            if !nature.iter().any(|n| n.0 == kind) {
                continue;
            }
            match t.nature.iter_mut().find(|n| n.0 == kind) {
                Some(n) => n.1 = n.1.max(min),
                None => t.nature.push((kind.to_string(), min)),
            }
        }
    }
    // Every kind of site exists somewhere: give a missing one to a random town.
    for (id, _) in nature {
        if !towns.iter().any(|t| t.nature.iter().any(|n| &n.0 == id)) {
            let k = rng.random_range(0..towns.len());
            towns[k].nature.push((id.clone(), 1));
        }
    }
    // Buildings and stock.
    for t in towns.iter_mut() {
        let has = |kind: &str| t.nature.iter().any(|n| n.0 == kind && n.1 > 0);
        let mut b = vec!["workshop".to_string(), "mill".into(), "bakery".into()];
        if has("ore_vein") || has("copper_vein") || has("tin_vein") {
            b.push("furnace".into());
        }
        if has("ore_vein") {
            b.push("smithy".into());
        }
        t.buildings = b;
        t.stock = vec![
            ("bread".into(), 3.0 * t.population as f32),
            ("wood".into(), 30.0),
        ];
    }
    // --- rivers: across the map, north to south or west to east, with two fords
    let mut rivers = Vec::new();
    for _ in 0..spec.rivers {
        let vertical = rng.random_bool(0.5);
        let mut pts = Vec::new();
        let base = if vertical {
            rng.random_range(-0.5 * hx..0.5 * hx)
        } else {
            rng.random_range(-0.5 * hy..0.5 * hy)
        };
        for k in 0..=5 {
            let f = -1.15 + 2.3 * k as f32 / 5.0;
            let wobble = rng.random_range(-80.0..80.0f32);
            pts.push(if vertical {
                (base + wobble, f * hy)
            } else {
                (f * hx, base + wobble)
            });
        }
        let fords = (0..2)
            .map(|_| {
                let k = rng.random_range(1..pts.len() - 1);
                pts[k]
            })
            .collect();
        rivers.push(RiverSpec {
            points: pts,
            width: 50.0,
            fords,
            ford_width: 70.0,
            ford_cost: 2.5,
        });
    }
    // The player's town keeps its place if named; else the first town.
    if scenario.player.is_some() {
        scenario.player = towns.first().map(|t| t.name.clone());
    }
    scenario.towns = towns;
    scenario.terrain = Some(TerrainSpec {
        cell: 20.0,
        half_size: spec.half_size,
        hill_cost: 1.5,
        hill_scale: 260.0,
        rivers,
    });
}

/// How common each kind of nature site is, for the generator: fields and forests
/// everywhere, pasture, flax and clay often, stone, ore and coal in places,
/// copper and tin rarely, silver very rarely.
pub fn nature_kinds(db: &crate::db::Db) -> Vec<(String, f32)> {
    db.content
        .nature
        .iter()
        .map(|n| {
            let common = match n.id.as_str() {
                "field" | "forest" => 1.0,
                "pasture" | "flax_field" | "clay_pit" => 0.6,
                "quarry" | "ore_vein" | "coal_seam" => 0.45,
                "copper_vein" | "tin_vein" => 0.35,
                "silver_vein" => 0.2,
                _ => 0.4,
            };
            (n.id.clone(), common)
        })
        .collect()
}

/// The scenario to play: a generated world if it asks for one (then it no
/// longer does, so a saved game keeps its world), else the scenario itself.
pub fn resolve(scenario: &Scenario, db: &crate::db::Db) -> Scenario {
    let mut s = scenario.clone();
    if let Some(spec) = s.generate.take() {
        generate(&mut s, &spec, &nature_kinds(db));
    }
    s
}

/// The map as an SVG picture: hills shaded, rivers, fords, towns with their land.
pub fn map_svg(scenario: &Scenario, terrain: &crate::terrain::Terrain) -> String {
    let (w, h) = (
        terrain.width as f32 * terrain.cell,
        terrain.height as f32 * terrain.cell,
    );
    let (x0, y0) = terrain.origin;
    let mut s = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{x0} {} {w} {h}\" width=\"{}\" height=\"{}\">\
",
        -(y0 + h),
        (w / 2.5) as u32,
        (h / 2.5) as u32
    );
    s.push_str(&format!(
        "<rect x=\"{x0}\" y=\"{}\" width=\"{w}\" height=\"{h}\" fill=\"#e8e4d0\"/>\
",
        -(y0 + h)
    ));
    // y grows up in the world, down in SVG: draw at -y.
    for c in 0..terrain.width * terrain.height {
        let (cx, cy) = terrain.center(c);
        let (x, y) = (cx - terrain.cell / 2.0, -(cy + terrain.cell / 2.0));
        if terrain.water[c] {
            s.push_str(&format!(
                "<rect x=\"{x}\" y=\"{y}\" width=\"{0}\" height=\"{0}\" fill=\"#4a7fb5\"/>\
",
                terrain.cell
            ));
        } else if terrain.cost[c].is_finite() && terrain.cost[c] > 1.05 {
            let a = ((terrain.cost[c] - 1.0) / 1.5).clamp(0.0, 1.0) * 0.55;
            s.push_str(&format!(
                "<rect x=\"{x}\" y=\"{y}\" width=\"{0}\" height=\"{0}\" fill=\"#7a6a4a\" fill-opacity=\"{a:.2}\"/>\
",
                terrain.cell
            ));
        } else if terrain.cost[c].is_finite() && terrain.cost[c] > 1.0 && terrain.cost[c] < 1.05 {
            // fords and near-flat ground: nothing
        }
    }
    if let Some(t) = &scenario.terrain {
        for r in &t.rivers {
            for f in &r.fords {
                s.push_str(&format!(
                    "<circle cx=\"{}\" cy=\"{}\" r=\"28\" fill=\"none\" stroke=\"#203040\" stroke-width=\"6\" stroke-dasharray=\"8 6\"/>\
",
                    f.0, -f.1
                ));
            }
        }
    }
    for t in &scenario.towns {
        let (x, y) = (t.position.0, -t.position.1);
        s.push_str(&format!(
            "<circle cx=\"{x}\" cy=\"{y}\" r=\"34\" fill=\"#8b2e2e\" stroke=\"#2a1a10\" stroke-width=\"5\"/>\
"
        ));
        s.push_str(&format!(
            "<text x=\"{x}\" y=\"{}\" font-family=\"sans-serif\" font-size=\"48\" text-anchor=\"middle\" fill=\"#2a1a10\">{} ({})</text>\
",
            y - 50.0,
            t.name,
            t.population
        ));
        let land: Vec<String> = t
            .nature
            .iter()
            .map(|(k, n)| format!("{} {n}", k.replace('_', " ")))
            .collect();
        s.push_str(&format!(
            "<text x=\"{x}\" y=\"{}\" font-family=\"sans-serif\" font-size=\"28\" text-anchor=\"middle\" fill=\"#3a3020\">{}</text>\
",
            y + 78.0,
            land.join(", ")
        ));
    }
    s.push_str(
        "</svg>\
",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nature() -> Vec<(String, f32)> {
        [
            ("field", 1.0),
            ("forest", 1.0),
            ("ore_vein", 0.4),
            ("silver_vein", 0.15),
        ]
        .iter()
        .map(|(k, c)| (k.to_string(), *c))
        .collect()
    }

    fn world(seed: u64) -> Scenario {
        let mut s = Scenario {
            seed,
            player: Some("x".into()),
            towns: Vec::new(),
            terrain: None,
            goal: None,
            generate: None,
        };
        generate(&mut s, &GenSpec::default(), &nature());
        s
    }

    #[test]
    fn the_same_seed_gives_the_same_world_and_others_differ() {
        let (a, b, c) = (world(1), world(1), world(2));
        let key = |s: &Scenario| {
            s.towns
                .iter()
                .map(|t| (t.name.clone(), t.position.0.to_bits(), t.nature.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(key(&a), key(&b));
        assert_ne!(key(&a), key(&c));
    }

    #[test]
    fn every_town_can_feed_itself_and_every_resource_exists() {
        for seed in 0..20 {
            let s = world(seed);
            assert_eq!(s.towns.len(), 5);
            assert_eq!(s.player.as_deref(), Some(s.towns[0].name.as_str()));
            for t in &s.towns {
                let n = |k: &str| t.nature.iter().find(|n| n.0 == k).map_or(0, |n| n.1);
                assert!(n("field") >= 3 && n("forest") >= 2, "{}", t.name);
            }
            for (id, _) in nature() {
                assert!(s.towns.iter().any(|t| t.nature.iter().any(|n| n.0 == id)));
            }
            // Towns keep their distance (it only shrinks when the map is full).
            for (i, a) in s.towns.iter().enumerate() {
                for b in &s.towns[i + 1..] {
                    let d = ((a.position.0 - b.position.0).powi(2)
                        + (a.position.1 - b.position.1).powi(2))
                    .sqrt();
                    assert!(d > 200.0);
                }
            }
        }
    }
}
