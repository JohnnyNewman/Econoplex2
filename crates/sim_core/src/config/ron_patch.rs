//! Patching one value of a RON document in place (`--set`, sweeps, presets).

use super::ConfigError;

/// Replace one value in a RON document, addressed by a dotted path such as
/// `military.tribute_share` or `success.steepness` (fields of an enum variant
/// are reached the same way). Comments and formatting elsewhere are kept, so
/// the patched text still parses with the same types; used for `--set` and
/// parameter sweeps.
pub fn set_ron_field(text: &str, path: &str, value: &str) -> Result<String, ConfigError> {
    let b = text.as_bytes();
    let missing = || ConfigError(format!("no field `{path}` in the parameter file"));
    // The document itself is one parenthesized struct.
    let mut scope = inner(b, 0, b.len()).ok_or_else(missing)?;
    let keys: Vec<&str> = path.split('.').collect();
    for (n, key) in keys.iter().enumerate() {
        let span = find_field(b, scope, key).ok_or_else(missing)?;
        if n + 1 == keys.len() {
            return Ok(format!("{}{}{}", &text[..span.0], value, &text[span.1..]));
        }
        scope = inner(b, span.0, span.1).ok_or_else(missing)?;
    }
    Err(missing())
}

/// Skip a comment or string starting at `i`; returns the index after it.
fn skip_trivia(b: &[u8], i: usize) -> Option<usize> {
    match (b[i], b.get(i + 1)) {
        (b'/', Some(b'/')) => Some(
            b[i..]
                .iter()
                .position(|&c| c == b'\n')
                .map_or(b.len(), |p| i + p),
        ),
        (b'/', Some(b'*')) => Some(
            b[i + 2..]
                .windows(2)
                .position(|w| w == b"*/")
                .map_or(b.len(), |p| i + 2 + p + 2),
        ),
        (b'"', _) => {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'"' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            Some(j + 1)
        }
        _ => None,
    }
}

/// The span inside the first `(` ... matching `)` within `[from, to)`.
fn inner(b: &[u8], from: usize, to: usize) -> Option<(usize, usize)> {
    let mut i = from;
    while i < to {
        if let Some(j) = skip_trivia(b, i) {
            i = j;
            continue;
        }
        if b[i] == b'(' {
            // Walk value by value to the closing bracket.
            let mut end = value_end(b, i + 1, to);
            while end < to && b[end] == b',' {
                end = value_end(b, end + 1, to);
            }
            return Some((i + 1, end));
        }
        i += 1;
    }
    None
}

/// Index of the `,` or closing bracket that ends a value starting at `i`.
fn value_end(b: &[u8], mut i: usize, to: usize) -> usize {
    let mut depth = 0i32;
    while i < to {
        if let Some(j) = skip_trivia(b, i) {
            i = j;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth == 0 => return i,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => return i,
            _ => {}
        }
        i += 1;
    }
    to
}

/// The value span of `key: value` directly inside `scope` (not nested deeper).
fn find_field(b: &[u8], scope: (usize, usize), key: &str) -> Option<(usize, usize)> {
    let (mut i, to) = scope;
    let mut depth = 0i32;
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    while i < to {
        if let Some(j) = skip_trivia(b, i) {
            i = j;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            c if depth == 0 && ident(c) && (i == 0 || !ident(b[i - 1])) => {
                let mut j = i;
                while j < to && ident(b[j]) {
                    j += 1;
                }
                let mut k = j;
                while k < to && b[k].is_ascii_whitespace() {
                    k += 1;
                }
                if &b[i..j] == key.as_bytes() && k < to && b[k] == b':' {
                    let mut s = k + 1;
                    while s < to && b[s].is_ascii_whitespace() {
                        s += 1;
                    }
                    let mut e = value_end(b, s, to);
                    while e > s && b[e - 1].is_ascii_whitespace() {
                        e -= 1;
                    }
                    return Some((s, e));
                }
                i = j;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::set_ron_field;

    #[test]
    fn patches_nested_and_variant_fields() {
        let text = "// c: 1\n(\n  a: (x: 1.0, y: [1, 2]), // x: 9\n  s: Sigmoid(steepness: 8.0),\n  x: 3,\n)";
        let t = set_ron_field(text, "a.x", "2.5").unwrap();
        assert!(t.contains("a: (x: 2.5, y: [1, 2])"));
        let t = set_ron_field(&t, "s.steepness", "4.0").unwrap();
        assert!(t.contains("Sigmoid(steepness: 4.0)"));
        let t = set_ron_field(&t, "x", "7").unwrap();
        assert!(t.contains("  x: 7,"));
        assert!(set_ron_field(text, "a.z", "1").is_err());
        assert!(set_ron_field(text, "y", "1").is_err());
    }
}
