//! Collision against a map's triangle mesh.
//!
//! A map's collision hull is a triangle soup in the simulation's own frame (`+Z` up) and units
//! (centimetres), so standing on it and bumping into it are two queries over that soup:
//!
//! * [`Collision::ground`] answers "what is the floor under this spot", which is what a walker
//!   needs to step onto a stair, a kerb, or a slope and to fall off a ledge; and
//! * [`Collision::slide`] pushes a player's vertical capsule out of the walls it has run into,
//!   so a room has walls rather than a map-shaped hole in them.
//!
//! Both are the same two geometric tests — is a point over a triangle, and how far is it from one —
//! so a triangle whose normal is near-vertical is treated as floor and one whose normal is near-
//! horizontal as a wall. A uniform grid over the horizontal plane keeps each query to the handful
//! of triangles actually nearby, because a map is tens of thousands of them.
//!
//! Nothing here reads a clock or owns the player: it answers questions about a mesh, and the caller
//! decides what to do with the answers.

use std::collections::HashMap;

/// How high a step [`Collision::ground`] will lift a player onto, centimetres. A stair or a kerb
/// within this of the player's feet is floor; anything higher is a wall to be climbed or walked
/// around, not walked through.
const STEP_UP: f64 = 55.0;

/// The largest horizontal extent a triangle may span and still be gridded, in cells. A triangle
/// larger than this (a skybox shell, a stray vertex) is placed only by its centre, so one huge face
/// cannot flood the grid.
const MAX_TRIANGLE_CELLS: i64 = 4096;

/// The size of one grid cell, centimetres: about two metres, so a map's cells hold a few triangles.
const CELL: f64 = 200.0;

/// A mesh to stand on and bump into, in the simulation's frame and units (centimetres, `+Z` up).
#[derive(Debug, Default)]
pub struct Collision {
    verts: Vec<[f64; 3]>,
    tris: Vec<[u32; 3]>,
    cells: HashMap<(i32, i32), Vec<u32>>,
    /// Triangles too large to grid — a map's big floor slabs — which every query also considers.
    /// There are few of them, and leaving them out would put holes in the floor they cover.
    big: Vec<u32>,
}

impl Collision {
    /// Build a collision from a triangle mesh. `indices` names vertices three at a time; indices
    /// that leave `verts`, or triangles with a repeating vertex, are dropped rather than trusted.
    pub fn new(verts: Vec<[f64; 3]>, indices: &[u32]) -> Collision {
        let mut tris: Vec<[u32; 3]> = Vec::with_capacity(indices.len() / 3);
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut big: Vec<u32> = Vec::new();
        for chunk in indices.as_chunks::<3>().0 {
            let tri = [chunk[0], chunk[1], chunk[2]];
            let (Some(a), Some(b), Some(c)) = (
                verts.get(tri[0] as usize),
                verts.get(tri[1] as usize),
                verts.get(tri[2] as usize),
            ) else {
                continue;
            };
            let (a, b, c) = (*a, *b, *c);
            if tri[0] == tri[1] || tri[1] == tri[2] || tri[0] == tri[2] {
                continue;
            }
            let t = tris.len() as u32;
            match cells_of(a, b, c) {
                Some(corners) => {
                    for corner in corners {
                        cells.entry(corner).or_default().push(t);
                    }
                }
                None => big.push(t),
            }
            tris.push(tri);
        }
        Collision {
            verts,
            tris,
            cells,
            big,
        }
    }

    /// Whether the collision has any triangles at all.
    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// How many triangles the collision holds.
    pub fn triangle_count(&self) -> usize {
        self.tris.len()
    }

    /// The lowest point of the hull, `None` when it is empty. Below this there is nothing to land
    /// on, so a walker that gets there has left the world.
    pub fn lowest(&self) -> Option<f64> {
        self.verts.iter().map(|v| v[2]).reduce(f64::min)
    }

    /// The floor under `at`, within `reach` horizontally, or `None` when there is nothing there.
    ///
    /// The answer is the *highest* floor triangle the player is over or within `reach` of,
    /// provided it is no more than [`STEP_UP`] above the feet: that is what lifts a walker onto a
    /// stair and what a floor above cannot do (a ceiling is not a place to stand). The reach is the
    /// width of a capsule, so a foot does not fall through the seam between two floor faces.
    pub fn ground(&self, at: [f64; 3], reach: f64) -> Option<f64> {
        self.floor_point(at, reach).map(|point| point[2])
    }

    /// The point of the floor under `at`, within `reach` horizontally: the closest point of the
    /// highest floor triangle the spot is over or near. Anchoring on this point rather than on a
    /// spot that is *over* floor puts a map up with the player standing on it.
    pub fn floor_point(&self, at: [f64; 3], reach: f64) -> Option<[f64; 3]> {
        let mut best: Option<[f64; 3]> = None;
        for t in self.near(at[0], at[1], reach) {
            let (a, b, c) = self.corners(t);
            if !is_floor(a, b, c) {
                continue;
            }
            let closest = closest_on_triangle_xz(a, b, c, at[0], at[1]);
            let distance = ((closest[0] - at[0]).powi(2) + (closest[1] - at[1]).powi(2)).sqrt();
            if distance > reach {
                continue;
            }
            let Some(z) = plane_height(a, b, c, closest[0], closest[1]) else {
                continue;
            };
            if z <= at[2] + STEP_UP && best.is_none_or(|b| z > b[2]) {
                best = Some([closest[0], closest[1], z]);
            }
        }
        best
    }

    /// Move `at` out of the walls its vertical capsule (`radius` wide, `height` tall from the feet)
    /// has run into. The feet stay at their height: pushing out is horizontal, and the floor is the
    /// ground query's job.
    pub fn slide(&self, at: [f64; 3], radius: f64, height: f64) -> [f64; 3] {
        let mut p = at;
        // A corner has two walls and a wall's neighbours help settle it, so a couple of passes.
        for _ in 0..3 {
            let mut moved = false;
            let foot = p[2] + 1.0;
            let head = p[2] + height;
            for t in self.near(p[0], p[1], radius + CELL) {
                let (a, b, c) = self.corners(t);
                if is_floor(a, b, c) {
                    continue;
                }
                let low = a[2].min(b[2]).min(c[2]);
                let high = a[2].max(b[2]).max(c[2]);
                // A step the ground query would lift the player onto is not a wall: it is only
                // something to run into if it stands more than a step above the feet. Nor is
                // anything entirely above the head.
                if high <= foot + STEP_UP || low > head {
                    continue;
                }
                let closest = closest_on_triangle_xz(a, b, c, p[0], p[1]);
                let (dx, dy) = (p[0] - closest[0], p[1] - closest[1]);
                let distance = (dx * dx + dy * dy).sqrt();
                if distance >= radius {
                    continue;
                }
                let (nx, ny) = if distance > 1e-6 {
                    (dx / distance, dy / distance)
                } else {
                    outward_normal(a, b, c, p)
                };
                let push = radius - distance;
                p[0] += nx * push;
                p[1] += ny * push;
                moved = true;
            }
            if !moved {
                break;
            }
        }
        p
    }

    /// The three corners of triangle `t`.
    fn corners(&self, t: u32) -> ([f64; 3], [f64; 3], [f64; 3]) {
        let [a, b, c] = self.tris[t as usize];
        (
            self.verts[a as usize],
            self.verts[b as usize],
            self.verts[c as usize],
        )
    }

    /// The triangles whose grid cell the horizontal point `(x, z)` falls in, within `pad`, plus the
    /// oversized ones that are never in a cell.
    fn near(&self, x: f64, z: f64, pad: f64) -> Vec<u32> {
        let lo = ((x - pad) / CELL).floor() as i32;
        let hi = ((x + pad) / CELL).floor() as i32;
        let zlo = ((z - pad) / CELL).floor() as i32;
        let zhi = ((z + pad) / CELL).floor() as i32;
        let mut found: Vec<u32> = Vec::new();
        let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for cx in lo..=hi {
            for cz in zlo..=zhi {
                if let Some(bucket) = self.cells.get(&(cx, cz)) {
                    for t in bucket {
                        if seen.insert(*t) {
                            found.push(*t);
                        }
                    }
                }
            }
        }
        for t in &self.big {
            if seen.insert(*t) {
                found.push(*t);
            }
        }
        found
    }
}

/// The grid cells a triangle's horizontal box covers, or `None` when it is too large to grid (it is
/// then kept aside so every query still sees it).
fn cells_of(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Option<Vec<(i32, i32)>> {
    let min_x = a[0].min(b[0]).min(c[0]);
    let max_x = a[0].max(b[0]).max(c[0]);
    let min_z = a[1].min(b[1]).min(c[1]);
    let max_z = a[1].max(b[1]).max(c[1]);
    let lo_x = (min_x / CELL).floor() as i64;
    let hi_x = (max_x / CELL).floor() as i64;
    let lo_z = (min_z / CELL).floor() as i64;
    let hi_z = (max_z / CELL).floor() as i64;
    if (hi_x - lo_x + 1) * (hi_z - lo_z + 1) > MAX_TRIANGLE_CELLS {
        return None;
    }
    let mut out = Vec::new();
    for cx in lo_x..=hi_x {
        for cz in lo_z..=hi_z {
            out.push((cx as i32, cz as i32));
        }
    }
    Some(out)
}

/// The unit normal of a triangle.
fn normal(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len <= 1e-12 {
        [0.0, 0.0, 0.0]
    } else {
        [n[0] / len, n[1] / len, n[2] / len]
    }
}

/// Whether a triangle is floor: its normal points mostly up or down, i.e. it is walkable rather
/// than a wall to be pushed out of.
fn is_floor(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> bool {
    normal(a, b, c)[2].abs() > 0.5
}

/// The height of the triangle's plane at horizontal `(x, y)`, when that point is over the triangle.
fn plane_height(a: [f64; 3], b: [f64; 3], c: [f64; 3], x: f64, y: f64) -> Option<f64> {
    // Barycentric coordinates in the horizontal plane; inside when all three are non-negative.
    let area = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
    if area.abs() <= 1e-9 {
        return None;
    }
    let wb = ((x - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (y - a[1])) / area;
    let wc = ((b[0] - a[0]) * (y - a[1]) - (x - a[0]) * (b[1] - a[1])) / area;
    let wa = 1.0 - wb - wc;
    let e = -1e-6;
    if wa < e || wb < e || wc < e {
        return None;
    }
    Some(wa * a[2] + wb * b[2] + wc * c[2])
}

/// The point of a triangle's horizontal projection closest to `(x, z)`, and whether `(x, z)` is
/// inside that projection.
fn closest_on_triangle_xz(a: [f64; 3], b: [f64; 3], c: [f64; 3], x: f64, z: f64) -> [f64; 2] {
    // Inside test first: a point over the triangle is its own closest point.
    let sign = |p: [f64; 3], q: [f64; 3]| (q[0] - p[0]) * (z - p[1]) - (q[1] - p[1]) * (x - p[0]);
    let (s0, s1, s2) = (sign(a, b), sign(b, c), sign(c, a));
    let inside = (s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0) || (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0);
    if inside {
        return [x, z];
    }
    [a, b, c]
        .iter()
        .zip([b, c, a].iter())
        .map(|(p, q)| closest_on_segment_xz(*p, *q, x, z))
        .min_by(|u, v| {
            let du = (u[0] - x).powi(2) + (u[1] - z).powi(2);
            let dv = (v[0] - x).powi(2) + (v[1] - z).powi(2);
            du.total_cmp(&dv)
        })
        .unwrap_or([x, z])
}

/// The point of segment `pq` closest to `(x, z)`, in the horizontal plane.
fn closest_on_segment_xz(p: [f64; 3], q: [f64; 3], x: f64, z: f64) -> [f64; 2] {
    let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
    let len2 = dx * dx + dz * dz;
    if len2 <= 1e-12 {
        return [p[0], p[1]];
    }
    let t = (((x - p[0]) * dx + (z - p[1]) * dz) / len2).clamp(0.0, 1.0);
    [p[0] + dx * t, p[1] + dz * t]
}

/// The horizontal normal of a triangle, oriented away from `point`, for a point already on it.
fn outward_normal(a: [f64; 3], b: [f64; 3], c: [f64; 3], point: [f64; 3]) -> (f64, f64) {
    let n = normal(a, b, c);
    let centre = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0];
    let mut nx = n[0];
    let mut ny = n[1];
    if (point[0] - centre[0]) * nx + (point[1] - centre[1]) * ny < 0.0 {
        nx = -nx;
        ny = -ny;
    }
    let len = (nx * nx + ny * ny).sqrt();
    if len <= 1e-9 {
        (1.0, 0.0)
    } else {
        (nx / len, ny / len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat floor at `z`, ten metres square, as a two-triangle mesh.
    fn floor(z: f64) -> Collision {
        let verts = vec![
            [0.0, 0.0, z],
            [1000.0, 0.0, z],
            [1000.0, 1000.0, z],
            [0.0, 1000.0, z],
        ];
        Collision::new(verts, &[0, 1, 2, 0, 2, 3])
    }

    /// A wall standing in the plane `x = 100`, two metres tall, from `z = 0`.
    fn wall() -> Collision {
        let verts = vec![
            [100.0, -500.0, 0.0],
            [100.0, 500.0, 0.0],
            [100.0, 500.0, 200.0],
            [100.0, -500.0, 200.0],
        ];
        Collision::new(verts, &[0, 1, 2, 0, 2, 3])
    }

    /// Standing on a floor: the ground query finds its height, and the wall query leaves the player
    /// alone (a floor is not a wall).
    #[test]
    fn a_floor_is_stood_on_and_not_pushed_against() {
        let floor = floor(10.0);
        assert_eq!(floor.ground([500.0, 500.0, 10.0], 34.0), Some(10.0));
        assert_eq!(
            floor.slide([500.0, 500.0, 10.0], 34.0, 180.0),
            [500.0, 500.0, 10.0]
        );
    }

    /// A wall pushes a capsule out to its own radius, and only horizontally.
    #[test]
    fn a_wall_is_pushed_out_to_the_capsule_radius() {
        let wall = wall();
        let moved = wall.slide([95.0, 0.0, 0.0], 34.0, 180.0);
        assert!(
            (moved[0] - 66.0).abs() < 1e-6,
            "pushed to x {}, expected 66",
            moved[0]
        );
        assert_eq!(moved[1], 0.0, "the push is horizontal");
        assert_eq!(moved[2], 0.0, "the feet stay put");
    }

    /// The ground query is a step, not a wall: a floor within [`STEP_UP`] of the feet is stood on,
    /// and one higher up is not (it is a ceiling, not a place to stand).
    #[test]
    fn ground_reaches_a_stair_but_not_a_ceiling() {
        let mut verts = vec![
            [0.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            [1000.0, 1000.0, 0.0],
            [0.0, 1000.0, 0.0],
        ];
        // A landing 40 cm up over the second half of the floor.
        verts.extend([
            [0.0, 500.0, 40.0],
            [1000.0, 500.0, 40.0],
            [1000.0, 1000.0, 40.0],
            [0.0, 1000.0, 40.0],
        ]);
        let mesh = Collision::new(verts, &[0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]);
        // Over the landing, a walker at 0 steps up onto it.
        assert_eq!(mesh.ground([500.0, 750.0, 0.0], 34.0), Some(40.0));
        // Over the low floor, the landing does not reach.
        assert_eq!(mesh.ground([500.0, 250.0, 0.0], 34.0), Some(0.0));
        // From high above, both are below and the higher one is the floor stood on.
        assert_eq!(mesh.ground([500.0, 750.0, 1000.0], 34.0), Some(40.0));
    }

    /// A wall's height matters: a step the ground query would lift the player onto is not a wall,
    /// and a wall entirely above the head is not either, but one that stands taller than a step
    /// does block.
    #[test]
    fn a_wall_below_the_feet_does_not_push() {
        let tall = vec![
            [100.0, -500.0, 0.0],
            [100.0, 500.0, 0.0],
            [100.0, 500.0, 300.0],
            [100.0, -500.0, 300.0],
        ];
        let tall = Collision::new(tall, &[0, 1, 2, 0, 2, 3]);
        // Feet 400 cm up: the wall's top is under them, not in front.
        assert_eq!(
            tall.slide([95.0, 0.0, 400.0], 34.0, 180.0),
            [95.0, 0.0, 400.0]
        );
        // And a step short enough to be walked onto is not a wall either.
        let step = vec![
            [100.0, -500.0, 0.0],
            [100.0, 500.0, 0.0],
            [100.0, 500.0, 40.0],
            [100.0, -500.0, 40.0],
        ];
        let step = Collision::new(step, &[0, 1, 2, 0, 2, 3]);
        assert_eq!(step.slide([95.0, 0.0, 0.0], 34.0, 180.0), [95.0, 0.0, 0.0]);
    }
}
