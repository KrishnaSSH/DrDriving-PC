//! Wavefront OBJ models as the original game loads them: positions + texture coordinates only,
//! faces taken as triangles, `v` flipped (`1 - vt.y`), no lighting.

use macroquad::models::{draw_mesh, Mesh, Vertex};
use macroquad::prelude::*;

pub struct Model {
    pos: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
}

fn index(tok: &str, count: usize) -> usize {
    let i: i32 = tok.parse().unwrap_or(1);
    if i < 0 {
        (count as i32 + i) as usize
    } else {
        (i - 1) as usize
    }
}

impl Model {
    pub fn parse(text: &str) -> Model {
        let mut v: Vec<[f32; 3]> = Vec::new();
        let mut vt: Vec<[f32; 2]> = Vec::new();
        let mut pos = Vec::new();
        let mut uv = Vec::new();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            match it.next() {
                Some("v") => {
                    let p: Vec<f32> = it.filter_map(|s| s.parse().ok()).collect();
                    if p.len() >= 3 {
                        v.push([p[0], p[1], p[2]]);
                    }
                }
                Some("vt") => {
                    let p: Vec<f32> = it.filter_map(|s| s.parse().ok()).collect();
                    if p.len() >= 2 {
                        vt.push([p[0], 1.0 - p[1]]);
                    }
                }
                Some("f") => {
                    for corner in it.take(3) {
                        let mut parts = corner.split('/');
                        let vi = index(parts.next().unwrap_or("1"), v.len());
                        pos.push(v.get(vi).copied().unwrap_or([0.0; 3]));
                        let t = parts.next().filter(|s| !s.is_empty()).map(|s| index(s, vt.len()));
                        uv.push(t.and_then(|i| vt.get(i).copied()).unwrap_or([0.0, 0.0]));
                    }
                }
                _ => {}
            }
        }
        Model { pos, uv }
    }

    pub fn load(path: &std::path::Path) -> Model {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        Self::parse(&text)
    }

    /// Draw with `m` as the model matrix (GL `glPushMatrix`/`glTranslatef`/`glRotatef` stack).
    pub fn draw(&self, tex: &Texture2D, m: &Mat4, tint: Color) {
        const CHUNK: usize = 4998; // macroquad's batch holds 5000 indices
        let mut start = 0;
        while start < self.pos.len() {
            let end = (start + CHUNK).min(self.pos.len());
            let mut vertices = Vec::with_capacity(end - start);
            for i in start..end {
                let p = m.transform_point3(vec3(self.pos[i][0], self.pos[i][1], self.pos[i][2]));
                vertices.push(Vertex::new2(p, vec2(self.uv[i][0], self.uv[i][1]), tint));
            }
            let indices: Vec<u16> = (0..vertices.len() as u16).collect();
            draw_mesh(&Mesh { vertices, indices, texture: Some(tex.clone()) });
            start = end;
        }
    }
}

/// `glTranslatef(x,y,z)` then `glRotatef(deg, 0,1,0)`.
pub fn at(x: f32, y: f32, z: f32, yaw_deg: f32) -> Mat4 {
    Mat4::from_translation(vec3(x, y, z)) * Mat4::from_rotation_y(yaw_deg.to_radians())
}
