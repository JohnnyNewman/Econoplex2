//! Skill embedding: turn the skill DAG into latent, nonnegative skill vectors.
//!
//! Each skill is first described by its *prerequisite closure*: itself plus all its
//! ancestors, weighted by `decay^depth`. The dot product of two closure vectors
//! measures shared prerequisites, which is Hidalgo's notion of relatedness.
//!
//! * [`EmbedMethod::Closure`] uses the closure vectors directly (one dimension per
//!   skill). Interpretable; needs `skills <= dim`.
//! * [`EmbedMethod::Nmf`] compresses the closure matrix with nonnegative matrix
//!   factorization to `dim` latent capabilities, for content with more skills than dimensions.
//!
//! All vectors are normalized to unit length so difficulty values are comparable.

use crate::Content;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EmbedMethod {
    /// Closure vectors if they fit, otherwise NMF.
    Auto,
    Closure,
    Nmf,
}

#[derive(Debug, Clone)]
pub struct Embedding {
    /// One unit-length vector of length `dim` per skill.
    pub vectors: Vec<Vec<f32>>,
    pub dim: usize,
    pub method: EmbedMethod,
    /// Relative Frobenius reconstruction error (0 for exact closure).
    pub error: f32,
}

impl Embedding {
    pub fn similarity(&self, a: usize, b: usize) -> f32 {
        self.vectors[a]
            .iter()
            .zip(&self.vectors[b])
            .map(|(x, y)| x * y)
            .sum()
    }

    /// The `k` most similar other skills to skill `i`.
    pub fn neighbors(&self, i: usize, k: usize) -> Vec<(usize, f32)> {
        let mut v: Vec<(usize, f32)> = (0..self.vectors.len())
            .filter(|&j| j != i)
            .map(|j| (j, self.similarity(i, j)))
            .collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        v.truncate(k);
        v
    }
}

/// Row `i` = closure vector of skill `i` over all skills (n x n, nonnegative).
pub fn closure_matrix(c: &Content, decay: f32) -> Vec<Vec<f32>> {
    let n = c.skills.len();
    let mut m = vec![vec![0.0f32; n]; n];
    for (i, row) in m.iter_mut().enumerate() {
        // Breadth-first walk up the DAG, keeping the shortest depth to each ancestor.
        let mut depth = vec![u32::MAX; n];
        depth[i] = 0;
        let mut frontier = vec![i];
        while let Some(x) = frontier.pop() {
            for p in c.prereqs(x) {
                let d = depth[x] + 1;
                if d < depth[p] {
                    depth[p] = d;
                    frontier.push(p);
                }
            }
        }
        for j in 0..n {
            if depth[j] != u32::MAX {
                row[j] = decay.powi(depth[j] as i32);
            }
        }
    }
    m
}

pub fn embed(c: &Content, dim: usize, decay: f32, method: EmbedMethod) -> Embedding {
    let m = closure_matrix(c, decay);
    let n = m.len();
    let method = match method {
        EmbedMethod::Auto if n <= dim => EmbedMethod::Closure,
        EmbedMethod::Auto => EmbedMethod::Nmf,
        m => m,
    };
    let (mut vectors, error) = match method {
        EmbedMethod::Closure => {
            assert!(
                n <= dim,
                "closure embedding needs dim >= number of skills ({n} > {dim})"
            );
            let v = m
                .iter()
                .map(|row| {
                    let mut v = vec![0.0; dim];
                    v[..n].copy_from_slice(row);
                    v
                })
                .collect();
            (v, 0.0)
        }
        EmbedMethod::Nmf => nmf(&m, dim, 400),
        EmbedMethod::Auto => unreachable!(),
    };
    for v in &mut vectors {
        let len = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
        v.iter_mut().for_each(|x| *x /= len);
    }
    Embedding {
        vectors,
        dim,
        method,
        error,
    }
}

/// Lee–Seung multiplicative-update NMF, `m (n x n) ≈ w (n x k) · h (k x n)`.
/// Deterministic initialization. Returns the rows of `w` and the relative error.
pub fn nmf(m: &[Vec<f32>], k: usize, iters: usize) -> (Vec<Vec<f32>>, f32) {
    let n = m.len();
    let cols = if n == 0 { 0 } else { m[0].len() };
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        0.1 + (seed % 10_000) as f32 / 10_000.0
    };
    let mut w: Vec<Vec<f32>> = (0..n).map(|_| (0..k).map(|_| rnd()).collect()).collect();
    let mut h: Vec<Vec<f32>> = (0..k).map(|_| (0..cols).map(|_| rnd()).collect()).collect();
    let eps = 1e-9f32;
    for _ in 0..iters {
        // h <- h * (wT m) / (wT w h)
        let wh = matmul(&w, &h);
        for a in 0..k {
            for j in 0..cols {
                let mut num = 0.0;
                let mut den = 0.0;
                for i in 0..n {
                    num += w[i][a] * m[i][j];
                    den += w[i][a] * wh[i][j];
                }
                h[a][j] *= num / (den + eps);
            }
        }
        // w <- w * (m hT) / (w h hT)
        let wh = matmul(&w, &h);
        for i in 0..n {
            for a in 0..k {
                let mut num = 0.0;
                let mut den = 0.0;
                for j in 0..cols {
                    num += m[i][j] * h[a][j];
                    den += wh[i][j] * h[a][j];
                }
                w[i][a] *= num / (den + eps);
            }
        }
    }
    let wh = matmul(&w, &h);
    let mut err = 0.0;
    let mut tot = 0.0;
    for i in 0..n {
        for j in 0..cols {
            err += (m[i][j] - wh[i][j]).powi(2);
            tot += m[i][j].powi(2);
        }
    }
    (w, (err / tot.max(eps)).sqrt())
}

fn matmul(a: &[Vec<f32>], b: &[Vec<f32>]) -> Vec<Vec<f32>> {
    let n = a.len();
    let k = b.len();
    let cols = if k == 0 { 0 } else { b[0].len() };
    let mut out = vec![vec![0.0; cols]; n];
    for i in 0..n {
        for a_ in 0..k {
            let x = a[i][a_];
            if x == 0.0 {
                continue;
            }
            for j in 0..cols {
                out[i][j] += x * b[a_][j];
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentFile, SkillDef};

    fn skill(id: &str, prereqs: &[&str]) -> SkillDef {
        SkillDef {
            id: id.into(),
            name: id.into(),
            domain: "d".into(),
            prereqs: prereqs.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn content() -> Content {
        let mut f = ContentFile::default();
        f.skills = vec![
            skill("mining", &[]),
            skill("smelting", &["mining"]),
            skill("smithing", &["smelting"]),
            skill("farming", &[]),
            skill("baking", &["farming"]),
        ];
        let mut c = Content::default();
        c.skills = f.skills;
        c.skill_index = c
            .skills
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.clone(), i as u32))
            .collect();
        c
    }

    #[test]
    fn related_skills_are_closer() {
        let c = content();
        for method in [EmbedMethod::Closure, EmbedMethod::Nmf] {
            let dim = if method == EmbedMethod::Nmf { 3 } else { 8 };
            let e = embed(&c, dim, 0.5, method);
            let smelt_smith = e.similarity(1, 2);
            let smith_bake = e.similarity(2, 4);
            assert!(
                smelt_smith > smith_bake + 0.2,
                "{method:?}: {smelt_smith} vs {smith_bake}"
            );
            for v in &e.vectors {
                assert!(v.iter().all(|x| *x >= 0.0), "vectors must stay nonnegative");
            }
        }
    }
}
