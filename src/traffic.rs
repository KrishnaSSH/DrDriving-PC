//! AI traffic (`drdriving.e`): one slot of the 50-car pool. Cars drive straight along a lane at
//! a capped speed, accelerate when the road ahead is free and brake when the "future" box in
//! front of them overlaps the player, a barricade or another car.

use crate::cars::car;
use crate::math::*;
use crate::world::Vehicle;

pub const MAX_TRAFFIC: usize = 50;

#[derive(Clone, Debug)]
pub struct TrafficCar {
    pub visible: bool,
    pub id: i32,
    pub kind: usize,
    /// Heading in degrees: 0, 90, 180 or 270 (`GridDir::index() * 90`).
    pub angle: i32,
    pub braking: bool,
    pub dir: Vec2,
    pub rec: Rect,
    pub future: Rect,
    pub v: Vec2,
    pub a: Vec2,
    pub v_max: f32,
    pub lane: i32,
    pub ang_distance: f32,
    pub ang_velocity: f32,
    pub wheel_height: f32,
    pub wheel_front: f32,
    pub wheel_back: f32,
    pub wheel_width_ratio: f32,
}

impl Default for TrafficCar {
    fn default() -> Self {
        Self {
            visible: false,
            id: -1,
            kind: 0,
            angle: 0,
            braking: false,
            dir: Vec2::ZERO,
            rec: Rect::default(),
            future: Rect::default(),
            v: Vec2::ZERO,
            a: Vec2::ZERO,
            v_max: 12.0,
            lane: 0,
            ang_distance: 0.0,
            ang_velocity: 100.0,
            wheel_height: 0.375,
            wheel_front: 0.0,
            wheel_back: 0.0,
            wheel_width_ratio: 0.0,
        }
    }
}

impl TrafficCar {
    /// `Set(...)`: place a car on a lane of the road leaving cell (`cell_x`,`cell_y`) in `dir`,
    /// `offset` units from the junction centre. Speeds are in km/h like the original call sites.
    #[allow(clippy::too_many_arguments)]
    pub fn set(
        &mut self,
        id: i32,
        kind: usize,
        cell_x: i32,
        cell_y: i32,
        dir: GridDir,
        offset: f32,
        lane: i32,
        accel_kmh: i32,
        init_kmh: i32,
        vmax_kmh: i32,
        right_driver: bool,
    ) {
        self.visible = true;
        self.id = id;
        self.v_max = vmax_kmh as f32 / 3.6;
        self.lane = lane;
        let idx = dir.index();
        self.kind = kind;
        let info = car(kind);
        self.wheel_height = info.wheel_height;
        self.wheel_front = info.wheel_front;
        self.wheel_back = info.wheel_back;
        self.wheel_width_ratio = info.wheel_width_ratio;
        self.ang_distance = 0.0;
        self.ang_velocity = 50.0 / info.wheel_height;

        self.rec.c.x = (cell_x * BLOCK) as f32 + dir.x as f32 * offset;
        self.rec.c.y = (cell_y * BLOCK) as f32 + dir.y as f32 * offset;
        let acc = accel_kmh as f32 / 3.6;
        self.a = Vec2::new(dir.x as f32 * acc, dir.y as f32 * acc);
        let v0 = init_kmh as f32 / 3.6;
        self.v = Vec2::new(dir.x as f32 * v0, dir.y as f32 * v0);
        self.dir = Vec2::new(dir.x as f32, dir.y as f32);
        self.angle = idx * 90;
        self.braking = false;
        if dir.x == 0 {
            self.rec.hy = info.half_length;
            self.rec.hx = info.half_width;
        } else {
            self.rec.hy = info.half_width;
            self.rec.hx = info.half_length;
        }
        let side = if right_driver { -1.0 } else { 1.0 };
        let lane_off = lane as f32 * 3.2 + 1.6;
        self.rec.c.x -= dir.y as f32 * lane_off * side;
        self.rec.c.y += lane_off * dir.x as f32 * side;
        self.future.hy = self.rec.hy + self.dir.y.abs() * 3.0;
        self.future.hx = self.rec.hx + self.dir.x.abs() * 3.0;
    }

    fn front_safe(&mut self, player: &Vehicle, barricades: &[Zone], cars: &[TrafficCar]) -> bool {
        self.future.c.x = self.rec.c.x + self.dir.x * 3.0 + self.v.x * 0.3;
        self.future.c.y = self.rec.c.y + self.dir.y * 3.0 + self.v.y * 0.3;
        let f = self.future;
        if circle_rect(&player.circle_cur[0], &f)
            && (circle_rect(&player.circle_cur[1], &f)
                || circle_rect(&player.circle_cur[2], &f)
                || circle_rect(&player.circle_cur[3], &f))
        {
            return false;
        }
        if barricades.iter().any(|b| b.active && rect_rect(&b.rect, &f)) {
            return false;
        }
        !cars.iter().any(|o| o.visible && o.id != self.id && rect_rect(&o.rec, &f))
    }

    fn brake(&mut self, dt: f32) {
        self.braking = true;
        self.v.x -= self.dir.x * 7.0 * dt;
        self.v.y -= self.dir.y * 7.0 * dt;
        if self.v.x.abs() < 1.0 {
            self.v.x = 0.0;
        }
        if self.v.y.abs() < 1.0 {
            self.v.y = 0.0;
        }
    }

    /// `Update(dt, player, barricades, cars)`.
    pub fn update(&mut self, dt: f32, player: &Vehicle, barricades: &[Zone], cars: &[TrafficCar]) {
        if !self.visible {
            return;
        }
        self.braking = false;
        if self.v_max < 1.0 {
            return;
        }
        if self.front_safe(player, barricades, cars) {
            self.v.x += self.a.x * dt;
            self.v.y += self.a.y * dt;
            let m = self.v_max;
            self.v.x = self.v.x.clamp(-m, m);
            self.v.y = self.v.y.clamp(-m, m);
        } else {
            self.brake(dt);
        }
        self.rec.c.x += self.v.x * dt;
        self.rec.c.y += self.v.y * dt;
        self.ang_distance += (self.v.x + self.v.y).abs() * dt * self.ang_velocity;
        if self.ang_distance > 360.0 {
            self.ang_distance -= 360.0;
        }
    }
}
