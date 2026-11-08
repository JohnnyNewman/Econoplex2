use sim_data::{embed, Content};
use std::path::Path;

fn load() -> Content {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/content");
    Content::load_dir(&dir).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn shipped_content_is_valid() {
    let c = load();
    assert!(c.skills.len() >= 10);
    assert!(c.recipe("assemble_longsword_3").is_some());
    let report = sim_data::validate::validate(&c);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
}

#[test]
fn product_space_clusters_by_domain() {
    let c = load();
    let e = embed::embed(&c, sim_data::dims::CAP_DIM, 0.5, embed::EmbedMethod::Auto);
    let s =
        |a: &str, b: &str| e.similarity(c.skill(a).unwrap() as usize, c.skill(b).unwrap() as usize);
    assert!(s("smelting", "steelmaking") > s("steelmaking", "baking"));
    assert!(s("bladesmithing", "tempering") > s("bladesmithing", "farming"));
}

#[test]
fn cycles_are_rejected() {
    let text = r#"(skills: [
        (id: "a", name: "A", domain: "x", prereqs: ["b"]),
        (id: "b", name: "B", domain: "x", prereqs: ["a"]),
    ])"#;
    let f: sim_data::ContentFile = ron::from_str(text).unwrap();
    let err = Content::from_files(vec![f]).unwrap_err().to_string();
    assert!(err.contains("cycle"), "{err}");
}

#[test]
fn every_product_and_building_can_be_made() {
    // From natural resources and the starter buildings, through recipes whose
    // inputs and buildings can themselves be made: no loop blocks the start.
    let c = load();
    let have = sim_data::validate::reachable(&c);
    let missing: Vec<&str> = c
        .products
        .iter()
        .zip(&have)
        .filter(|(_, h)| !**h)
        .map(|(p, _)| p.id.as_str())
        .collect();
    assert!(missing.is_empty(), "unreachable: {missing:?}");
}

#[test]
fn the_tree_spans_five_eras_and_fits_the_product_space() {
    let c = load();
    assert_eq!(c.eras.len(), 5);
    for e in 1..c.eras.len() as u8 {
        assert!(
            c.products.iter().any(|p| p.era == e),
            "nothing in era {}",
            c.eras[e as usize]
        );
    }
    // One dimension per skill keeps the product space exact.
    assert!(c.skills.len() <= sim_data::dims::CAP_DIM);
    let e = embed::embed(&c, sim_data::dims::CAP_DIM, 0.5, embed::EmbedMethod::Auto);
    let s =
        |a: &str, b: &str| e.similarity(c.skill(a).unwrap() as usize, c.skill(b).unwrap() as usize);
    assert!(s("bronze_casting", "smelting") > s("bronze_casting", "weaving"));
    assert!(s("spinning", "weaving") > s("spinning", "smelting"));
    assert!(s("engineering", "toolmaking") > s("engineering", "baking"));
}

#[test]
fn a_building_loop_without_a_starter_is_rejected() {
    let text = r#"(
        skills: [(id: "s", name: "S", domain: "x")],
        products: [
            (id: "plank", name: "Plank", category: Material, base_price: 1.0),
            (id: "shop", name: "Shop", category: Building, base_price: 1.0),
        ],
        recipes: [
            (id: "saw", name: "Saw", skill: "s", difficulty: 0.1,
                outputs: [("plank", 1)], building: Some("shop"), duration: 1.0),
            (id: "build", name: "Build", skill: "s", difficulty: 0.1,
                inputs: [("plank", 5)], outputs: [("shop", 1)], duration: 1.0),
        ],
    )"#;
    let f: sim_data::ContentFile = ron::from_str(text).unwrap();
    let err = Content::from_files(vec![f.clone()])
        .unwrap_err()
        .to_string();
    assert!(err.contains("cannot be produced"), "{err}");
    // A town may start with the shop: then the loop is fine.
    let ok = text.replace(
        "base_price: 1.0),\n        ],",
        "base_price: 1.0, starter: true),\n        ],",
    );
    let f: sim_data::ContentFile = ron::from_str(&ok).unwrap();
    assert!(Content::from_files(vec![f]).is_ok());
}
