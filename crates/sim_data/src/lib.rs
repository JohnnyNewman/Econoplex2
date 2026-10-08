//! Econoplex content layer.
//!
//! Designers author skills, products, recipes and natural resources in RON files
//! (one file per domain). This crate loads and merges those files, interns string
//! ids into compact indices, validates the graphs, and turns the skill DAG into
//! latent skill vectors (see [`embed`]).

pub mod defs;
pub mod dims;
pub mod embed;
pub mod validate;

use std::collections::BTreeMap;
use std::path::Path;

pub use defs::*;

/// Fully loaded, merged and indexed content.
#[derive(Debug, Clone, Default)]
pub struct Content {
    pub skills: Vec<SkillDef>,
    pub products: Vec<ProductDef>,
    pub recipes: Vec<RecipeDef>,
    pub nature: Vec<NatureDef>,
    /// Distinct skill domains, in order of first appearance.
    pub domains: Vec<String>,

    pub skill_index: BTreeMap<String, u32>,
    pub product_index: BTreeMap<String, u32>,
    pub recipe_index: BTreeMap<String, u32>,
    pub nature_index: BTreeMap<String, u32>,
    pub domain_index: BTreeMap<String, u32>,
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::path::PathBuf, std::io::Error),
    Parse(std::path::PathBuf, String),
    Invalid(Vec<String>),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(p, e) => write!(f, "cannot read {}: {e}", p.display()),
            LoadError::Parse(p, e) => write!(f, "cannot parse {}: {e}", p.display()),
            LoadError::Invalid(errs) => {
                writeln!(f, "content is invalid:")?;
                for e in errs {
                    writeln!(f, "  - {e}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for LoadError {}

impl Content {
    /// Load every `*.ron` file in `dir`, in alphabetical order. Later files can add
    /// new entries or override earlier ones with the same id (this is also the modding hook).
    pub fn load_dir(dir: &Path) -> Result<Content, LoadError> {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| LoadError::Io(dir.to_path_buf(), e))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "ron"))
            .collect();
        files.sort();
        let mut parts = Vec::new();
        for path in files {
            let text =
                std::fs::read_to_string(&path).map_err(|e| LoadError::Io(path.clone(), e))?;
            let file: ContentFile =
                ron::from_str(&text).map_err(|e| LoadError::Parse(path.clone(), e.to_string()))?;
            parts.push(file);
        }
        Content::from_files(parts)
    }

    /// Merge content files (later entries override earlier ones by id), index and validate.
    pub fn from_files(files: Vec<ContentFile>) -> Result<Content, LoadError> {
        fn merge<T: Clone>(into: &mut Vec<T>, add: Vec<T>, id: impl Fn(&T) -> &str) {
            for item in add {
                if let Some(slot) = into.iter_mut().find(|x| id(x) == id(&item)) {
                    *slot = item;
                } else {
                    into.push(item);
                }
            }
        }
        let mut c = Content::default();
        for f in files {
            merge(&mut c.skills, f.skills, |s| &s.id);
            merge(&mut c.products, f.products, |s| &s.id);
            merge(&mut c.recipes, f.recipes, |s| &s.id);
            merge(&mut c.nature, f.nature, |s| &s.id);
        }
        c.reindex();
        let report = validate::validate(&c);
        if !report.errors.is_empty() {
            return Err(LoadError::Invalid(report.errors));
        }
        Ok(c)
    }

    fn reindex(&mut self) {
        self.skill_index = self
            .skills
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.clone(), i as u32))
            .collect();
        self.product_index = self
            .products
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.clone(), i as u32))
            .collect();
        self.recipe_index = self
            .recipes
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.clone(), i as u32))
            .collect();
        self.nature_index = self
            .nature
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.clone(), i as u32))
            .collect();
        self.domains.clear();
        for s in &self.skills {
            if !self.domains.contains(&s.domain) {
                self.domains.push(s.domain.clone());
            }
        }
        self.domain_index = self
            .domains
            .iter()
            .enumerate()
            .map(|(i, d)| (d.clone(), i as u32))
            .collect();
    }

    pub fn skill(&self, id: &str) -> Option<u32> {
        self.skill_index.get(id).copied()
    }
    pub fn product(&self, id: &str) -> Option<u32> {
        self.product_index.get(id).copied()
    }
    pub fn recipe(&self, id: &str) -> Option<u32> {
        self.recipe_index.get(id).copied()
    }
    pub fn nature_kind(&self, id: &str) -> Option<u32> {
        self.nature_index.get(id).copied()
    }
    pub fn domain(&self, id: &str) -> Option<u32> {
        self.domain_index.get(id).copied()
    }

    /// Indices of the direct prerequisites of skill `i`.
    pub fn prereqs(&self, i: usize) -> Vec<usize> {
        self.skills[i]
            .prereqs
            .iter()
            .filter_map(|p| self.skill(p).map(|x| x as usize))
            .collect()
    }
}
