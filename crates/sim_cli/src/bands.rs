//! Health bands: the range each batch result is expected to stay in.
//!
//! A bands file maps a column of the batch table (`pop`, `starve%`, ...) to a
//! `(low, high)` range. After a batch run every setting's mean across seeds is
//! checked against it; CI fails when one leaves its band.

use std::collections::BTreeMap;

/// Metric name to inclusive `(low, high)` range.
pub type Bands = BTreeMap<String, (f64, f64)>;

/// One metric's mean against its band.
pub struct Check {
    pub metric: String,
    pub mean: f64,
    pub low: f64,
    pub high: f64,
}

impl Check {
    pub fn ok(&self) -> bool {
        (self.low..=self.high).contains(&self.mean)
    }
}

pub fn read(path: &std::path::Path) -> Result<Bands, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let bands: Bands = ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    for (name, (low, high)) in &bands {
        if low > high {
            return Err(format!(
                "{}: band `{name}` has low above high",
                path.display()
            ));
        }
    }
    Ok(bands)
}

/// Compare one setting's means (in `columns` order) with the bands. A band for
/// a metric the table does not have is an error, so a typo cannot pass silently.
pub fn check(bands: &Bands, columns: &[&str], means: &[f64]) -> Result<Vec<Check>, String> {
    bands
        .iter()
        .map(|(name, &(low, high))| {
            let k = columns.iter().position(|c| c == name).ok_or_else(|| {
                format!("unknown metric `{name}` (columns: {})", columns.join(" "))
            })?;
            Ok(Check {
                metric: name.clone(),
                mean: means[k],
                low,
                high,
            })
        })
        .collect()
}

/// Print every setting's means against the bands. Whether all are inside.
pub fn report(
    bands: &Bands,
    columns: &[&str],
    settings: &[(String, Vec<f64>)],
) -> Result<bool, String> {
    let mut all_ok = true;
    println!("\nhealth bands");
    for (label, means) in settings {
        println!("{label}");
        println!(
            "  {:<10} {:>10} {:>10} {:>10}",
            "metric", "mean", "low", "high"
        );
        for c in check(bands, columns, means)? {
            let verdict = if c.ok() { "ok" } else { "FAIL" };
            println!(
                "  {:<10} {:>10.2} {:>10.2} {:>10.2}  {verdict}",
                c.metric, c.mean, c.low, c.high
            );
            all_ok &= c.ok();
        }
    }
    if !all_ok {
        println!("\nresults left their health band");
    }
    Ok(all_ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bands(text: &str) -> Bands {
        ron::from_str(text).unwrap()
    }

    #[test]
    fn means_inside_below_and_above_their_band() {
        let b = bands(r#"{ "pop": (900, 2000), "div": (18, 32), "starve%": (0, 2.5) }"#);
        let checks = check(&b, &["pop", "starve%", "div"], &[1437.0, 3.1, 12.0]).unwrap();
        let by = |m: &str| checks.iter().find(|c| c.metric == m).unwrap().ok();
        assert!(by("pop"));
        assert!(!by("starve%"), "above the band");
        assert!(!by("div"), "below the band");
    }

    #[test]
    fn band_edges_are_inside() {
        let b = bands(r#"{ "pop": (900, 2000) }"#);
        assert!(check(&b, &["pop"], &[900.0]).unwrap()[0].ok());
        assert!(check(&b, &["pop"], &[2000.0]).unwrap()[0].ok());
    }

    #[test]
    fn unknown_metric_is_an_error() {
        let b = bands(r#"{ "population": (900, 2000) }"#);
        assert!(check(&b, &["pop"], &[1437.0]).is_err());
    }

    #[test]
    fn the_shipped_bands_name_real_columns() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/config/health_bands.ron"
        );
        let b = read(std::path::Path::new(path)).unwrap();
        assert!(!b.is_empty());
        check(&b, &crate::sweep::COLUMNS, &[0.0; crate::sweep::NCOL]).unwrap();
    }
}
