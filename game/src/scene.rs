//! 3D scene rendering: a port of `drdriving.m` (`renderObject` and the camera rigs).

use crate::assets::Assets;
use crate::gfx::Gfx;
use macroquad::miniquad::{BlendFactor, BlendState, BlendValue, Comparison, CullFace, Equation, FrontFaceOrder, PipelineParams};
use crate::model::at;
use drdriving_core::cars::car;
use drdriving_core::mission::Mission;
use drdriving_core::world::World;
use macroquad::prelude::*;

const DEG: f32 = std::f32::consts::PI / 180.0;

const VERT: &str = "#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
varying lowp vec2 uv;
varying lowp vec4 color;
uniform mat4 Model;
uniform mat4 Projection;
void main() {
    gl_Position = Projection * Model * vec4(position, 1);
    color = color0 / 255.0;
    uv = texcoord;
}";
const FRAG: &str = "#version 100
varying lowp vec4 color;
varying lowp vec2 uv;
uniform sampler2D Texture;
void main() {
    gl_FragColor = color * texture2D(Texture, uv);
}";

/// The original draws with `GL_CULL_FACE` (CCW front faces); the mirror cameras use a negative
/// aspect ratio, which flips handedness, so they get the opposite winding.
pub struct Materials {
    /// no blending: for surfaces that are always opaque (sky dome, road)
    pub opaque: Material,
    pub normal: Material,
    pub mirrored: Material,
}

fn material(front: FrontFaceOrder, blend: bool) -> Material {
    load_material(
        ShaderSource::Glsl { vertex: VERT, fragment: FRAG },
        MaterialParams {
            pipeline_params: PipelineParams {
                depth_test: Comparison::LessOrEqual,
                depth_write: true,
                cull_face: CullFace::Back,
                front_face_order: front,
                color_blend: blend.then(|| BlendState::new(Equation::Add, BlendFactor::Value(BlendValue::SourceAlpha), BlendFactor::OneMinusValue(BlendValue::SourceAlpha))),
                alpha_blend: blend.then(|| BlendState::new(Equation::Add, BlendFactor::One, BlendFactor::OneMinusValue(BlendValue::SourceAlpha))),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .expect("3D material")
}

pub fn make_materials() -> Materials {
    Materials {
        opaque: material(FrontFaceOrder::CounterClockwise, false),
        normal: material(FrontFaceOrder::CounterClockwise, true),
        mirrored: material(FrontFaceOrder::Clockwise, true),
    }
}

#[derive(Clone, Copy)]
pub struct DriftMark {
    pub x: f32,
    pub y: f32,
    pub angle: f32,
}

/// Ring of recent skid marks (`tool.s`, 50 entries).
#[derive(Default)]
pub struct DriftMarks {
    pub marks: Vec<DriftMark>,
    next: usize,
}

impl DriftMarks {
    pub fn clear(&mut self) {
        self.marks.clear();
        self.next = 0;
    }
    pub fn push(&mut self, m: DriftMark) {
        if self.marks.len() < 50 {
            self.marks.push(m);
        } else {
            self.marks[self.next] = m;
        }
        self.next = (self.next + 1) % 50;
    }
}

/// A view into the world: where the camera is and what it uses to render.
pub struct View {
    pub cam: Camera3D,
    /// Extra cells drawn around the player (`renderObject(cam, radius)`).
    pub radius: i32,
}

fn camera(fov_deg: f32, near: f32, far: f32, pos: Vec3, target: Vec3, up: Vec3, aspect: Option<f32>) -> Camera3D {
    Camera3D {
        position: pos,
        target,
        up,
        fovy: fov_deg.to_radians(),
        aspect,
        projection: Projection::Perspective,
        render_target: None,
        viewport: None,
        z_near: near,
        z_far: far,
    }
}

/// Full-layout viewport with the window's aspect (cams 0-3, ending, success, car select).
fn with_full(mut cam: Camera3D, g: &Gfx) -> Camera3D {
    cam.viewport = Some(g.viewport_px(0.0, 0.0, g.vw, g.sh));
    cam.aspect = Some(g.vw / g.sh);
    cam
}

/// Chase cameras 2 (far) and 3 (near): behind the car along its (slightly lagged) heading.
pub fn chase_view(w: &World, idx: i32, g: &Gfx) -> View {
    let (fov, near, far, h, look_y, dist) = match idx {
        2 => (20.0, 6.0, 210.0, 12.0, 1.0, 40.0),
        _ => (20.0, 6.0, 210.0, 5.0, 1.0, 19.5),
    };
    let a = w.poslist.last_angle(0.09) * DEG;
    let p = &w.player;
    let pos = vec3(p.cx - dist * a.sin(), h, p.cy + dist * a.cos());
    View { cam: with_full(camera(fov, near, far, pos, vec3(p.cx, look_y, p.cy), Vec3::Y, None), g), radius: 0 }
}

/// Driver's-seat camera (mode 4): eye placed ahead of the rear axle, leaning with inertia.
pub fn driver_view(w: &World, right_driver: bool, g: &Gfx) -> View {
    let p = &w.player;
    let info = car(p.kind);
    let eye = info.eye;
    let eh = info.eye_height;
    let e1 = eye - 1.0;
    let mut pos = vec3(p.cx * eye - p.x * e1, eh, p.cy * eye - p.y * e1);
    let mut tgt = vec3(p.x * -7.5 - p.cx * -8.5, eh, p.y * -7.5 - p.cy * -8.5);
    let mut side = vec2(p.cx - p.x, p.cy - p.y);
    side = vec2(-side.y, side.x); // rotate 90 degrees
    let s = if right_driver { 1.0 } else { -1.0 };
    pos.x += s * side.x * 0.12;
    pos.z += s * side.y * 0.12;
    tgt.x += s * side.x * 0.08;
    tgt.z += s * side.y * 0.08;
    let up = vec3(-w.vec_inertia.x * 0.005, 1.0, -w.vec_inertia.y * 0.005);
    let mut cam = camera(45.0, 0.8, 210.0, pos, tgt, up, None);
    // the original lifts the 3D view W/8 above the bottom edge (the dashboard covers it) and, on
    // 16:9-ish screens, uses a fixed 1.47 aspect with a taller-than-screen viewport
    if g.sh >= 400.0 {
        cam.viewport = Some(g.viewport_px(0.0, 100.0, g.vw, g.vw * 17.0 / 25.0));
        cam.aspect = Some(1.47);
    } else {
        cam.viewport = Some(g.viewport_px(0.0, 100.0, g.vw, g.sh));
        cam.aspect = Some(g.vw / g.sh);
    }
    View { cam, radius: 0 }
}

pub fn ending_view(w: &World, t: f32, g: &Gfx) -> View {
    let f4 = (t + 15.0).min(20.0);
    let p = &w.player;
    let pos = vec3(p.cx - (0.3 * t).cos() * f4, 0.2 * f4, p.cy + f4 * (0.3 * t).sin());
    View { cam: with_full(camera(20.0, 5.0, 280.0, pos, vec3(p.cx, 1.0, p.cy), Vec3::Y, None), g), radius: 1 }
}

/// Fixed spot just beside the road, chosen when the mission is completed.
pub fn success_pos(w: &World) -> Vec3 {
    let p = &w.player;
    vec3(p.cx + p.v.x * 0.9 + w.curr_dir.y as f32 * 16.0, 4.0, p.cy + p.v.y * 0.9 + w.curr_dir.x as f32 * 16.0)
}

pub fn success_view(w: &World, pos: Vec3, g: &Gfx) -> View {
    let p = &w.player;
    View { cam: with_full(camera(25.0, 1.0, 280.0, pos, vec3(p.cx, 1.0, p.cy), Vec3::Y, None), g), radius: 1 }
}

/// Car-select screen: slow orbit around the parked car.
pub fn carselect_view(w: &World, t: f32, right_driver: bool, g: &Gfx) -> View {
    let a = t * 0.1;
    let s = if right_driver { 1.0 } else { -1.0 };
    let pos = vec3(s * (4.8 - 10.0 * a.cos()) + 1960.0, 3.0, 1984.0 - 10.0 * a.sin());
    View { cam: with_full(camera(36.0, 2.0, 210.0, pos, vec3(w.player.cx, 0.5, w.player.cy), Vec3::Y, None), g), radius: 0 }
}

/// Rear-view and side mirrors (cameras `camBack`, `camLeft`, `camRight`).
pub fn mirror_view(w: &World, which: u8) -> View {
    let p = &w.player;
    let eh = car(p.kind).eye_height;
    match which {
        0 => {
            let pos = vec3(p.x * -1.5 + p.cx * 2.5, eh, p.y * -1.5 + p.cy * 2.5);
            let tgt = vec3(p.cx * -1.5 + p.x * 2.5, eh, p.cy * -1.5 + p.y * 2.5);
            View { cam: camera(15.0, 4.0, 210.0, pos, tgt, Vec3::Y, Some(-2.5)), radius: 0 }
        }
        _ => {
            let mut v = vec2(p.cx - p.x, p.cy - p.y);
            v = vec2(-v.y, v.x);
            let k = (p.half_width * 1.42) / p.pitch;
            let mut pos = vec3(p.x * -0.5 + p.cx * 1.5, eh - 0.2, p.y * -0.5 + p.cy * 1.5);
            let mut tgt = vec3(p.cx * -0.5 + p.x * 1.5, eh - 0.2, p.cy * -0.5 + p.y * 1.5);
            let sgn = if which == 1 { -1.0 } else { 1.0 };
            pos.x += sgn * v.x * k;
            pos.z += sgn * v.y * k;
            tgt.x += sgn * v.x * k;
            tgt.z += sgn * v.y * k;
            View { cam: camera(30.0, 0.1, 210.0, pos, tgt, Vec3::Y, Some(-1.5)), radius: 0 }
        }
    }
}

fn draw_wheels(a: &Assets, cx: f32, cz: f32, yaw: f32, h: f32, front: f32, back: f32, ratio: f32, spin: f32) {
    draw_wheels_tinted(a, cx, cz, yaw, h, front, back, ratio, spin, WHITE);
}

#[allow(clippy::too_many_arguments)]
fn draw_wheels_tinted(a: &Assets, cx: f32, cz: f32, yaw: f32, h: f32, front: f32, back: f32, ratio: f32, spin: f32, tint: Color) {
    let base = at(cx, h, cz, yaw);
    let scale = Mat4::from_scale(vec3(ratio, h, h));
    let spin = Mat4::from_rotation_x(spin * DEG);
    for z in [front, -back] {
        let m = base * Mat4::from_translation(vec3(0.0, 0.0, z)) * spin * scale;
        a.road_model[7].draw(&a.road_tex[7], &m, tint);
    }
}

/// Draws the whole scene for the active camera (`renderObject`).
pub fn render_world(a: &Assets, w: &World, right_driver: bool, view: &View, drifts: &DriftMarks) {
    set_camera(&view.cam);
    if view.cam.render_target.is_some() {
        clear_background(Color::new(0.0, 0.0, 0.0, 1.0));
    }
    let mirrored = view.cam.aspect.map_or(false, |a| a < 0.0);
    gl_use_material(if mirrored { &a.mats.mirrored } else { &a.mats.normal });
    let cam = view.cam.position;
    let rm = &a.road_model;
    let rt = &a.road_tex;

    // which strip of city blocks is visible: along the direction of travel
    let (cxi, cyi) = (w.player.cell_x, w.player.cell_y);
    let near_junction = (w.g_cx < 30.0 && w.g_cy < 30.0) || (w.g_dx < 30.0 && w.g_dy < 30.0);
    let heading = if near_junction { w.player.angle } else { (w.curr_dir.index() * 90) as f32 };
    let r = view.radius;
    let (mut x0, mut y0, mut x1, mut y1) = (cxi - r, cyi - r, cxi + r, cyi + r);
    if !(45.0..=315.0).contains(&heading) {
        y0 = cyi - 1;
        y1 = cyi + 1;
    } else if heading < 135.0 {
        x1 = cxi + 1;
        x0 = cxi - 1;
    } else if heading < 225.0 {
        y1 = cyi + 1;
        y0 = cyi - 1;
    } else {
        x0 = cxi - 1;
        x1 = cxi + 1;
    }

    // sky dome follows the camera
    rm[3].draw(&rt[3], &Mat4::from_translation(vec3(cam.x, 0.0, cam.z)), WHITE);

    // traffic lights at every corner of each block
    let rd_flag = if right_driver { 180.0 } else { 0.0 };
    let lights_on = ((w.travel_time * 3.0) as i32) % 3 != 0;
    let (lt, lm) = (&rt[if lights_on { 5 } else { 4 }], &rm[if right_driver { 5 } else { 4 }]);
    for i in 0..8 {
        let z = match i {
            0 | 1 => -15.0,
            4 | 5 => 15.0,
            3 | 6 => 7.5,
            _ => -7.5,
        };
        let x = match i {
            2 | 3 => -15.0,
            6 | 7 => 15.0,
            0 | 5 => 7.5,
            _ => -7.5,
        };
        for cx in x0..=x1 {
            for cy in y0..=y1 {
                let yaw = ((i / 2) * 90 + (i % 2) * 180) as f32 + rd_flag;
                lm.draw(lt, &at((cx * 140) as f32 + x, 0.0, (cy * 140) as f32 + z, yaw), WHITE);
            }
        }
    }

    // roads
    for cx in x0..=x1 {
        for cy in y0..=y1 {
            rm[0].draw(&rt[0], &at((cx * 140) as f32, 0.0, (cy * 140) as f32, 0.0), WHITE);
        }
    }
    // buildings alternate between two facades
    for cx in x0..=x1 {
        for cy in y0..=y1 {
            let tex = if (cx + cy) % 2 == 0 { &rt[13] } else { &rt[14] };
            rm[18].draw(tex, &at((cx * 140) as f32, 0.0, (cy * 140) as f32, 0.0), WHITE);
        }
    }

    // parking spot and checkpoint
    if w.parking.active {
        let m = if w.in_parking_lot { &rm[13] } else { &rm[12] };
        let rect = &w.parking.rect;
        let yaw = if rect.hy < rect.hx { 90.0 } else { 0.0 };
        m.draw(&rt[12], &at(rect.c.x, 0.0, rect.c.y, yaw), WHITE);
    }
    if w.checkpoint.active {
        let rect = &w.checkpoint.rect;
        let yaw = if rect.hy < rect.hx { 90.0 } else { 0.0 };
        rm[2].draw(&rt[2], &at(rect.c.x, 0.0, rect.c.y, yaw), WHITE);
    }

    // barricades: two posts per arm
    for b in w.barricades.iter().filter(|b| b.active) {
        let horiz = b.rect.hy < b.rect.hx;
        for d in [-5.0, 5.0] {
            let m = if horiz {
                at(b.rect.c.x + d, 0.0, b.rect.c.y, 90.0)
            } else {
                at(b.rect.c.x, 0.0, b.rect.c.y + d, 0.0)
            };
            rm[1].draw(&rt[1], &m, WHITE);
        }
    }

    // skid marks
    for d in &drifts.marks {
        rm[6].draw(&rt[6], &at(d.x, 0.0, d.y, d.angle), WHITE);
    }

    // traffic, brake lights, wheels
    for c in w.cars.iter().filter(|c| c.visible) {
        let m = at(c.rec.c.x, 0.0, c.rec.c.y, 180.0 - c.angle as f32);
        a.car_model[c.kind].draw(&a.car_tex[c.kind], &m, WHITE);
        if c.braking {
            a.light_model[c.kind].draw(&rt[9], &m, WHITE);
        }
        draw_wheels(a, c.rec.c.x, c.rec.c.y, 180.0 - c.angle as f32, c.wheel_height, c.wheel_front, c.wheel_back, c.wheel_width_ratio, c.ang_distance);
    }
    let p = &w.player;
    let m = at(p.cx, 0.0, p.cy, 180.0 - p.angle);
    a.car_model[p.kind].draw(&a.car_tex[p.kind], &m, WHITE);
    if p.braking {
        a.light_model[p.kind].draw(&rt[9], &m, WHITE);
    }
    draw_wheels(a, p.cx, p.cy, 180.0 - p.angle, p.wheel_height, car(p.kind).wheel_front, car(p.kind).wheel_back, car(p.kind).wheel_width_ratio, p.ang_distance);

    // the remote opponent (`renderOpponentCar`): drawn with glColor4f(0.2, 0.2, 0.2, 0.2), a faint
    // dark shadow of a car that never collides with ours
    if w.online {
        let o = &w.opponent;
        let ghost = Color::new(0.2, 0.2, 0.2, 0.2);
        let m = at(o.cx, 0.0, o.cy, 180.0 - o.angle);
        a.car_model[o.kind].draw(&a.car_tex[o.kind], &m, ghost);
        draw_wheels_tinted(a, o.cx, o.cy, 180.0 - o.angle, o.wheel_height, car(o.kind).wheel_front, car(o.kind).wheel_back, car(o.kind).wheel_width_ratio, o.ang_distance, ghost);
    }

    // roadside fences
    for cx in x0..=x1 {
        for cy in y0..=y1 {
            rm[19].draw(&rt[13], &at((cx * 140) as f32, 0.0, (cy * 140) as f32, 0.0), WHITE);
        }
    }

    // characters at the end of a crash / VIP drop-off (ending camera only)
    if view.radius == 1 {
        draw_characters(a, w, w.mission);
    }
    gl_use_default_material();
}

fn draw_characters(a: &Assets, w: &World, _mission: Mission) {
    let t = w.travel_time; // animation clock: bob between two poses
    let rt = &a.road_tex;
    let rm = &a.road_model;
    if w.driver.active {
        let d = &w.driver.rect.c;
        let (m1, m2) = ((1.5 * t) as i32 % 2 == 0, ((t + 0.5) as i32) % 2 == 0);
        rm[if m1 { 9 } else { 8 }].draw(&rt[8], &at(d.x, 0.0, d.y - 0.8, 0.0), WHITE);
        rm[if m2 { 11 } else { 10 }].draw(&rt[10], &at(d.x, 0.0, d.y + 0.8, 180.0), WHITE);
    }
    if w.professor.active {
        let pr = &w.professor.rect;
        let alt = ((t + 0.5) as i32) % 2 == 0;
        let (m1, m2) = (if alt { 15 } else { 14 }, if alt { 17 } else { 16 });
        if pr.hy > pr.hx {
            rm[m1].draw(&rt[8], &at(pr.c.x + 0.8, 0.0, pr.c.y, 270.0), WHITE);
            rm[m2].draw(&rt[11], &at(pr.c.x - 0.8, 0.0, pr.c.y, 90.0), WHITE);
        } else {
            rm[m1].draw(&rt[8], &at(pr.c.x, 0.0, pr.c.y - 0.8, 0.0), WHITE);
            rm[m2].draw(&rt[11], &at(pr.c.x, 0.0, pr.c.y + 0.8, 180.0), WHITE);
        }
    }
}

/// Garage car (`renderCurCar`): fixed 330 degree turn, camera 22 units back, drawn straight onto
/// the screen in an 800x500 viewport anchored at the bottom (taller than most screens).
pub fn render_cur_car(a: &Assets, g: &Gfx, kind: usize) {
    let mut cam = camera(13.0, 4.0, 25.0, vec3(0.0, 5.0, 22.0), vec3(0.0, 1.5, 0.0), Vec3::Y, Some(1.6));
    cam.viewport = Some(g.viewport_px(g.shift, 0.0, 800.0, 500.0));
    set_camera(&cam);
    gl_use_material(&a.mats.normal);
    let info = car(kind);
    let m = Mat4::from_rotation_y(330.0_f32.to_radians());
    a.car_model[kind].draw(&a.car_tex[kind], &m, WHITE);
    let scale = Mat4::from_scale(vec3(info.wheel_width_ratio, info.wheel_height, info.wheel_height));
    for z in [info.wheel_front, -info.wheel_back] {
        let wm = m * Mat4::from_translation(vec3(0.0, info.wheel_height, z)) * scale;
        a.road_model[7].draw(&a.road_tex[7], &wm, WHITE);
    }
    gl_use_default_material();
    set_default_camera();
}

/// Render a mirror view into its render target.
pub fn render_mirror(a: &Assets, w: &World, right_driver: bool, which: u8, rt: &RenderTarget, drifts: &DriftMarks) {
    let mut view = mirror_view(w, which);
    view.cam.render_target = Some(rt.clone());
    render_world(a, w, right_driver, &view, drifts);
    set_default_camera();
}
