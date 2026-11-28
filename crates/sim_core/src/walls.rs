//! Town walls (war idea 10): a ring round the town, built a piece at a time.
//!
//! - **The ring.** A town that wants a wall (`Policy::walls`, the "Build
//!   walls" order; AI governors order one after raids) lays it out once:
//!   a circle `MARGIN` outside its furthest house or workplace, cut into
//!   stretches about `PIECE` map units long, a gate where a road crosses
//!   it (`plan`).
//! - **Building.** A wall piece is a building product (content `wall`). The
//!   town market pays for one at its base price while some piece of the
//!   ring needs it ([`TownWalls::wants`]): a gap, a piece below half its
//!   hit points, or one of a lower rank (a palisade once the town can build
//!   stone). A piece made goes up where it is most needed
//!   ([`TownWalls::put_up`]).
//! - **Upkeep.** Pieces lose `decay` of their hit points a year
//!   ([`TownWalls::weather`]), and a battle batters them; damage stays
//!   until the town mends it with a new piece.
//! - **Battle.** Every piece is an obstacle on a battle's field at the
//!   town, with its hit points (`layout::battle_field`); a gate lets only
//!   the defenders through, a gap or a piece battered down everybody. The
//!   damage a battle does comes back here ([`TownWalls::after_battle`]).

use crate::battle::{Obstacle, Shape, WallHp};
use crate::db::Db;
use crate::layout::Road;
use serde::{Deserialize, Serialize};

/// Map units of one stretch of wall (about 20 m).
pub const PIECE: f32 = 16.0;
/// Map units from the furthest house or workplace out to the wall.
pub const MARGIN: f32 = 20.0;
/// The fewest pieces a ring has.
const FEWEST: usize = 12;
/// Wall pieces' obstacle ids: this bit, the town and the piece's number.
const WALL_ID: u64 = 1 << 61;

/// One piece of a town's ring: its ends (map units), whether it is a gate,
/// the wall product standing there (`None`: a gap) and its hit points.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WallPiece {
    pub a: (f32, f32),
    pub b: (f32, f32),
    pub gate: bool,
    pub built: Option<u32>,
    pub hp: f32,
}

/// A town's wall: no pieces until it is laid out.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TownWalls {
    pub pieces: Vec<WallPiece>,
}

/// The obstacle id of piece `k` of town `ti`'s wall.
pub fn piece_id(ti: usize, k: usize) -> u64 {
    WALL_ID | ((ti as u64) << 24) | k as u64
}

/// A ring round `centre`, `radius` map units out, of pieces about `PIECE`
/// long, a gate in each piece a road crosses (one at least, toward +x if
/// no road comes).
pub fn ring(centre: (f32, f32), radius: f32, roads: &[Road]) -> Vec<WallPiece> {
    let n = ((std::f32::consts::TAU * radius / PIECE).ceil() as usize).max(FEWEST);
    let at = |k: usize| {
        let a = k as f32 / n as f32 * std::f32::consts::TAU;
        (centre.0 + radius * a.cos(), centre.1 + radius * a.sin())
    };
    let mut pieces: Vec<WallPiece> = (0..n)
        .map(|k| {
            let (a, b) = (at(k), at((k + 1) % n));
            WallPiece {
                a,
                b,
                gate: roads.iter().any(|&(p, q)| crosses(a, b, p, q)),
                built: None,
                hp: 0.0,
            }
        })
        .collect();
    if !pieces.iter().any(|p| p.gate) {
        pieces[0].gate = true;
    }
    pieces
}

/// Whether road `p`-`q` crosses piece `a`-`b`. A road through a corner of
/// the ring crosses the piece that starts there, not the one that ends
/// there, so it gets exactly one gate.
fn crosses(a: (f32, f32), b: (f32, f32), p: (f32, f32), q: (f32, f32)) -> bool {
    let side = |o: (f32, f32), u: (f32, f32), v: (f32, f32)| {
        (u.0 - o.0) * (v.1 - o.1) - (u.1 - o.1) * (v.0 - o.0)
    };
    let (d1, d2) = (side(a, b, p), side(a, b, q));
    let (d3, d4) = (side(p, q, a), side(p, q, b));
    d1 * d2 < 0.0 && (d3 > 0.0) != (d4 > 0.0)
}

/// Once a day: a town that wants a wall and has none lays one out, and
/// every wall weathers.
pub fn upkeep(
    mut towns: bevy_ecs::prelude::ResMut<crate::world::Towns>,
    db: bevy_ecs::prelude::Res<Db>,
    terrain: bevy_ecs::prelude::Res<crate::terrain::Terrain>,
    params: bevy_ecs::prelude::Res<crate::world::Params>,
) {
    let cal = params.calendar();
    let years = cal.month_days() / cal.days_per_year().max(1.0);
    for ti in 0..towns.0.len() {
        if towns.0[ti].policy.walls && !towns.0[ti].walls.planned() {
            let pieces = plan(&towns, ti, &db, &terrain);
            towns.0[ti].walls.pieces = pieces;
        }
        towns.0[ti].walls.weather(&db, years);
    }
}

/// Town `ti`'s ring as it would be laid out now: `MARGIN` outside its
/// furthest house or workplace, a gate on each road in.
pub fn plan(
    towns: &crate::world::Towns,
    ti: usize,
    db: &Db,
    terrain: &crate::terrain::Terrain,
) -> Vec<WallPiece> {
    let layout = crate::layout::TownLayout::of(towns, ti, db, terrain);
    let c = layout.centre;
    let far = layout
        .footprints()
        .map(|f| {
            let (dx, dy) = (f.centre.0 - c.0, f.centre.1 - c.1);
            dx.hypot(dy) + f.half.0.hypot(f.half.1)
        })
        .fold(0.0f32, f32::max);
    ring(
        c,
        far + MARGIN,
        &crate::layout::roads_into(terrain, towns, ti),
    )
}

impl TownWalls {
    /// Whether the town has laid out a wall.
    pub fn planned(&self) -> bool {
        !self.pieces.is_empty()
    }

    /// Pieces standing (built and not battered down), and pieces in all.
    pub fn standing(&self) -> (usize, usize) {
        let up = self
            .pieces
            .iter()
            .filter(|p| p.built.is_some() && p.hp > 0.0)
            .count();
        (up, self.pieces.len())
    }

    /// Whether some piece of the ring needs wall product `p`, for a town
    /// whose best wall is of rank `best`: a gap, a piece of `p`'s kind (a
    /// gate or a stretch) below half its hit points, or one of a lower
    /// rank than `p`'s. Below its best rank a town builds only into gaps:
    /// it closes its ring with what goes up quickest, then builds better.
    pub fn wants(&self, db: &Db, p: u32, best: u8) -> bool {
        let Some(w) = db.wall[p as usize] else {
            return false;
        };
        self.pieces.iter().any(|q| {
            q.gate == w.gate
                && if w.rank >= best {
                    needs(db, q, w.rank)
                } else {
                    q.built.is_none()
                }
        })
    }

    /// Wall product `p` made: it goes up in the piece that needs it most
    /// (a gap first, then the most battered, then a lower rank). Returns
    /// whether some piece took it.
    pub fn put_up(&mut self, db: &Db, p: u32) -> bool {
        let Some(w) = db.wall[p as usize] else {
            return false;
        };
        let worth = |q: &WallPiece| match q.built.and_then(|b| db.wall[b as usize]) {
            None => -1.0,
            Some(d) if d.rank < w.rank => 2.0,
            Some(d) => q.hp / d.hp,
        };
        let best = self
            .pieces
            .iter()
            .enumerate()
            .filter(|(_, q)| q.gate == w.gate && needs(db, q, w.rank))
            .min_by(|a, b| worth(a.1).total_cmp(&worth(b.1)).then(a.0.cmp(&b.0)))
            .map(|(k, _)| k);
        let Some(k) = best else {
            return false;
        };
        self.pieces[k].built = Some(p);
        self.pieces[k].hp = w.hp;
        true
    }

    /// `years` of weather: every piece standing loses its `decay` share of
    /// its hit points a year (never below one: rot alone opens no breach).
    pub fn weather(&mut self, db: &Db, years: f32) {
        for q in &mut self.pieces {
            let Some(w) = q.built.and_then(|b| db.wall[b as usize]) else {
                continue;
            };
            if q.hp > 1.0 {
                q.hp = (q.hp - w.hp * w.decay * years).max(1.0);
            }
        }
    }

    /// A battle at town `ti` is over: its pieces keep the hit points it
    /// left them (a piece battered down is a breach until mended).
    pub fn after_battle(&mut self, ti: usize, walls: &[WallHp]) {
        for w in walls {
            if let Some(q) = (0..self.pieces.len())
                .find(|&k| piece_id(ti, k) == w.id)
                .map(|k| &mut self.pieces[k])
            {
                if q.built.is_some() {
                    q.hp = w.hp.max(0.0);
                }
            }
        }
    }
}

/// The first wall products of the lowest rank: a stretch and a gate (the
/// palisade), if the content has both.
pub fn palisade(db: &Db) -> Option<(u32, u32)> {
    let lowest = |gate: bool| {
        (0..db.wall.len())
            .filter_map(|p| {
                db.wall[p]
                    .filter(|w| w.gate == gate)
                    .map(|w| (w.rank, p as u32))
            })
            .min()
            .map(|(_, p)| p)
    };
    Some((lowest(false)?, lowest(true)?))
}

/// The highest rank of wall `town` can build: one whose every input it
/// made last year or holds.
pub fn best_rank(db: &Db, town: &crate::world::Town) -> u8 {
    db.recipes
        .iter()
        .filter_map(|m| {
            let &(p, _) = m.outputs.first()?;
            let w = db.wall[p as usize]?;
            let can = m.inputs.iter().all(|&(i, _)| {
                let i = i as usize;
                town.produced_last_year.get(i).is_some_and(|&q| q > 0.0) || town.stock[i] >= 1.0
            });
            can.then_some(w.rank)
        })
        .max()
        .unwrap_or(0)
}

/// Whether piece `q` needs a piece of rank `rank`: it is a gap, below half
/// its hit points, or of a lower rank.
fn needs(db: &Db, q: &WallPiece, rank: u8) -> bool {
    match q.built.and_then(|b| db.wall[b as usize]) {
        None => true,
        Some(d) => d.rank < rank || q.hp < 0.5 * d.hp,
    }
}

/// Pieces `pieces` of town `ti`'s wall as obstacles (field metres, by
/// `to_field`) and their hit points, wall products' kinds from `defs`
/// (`Db::wall`). A gap is an open obstacle of the thinnest wall's
/// thickness, at no hit points.
pub fn on_field(
    pieces: &[WallPiece],
    defs: &[Option<sim_data::WallDef>],
    ti: usize,
    to_field: impl Fn((f32, f32)) -> (f32, f32),
) -> (Vec<Obstacle>, Vec<WallHp>) {
    let thinnest = defs
        .iter()
        .flatten()
        .map(|w| w.thickness)
        .fold(f32::INFINITY, f32::min);
    let thinnest = if thinnest.is_finite() { thinnest } else { 0.6 };
    let mut obstacles = Vec::with_capacity(pieces.len());
    let mut hp = Vec::with_capacity(pieces.len());
    for (k, q) in pieces.iter().enumerate() {
        let def = q
            .built
            .and_then(|b| defs.get(b as usize).copied().flatten());
        let wall = WallHp {
            id: piece_id(ti, k),
            hp: if def.is_some() { q.hp } else { 0.0 },
            max: def.map_or(0.0, |d| d.hp),
            gate: q.gate,
        };
        obstacles.push(Obstacle {
            id: wall.id,
            shape: Shape::Segment {
                a: to_field(q.a),
                b: to_field(q.b),
                half_width: 0.5 * def.map_or(thinnest, |d| d.thickness),
            },
            passage: wall.passage(),
        });
        hp.push(wall);
    }
    (obstacles, hp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AssetPaths, Simulation};

    fn db() -> Db {
        crate::load(&AssetPaths::discover()).unwrap().0
    }

    #[test]
    fn a_ring_closes_with_a_gate_on_each_road() {
        let roads = [((0.0, 0.0), (300.0, 0.0)), ((0.0, 0.0), (0.0, -300.0))];
        let pieces = ring((0.0, 0.0), 100.0, &roads);
        assert!(pieces.len() >= FEWEST);
        assert_eq!(pieces.iter().filter(|p| p.gate).count(), 2);
        for k in 0..pieces.len() {
            let next = pieces[(k + 1) % pieces.len()];
            assert_eq!(pieces[k].b, next.a, "piece {k} meets the next");
            let (dx, dy) = (pieces[k].b.0 - pieces[k].a.0, pieces[k].b.1 - pieces[k].a.1);
            assert!(dx.hypot(dy) <= PIECE + 0.5);
        }
        // No road: one gate all the same.
        assert_eq!(
            ring((0.0, 0.0), 50.0, &[])
                .iter()
                .filter(|p| p.gate)
                .count(),
            1
        );
    }

    #[test]
    fn pieces_fill_gaps_first_then_mend_and_rot_opens_no_breach() {
        let db = db();
        let (stretch, gate) = palisade(&db).unwrap();
        let stone = db.content.product("stone_wall").unwrap();
        let mut w = TownWalls {
            pieces: ring((0.0, 0.0), 60.0, &[]),
        };
        let n = w.pieces.len();
        assert!(w.wants(&db, stretch, 0) && w.wants(&db, gate, 0));
        // A town that can build stone closes its ring with palisade first.
        assert!(w.wants(&db, stretch, 1) && w.wants(&db, stone, 1));
        for _ in 0..n - 1 {
            assert!(w.put_up(&db, stretch));
        }
        assert!(w.put_up(&db, gate));
        assert_eq!(w.standing(), (n, n));
        assert!(!w.wants(&db, stretch, 0), "whole: nothing to build");
        assert!(
            !w.wants(&db, stretch, 1),
            "whole: no palisade where stone can go"
        );
        assert!(w.wants(&db, stone, 1), "but stone replaces it");
        assert!(!w.put_up(&db, stretch));
        // Years of weather wear it down, never through.
        w.weather(&db, 100.0);
        assert!(w.pieces.iter().all(|p| (p.hp - 1.0).abs() < 1e-3));
        assert_eq!(w.standing(), (n, n));
        // A battle batters one down; a new piece goes up there first.
        let down = 3;
        let mut hp: Vec<WallHp> = (0..n)
            .map(|k| WallHp {
                id: piece_id(7, k),
                hp: 50.0,
                max: 400.0,
                gate: w.pieces[k].gate,
            })
            .collect();
        hp[down].hp = 0.0;
        w.after_battle(7, &hp);
        assert_eq!(w.standing(), (n - 1, n));
        assert!(w.wants(&db, stretch, 0));
        assert!(w.put_up(&db, stretch));
        let def = db.wall[stretch as usize].unwrap();
        assert_eq!(w.pieces[down].hp, def.hp, "the breach is mended first");
        // Stone replaces the palisade, a stretch at a time.
        assert!(w.put_up(&db, stone));
        assert_eq!(
            w.pieces.iter().filter(|p| p.built == Some(stone)).count(),
            1
        );
    }

    #[test]
    fn a_town_that_orders_a_wall_lays_it_out_and_builds_it() {
        let paths = AssetPaths::discover();
        let (db, params, scenario) = crate::load(&paths).unwrap();
        let mut sim = Simulation::new(db, params, &scenario);
        let name = sim.world.resource::<crate::world::Towns>().0[0]
            .name
            .clone();
        sim.order(&format!("walls {name} on")).unwrap();
        for _ in 0..30 {
            sim.step();
        }
        let towns = sim.world.resource::<crate::world::Towns>();
        let walls = &towns.0[0].walls;
        assert!(walls.planned(), "laid out");
        assert!(walls.pieces.iter().any(|p| p.gate), "with a gate");
        // Paid for at its builders' better pay, half the ring is up in two
        // and a half years.
        let (up, all) = walls.standing();
        assert!(2 * up >= all, "{up} of {all} pieces built in 30 months");
    }
}
