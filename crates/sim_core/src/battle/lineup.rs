//! Battle line-ups written as text (`econoplex-sim --battle`).

use super::*;

/// Parse a battle line-up like `cavalry*20 vs spearmen*30` or
/// `swordsmen*50,archers*20 vs walls spearmen*30+5!0.6`: each squad is
/// `kit*count`, optionally `+battles` of experience, `@skill` and `!morale`.
/// `walls` before the defenders puts them behind a wall.
pub fn parse_lineup(text: &str) -> Result<(Field, Vec<SquadSpec>), String> {
    let (att, def) = text
        .split_once(" vs ")
        .ok_or("a line-up needs `ATTACKERS vs DEFENDERS`")?;
    let mut def = def.trim();
    let mut field = Field::default();
    let walls = def.starts_with("walls ");
    if walls {
        def = def["walls ".len()..].trim();
        field.wall_x = Some(field.width * 0.6);
    }
    let mut specs = Vec::new();
    for (side, list) in [(0u8, att), (1u8, def)] {
        let parts: Vec<&str> = list
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let k = parts.len().max(1) as f32;
        for (n, part) in parts.iter().enumerate() {
            let mut rest = part.to_string();
            let num = |v: Option<String>, d: f32, what: &str| -> Result<f32, String> {
                v.map_or(Ok(d), |v| {
                    v.parse().map_err(|_| format!("bad {what} in `{part}`"))
                })
            };
            let morale = num(take(&mut rest, '!'), 0.8, "morale")?;
            let skill = num(take(&mut rest, '@'), 0.5, "skill")?;
            let exp = num(take(&mut rest, '+'), 0.0, "experience")?;
            let count = num(take(&mut rest, '*'), 20.0, "count")? as usize;
            let kit_name = rest.trim();
            let kit = Kit::preset(kit_name).ok_or_else(|| {
                format!(
                    "unknown kit `{kit_name}` (one of {})",
                    Kit::PRESETS.join(", ")
                )
            })?;
            let y = field.height * (n as f32 + 0.5) / k;
            let spacing = if kit.mounted { 2.0 } else { 1.0 };
            let x = match (side, walls) {
                (0, _) => field.width * 0.15,
                // Right behind the wall.
                (_, true) => {
                    field.wall_x.unwrap_or(0.0) + (count as f32).sqrt() * spacing * 0.5 + 0.6
                }
                _ => field.width * 0.75,
            };
            let level = if kit_name == "levy" {
                skill.min(0.2)
            } else {
                skill
            };
            let mut spec = SquadSpec::uniform(
                &format!("{kit_name}{}", n + 1),
                side,
                kit,
                count,
                level,
                exp,
                morale,
                (x, y),
            );
            spec.behind_wall = side == 1 && walls;
            specs.push(spec);
        }
    }
    Ok((field, specs))
}

/// Cut `<c>value` out of a squad description and return the value.
fn take(rest: &mut String, c: char) -> Option<String> {
    let at = rest.find(c)?;
    let end = rest[at + 1..]
        .find(['*', '+', '@', '!'])
        .map_or(rest.len(), |e| at + 1 + e);
    let v = rest[at + 1..end].to_string();
    rest.replace_range(at..end, "");
    Some(v)
}
