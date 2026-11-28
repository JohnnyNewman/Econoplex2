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

/// Metres between the lines of a side that stands in several, when the
/// field has room for it.
const LINE_GAP: f32 = 30.0;

/// Metres from a squad's back to the field's edge behind it, at least, and
/// between one line's back and the front of the line behind it.
const EDGE: f32 = 5.0;
const MIN_GAP: f32 = 5.0;

/// The width of a squad's front in a line-up, metres (the 50 m rows leave
/// gaps between them).
const LINEUP_FRONT: f32 = 40.0;

/// Where the attackers' first line stands, as a share of the field's width
/// from their edge, and how much of the field lies behind the defenders'.
const ATTACK_AT: f32 = 0.15;
const DEFENCE_BEHIND: f32 = 0.25;

/// Parse a battle line-up like `cavalry*20 vs spearmen*30` or
/// `swordsmen*50,archers*20 vs walls spearmen*30+5!0.6`: each squad is
/// `kit*count`, optionally `+battles` of experience, `@skill`, `!morale`,
/// `%fatigue` (0 rested, 1 spent) and `#flank` or `#rear` (deploy beside or
/// behind the enemy, facing in). `walls` before the defenders puts them
/// behind a wall, `town` fights in a town of the size the scenario starts
/// with (`sample_town`), `walled town` in that town with a palisade round
/// it (`sample_walled_town`). A side with more squads than fit across the
/// field stands in several lines, one behind the other: `LINE_GAP` apart, or
/// as close as it takes for its last line to stand on the field, both sides
/// alike. When even lines back to back would not fit, the field is made
/// longer.
pub fn parse_lineup(text: &str) -> Result<(Field, Vec<SquadSpec>), String> {
    let (att, def) = text
        .split_once(" vs ")
        .ok_or("a line-up needs `ATTACKERS vs DEFENDERS`")?;
    let mut def = def.trim();
    let mut field = Field::default();
    let town = def.starts_with("town ");
    let walled_town = def.starts_with("walled town ");
    if town {
        def = def["town ".len()..].trim();
    } else if walled_town {
        def = def["walled town ".len()..].trim();
    }
    let walls = def.starts_with("walls ");
    if walls {
        def = def["walls ".len()..].trim();
    }
    // Every squad first, then where each stands along the field, once the
    // field is long enough for every line: each squad's line and half its
    // depth in a square.
    let mut specs = Vec::new();
    let mut deploys = Vec::new();
    let mut places: Vec<(usize, f32)> = Vec::new();
    // Each side's lines, and its deepest squad's depth, metres.
    let mut lines = [1usize; 2];
    let mut depth = [0.0f32; 2];
    for (side, list) in [(0u8, att), (1u8, def)] {
        let parts: Vec<&str> = list
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        // Squads per line across the field (40 m fronts and gaps between).
        let per_line = ((field.height / 50.0) as usize).max(1);
        let n_lines = parts.len().div_ceil(per_line).max(1);
        let k = parts.len().div_ceil(n_lines).max(1) as f32;
        lines[side as usize] = n_lines;
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
            let y = field.height * (row as f32 + 0.5) / k;
            let spacing = if kit.mounted { 2.0 } else { 1.0 };
            let files = (LINEUP_FRONT / spacing).max(1.0);
            let ranks = (count as f32 / files).ceil().max(1.0);
            if deploy == Deploy::Line {
                depth[side as usize] = depth[side as usize].max(ranks * spacing);
            }
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
                (0.0, y),
            );
            for m in &mut spec.members {
                m.fatigue = fatigue.clamp(0.0, 1.0);
            }
            spec.behind_wall = side == 1 && walls;
            specs.push(spec);
            deploys.push(deploy);
            places.push((line, (count as f32).sqrt() * spacing * 0.5));
        }
    }
    // Lines back to back must fit behind each side's first: the field grows
    // until they do (the defenders' first line stands at most three
    // quarters across, `Field::defenders_x`).
    let closest = depth[0].max(depth[1]) + MIN_GAP;
    let behind = |side: usize| (lines[side] - 1) as f32 * closest + EDGE + depth[side] * 0.5;
    field.width = field
        .width
        .max(behind(0) / ATTACK_AT)
        .max(behind(1) / DEFENCE_BEHIND);
    if town {
        field.obstacles = sample_town(&field);
    } else if walled_town {
        let (obstacles, walls) = sample_walled_town(&field);
        field.obstacles = obstacles;
        field.walls = walls;
    }
    if walls {
        field.wall_x = Some(field.width * 0.6);
    }
    // Both sides' lines as far apart as both have room for (behind a
    // wall, the defenders' first line stands right behind it).
    let wall_front = match field.wall_x {
        Some(w) => {
            let half = specs
                .iter()
                .zip(&places)
                .filter(|(s, p)| s.side == 1 && p.0 == 0)
                .map(|(_, p)| p.1)
                .fold(0.0, f32::max);
            w + half + 0.6
        }
        None => field.defenders_x(),
    };
    let front = [field.width * ATTACK_AT, wall_front];
    let room = [
        front[0] - EDGE - depth[0] * 0.5,
        field.width - front[1] - EDGE - depth[1] * 0.5,
    ];
    let gap = (0..2)
        .filter(|&s| lines[s] > 1)
        .map(|s| room[s] / (lines[s] - 1) as f32)
        .fold(LINE_GAP, f32::min)
        .max(closest);
    for (spec, &(line, half)) in specs.iter_mut().zip(&places) {
        let back = line as f32 * gap;
        spec.pos.0 = match (spec.side, field.wall_x) {
            (0, _) => front[0] - back,
            // Right behind the wall.
            (_, Some(w)) => w + half + 0.6 + back,
            // Inside a town's wall at that height, if it has one.
            _ => {
                let (x, at) = field.defenders_spot(spec.pos.1);
                spec.pos.1 = at;
                x + back
            }
        };
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
