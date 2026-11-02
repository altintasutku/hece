//! The wireframe: an icosphere subdivided twice (162 vertices, 480 edges) on the unit sphere.

use std::collections::HashMap;

/// Unit-sphere vertices and the edges between them.
#[derive(Debug)]
pub struct Sphere {
    /// Points on the unit sphere.
    pub vertices: Vec<[f32; 3]>,
    /// Index pairs into `vertices`.
    pub edges: Vec<(usize, usize)>,
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / len, v[1] / len, v[2] / len]
}

impl Sphere {
    /// Builds the sphere.
    #[must_use]
    pub fn new() -> Self {
        let p = f32::midpoint(1.0, 5.0_f32.sqrt());
        let mut vertices: Vec<[f32; 3]> = [
            [-1.0, p, 0.0],
            [1.0, p, 0.0],
            [-1.0, -p, 0.0],
            [1.0, -p, 0.0],
            [0.0, -1.0, p],
            [0.0, 1.0, p],
            [0.0, -1.0, -p],
            [0.0, 1.0, -p],
            [p, 0.0, -1.0],
            [p, 0.0, 1.0],
            [-p, 0.0, -1.0],
            [-p, 0.0, 1.0],
        ]
        .into_iter()
        .map(normalize)
        .collect();
        let mut faces: Vec<[usize; 3]> = vec![
            [0, 11, 5],
            [0, 5, 1],
            [0, 1, 7],
            [0, 7, 10],
            [0, 10, 11],
            [1, 5, 9],
            [5, 11, 4],
            [11, 10, 2],
            [10, 7, 6],
            [7, 1, 8],
            [3, 9, 4],
            [3, 4, 2],
            [3, 2, 6],
            [3, 6, 8],
            [3, 8, 9],
            [4, 9, 5],
            [2, 4, 11],
            [6, 2, 10],
            [8, 6, 7],
            [9, 8, 1],
        ];
        for _ in 0..2 {
            let mut middles: HashMap<(usize, usize), usize> = HashMap::new();
            let mut middle = |a: usize, b: usize, vertices: &mut Vec<[f32; 3]>| {
                *middles.entry((a.min(b), a.max(b))).or_insert_with(|| {
                    let (va, vb) = (vertices[a], vertices[b]);
                    vertices.push(normalize([
                        f32::midpoint(va[0], vb[0]),
                        f32::midpoint(va[1], vb[1]),
                        f32::midpoint(va[2], vb[2]),
                    ]));
                    vertices.len() - 1
                })
            };
            faces = faces
                .iter()
                .flat_map(|&[a, b, c]| {
                    let ab = middle(a, b, &mut vertices);
                    let bc = middle(b, c, &mut vertices);
                    let ca = middle(c, a, &mut vertices);
                    [[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]
                })
                .collect();
        }
        let mut edges: Vec<(usize, usize)> = faces
            .iter()
            .flat_map(|&[a, b, c]| [(a, b), (b, c), (c, a)])
            .map(|(a, b)| (a.min(b), a.max(b)))
            .collect();
        edges.sort_unstable();
        edges.dedup();
        Self { vertices, edges }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icosphere_has_expected_size() {
        let sphere = Sphere::new();
        assert_eq!(sphere.vertices.len(), 162);
        assert_eq!(sphere.edges.len(), 480);
        for v in &sphere.vertices {
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-5);
        }
    }
}
