//! What stands on each town's ground (`sim_core::layout`): the houses are
//! drawn where the layout puts them, and walkers keep out of every house and
//! workplace, going round them by their nearer corner. The 3D client draws
//! and walks by the same layout, from its snapshots.

use crate::view::WorldView;
use bevy::prelude::*;
use sim_core::battle::{Shapes, V2};
use sim_core::layout::{TownLayout, HOUSE};
use sim_core::{Db, Towns};

/// A walker's radius, map units: they pass this far from a wall.
pub const WALKER: f32 = 1.5;

/// Each town's layout, the shapes walkers keep out of, and the house sprites
/// drawn for it, with what they were made from.
#[derive(Resource, Default)]
pub struct TownGround {
    towns: Vec<Ground>,
}

#[derive(Default)]
struct Ground {
    /// Workplaces, houses, building places taken: the layout changes with them.
    key: (usize, u32, u32, usize),
    layout: TownLayout,
    shapes: Option<Shapes>,
    houses: Vec<Entity>,
}

/// A house of a town, drawn.
#[derive(Component)]
pub struct HouseSprite;

const HOUSE_COLOUR: Color = Color::srgb(0.62, 0.5, 0.36);

/// Lay out each town again when what stands in it changed, and draw its
/// houses (again, too, after a load cleared the view).
pub fn update_town_ground(
    mut commands: Commands,
    mut ground: ResMut<TownGround>,
    towns: Res<Towns>,
    db: Res<Db>,
    terrain: Res<sim_core::terrain::Terrain>,
    drawn: Query<(), With<HouseSprite>>,
) {
    ground.towns.resize_with(towns.0.len(), Ground::default);
    for (ti, town) in towns.0.iter().enumerate() {
        let key = (
            town.buildings.len(),
            town.houses.iter().sum::<u32>(),
            town.built,
            town.policy.commissions.len(),
        );
        let g = &mut ground.towns[ti];
        let gone = g.houses.first().is_some_and(|&e| drawn.get(e).is_err());
        if g.shapes.is_some() && g.key == key && !gone {
            continue;
        }
        g.key = key;
        g.layout = TownLayout::of(&towns, ti, &db, &terrain);
        g.shapes = Some(g.layout.shapes());
        for e in g.houses.drain(..) {
            if drawn.get(e).is_ok() {
                commands.entity(e).despawn();
            }
        }
        g.houses = g
            .layout
            .house_footprints
            .iter()
            .map(|f| {
                commands
                    .spawn((
                        HouseSprite,
                        WorldView,
                        Sprite::from_color(HOUSE_COLOUR, Vec2::new(HOUSE.0, HOUSE.1)),
                        Transform::from_xyz(f.centre.0, f.centre.1, 0.8)
                            .with_rotation(Quat::from_rotation_z(f.angle())),
                    ))
                    .id()
            })
            .collect();
    }
}

impl TownGround {
    /// The shapes of the town whose ground `p` is on, if any.
    fn shapes_at(&self, p: Vec2) -> Option<&Shapes> {
        self.towns
            .iter()
            .filter(|g| {
                let c = Vec2::new(g.layout.centre.0, g.layout.centre.1);
                c.distance(p) < sim_core::layout::REACH
            })
            .min_by(|a, b| {
                let d = |g: &Ground| Vec2::new(g.layout.centre.0, g.layout.centre.1).distance(p);
                d(a).total_cmp(&d(b))
            })
            .and_then(|g| g.shapes.as_ref())
    }

    /// Where a walker heading for `target` stands: outside whatever stands
    /// there (a worker waits at their workplace's wall).
    pub fn clear(&self, target: Vec2) -> Vec2 {
        match self.shapes_at(target) {
            Some(s) => {
                let p = s.push_out(V2::new(target.x, target.y), WALKER, 0);
                Vec2::new(p.x, p.y)
            }
            None => target,
        }
    }

    /// A walker's step from `pos` to `next` on the way to `target`, `step`
    /// long: round a building in the way by its nearer corner, and never
    /// into one.
    pub fn step(&self, pos: Vec2, next: Vec2, target: Vec2, step: f32) -> Vec2 {
        let Some(s) = self.shapes_at(pos) else {
            return next;
        };
        let (me, to) = (V2::new(pos.x, pos.y), V2::new(next.x, next.y));
        if !s.any_near(me) && !s.any_near(to) {
            return next;
        }
        let mut to = to;
        // Only a short hop straight on is steered; a long walk follows the
        // town's roads (`terrain_view::walk`), which houses keep clear of.
        if pos.distance(target) < 160.0 {
            if let Some(c) = s.steer(me, V2::new(target.x, target.y), WALKER, 0) {
                to = me + (c - me).norm_or(V2::ZERO) * step.min((c - me).len());
            }
        }
        let p = s.resolve(me, to, WALKER, 0);
        Vec2::new(p.x, p.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::layout::building_footprint;

    fn ground_with(f: sim_core::layout::Footprint) -> TownGround {
        let layout = TownLayout {
            centre: (0.0, 0.0),
            buildings: vec![f],
            ..TownLayout::default()
        };
        TownGround {
            towns: vec![Ground {
                shapes: Some(layout.shapes()),
                layout,
                ..Ground::default()
            }],
        }
    }

    /// A walker going past a workplace walks round it, and one going to it
    /// stops at its wall.
    #[test]
    fn walkers_go_round_a_building_and_stop_at_its_wall() {
        let f = building_footprint(1, (0.0, 0.0), (50.0, 0.0), (24.0, 24.0));
        let g = ground_with(f);
        let target = Vec2::new(90.0, 0.0);
        let mut p = Vec2::new(10.0, 1.0);
        for _ in 0..400 {
            let step: f32 = 0.5;
            let next = p + (target - p).normalize_or_zero() * step.min(p.distance(target));
            p = g.step(p, next, target, step);
            assert!(!f.contains((p.x, p.y), WALKER - 0.05), "{p} inside");
        }
        assert!(p.distance(target) < 1.0, "stuck at {p}");
        let at = g.clear(Vec2::new(50.0, 0.0));
        assert!(!f.contains((at.x, at.y), WALKER - 0.05) && at.distance(Vec2::new(50.0, 0.0)) < 15.0);
    }
}
