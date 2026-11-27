//! Where a town's buildings and houses stand on the ground, how they are
//! turned and how much ground they cover. The views draw them from here, a
//! battle fought in a town takes them as its obstacles, and walkers keep out
//! of them, so all three agree on where a wall is.
//!
//! - **Workplaces** stand where the simulation put them (`Town::buildings`),
//!   their front toward the town centre, covering their kind's `footprint`
//!   (content, map units).
//! - **Houses** are counted per town (`Town::houses`), not placed by the
//!   simulation. They stand on a spiral round the centre, one per spot,
//!   skipping water, spots near a workplace or a building site, and the
//!   roads into the town, so carts and marching armies keep to clear ground.
//!   Each covers [`HOUSE`], turned along the spiral.
//!
//! Everything here is in map units; a battle's field is in metres
//! (`battle::MAP_PER_METRE`).

use crate::battle::{self, Field, Obstacle, Shapes, V2};
use crate::db::Db;
use crate::terrain::Terrain;
use crate::world::Towns;
use serde::{Deserialize, Serialize};

/// The ground a house covers, map units: across its front and from front to
/// back (the 3D view's biggest house body; smaller ones stand inside it).
pub const HOUSE: (f32, f32) = (11.0, 13.0);
/// Houses keep this far from a workplace's or a site's centre, map units.
const TAKEN: f32 = 32.0;
/// And this far from the middle of a road into the town (half the road and
/// half a house's diagonal, with a little to spare).
const ROAD_CLEAR: f32 = 18.0;
/// The town's next building places a house keeps off, for the sites to come.
const RESERVED: u32 = 3;
/// Ids of houses: this bit, the town and the house's number on the spiral.
const HOUSE_ID: u64 = 1 << 62;
/// How far from its centre a town's layout reaches at most, map units.
pub const REACH: f32 = 400.0;

/// The ground one building or house covers, map units: its centre, half its
/// size along `axis` and across it, and `axis` (unit, along its front). The
/// front faces `axis` turned a quarter to the right.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Footprint {
    /// A workplace's entity bits; for a house, `HOUSE_ID` with its town and
    /// number.
    pub id: u64,
    pub centre: (f32, f32),
    pub half: (f32, f32),
    pub axis: (f32, f32),
}

impl Footprint {
    /// The angle of `axis` from the map's +x, counterclockwise, radians.
    /// A 3D view (y up, map y = -z) turns the shape's local x onto `axis`
    /// with a rotation about y by this angle.
    pub fn angle(&self) -> f32 {
        self.axis.1.atan2(self.axis.0)
    }

    /// Whether `p` lies inside it, grown by `r`.
    pub fn contains(&self, p: (f32, f32), r: f32) -> bool {
        let (dx, dy) = (p.0 - self.centre.0, p.1 - self.centre.1);
        let (u, v) = (
            dx * self.axis.0 + dy * self.axis.1,
            -dx * self.axis.1 + dy * self.axis.0,
        );
        u.abs() < self.half.0 + r && v.abs() < self.half.1 + r
    }

    /// As an obstacle in map units.
    pub fn obstacle(&self) -> Obstacle {
        Obstacle::rect(self.id, self.centre, self.half, self.axis)
    }

    /// As an obstacle on a battlefield `size` metres big, centred at `at`
    /// with +x along `facing` (`battle::field_to_map`), in metres.
    pub fn obstacle_on(&self, at: (f32, f32), facing: (f32, f32), size: (f32, f32)) -> Obstacle {
        let c = battle::map_to_field(at, facing, size, self.centre);
        let l = facing.0.hypot(facing.1).max(1e-6);
        let (fx, fy) = (facing.0 / l, facing.1 / l);
        let (ax, ay) = self.axis;
        Obstacle::rect(
            self.id,
            c,
            (
                self.half.0 / battle::MAP_PER_METRE,
                self.half.1 / battle::MAP_PER_METRE,
            ),
            (fx * ax + fy * ay, -fy * ax + fx * ay),
        )
    }
}

/// A house: where it stands (map), how it is turned (`Footprint::angle` of
/// its footprint), and its number on the town's spiral.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HouseSpot {
    pub pos: (f32, f32),
    pub angle: f32,
    pub k: u32,
}

fn unit((x, y): (f32, f32)) -> (f32, f32) {
    let l = x.hypot(y);
    if l > 1e-6 {
        (x / l, y / l)
    } else {
        (0.0, 1.0)
    }
}

/// The footprint of a workplace `size` big (map units) at `pos` in the town
/// centred at `town`: its front faces the centre.
pub fn building_footprint(id: u64, town: (f32, f32), pos: (f32, f32), size: (f32, f32)) -> Footprint {
    let (ux, uy) = unit((town.0 - pos.0, town.1 - pos.1));
    Footprint {
        id,
        centre: pos,
        half: (0.5 * size.0, 0.5 * size.1),
        // The front (axis turned right) toward the centre.
        axis: (-uy, ux),
    }
}

/// The footprint of house `spot` of town `ti`.
pub fn house_footprint(ti: usize, spot: &HouseSpot) -> Footprint {
    Footprint {
        id: HOUSE_ID | ((ti as u64) << 24) | spot.k as u64,
        centre: spot.pos,
        half: (0.5 * HOUSE.0, 0.5 * HOUSE.1),
        axis: (spot.angle.cos(), spot.angle.sin()),
    }
}

/// Distance from `p` to the segment `a`..`b`.
fn to_segment(p: (f32, f32), (a, b): ((f32, f32), (f32, f32))) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
}

/// The houses of town `ti` (centred at `town`), `count` of them: on a
/// golden-angle spiral round the centre, skipping spots on water (`water`),
/// within `TAKEN` of a point of `taken` or within `ROAD_CLEAR` of a road
/// segment. The first spots go first, so a town that shrinks loses its
/// outermost houses.
pub fn house_spots(
    ti: usize,
    town: (f32, f32),
    count: usize,
    taken: &[(f32, f32)],
    roads: &[((f32, f32), (f32, f32))],
    water: impl Fn((f32, f32)) -> bool,
) -> Vec<HouseSpot> {
    let mut spots = Vec::with_capacity(count);
    let mut k = 0u32;
    let tries = count as u32 * 4 + 400;
    while spots.len() < count && k < tries {
        let a = k as f32 * 2.399_963 + ti as f32;
        let r = 18.0 + (k as f32).sqrt() * 10.5;
        let p = (town.0 + r * a.cos(), town.1 + r * a.sin());
        k += 1;
        if water(p) || taken.iter().any(|t| (t.0 - p.0).hypot(t.1 - p.1) < TAKEN) {
            continue;
        }
        if roads.iter().any(|&s| to_segment(p, s) < ROAD_CLEAR) {
            continue;
        }
        spots.push(HouseSpot {
            pos: p,
            angle: a,
            k: k - 1,
        });
    }
    spots
}

/// The roads into town `ti` near it: the segments of the way from every
/// town a road connects to it and of the way out to it (`Terrain::path`:
/// each follows its own town's road field, so they can part), within
/// `REACH`.
pub fn roads_into(terrain: &Terrain, towns: &Towns, ti: usize) -> Vec<((f32, f32), (f32, f32))> {
    let here = towns.0[ti].pos;
    let near = |p: (f32, f32)| (p.0 - here.0).hypot(p.1 - here.1) < REACH;
    let mut segs = Vec::new();
    for (o, other) in towns.0.iter().enumerate() {
        if o == ti || !terrain.road(o, ti).is_finite() {
            continue;
        }
        let mut into = terrain.path(other.pos, ti);
        into.push(here);
        let mut out = terrain.path(here, o);
        out.push(other.pos);
        for path in [into, out] {
            for w in path.windows(2) {
                if near(w[0]) || near(w[1]) {
                    segs.push((w[0], w[1]));
                }
            }
        }
    }
    segs
}

/// A way along `path` that keeps `r` clear of everything in `shapes` (same
/// units): a point of it inside something is moved out of it, and between
/// the points the way goes round what stands in it, as a walker `step` at a
/// time would (`Shapes::steer`). For carts on the roads through a town.
pub fn detour(shapes: &Shapes, path: &[(f32, f32)], r: f32, step: f32) -> Vec<(f32, f32)> {
    let points: Vec<V2> = path
        .iter()
        .map(|&(x, y)| shapes.push_out(V2::new(x, y), r, 0))
        .collect();
    let Some(&first) = points.first() else {
        return Vec::new();
    };
    let mut way = vec![first];
    let mut me = first;
    for &w in &points[1..] {
        let mut left = ((w - me).len() / step) as usize * 3 + 20;
        while (w - me).len() > step && left > 0 {
            left -= 1;
            let mut to = me + (w - me).norm_or(V2::ZERO) * step;
            if shapes.any_near(me) || shapes.any_near(to) {
                if let Some(c) = shapes.steer(me, w, r, 0) {
                    to = me + (c - me).norm_or(V2::ZERO) * step.min((c - me).len());
                }
                to = shapes.resolve(me, to, r, 0);
                if (to - me).len() < 1e-3 * step {
                    break;
                }
                way.push(to);
            }
            me = to;
        }
        me = w;
        way.push(w);
    }
    way.into_iter().map(|p| (p.x, p.y)).collect()
}

/// Everything standing in one town.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TownLayout {
    pub centre: (f32, f32),
    /// Workplaces, in the order of `Town::buildings`.
    pub buildings: Vec<Footprint>,
    pub houses: Vec<HouseSpot>,
    pub house_footprints: Vec<Footprint>,
}

impl TownLayout {
    /// Town `ti` as it stands now.
    pub fn of(towns: &Towns, ti: usize, db: &Db, terrain: &Terrain) -> Self {
        let town = &towns.0[ti];
        let buildings: Vec<Footprint> = town
            .buildings
            .iter()
            .filter_map(|&(def, e, pos)| {
                let p = &db.content.products[def as usize];
                let size = p.footprint.filter(|_| p.workplace())?;
                Some(building_footprint(e.to_bits(), town.pos, pos, size))
            })
            .collect();
        // Workplaces, the places of buildings ordered, and the next few
        // building places, for the sites to come.
        let mut taken: Vec<(f32, f32)> = town.buildings.iter().map(|b| b.2).collect();
        taken.extend(town.policy.commissions.iter().filter_map(|c| c.pos));
        taken.extend((0..RESERVED).map(|n| town.building_pos(town.built + n)));
        let count = town.houses.iter().map(|&h| h as usize).sum();
        let roads = roads_into(terrain, towns, ti);
        let water = |p: (f32, f32)| terrain.cell_at(p).is_some_and(|c| terrain.water[c]);
        let houses = house_spots(ti, town.pos, count, &taken, &roads, water);
        let house_footprints = houses.iter().map(|h| house_footprint(ti, h)).collect();
        Self {
            centre: town.pos,
            buildings,
            houses,
            house_footprints,
        }
    }

    /// Every footprint: workplaces, then houses.
    pub fn footprints(&self) -> impl Iterator<Item = &Footprint> {
        self.buildings.iter().chain(&self.house_footprints)
    }

    /// The footprints as shapes in map units, for walkers in and near the town.
    pub fn shapes(&self) -> Shapes {
        let obstacles: Vec<Obstacle> = self.footprints().map(Footprint::obstacle).collect();
        Shapes::new(
            &obstacles,
            (self.centre.0 - REACH, self.centre.1 - REACH),
            (2.0 * REACH, 2.0 * REACH),
        )
    }

    /// The footprints on a battlefield `size` metres big, centred at `at`
    /// with +x along `facing`: those that reach onto it, in metres.
    pub fn obstacles_on(&self, at: (f32, f32), facing: (f32, f32), size: (f32, f32)) -> Vec<Obstacle> {
        self.footprints()
            .map(|f| f.obstacle_on(at, facing, size))
            .filter(|o| match o.shape {
                battle::Shape::Rect { centre, half, .. } => {
                    let r = half.0.hypot(half.1);
                    centre.0 > -r && centre.1 > -r && centre.0 < size.0 + r && centre.1 < size.1 + r
                }
                battle::Shape::Segment { .. } => true,
            })
            .collect()
    }
}

/// Where a raid from town `attacker` on town `target` is fought: the field,
/// with the target's buildings and houses on it as obstacles, its centre on
/// the map (60 map units out from the town toward the attackers) and the
/// attackers' heading.
pub fn battle_field(
    towns: &Towns,
    target: usize,
    attacker: usize,
    db: &Db,
    terrain: &Terrain,
) -> (Field, (f32, f32), (f32, f32)) {
    let (home, there) = (towns.0[attacker].pos, towns.0[target].pos);
    let (dx, dy) = (there.0 - home.0, there.1 - home.1);
    let len = dx.hypot(dy).max(1e-3);
    let facing = (dx / len, dy / len);
    let at = (there.0 - facing.0 * 60.0, there.1 - facing.1 * 60.0);
    let mut field = Field::default();
    let layout = TownLayout::of(towns, target, db, terrain);
    field.obstacles = layout.obstacles_on(at, facing, (field.width, field.height));
    (field, at, facing)
}

impl TownLayout {
    /// A town of the size the scenario starts with, for battles without a
    /// world (`--battle "... vs town ..."`): `buildings` workplaces of a few
    /// sizes on the starting ring (70 map units out) and `houses` houses,
    /// centred at `centre`, with no roads or water.
    pub fn sample(centre: (f32, f32), buildings: usize, houses: usize) -> Self {
        const SIZES: [(f32, f32); 4] = [(27.0, 45.0), (24.0, 28.0), (27.0, 28.0), (21.0, 21.0)];
        let mut taken = Vec::new();
        let buildings: Vec<Footprint> = (0..buildings)
            .map(|b| {
                let a = b as f32 / buildings as f32 * std::f32::consts::TAU;
                let pos = (centre.0 + 70.0 * a.cos(), centre.1 + 70.0 * a.sin());
                taken.push(pos);
                building_footprint(b as u64 + 1, centre, pos, SIZES[b % SIZES.len()])
            })
            .collect();
        let houses = house_spots(0, centre, houses, &taken, &[], |_| false);
        let house_footprints = houses.iter().map(|h| house_footprint(0, h)).collect();
        Self {
            centre,
            buildings,
            houses,
            house_footprints,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cart on a road straight through a workplace goes round it and on
    /// to the road's end.
    #[test]
    fn a_cart_goes_round_a_building_on_its_road() {
        let f = building_footprint(1, (0.0, 0.0), (70.0, 0.0), (27.0, 45.0));
        let layout = TownLayout {
            centre: (0.0, 0.0),
            buildings: vec![f],
            ..TownLayout::default()
        };
        let way = detour(&layout.shapes(), &[(0.0, 0.0), (200.0, 0.0)], 6.0, 2.0);
        assert_eq!(way.last(), Some(&(200.0, 0.0)));
        for w in way.windows(2) {
            for k in 0..=10 {
                let t = k as f32 / 10.0;
                let p = (w[0].0 + t * (w[1].0 - w[0].0), w[0].1 + t * (w[1].1 - w[0].1));
                assert!(!f.contains(p, 5.0), "{p:?} on the building");
            }
        }
    }

    /// The houses of a sample town stand clear of its workplaces and of each
    /// other, and houses keep off a road through the town.
    #[test]
    fn houses_keep_off_workplaces_roads_and_each_other() {
        let town = TownLayout::sample((0.0, 0.0), 8, 80);
        assert_eq!(town.houses.len(), 80);
        let road = ((-300.0, 5.0), (300.0, 5.0));
        let spots = house_spots(0, (0.0, 0.0), 80, &[], &[road], |_| false);
        assert!(spots.iter().all(|h| to_segment(h.pos, road) >= ROAD_CLEAR));
        let corners = |f: &Footprint| {
            [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(s, t)| {
                let (u, v) = (s * f.half.0, t * f.half.1);
                (
                    f.centre.0 + u * f.axis.0 - v * f.axis.1,
                    f.centre.1 + u * f.axis.1 + v * f.axis.0,
                )
            })
        };
        let all: Vec<&Footprint> = town.footprints().collect();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                let overlap = corners(a).iter().any(|&p| b.contains(p, -0.01))
                    || corners(b).iter().any(|&p| a.contains(p, -0.01));
                assert!(!overlap, "{a:?} and {b:?} overlap");
            }
        }
    }
}
