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
