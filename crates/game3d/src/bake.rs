//! Baking: many small meshes, each moved into place and given a colour,
//! appended into one mesh.
//!
//! Every mesh entity costs CPU time every frame (visibility, instance data,
//! draw commands), once for the main view and once per shadow cascade. A tree
//! as a trunk and a crown, or a house as walls, roof and chimney (each with an
//! outline hull), multiplies that. Baked, a forest chunk is one entity and a
//! building two (the body and its outline). The colour moves from the
//! material into a vertex colour, so one white material serves them all.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues as V};
use bevy::prelude::*;

/// Meshes moved into place and appended into one.
#[derive(Default)]
pub struct Baked {
    pub pos: Vec<[f32; 3]>,
    pub nrm: Vec<[f32; 3]>,
    pub col: Vec<[f32; 4]>,
    /// UV 0: x is how far up a swaying crown the vertex is (0 for still parts).
    pub up: Vec<[f32; 2]>,
    /// UV 1: the swaying tree's root (x, z), for its phase.
    pub root: Vec<[f32; 2]>,
    pub idx: Vec<u32>,
}

impl Baked {
    /// Append `mesh`, moved by `at`, in `colour` (linear RGBA). `sway`: the
    /// tree root, if this part sways (its height runs from the mesh's lowest
    /// to its highest point, 0 to 1).
    pub fn add(&mut self, mesh: &Mesh, at: Transform, colour: [f32; 4], sway: Option<Vec2>) {
        let Some(V::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            return;
        };
        let Some(V::Float32x3(nrm)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else {
            return;
        };
        let (lo, hi) = pos.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
            (lo.min(p[1]), hi.max(p[1]))
        });
        let base = self.pos.len() as u32;
        let m = at.to_matrix();
        for (p, n) in pos.iter().zip(nrm) {
            self.pos.push(m.transform_point3(Vec3::from(*p)).to_array());
            // Normals under a non-uniform scale: divide by the scale (the
            // inverse transpose of a scale), then rotate.
            let n = at.rotation * (Vec3::from(*n) / at.scale).normalize_or_zero();
            self.nrm.push(n.to_array());
            self.col.push(colour);
            let up = match sway {
                Some(_) => ((p[1] - lo) / (hi - lo).max(1e-6)).clamp(0.0, 1.0),
                None => 0.0,
            };
            self.up.push([up, 0.0]);
            self.root.push(sway.unwrap_or_default().to_array());
        }
        match mesh.indices() {
            Some(Indices::U16(i)) => self.idx.extend(i.iter().map(|&k| base + k as u32)),
            Some(Indices::U32(i)) => self.idx.extend(i.iter().map(|&k| base + k)),
            None => self.idx.extend(base..self.pos.len() as u32),
        }
    }

    pub fn mesh(self) -> Mesh {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.pos);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.nrm);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.col);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.up);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, self.root);
        mesh.insert_indices(Indices::U32(self.idx));
        mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baking_moves_scales_and_colours_every_part() {
        let cube = Mesh::from(Cuboid::new(1.0, 1.0, 1.0));
        let n = cube.count_vertices();
        let mut b = Baked::default();
        b.add(
            &cube,
            Transform::from_xyz(10.0, 0.0, 0.0),
            [1.0, 0.0, 0.0, 1.0],
            None,
        );
        b.add(
            &cube,
            Transform::from_scale(Vec3::new(2.0, 1.0, 1.0)),
            [0.0, 1.0, 0.0, 1.0],
            Some(Vec2::new(3.0, 4.0)),
        );
        assert_eq!(b.pos.len(), 2 * n);
        assert!(b.pos[..n].iter().all(|p| (p[0] - 10.0).abs() <= 0.5 + 1e-5));
        assert!(b.pos[n..].iter().all(|p| p[0].abs() <= 1.0 + 1e-5));
        assert_eq!(b.col[0], [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(b.col[n], [0.0, 1.0, 0.0, 1.0]);
        // Still parts don't sway; a swaying part runs from 0 to 1, rooted.
        assert!(b.up[..n].iter().all(|u| u[0] == 0.0));
        assert!(b.up[n..].iter().any(|u| u[0] == 1.0));
        assert_eq!(b.root[n], [3.0, 4.0]);
        // Normals stay unit length under the non-uniform scale.
        assert!(b
            .nrm
            .iter()
            .all(|v| (Vec3::from(*v).length() - 1.0).abs() < 1e-4));
        assert!(b.idx.iter().all(|&i| (i as usize) < b.pos.len()));
    }
}
