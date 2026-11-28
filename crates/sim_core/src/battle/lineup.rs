//! Battle line-ups written as text (`econoplex-sim --battle`).

use super::*;

/// Where a squad deploys.
#[derive(Clone, Copy, PartialEq)]
enum Deploy {
    /// In its side's line.
    Line,
    /// Beside the enemy's line, facing in.
    Flank,
    /// Behind the enemy, facing in.
    Rear,
}

/// Metres between a squad sent round the enemy and the squad it faces.
const ROUND_GAP: f32 = 30.0;

/// Parse a battle line-up like `cavalry*20 vs spearmen*30` or
/// `swordsmen*50,archers*20 vs walls spearmen*30+5!0.6`: each squad is
/// `kit*count`, optionally `+battles` of experience, `@skill`, `!morale`,
/// `%fatigue` (0 rested, 1 spent) and `#flank` or `#rear` (deploy beside or
/// behind the enemy, facing in). `walls` before the defenders puts them
/// behind a wall, `town` fights in a town of the size the scenario starts
/// with (`sample_town`), `walled town` in that town with a palisade round
/// it (`sample_walled_town`). A side with more squads than fit across the field
/// stands in several lines, one behind the other.
pub fn parse_lineup(text: &str) -> Result<(Field, Vec<SquadSpec>), String> {
    let (att, def) = text
        .split_once(" vs ")
        .ok_or("a line-up needs `ATTACKERS vs DEFENDERS`")?;
    let mut def = def.trim();
    let mut field = Field::default();
    if let Some(rest) = def.strip_prefix("town ") {
        def = rest.trim();
        field.obstacles = sample_town(&field);
    } else if let Some(rest) = def.strip_prefix("walled town ") {
        def = rest.trim();
        let (obstacles, walls) = sample_walled_town(&field);
        field.obstacles = obstacles;
        field.walls = walls;
    }
    let walls = def.starts_with("walls ");
    if walls {
        def = def["walls ".len()..].trim();
        field.wall_x = Some(field.width * 0.6);
    }
    let mut specs = Vec::new();
    let mut deploys = Vec::new();
    for (side, list) in [(0u8, att), (1u8, def)] {
        let parts: Vec<&str> = list
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        // Squads per line across the field (40 m fronts and gaps between).
        let per_line = ((field.height / 50.0) as usize).max(1);
        let lines = parts.len().div_ceil(per_line).max(1);
        let k = parts.len().div_ceil(lines).max(1) as f32;
        for (n, part) in parts.iter().enumerate() {
            let mut rest = part.to_string();
            let num = |v: Option<String>, d: f32, what: &str| -> Result<f32, String> {
                v.map_or(Ok(d), |v| {
                    v.parse().map_err(|_| format!("bad {what} in `{part}`"))
                })
            };
            let deploy = match take(&mut rest, '#').as_deref() {
                None => Deploy::Line,
                Some("flank") => Deploy::Flank,
                Some("rear") => Deploy::Rear,
                Some(other) => return Err(format!("`#{other}` in `{part}`: #flank or #rear")),
            };
            let fatigue = num(take(&mut rest, '%'), 0.0, "fatigue")?;
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
            let (line, row) = (n / k as usize, n % k as usize);
            let mut y = field.height * (row as f32 + 0.5) / k;
            let spacing = if kit.mounted { 2.0 } else { 1.0 };
            let back = line as f32 * 30.0;
            let x = match (side, walls) {
                (0, _) => field.width * 0.15 - back,
                // Right behind the wall.
                (_, true) => {
                    field.wall_x.unwrap_or(0.0) + (count as f32).sqrt() * spacing * 0.5 + 0.6 + back
                }
                _ => {
                    let (x, at) = field.defenders_spot(y);
                    y = at;
                    x + back
                }
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
            for m in &mut spec.members {
                m.fatigue = fatigue.clamp(0.0, 1.0);
            }
            spec.behind_wall = side == 1 && walls;
            specs.push(spec);
            deploys.push(deploy);
        }
    }
    // Squads sent round the enemy: beside the enemy squad nearest their place
    // in the line, on its outer side, or behind it.
    let line: Vec<SquadSpec> = specs.clone();
    for (spec, &deploy) in specs.iter_mut().zip(&deploys) {
        if deploy == Deploy::Line {
            continue;
        }
        let radius = |s: &SquadSpec| (s.members.len() as f32).sqrt() * 0.5;
        let Some(e) = line
            .iter()
            .zip(&deploys)
            .filter(|(e, d)| e.side != spec.side && **d == Deploy::Line)
            .map(|(e, _)| e)
            .min_by(|a, b| {
                (a.pos.1 - spec.pos.1)
                    .abs()
                    .total_cmp(&(b.pos.1 - spec.pos.1).abs())
            })
        else {
            continue;
        };
        let off = radius(e) + radius(spec) + ROUND_GAP;
        spec.pos = match deploy {
            Deploy::Flank => {
                let out = if e.pos.1 < field.height * 0.5 {
                    -1.0
                } else {
                    1.0
                };
                let y = e.pos.1 + out * (e.members.len() as f32).min(40.0) * 0.5 + out * off;
                (e.pos.0, y.clamp(5.0, field.height - 5.0))
            }
            _ => {
                let behind = if e.side == 0 { -1.0 } else { 1.0 };
                let x = e.pos.0 + behind * off;
                (x.clamp(5.0, field.width - 5.0), e.pos.1)
            }
        };
    }
    Ok((field, specs))
}

/// The sample town of [`sample_town`] with a whole palisade round it, a
/// gate on the side the attackers come from and one behind: its obstacles
/// (the wall's among them) and the wall's hit points.
pub fn sample_walled_town(field: &Field) -> (Vec<Obstacle>, Vec<WallHp>) {
    let defs = [Some(SAMPLE_PALISADE), Some(SAMPLE_GATE)];
    let (facing, size) = ((1.0, 0.0), (field.width, field.height));
    let town = crate::layout::TownLayout::sample((0.0, 0.0), 8, 60).walled(&defs, 0, 1);
    let at = town.battle_at(facing, size);
    let mut obstacles = town.obstacles_on(at, facing, size);
    let (walls, hp) = crate::walls::on_field(&town.walls, &defs, 0, |p| {
        crate::battle::map_to_field(at, facing, size, p)
    });
    obstacles.extend(walls);
    (obstacles, hp)
}

/// The sample wall's pieces: as the content's palisade and its gate.
const SAMPLE_PALISADE: sim_data::WallDef = sim_data::WallDef {
    hp: 400.0,
    thickness: 0.6,
    gate: false,
    decay: 0.06,
    rank: 0,
};
const SAMPLE_GATE: sim_data::WallDef = sim_data::WallDef {
    hp: 300.0,
    thickness: 0.6,
    gate: true,
    decay: 0.06,
    rank: 0,
};

/// Cut `<c>value` out of a squad description and return the value.
fn take(rest: &mut String, c: char) -> Option<String> {
    let at = rest.find(c)?;
    let end = rest[at + 1..]
        .find(['*', '+', '@', '!', '%', '#'])
        .map_or(rest.len(), |e| at + 1 + e);
    let v = rest[at + 1..end].to_string();
    rest.replace_range(at..end, "");
    Some(v)
}

/// The obstacles of a battle in a town without a world: a town of eight
/// workplaces and sixty houses, where a raid's field has the target town
/// (beginning `layout::TOWN_FRONT` of the way across).
pub fn sample_town(field: &Field) -> Vec<Obstacle> {
    let (facing, size) = ((1.0, 0.0), (field.width, field.height));
    let town = crate::layout::TownLayout::sample((0.0, 0.0), 8, 60);
    town.obstacles_on(town.battle_at(facing, size), facing, size)
}
