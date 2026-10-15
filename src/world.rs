//! World model, driving physics and mission state machine (`drdriving.l` + `tool.l`).
//!
//! **Not ported:** the 50-slot AI traffic system (`tool.e`, class `e`): spawning, movement and
//! car-vs-car collision. Everywhere the Java touches `cars[]`, this port simply has no cars, so
//! `check_collision` only reports buildings / barricades / centre splits. The rendering-only
//! histories (`poslist`, `driftlist`) are also omitted.

use crate::cars::{car, TRUCK_CAR};
use crate::math::*;
use crate::mission::{Mission, MissionInfo};
use crate::rng::JavaRandom;
use crate::save::SaveGame;
use crate::traffic::{TrafficCar, MAX_TRAFFIC};

/// Game-mode numbers from `drdriving.d` that the physics step cares about.
pub mod mode {
    pub const STARTING: i32 = 2;
    pub const PLAYING: i32 = 3;
    pub const SUCCESS: i32 = 5;
    pub const MULTI_ENDING: i32 = 7;
}

pub const GEAR_P: i32 = 0;
pub const GEAR_R: i32 = 1;
pub const GEAR_N: i32 = 2;
pub const GEAR_D: i32 = 3;

/// Maximum run length in seconds before throttle is cut and brakes are forced.
pub const MAX_RUN_SECONDS: f32 = 180.0;

// ---------------------------------------------------------------- angle history (`tool.c`)

/// Ring buffer of the last 25 (dt, car angle) samples. The chase camera looks up the heading
/// from ~0.09 s ago, which makes it swing smoothly behind the car.
#[derive(Clone, Debug)]
pub struct AngleHistory {
    index: usize,
    time: [f32; 25],
    angle: [f32; 25],
}

impl Default for AngleHistory {
    fn default() -> Self {
        Self { index: 0, time: [0.15; 25], angle: [180.0; 25] }
    }
}

impl AngleHistory {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn insert(&mut self, dt: f32, angle: f32) {
        self.index = (self.index + 1) % 25;
        self.time[self.index] = dt;
        self.angle[self.index] = angle;
    }
    /// `GetLastAngle(t)`: heading `t` seconds ago, interpolated, in 0..=360.
    pub fn last_angle(&self, t: f32) -> f32 {
        let mut i = self.index as i32;
        let mut acc = 0.0f32;
        for k in 0..25 {
            i = self.index as i32 - k;
            if i < 0 {
                i += 25;
            }
            let ft = self.time[i as usize];
            if acc + ft > t {
                break;
            }
            acc += ft;
        }
        let mut j = i - 1;
        if j < 0 {
            j = i + 24;
        }
        let total = self.time[i as usize] + acc;
        let w1 = t - acc;
        let w2 = total - t;
        let mut a = self.angle[i as usize];
        let mut b = self.angle[j as usize];
        if (b - a).abs() > 180.0 {
            if b > a {
                b -= 360.0;
            } else {
                a -= 360.0;
            }
        }
        let mut r = (b * w1 + a * w2) / (w1 + w2);
        while r > 360.0 {
            r -= 360.0;
        }
        while r < 0.0 {
            r += 360.0;
        }
        r
    }
}

// ---------------------------------------------------------------- vehicle (`tool.l`)

#[derive(Clone, Debug)]
pub struct Vehicle {
    pub kind: usize,
    /// Centre of the car body (`m_cx`, `m_cy`).
    pub cx: f32,
    pub cy: f32,
    /// Rear-axle reference point that is actually integrated (`m_x`, `m_y`).
    pub x: f32,
    pub y: f32,
    pub angle: f32,
    pub wheel_angle: f32,
    pub wheel_factor: f32,
    pub pitch: f32,
    pub half_width: f32,
    pub v: Vec2,
    pub a: Vec2,
    pub circle_org: [Circle; 4],
    pub circle_cur: [Circle; 4],
    pub circle_danger: Circle,
    pub cell_x: i32,
    pub cell_y: i32,
    pub mission_distance: f32,
    pub ang_distance: f32,
    pub ang_velocity: f32,
    pub wheel_height: f32,
    pub braking: bool,
    pub travel_time: f32,
    pub rating: i32,
    pub country: i32,
    pub name: String,
}

impl Default for Vehicle {
    fn default() -> Self {
        Self {
            kind: 0,
            cx: 0.0,
            cy: 0.0,
            x: 0.0,
            y: 0.0,
            angle: 0.0,
            wheel_angle: 0.0,
            wheel_factor: 0.0,
            pitch: 0.0,
            half_width: 0.0,
            v: Vec2::ZERO,
            a: Vec2::ZERO,
            circle_org: [Circle::default(); 4],
            circle_cur: [Circle::default(); 4],
            circle_danger: Circle { c: Vec2::ZERO, r: 2.5 },
            cell_x: 0,
            cell_y: 0,
            mission_distance: 0.0,
            ang_distance: 0.0,
            ang_velocity: 0.0,
            wheel_height: 0.0,
            braking: false,
            travel_time: 0.0,
            rating: 0,
            country: 0,
            name: String::new(),
        }
    }
}

impl Vehicle {
    /// `SetCar(type, cx, cy, angle)`.
    pub fn set_car(&mut self, kind: usize, cx: f32, cy: f32, angle: f32) {
        let info = car(kind);
        self.kind = kind;
        self.cx = cx;
        self.cy = cy;
        self.cell_x = java_round(cx / BLOCK as f32);
        self.cell_y = java_round(cy / BLOCK as f32);
        self.angle = angle;
        self.wheel_height = info.wheel_height;
        self.ang_distance = 0.0;
        self.ang_velocity = 50.0 / info.wheel_height;
        self.pitch = info.pitch;
        self.wheel_factor = info.wheel_factor;
        let (l, w) = (info.half_length, info.half_width);
        self.circle_org[0] = Circle { c: Vec2::ZERO, r: (l * l + w * w).sqrt() };
        self.circle_org[1] = Circle { c: Vec2::ZERO, r: w };
        self.circle_org[2] = Circle { c: Vec2::new(0.0, w - l), r: w };
        self.circle_org[3] = Circle { c: Vec2::new(0.0, l - w), r: w };
        self.half_width = w;
        self.braking = false;
        self.x = cx - self.pitch * (DEG2RAD * angle).sin();
        self.y = cy + self.pitch * (DEG2RAD * angle).cos();
        self.move_to();
    }

    /// `changeType(i)`: swap the model without moving (used for the remote opponent).
    pub fn change_type(&mut self, kind: usize) {
        let kind = kind.min(27); // remote players can never be the bus/truck
        let info = car(kind);
        self.kind = kind;
        self.cell_x = java_round(self.cx / BLOCK as f32);
        self.cell_y = java_round(self.cy / BLOCK as f32);
        self.wheel_height = info.wheel_height;
        self.ang_distance = 0.0;
        self.ang_velocity = 50.0 / info.wheel_height;
        self.pitch = info.pitch;
        self.wheel_factor = info.wheel_factor;
    }

    /// `ResetCar()`
    pub fn reset(&mut self) {
        self.angle = 0.0;
        self.wheel_angle = 0.0;
        for i in 0..4 {
            self.circle_org[i] = Circle::default();
            self.circle_cur[i] = Circle::default();
        }
    }

    /// `MoveTo()`: place the four collision circles for the current pose.
    pub fn move_to(&mut self) {
        for i in 0..4 {
            let mut v = Vec2::new(self.circle_org[i].c.x, self.circle_org[i].c.y - self.pitch);
            v.rotate_deg(self.angle);
            self.circle_cur[i] = Circle { c: Vec2::new(self.x + v.x, self.y + v.y), r: self.circle_org[i].r };
        }
        let mut v = Vec2::new(0.0, -self.pitch);
        v.rotate_deg(self.angle);
        self.circle_danger = Circle { c: Vec2::new(self.x + v.x, self.y + v.y), r: 2.5 };
    }

    /// `bCollisionBuilding()`: the road is a 20-unit-wide corridor centred on each 140-unit grid
    /// line. The body circle fully inside either corridor clears the test; otherwise any of the
    /// three smaller circles poking out of *both* corridors (beyond a diamond-shaped junction
    /// allowance) is a hit.
    pub fn collides_building(&self) -> bool {
        let b = BLOCK as f32;
        let c0 = &self.circle_cur[0];
        let gx = self.cell_x as f32 * b;
        let gy = self.cell_y as f32 * b;
        if c0.c.x - c0.r > gx - 10.0 && c0.c.x + c0.r < gx + 10.0 {
            return false;
        }
        if c0.c.y - c0.r > gy - 10.0 && c0.c.y + c0.r < gy + 10.0 {
            return false;
        }
        for i in 1..4 {
            let c = &self.circle_cur[i];
            let in_x = c.c.x - c.r > gx - 10.0 && c.c.x + c.r < gx + 10.0;
            let in_y = c.c.y - c.r > gy - 10.0 && c.c.y + c.r < gy + 10.0;
            if !in_x && !in_y && (c.c.x - gx).abs() + (c.c.y - gy).abs() >= 20.0 - c.r {
                return true;
            }
        }
        false
    }
}

// ---------------------------------------------------------------- world (`l`)

#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    pub accel: bool,
    pub brake: bool,
    /// Steering input, roughly -1..1 (multiplied by the car's `wheel_factor`).
    pub steer: f32,
}

#[derive(Clone, Debug)]
pub struct World {
    pub mission: Mission,
    pub car_mode: usize,
    pub online: bool,
    pub rng: JavaRandom,
    pub info_cur: MissionInfo,
    pub info_online: MissionInfo,

    pub player: Vehicle,
    pub opponent: Vehicle,

    pub barricades: [Zone; 8],
    pub centersplit: [Zone; 4],
    pub checkpoint: Zone,
    pub parking: Zone,
    pub professor: Zone,
    pub driver: Zone,
    pub cars: Vec<TrafficCar>,
    cur_ai: usize,
    pub poslist: AngleHistory,
    /// Velocity error vs. heading (`vecInertia`); the interior camera leans with it.
    pub vec_inertia: Vec2,
    last_barricade: usize,
    next_barricade: usize,

    pub curr_dir: GridDir,
    pub next_dir: GridDir,
    pub next_r_dir: GridDir,

    // route progress
    pub cur: (i32, i32),
    pub dst: (i32, i32),
    pub nxt: (i32, i32),
    pub blocks_passed: i32,
    pub blocks_for_mission: i32,
    pub rel_dir_last: i32,
    pub rel_dir_cur: i32,
    pub rel_dir_next: i32,
    pub now_turning: bool,
    pub turn_start_time: f32,
    prepared_for_next: bool,
    flipped_last_barricades: bool,
    /// Distances to the previous/destination intersections (`g_cx`,`g_cy`,`g_dx`,`g_dy`).
    pub g_cx: f32,
    pub g_cy: f32,
    pub g_dx: f32,
    pub g_dy: f32,

    // dynamics
    pub gear: i32,
    pub speed: f32,
    pub speed_last: f32,
    pub speed_max: f32,
    pub engine_power: f32,
    pub brake_power: f32,
    pub tire_power: f32,
    pub fuel_consume: f32,
    pub comfort_consume: f32,
    pub drifting: bool,
    pub emergency: bool,
    pub blinker: i32,
    pub in_parking_lot: bool,

    // run statistics
    pub gas_left: f32,
    pub gas_initial: f32,
    pub score: f32,
    pub score_last: f32,
    pub travel_time: f32,
    pub travel_distance: f32,
    pub drift_distance: f32,
    pub brake_time: f32,
    pub coins_acquired: i32,
    pub perfect_turns: i32,
    pub sub_mission_count: i32,
    pub sub_mission_lane: i32,
    pub travel_distance_at_sub_mission: f32,
}

impl World {
    pub fn new(car_mode: usize, seed: Option<i64>) -> Self {
        let mut barricades = [Zone::new(); 8];
        for (i, b) in barricades.iter_mut().enumerate() {
            *b = if i % 2 == 0 { Zone::with_half(9.5, 2.2) } else { Zone::with_half(2.2, 9.5) };
        }
        let mut centersplit = [Zone::new(); 4];
        for (i, c) in centersplit.iter_mut().enumerate() {
            if i < 2 {
                c.rect.hy = 60.0;
                c.rect.hx = 0.22;
            } else {
                c.rect.hy = 0.22;
                c.rect.hx = 60.0;
            }
        }
        let rng = match seed {
            Some(s) => JavaRandom::new(s),
            None => JavaRandom::from_entropy(),
        };
        Self {
            mission: Mission::Vip, // the Java constructor starts on mission type 4
            car_mode,
            online: false,
            rng,
            info_cur: MissionInfo::new(Mission::Vip),
            info_online: MissionInfo::new(Mission::Vip),
            player: Vehicle::default(),
            opponent: Vehicle::default(),
            barricades,
            centersplit,
            checkpoint: Zone::new(),
            parking: Zone::new(),
            professor: Zone::new(),
            driver: Zone::with_half(0.9, 0.9),
            cars: vec![TrafficCar::default(); MAX_TRAFFIC],
            cur_ai: 0,
            poslist: AngleHistory::default(),
            vec_inertia: Vec2::ZERO,
            last_barricade: 0,
            next_barricade: 4,
            curr_dir: GridDir::default(),
            next_dir: GridDir::default(),
            next_r_dir: GridDir::default(),
            cur: (0, 0),
            dst: (0, 0),
            nxt: (0, 0),
            blocks_passed: 0,
            blocks_for_mission: 5,
            rel_dir_last: 0,
            rel_dir_cur: 0,
            rel_dir_next: 0,
            now_turning: false,
            turn_start_time: 0.0,
            prepared_for_next: false,
            flipped_last_barricades: false,
            g_cx: 0.0,
            g_cy: 0.0,
            g_dx: 0.0,
            g_dy: 0.0,
            gear: GEAR_D,
            speed: 0.0,
            speed_last: 0.0,
            speed_max: 0.0,
            engine_power: 2.0,
            brake_power: 6.0,
            tire_power: 11.0,
            fuel_consume: 1.0,
            comfort_consume: 1.0,
            drifting: false,
            emergency: false,
            blinker: 0,
            in_parking_lot: false,
            gas_left: 500.0,
            gas_initial: 500.0,
            score: 0.0,
            score_last: 0.0,
            travel_time: 0.0,
            travel_distance: 0.0,
            drift_distance: 0.0,
            brake_time: 0.0,
            coins_acquired: 0,
            perfect_turns: 0,
            sub_mission_count: 0,
            sub_mission_lane: -1,
            travel_distance_at_sub_mission: 0.0,
        }
    }

    fn side(save: &SaveGame) -> f32 {
        if save.right_driver {
            1.0
        } else {
            -1.0
        }
    }

    /// `initMap()` — garage / menu pose.
    pub fn init_map(&mut self, save: &SaveGame) {
        self.checkpoint.active = false;
        self.driver.active = false;
        self.parking.active = false;
        self.professor.active = false;
        self.vec_inertia = Vec2::ZERO;
        self.curr_dir.set_index(0);
        for c in self.cars.iter_mut() {
            c.visible = false;
        }
        for b in self.barricades.iter_mut() {
            b.active = false;
        }
        self.player.set_car(self.car_mode, Self::side(save) * 4.8 + 1960.0, 1904.0, 180.0);
        self.player.braking = true;
    }

    pub fn change_mission(&mut self, delta: i32) {
        let mut i = self.mission.index() as i32 + delta;
        if i > 8 {
            i = 0;
        }
        self.mission = Mission::from_index(if i >= 0 { i } else { 8 }).unwrap();
    }

    pub fn change_mission_random(&mut self) {
        let cur = self.mission.index() as i32;
        let n = loop {
            let n = ((cur + 1) + self.rng.next_int_bound(8)) % 9;
            if n != cur {
                break n;
            }
        };
        self.mission = Mission::from_index(n).unwrap();
    }

    /// `changeCar(dir)` — step to the next owned/rented car in `dir` (+1 / -1), wrapping 0..=27.
    pub fn change_car(&mut self, dir: i32, save: &SaveGame) {
        let mut i = self.car_mode as i32;
        for _ in 0..28 {
            i += dir;
            if i > 27 {
                i = 0;
            }
            if i < 0 {
                i = 27;
            }
            let u = i as usize;
            if (u < 16 && save.has_car[u]) || (u >= 16 && save.car_rent[u] > 0) {
                break;
            }
        }
        if i as usize != self.car_mode {
            self.car_mode = i as usize;
            if self.online {
                self.player.change_type(self.car_mode);
            } else {
                self.player.set_car(self.car_mode, Self::side(save) * 4.8 + 1960.0, 1904.0, 180.0);
                self.player.braking = true;
            }
        }
    }

    /// `prepareRace()` — resets all run state and derives the car's performance from base
    /// ability + upgrades, scaled by accumulated damage.
    pub fn prepare_race(&mut self, save: &SaveGame) {
        self.now_turning = false;
        self.emergency = false;
        self.blinker = 0;
        self.speed = 0.0;
        self.speed_last = 0.0;
        self.speed_max = 0.0;
        self.brake_time = 0.0;
        self.coins_acquired = 0;
        self.perfect_turns = 0;
        self.sub_mission_count = 0;
        self.gear = GEAR_D;
        self.travel_distance = 0.0;
        self.drift_distance = 0.0;
        self.travel_distance_at_sub_mission = 0.0;
        self.checkpoint.active = false;
        self.prepared_for_next = false;
        self.flipped_last_barricades = false;
        self.blocks_passed = 0;
        self.travel_time = 0.0;
        self.turn_start_time = 0.0;

        if self.mission == Mission::Truck {
            self.car_mode = TRUCK_CAR;
        }
        self.blocks_for_mission = self.mission.blocks_for_mission();
        self.gas_left = self.mission.start_fuel();
        self.gas_initial = self.gas_left;

        self.player.a = Vec2::ZERO;
        self.player.v = Vec2::ZERO;
        self.player.mission_distance = 0.0;
        self.opponent.mission_distance = 0.0;

        self.cur = (14, 20);
        self.dst = (14, 21);
        self.nxt = (14, 22);
        self.curr_dir.set_index(2);
        self.next_dir.set_index(2);
        self.rel_dir_last = 2;
        self.rel_dir_cur = 0;
        self.rel_dir_next = 0;
        self.sub_mission_lane = -1;

        let c = self.car_mode;
        let damage = save.car_damage[c];
        let f3 = if self.mission == Mission::Truck {
            1.0
        } else {
            let d = damage.min(9000);
            if d > 1000 {
                (11000 - d) as f32 * 1.0e-4
            } else {
                1.0
            }
        };
        let ab = &car(c).ability;
        let up = &save.upgrade[c];
        self.engine_power = ((ab[0] + up[0]) as f32 * f3 * 0.029) + 2.9;
        self.brake_power = ((ab[1] + up[1]) as f32 * f3 * 0.08) + 7.0;
        self.tire_power = ((ab[2] + up[2]) as f32 * f3 * 0.12) + 15.5;
        self.fuel_consume = 60.0 / (((ab[3] + up[3]) as f32 * f3) + 20.0);
        self.comfort_consume = 170.0 / (((ab[4] + up[4]) as f32 * f3) + 40.0);

        self.score = match self.mission {
            Mission::Vip | Mission::Fuel | Mission::BrokenBrake => 150.0,
            _ => 135.0,
        };
        self.score_last = save.record[self.mission.index()] as f32 * 0.01;

        self.player.reset();
        for c in self.cars.iter_mut() {
            c.visible = false;
        }
        self.poslist.clear();
        let side = Self::side(save) * 4.8;
        let y_off = if matches!(self.mission, Mission::Highway | Mission::Lane) { 100.0 } else { 30.0 };
        self.player.set_car(c, self.cur.0 as f32 * 140.0 + side, self.cur.1 as f32 * 140.0 + y_off, 180.0);

        if self.mission == Mission::Truck {
            self.opponent.kind = TRUCK_CAR;
        }
        self.opponent.cx = self.player.cx;
        self.opponent.cy = self.player.cy;
        self.opponent.angle = 180.0;
        self.opponent.v = Vec2::ZERO;
        self.opponent.mission_distance = 0.0;

        self.update_center_split();
        let (cx, cy) = (self.cur.0 as f32 * 140.0, self.cur.1 as f32 * 140.0);
        let n = self.next_barricade;
        self.barricades[n].rect.c = Vec2::new(cx, cy - 32.0);
        self.barricades[n + 1].rect.c = Vec2::new(cx + 32.0, cy);
        self.barricades[n + 2].rect.c = Vec2::new(cx, cy + 32.0);
        self.barricades[n + 3].rect.c = Vec2::new(cx - 32.0, cy);

        if self.online {
            self.info_cur.directions = self.info_online.directions;
            self.info_cur.aicar = self.info_online.aicar;
        } else {
            self.info_cur.mission = self.mission;
            self.info_cur.generate_directions(&mut self.rng);
        }
        self.cur_ai = 0;
        self.blocks_passed = 0;
        self.rel_dir_last = 2;
        self.rel_dir_cur = 0;
        self.rel_dir_next = 0;
        self.find_next_mission(save);
    }

    // ------------------------------------------------------------ route / zones

    /// `iFindNextMission()` — look ahead one block, rebuild barricades and, once enough blocks
    /// have been driven, open the finish zone (checkpoint, or parking spot for VIP/Parking).
    pub fn find_next_mission(&mut self, save: &SaveGame) {
        let rel = self.info_cur.directions[(self.blocks_passed % 20) as usize];
        self.rel_dir_next = rel as i32;
        let idx = self.next_dir.turned_index(rel as i32);
        self.next_dir.set_index(idx);
        self.next_r_dir = GridDir { x: -self.next_dir.x, y: -self.next_dir.y };
        self.nxt = (self.dst.0 + self.next_dir.x, self.dst.1 + self.next_dir.y);
        self.prepared_for_next = false;
        self.flipped_last_barricades = false;
        self.update_barricade();
        self.clear_far_away_cars();
        if self.rel_dir_next == 0 {
            self.prepare_cars_for_straight(save);
        }

        if self.blocks_passed <= self.blocks_for_mission {
            return;
        }
        let (cx, cy) = (self.cur.0, self.cur.1);
        let (dx, dy) = (self.dst.0, self.dst.1);
        let mid_x = (((cx + dx) * 140) / 2) as f32;
        let mid_y = (((cy + dy) * 140) / 2) as f32;
        if !matches!(self.mission, Mission::Vip | Mission::Parking) {
            self.checkpoint.active = true;
            if self.mission == Mission::Drift && self.drift_distance < 40.0 {
                self.checkpoint.active = false;
            }
            if self.mission == Mission::Lane && self.sub_mission_count < 12 {
                self.checkpoint.active = false;
            }
            self.checkpoint.rect.c = Vec2::new(mid_x, mid_y);
            if cx == dx {
                self.checkpoint.rect.hy = 2.0;
                self.checkpoint.rect.hx = 10.0;
            } else {
                self.checkpoint.rect.hy = 10.0;
                self.checkpoint.rect.hx = 2.0;
            }
            return;
        }
        self.parking.active = true;
        self.parking.rect.c = Vec2::new(mid_x, mid_y);
        self.professor.rect.c = Vec2::new(mid_x, mid_y);
        let side = if save.right_driver { 1.0 } else { -1.0 };
        if cx == dx {
            let dir = if cy > dy { -1.0 } else { 1.0 };
            self.parking.rect.c.x += dir * 8.0 * side;
            self.professor.rect.c.x += dir * 5.0 * side;
            for z in [&mut self.parking, &mut self.professor] {
                z.rect.hy = 2.0;
                z.rect.hx = 10.0;
            }
        } else {
            let dir = if cx > dx { 1.0 } else { -1.0 };
            self.parking.rect.c.y += dir * 8.0 * side;
            self.professor.rect.c.y += dir * 5.0 * side;
            for z in [&mut self.parking, &mut self.professor] {
                z.rect.hy = 10.0;
                z.rect.hx = 2.0;
            }
        }
    }

    /// `updateBarricade()` — swap the two barricade sets and enable every side of the
    /// destination intersection except the road we arrive on and the one we leave by.
    pub fn update_barricade(&mut self) {
        std::mem::swap(&mut self.next_barricade, &mut self.last_barricade);
        for b in self.barricades.iter_mut() {
            b.active = true;
        }
        let arrive = self.curr_dir.index() as usize;
        self.barricades[self.last_barricade + arrive].active = false;
        let parking_phase = self.blocks_passed > self.blocks_for_mission
            && matches!(self.mission, Mission::Vip | Mission::Parking);
        if !parking_phase {
            let leave = self.next_dir.index() as usize;
            self.barricades[leave + self.next_barricade].active = false;
            self.barricades[self.next_barricade + (arrive + 2) % 4].active = false;
        }
        let (gx, gy) = (self.dst.0 as f32 * 140.0, self.dst.1 as f32 * 140.0);
        let n = self.next_barricade;
        self.barricades[n].rect.c = Vec2::new(gx, gy - 32.0);
        self.barricades[n + 1].rect.c = Vec2::new(gx + 32.0, gy);
        self.barricades[n + 2].rect.c = Vec2::new(gx, gy + 32.0);
        self.barricades[n + 3].rect.c = Vec2::new(gx - 32.0, gy);
        for b in self.barricades.iter().filter(|b| b.active) {
            for c in self.cars.iter_mut() {
                if c.visible && rect_rect(&c.rec, &b.rect) {
                    c.visible = false;
                }
            }
        }
    }

    /// `flipLastBarricade()` — once the car is past the previous junction, its barricade set is
    /// inverted (closed arms open and vice versa).
    pub fn flip_last_barricade(&mut self) {
        for i in 0..4 {
            let z = &mut self.barricades[self.last_barricade + i];
            z.active = !z.active;
        }
        for i in 0..4 {
            let z = self.barricades[self.last_barricade + i];
            if z.active {
                for c in self.cars.iter_mut() {
                    if c.visible && rect_rect(&c.rec, &z.rect) {
                        c.visible = false;
                    }
                }
            }
        }
    }

    /// `updateCenterSplit()` — the four median strips around the player's current cell.
    pub fn update_center_split(&mut self) {
        let x = (self.player.cell_x * 140) as f32;
        let y = (self.player.cell_y * 140) as f32;
        self.centersplit[0].rect.c = Vec2::new(x, y - 70.0);
        self.centersplit[1].rect.c = Vec2::new(x, y + 70.0);
        self.centersplit[2].rect.c = Vec2::new(x - 70.0, y);
        self.centersplit[3].rect.c = Vec2::new(x + 70.0, y);
    }

    /// `bCompleteMission()` — returns true on the frame the car reaches the destination
    /// intersection; the caller should then call [`World::find_next_mission`]. (Name kept from
    /// the Java; it really means "advanced to the next block".)
    pub fn advance_block(&mut self, save: &SaveGame) -> bool {
        if self.now_turning && self.g_dx < 122.0 && self.g_dy < 122.0 {
            self.now_turning = false;
        }
        if !self.prepared_for_next {
            if self.rel_dir_next == 2 {
                if self.g_dx < 13.0 && self.g_dy < 13.0 {
                    self.prepared_for_next = true;
                    let cd = Vec2::new(self.curr_dir.x as f32, self.curr_dir.y as f32);
                    for c in self.cars.iter_mut() {
                        if c.visible && (c.dir.x != cd.x || c.dir.y != cd.y) {
                            c.visible = false;
                        }
                    }
                    self.prepare_cars_except_straight(save);
                }
            } else if self.g_dx < 30.0 && self.g_dy < 30.0 {
                self.prepared_for_next = true;
                self.prepare_cars_except_straight(save);
            }
        }
        if !self.flipped_last_barricades && (self.g_cx > 50.0 || self.g_cy > 50.0) {
            self.clear_across_cars();
            self.flipped_last_barricades = true;
            self.flip_last_barricade();
        }
        if self.g_dx < 20.0 && self.g_dy < 20.0 {
            if self.rel_dir_next != 0 {
                if !self.now_turning {
                    self.turn_start_time = self.travel_time;
                }
                self.now_turning = true;
            } else {
                self.now_turning = false;
            }
        }
        if self.g_dx >= 10.0 || self.g_dy >= 10.0 {
            return false;
        }
        self.blocks_passed += 1;
        self.prepared_for_next = false;
        self.flipped_last_barricades = false;
        self.cur = self.dst;
        self.dst = self.nxt;
        self.curr_dir = self.next_dir;
        self.rel_dir_last = self.rel_dir_cur;
        self.rel_dir_cur = self.rel_dir_next;
        self.update_center_split();
        true
    }

    /// `iGetCurrentLane()` — lane 0..=2 from the lateral distance to the junction (3.2 per lane).
    pub fn current_lane(&self) -> i32 {
        let d = if self.curr_dir.x == 0 { self.g_dx } else { self.g_dy };
        ((d / 3.2) as i32).clamp(0, 2)
    }

    /// `makeSubMission()` — Lane mode: pick a lane to move into after 20 units of driving.
    pub fn make_sub_mission(&mut self) {
        if !self.parking.active
            && self.sub_mission_lane < 0
            && self.travel_distance >= self.travel_distance_at_sub_mission + 20.0
            && (self.g_cx >= 12.0 || self.g_cy >= 12.0)
            && (self.g_dx >= 20.0 || self.g_dy >= 20.0)
        {
            self.sub_mission_lane = ((self.current_lane() + 1) + self.rng.next_int_bound(2)) % 3;
        }
    }

    /// `bCompleteSubMission()` — true when the car is centred in the requested lane and
    /// aligned with the road (within 5 degrees of an axis).
    pub fn complete_sub_mission(&mut self) -> bool {
        let lane = self.sub_mission_lane;
        if self.parking.active || !(0..=2).contains(&lane) {
            return false;
        }
        if self.g_cx < 11.0 && self.g_cy < 11.0 {
            return false;
        }
        if self.g_dx < 11.0 && self.g_dy < 11.0 {
            return false;
        }
        let d = if self.curr_dir.x != 0 { self.g_dy } else { self.g_dx };
        if ((lane as f32 * 3.2 + 1.6) - d).abs() > 1.5 - self.player.half_width {
            return false;
        }
        let mut a = self.player.angle;
        while a > 90.0 {
            a -= 90.0;
        }
        if a > 5.0 && a < 85.0 {
            return false;
        }
        self.sub_mission_lane = -1;
        self.travel_distance_at_sub_mission = self.travel_distance;
        self.coins_acquired += 1;
        self.sub_mission_count += 1;
        true
    }

    /// `bPassCheckPoint()` — finish test. For VIP/Parking this is the parking-spot check
    /// (within 0.8 units of the spot, gear in P); otherwise a circle-vs-checkpoint test.
    ///
    /// The angle tests are reproduced exactly as decompiled, including the structure of the
    /// early `return false`s.
    pub fn pass_check_point(&mut self) -> bool {
        if !matches!(self.mission, Mission::Vip | Mission::Parking) {
            return self.checkpoint.active && circle_rect(&self.player.circle_cur[0], &self.checkpoint.rect);
        }
        if !self.parking.active {
            return false;
        }
        self.in_parking_lot = false;
        let p = &self.parking.rect.c;
        if (p.x - self.player.cx).abs() > 0.8 || (p.y - self.player.cy).abs() > 0.8 {
            return false;
        }
        let mut a = self.player.angle;
        if self.curr_dir.x == 0 && ((a > 80.0 && a < 100.0) || (a > 260.0 && a < 280.0)) {
            return false;
        }
        if self.curr_dir.y == 0 && (a < 10.0 || a > 350.0 || (a > 170.0 && a < 190.0)) {
            return false;
        }
        while a > 90.0 {
            a -= 90.0;
        }
        if a > 8.0 && a < 82.0 {
            return false;
        }
        self.in_parking_lot = true;
        if self.gear != GEAR_P {
            return false;
        }
        self.professor.active = true;
        true
    }

    /// `onChangeSuccess()` — snap the car straight onto the road axis after a mission ends.
    pub fn on_change_success(&mut self) {
        if matches!(self.mission, Mission::Vip | Mission::Parking) {
            return;
        }
        if self.curr_dir.x == 0 {
            self.player.v.x = 0.0;
        }
        if self.curr_dir.y == 0 {
            self.player.v.y = 0.0;
        }
        self.player.angle = (self.curr_dir.index() * 90) as f32;
        let lane = self.current_lane();
        let fx = self.player.cx + self.player.v.x * 0.4;
        let fy = self.player.cy + self.player.v.y * 0.4;
        for c in self.cars.iter_mut() {
            if c.visible && c.lane == lane && (fx - c.rec.c.x).abs() <= 15.0 && (fy - c.rec.c.y).abs() <= 15.0 {
                c.visible = false;
            }
        }
        for b in self.barricades.iter_mut() {
            b.active = false;
        }
    }

    // ------------------------------------------------------------ collisions

    fn circles_hit(&self, r: &Rect) -> bool {
        circle_rect(&self.player.circle_cur[0], r) && (1..4).any(|i| circle_rect(&self.player.circle_cur[i], r))
    }

    fn collides_barricade(&self) -> bool {
        self.barricades.iter().any(|b| b.active && self.circles_hit(&b.rect))
            || self.centersplit.iter().any(|c| self.circles_hit(&c.rect))
    }

    fn collides_cars(&self) -> bool {
        self.cars.iter().any(|c| c.visible && self.circles_hit(&c.rec))
    }

    /// `bCheckDanger()` — a car travelling the same heading is inside the 2.5-unit danger circle.
    pub fn check_danger(&self) -> bool {
        let a = self.curr_dir.index() * 90;
        self.cars.iter().any(|c| c.visible && c.angle == a && circle_rect(&self.player.circle_danger, &c.rec))
    }

    /// `bCheckCollision()`: buildings, barricades, centre splits and traffic.
    pub fn check_collision(&self) -> bool {
        self.player.collides_building() || self.collides_barricade() || self.collides_cars()
    }

    /// `bCheckAccident()` — also drops the "driver" figure next to the car on a crash.
    pub fn check_accident(&mut self) -> bool {
        let hit = self.check_collision();
        if hit {
            self.place_driver();
        }
        hit
    }

    fn driver_spot_free(&self) -> bool {
        let d = &self.driver.rect;
        if circle_rect(&self.player.circle_cur[0], d) && (1..4).any(|i| circle_rect(&self.player.circle_cur[i], d)) {
            return false;
        }
        if self.barricades.iter().any(|b| b.active && rect_rect(d, &b.rect)) {
            return false;
        }
        if self.centersplit.iter().any(|c| rect_rect(d, &c.rect)) {
            return false;
        }
        if self.cars.iter().any(|c| c.visible && rect_rect(d, &c.rec)) {
            return false;
        }
        let gx = (self.player.cell_x * 140) as f32;
        let gy = (self.player.cell_y * 140) as f32;
        if d.c.x - d.hx > gx - 10.0 && d.c.x + d.hx < gx + 10.0 {
            return true;
        }
        d.c.y - d.hy > gy - 10.0 && d.c.y + d.hy < gy + 10.0
    }

    /// `placeDriver()` — try 20 spots on a 3-unit ring around the car (18° apart) and keep the
    /// first that is on the road and unobstructed.
    pub fn place_driver(&mut self) {
        for i in 0..20 {
            let mut v = Vec2::new((DEG2RAD * self.player.angle).sin(), -(DEG2RAD * self.player.angle).cos());
            v.rotate_deg((270 - i * 18) as f32);
            v.scale(3.0);
            self.driver.rect.c = Vec2::new(self.player.cx + v.x, self.player.cy + v.y);
            if self.driver_spot_free() {
                break;
            }
        }
        self.driver.active = true;
    }

    // ------------------------------------------------------------ physics step

    /// `update(dt, mode, accel, brake, steer)` — one frame of driving physics.
    ///
    /// `game_mode` is one of [`mode`]; the car only moves in PLAYING, SUCCESS and MULTI_ENDING
    /// (the last two let it coast to a stop). In STARTING the throttle is held down.
    pub fn update(&mut self, dt_in: f32, game_mode: i32, input: Controls) {
        self.drifting = false;
        let dt = dt_in.min(0.1);
        if game_mode == mode::PLAYING {
            self.travel_time += dt;
        }
        let steer = if game_mode == mode::PLAYING || game_mode == mode::STARTING { input.steer } else { 0.0 };

        self.player.braking = false;
        let brake_in = if self.mission == Mission::BrokenBrake { false } else { input.brake };

        let (brake, mut accel);
        if game_mode == mode::PLAYING {
            if self.travel_time > MAX_RUN_SECONDS {
                brake = true;
                accel = false;
            } else {
                brake = brake_in;
                accel = input.accel;
            }
            if self.gas_left < 0.0 {
                accel = false;
            } else if accel {
                // fuel burn is heavier at low speed; note the original uses the *raw* dt here
                let k = match self.speed_last {
                    s if s < 5.0 => 1.6,
                    s if s < 10.0 => 1.5,
                    s if s < 15.0 => 1.4,
                    s if s < 20.0 => 1.3,
                    s if s < 25.0 => 1.2,
                    s if s < 30.0 => 1.1,
                    _ => 1.0,
                };
                self.gas_left -= self.fuel_consume * dt_in * k;
            } else {
                self.gas_left -= 0.2 * dt_in * self.fuel_consume;
            }
        } else if game_mode == mode::STARTING {
            brake = brake_in;
            accel = true;
        } else if game_mode == mode::SUCCESS || game_mode == mode::MULTI_ENDING {
            brake = true;
            accel = false;
        } else {
            brake = brake_in;
            accel = input.accel;
        }

        self.player.a = Vec2::ZERO;
        let car_dir = {
            let mut d = Vec2::new((DEG2RAD * self.player.angle).sin(), -(DEG2RAD * self.player.angle).cos());
            d.normalize();
            d
        };
        let move_dir = {
            let mut d = self.player.v;
            d.normalize();
            d
        };
        self.speed_last = self.speed;
        let sl = self.speed_last;

        // creep when nearly stopped
        if sl < 2.0 && self.gas_left > 0.0 && game_mode == mode::PLAYING {
            match self.gear {
                GEAR_D => self.player.a.add_scaled(car_dir, 1.0),
                GEAR_R => self.player.a.add_scaled(car_dir, -1.0),
                _ => {}
            }
            if !accel && !brake {
                self.gas_left -= 0.8 * dt_in * self.fuel_consume;
            }
        }
        if self.gas_left < 0.0 {
            self.player.a.add_scaled(move_dir, -2.0);
        }
        if brake {
            if game_mode == mode::PLAYING && sl > 3.0 {
                self.brake_time += dt;
            }
            if game_mode == mode::SUCCESS || game_mode == mode::MULTI_ENDING {
                self.player.a.add_scaled(move_dir, -sl * 0.5);
            } else {
                self.player.a.add_scaled(move_dir, -self.brake_power);
            }
            self.player.braking = true;
        } else if accel {
            match self.gear {
                GEAR_D => self.player.a.add_scaled(car_dir, self.engine_power),
                GEAR_R => self.player.a.add_scaled(car_dir, -self.engine_power),
                _ => {}
            }
        }

        // rolling / air drag
        let v = self.player.v;
        if sl > 0.1 {
            if game_mode == mode::PLAYING && self.online {
                let (opp, me) = (self.opponent.mission_distance, self.player.mission_distance);
                // rubber-banding: a clear leader gets extra drag
                if opp > 10.0 && me > 10.0 && me > opp + 20.0 {
                    self.player.a.add_scaled(v, ((-0.2 / sl) - 0.08) - sl * 0.0025);
                }
            }
            self.player.a.add_scaled(v, ((-0.2 / sl) - 0.07) - sl * 0.0015);
        } else {
            self.player.a.add_scaled(v, -0.07 - sl * 0.0015);
        }

        // tyre grip: the part of the velocity change that exceeds tire_power*dt becomes drift
        let heading = if self.gear == GEAR_R {
            Vec2::new(-(DEG2RAD * self.player.angle).sin() * sl, (DEG2RAD * self.player.angle).cos() * sl)
        } else {
            Vec2::new((DEG2RAD * self.player.angle).sin() * sl, -(DEG2RAD * self.player.angle).cos() * sl)
        };
        let inertia = Vec2::new(heading.x - self.player.v.x, heading.y - self.player.v.y);
        self.vec_inertia = inertia;
        let mut drift_v = inertia;
        let inertia_len = inertia.len();
        let grip = self.tire_power;
        if inertia_len > grip * dt {
            self.drifting = true;
            drift_v.scale((grip * dt) / inertia_len);
        }
        let a = self.player.a;
        self.player.v.add_scaled(a, dt);
        self.player.v.add(drift_v);
        if brake && sl < 0.5 {
            self.player.v = Vec2::ZERO;
        }

        if game_mode == mode::PLAYING && self.online {
            let ov = self.opponent.v;
            let ol = ov.len();
            self.opponent.cx += ov.x * dt;
            self.opponent.cy += ov.y * dt;
            let step = ol * dt;
            self.opponent.mission_distance += step;
            self.opponent.ang_distance += step * self.opponent.ang_velocity;
        }

        self.speed = self.player.v.len();

        if game_mode == mode::PLAYING || game_mode == mode::SUCCESS || game_mode == mode::MULTI_ENDING {
            let wheel = steer * self.player.wheel_factor;
            self.player.wheel_angle = wheel;
            if sl > 0.3 {
                match self.gear {
                    GEAR_D => self.player.angle += wheel * sl * dt,
                    GEAR_R => self.player.angle -= wheel * sl * dt,
                    _ => {}
                }
            }
            while self.player.angle > 360.0 {
                self.player.angle -= 360.0;
            }
            while self.player.angle < 0.0 {
                self.player.angle += 360.0;
            }
            self.player.x += self.player.v.x * dt;
            self.player.y += self.player.v.y * dt;
            self.player.ang_distance += self.speed * dt * self.player.ang_velocity;

            let along = if self.curr_dir.x == 0 {
                (self.dst.1 * 140) as f32 - self.player.y
            } else {
                (self.dst.0 * 140) as f32 - self.player.x
            };
            self.player.mission_distance = ((self.blocks_passed * 140) + 130) as f32 - along.abs();
            self.travel_distance += self.speed * dt;
            if self.drifting {
                self.drift_distance += (inertia_len - self.tire_power * dt) * dt;
            }
            self.player.move_to();
            let (s, c) = ((DEG2RAD * self.player.angle).sin(), (DEG2RAD * self.player.angle).cos());
            self.player.cx = self.player.x + s * self.player.pitch;
            self.player.cy = self.player.y - c * self.player.pitch;
            self.player.cell_x = java_round(self.player.cx / BLOCK as f32);
            self.player.cell_y = java_round(self.player.cy / BLOCK as f32);
        } else {
            self.player.wheel_angle = 0.0;
        }

        if game_mode != mode::STARTING {
            for i in 0..self.cars.len() {
                let mut c = self.cars[i].clone();
                c.update(dt, &self.player, &self.barricades, &self.cars);
                self.cars[i] = c;
            }
        }

        self.g_cx = ((self.cur.0 * 140) as f32 - self.player.cx).abs();
        self.g_cy = ((self.cur.1 * 140) as f32 - self.player.cy).abs();
        self.g_dx = ((self.dst.0 * 140) as f32 - self.player.cx).abs();
        self.g_dy = ((self.dst.1 * 140) as f32 - self.player.cy).abs();

        if self.blocks_passed > self.blocks_for_mission && self.mission == Mission::Drift && self.drift_distance > 40.0 {
            self.checkpoint.active = true;
        }
        if self.speed > self.speed_max {
            self.speed_max = self.speed;
        }

        self.score = match self.mission {
            Mission::Vip => {
                150.0 - (((self.travel_time * 0.4) + self.drift_distance) + (self.brake_time * 0.5)) * self.comfort_consume
            }
            Mission::Fuel => (self.gas_left * 150.0) / self.gas_initial,
            Mission::BrokenBrake | Mission::Truck => 150.0 - self.travel_time,
            Mission::Parking => 135.0 - self.travel_time * 2.0,
            _ => 135.0 - self.travel_time,
        };
        self.poslist.insert(dt, self.player.angle);
    }

    // ------------------------------------------------------------ AI traffic management

    /// `addCars`: put a car of `kind` (never the player's own model) into the first free slot.
    #[allow(clippy::too_many_arguments)]
    fn add_cars(&mut self, kind: i32, cell: (i32, i32), dir: GridDir, offset: f32, lane: i32, accel: i32, init: i32, vmax: i32, rd: bool) {
        if let Some(slot) = self.cars.iter().position(|c| !c.visible) {
            let mut k = kind;
            if k >= self.car_mode as i32 {
                k += 1;
            }
            self.cars[slot].set(slot as i32, k as usize, cell.0, cell.1, dir, offset, lane, accel, init, vmax, rd);
        }
    }

    /// `iGetNextRandom()` — next pre-agreed traffic pick (0..=2) from the course data.
    fn next_ai_pick(&mut self) -> i32 {
        let v = self.info_cur.aicar[self.cur_ai % 40] as i32;
        self.cur_ai += 1;
        v
    }

    fn half_len(kind: i32) -> f32 {
        car(kind as usize).half_length
    }

    pub fn clear_across_cars(&mut self) {
        let cd = self.curr_dir;
        for c in self.cars.iter_mut().filter(|c| c.visible) {
            if cd.x == 0 && c.dir.y == 0.0 {
                c.visible = false;
            }
            if cd.y == 0 && c.dir.x == 0.0 {
                c.visible = false;
            }
        }
    }

    pub fn clear_far_away_cars(&mut self) {
        let (px, py) = (self.player.cx, self.player.cy);
        for c in self.cars.iter_mut() {
            if c.visible && (c.rec.c.x - px).abs() + (c.rec.c.y - py).abs() > 140.0 {
                c.visible = false;
            }
        }
    }

    pub fn clear_zone(&mut self, cx: i32, cy: i32, dir: GridDir) {
        let fx = (((cx * 4) + dir.x * 3) * 140 / 4) as f32;
        let fy = (((cy * 4) + dir.y * 3) * 140 / 4) as f32;
        for c in self.cars.iter_mut() {
            if c.visible
                && c.dir.x == dir.x as f32
                && c.dir.y == dir.y as f32
                && (fx - c.rec.c.x).abs() + (fy - c.rec.c.y).abs() < 70.0
            {
                c.visible = false;
            }
        }
    }

    /// `prepareCarsExceptStraight()` — populate the junction ahead when the route turns. During
    /// the final approach of VIP/Parking a queue of parked cars is laid out instead.
    pub fn prepare_cars_except_straight(&mut self, save: &SaveGame) {
        if self.rel_dir_next == 0 {
            return;
        }
        let rd = save.right_driver;
        let (dst, nxt) = (self.dst, self.nxt);
        let (nd, nrd) = (self.next_dir, self.next_r_dir);
        let mut lane_seed = self.rng.next_int_bound(3);
        let mut kind = self.rng.next_int_bound(29);
        let final_approach =
            self.blocks_passed >= self.blocks_for_mission && matches!(self.mission, Mission::Vip | Mission::Parking);
        if !final_approach {
            let mut last_lane = lane_seed;
            for i in 0..2 {
                let pick = self.next_ai_pick();
                let k1 = ((kind + 1) + self.rng.next_int_bound(28)) % 29;
                let base = (i * 21 + 38) as f32;
                let jitter = (self.rng.next_float() - 0.5) * 4.0;
                self.add_cars(k1, dst, nd, base + jitter * (4.5 - Self::half_len(k1)), (pick + 1) % 3, 4, i + 10, i * 4 + 30, rd);
                kind = ((k1 + 1) + self.rng.next_int_bound(24)) % 25;
                let j2 = (self.rng.next_float() - 0.5) * 4.0 * (4.5 - Self::half_len(kind));
                self.add_cars(kind, dst, nd, base + j2, (pick + 2) % 3, 4, i + 10, i * 4 + 32, rd);
                last_lane = pick;
            }
            kind = ((kind + 1) + self.rng.next_int_bound(28)) % 29;
            let j = (self.rng.next_float() - 0.5) * 4.0 * (4.5 - Self::half_len(kind)) + 80.0;
            self.add_cars(kind, dst, nd, j, last_lane, 4, 12, 38, rd);
            lane_seed = last_lane;
        } else {
            for i in 0..9 {
                if i == 4 {
                    continue; // the gap where the player parks
                }
                kind = ((kind + 1) + self.rng.next_int_bound(22)) % 27;
                self.add_cars(kind, dst, nd, i as f32 * 6.25 + 45.0, 2, 0, 0, 0, rd);
            }
        }
        for i in 0..4 {
            let k = ((kind + 1) + self.rng.next_int_bound(28)) % 29;
            lane_seed = if lane_seed != 1 { 1 } else { ((lane_seed + 1) + self.rng.next_int_bound(2)) % 3 };
            let base = ((i - 1) * 21 + 28) as f32;
            let jitter = (self.rng.next_float() - 0.5) * 4.0;
            self.add_cars(k, nxt, nrd, base + jitter * (4.5 - Self::half_len(k)), (lane_seed + 1) % 3, 4, i + 1, 32, rd);
            kind = ((k + 1) + self.rng.next_int_bound(28)) % 29;
            let j2 = (self.rng.next_float() - 0.5) * 4.0 * (4.5 - Self::half_len(kind));
            self.add_cars(kind, nxt, nrd, base + j2, (lane_seed + 2) % 3, 4, i + 1, 32, rd);
        }
    }

    /// `prepareCarsForStraight()` — traffic for a straight block (4 rows in Highway/Lane).
    pub fn prepare_cars_for_straight(&mut self, save: &SaveGame) {
        let rd = save.right_driver;
        let (dst, nxt) = (self.dst, self.nxt);
        let (nd, nrd) = (self.next_dir, self.next_r_dir);
        self.clear_zone(dst.0, dst.1, nd);
        let mut kind = self.rng.next_int_bound(29);
        let mut lane = self.rng.next_int_bound(3);
        if matches!(self.mission, Mission::Highway | Mission::Lane) {
            for i in 0..4 {
                let pick = self.next_ai_pick();
                let k1 = ((kind + 1) + self.rng.next_int_bound(28)) % 29;
                let base = (i * 21 + 28) as f32;
                let jitter = (self.rng.next_float() - 0.5) * 3.0;
                self.add_cars(k1, dst, nd, base + jitter * (4.5 - Self::half_len(k1)), (pick + 1) % 3, 4, i + 12, 38, rd);
                kind = ((k1 + 1) + self.rng.next_int_bound(28)) % 29;
                let j2 = (self.rng.next_float() - 0.5) * 3.0 * (4.5 - Self::half_len(kind));
                self.add_cars(kind, dst, nd, base + j2, (pick + 2) % 3, 4, i + 12, 38, rd);
                lane = pick;
            }
        } else {
            for i in 0..3 {
                let pick = self.next_ai_pick();
                let k1 = ((kind + 1) + self.rng.next_int_bound(28)) % 29;
                let base = (i * 21 - 10) as f32;
                let jitter = (self.rng.next_float() - 0.5) * 4.0;
                self.add_cars(k1, dst, nd, jitter * (4.5 - Self::half_len(k1)) + base, (pick + 1) % 3, 2, i + 18, 38, rd);
                kind = ((k1 + 1) + self.rng.next_int_bound(28)) % 29;
                let j2 = (self.rng.next_float() - 0.5) * 4.0 * (4.5 - Self::half_len(kind));
                self.add_cars(kind, dst, nd, base + j2, (pick + 2) % 3, 2, i + 18, 38, rd);
                lane = pick;
            }
        }
        for i in 0..3 {
            let l5 = if lane != 1 { 1 } else { ((lane + 1) + self.rng.next_int_bound(2)) % 3 };
            let k = ((kind + 1) + self.rng.next_int_bound(28)) % 29;
            let base = ((i - 1) * 21 + 28) as f32;
            let jitter = (self.rng.next_float() - 0.5) * 4.0;
            self.add_cars(k, nxt, nrd, base + jitter * (4.5 - Self::half_len(k)), (l5 + 1) % 3, 4, i + 1, 30, rd);
            kind = ((k + 1) + self.rng.next_int_bound(28)) % 29;
            let j2 = (self.rng.next_float() - 0.5) * 4.0 * (4.5 - Self::half_len(kind));
            self.add_cars(kind, nxt, nrd, base + j2, (l5 + 2) % 3, 4, i + 1, 30, rd);
            lane = l5;
        }
    }

    /// Snapshot for the economy layer (`onMissionFinished`).
    pub fn run_stats(&self) -> crate::economy::RunStats {
        crate::economy::RunStats {
            mission: Some(self.mission),
            car: self.car_mode,
            travel_distance: self.travel_distance,
            drift_distance: self.drift_distance,
            travel_time: self.travel_time,
            coins_acquired: self.coins_acquired,
            sub_missions: self.sub_mission_count,
            perfect_turns: self.perfect_turns,
            speed_max: self.speed_max,
            speed: self.speed,
            score: self.score,
        }
    }
}
