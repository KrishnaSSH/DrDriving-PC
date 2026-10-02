//! Port of package `Q` (vectors, circles, boxes, direction, rating maths).

/// `Q.f` rotates with this truncated constant (not the precise one used elsewhere).
const VEC_DEG2RAD: f32 = 0.01745;
/// Constant used by `drdriving.l` / `tool.l`.
pub const DEG2RAD: f32 = 0.017453292;
pub const RAD2DEG: f32 = 57.29578;

/// Grid pitch of the city: one block is 140 world units.
pub const BLOCK: i32 = 140;

/// Java `Math.round(float)`: floor(x + 0.5).
pub fn java_round(x: f32) -> i32 {
    (x + 0.5).floor() as i32
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn len(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    pub fn dist_sq(self, x: f32, y: f32) -> f32 {
        let dx = self.x - x;
        let dy = self.y - y;
        dx * dx + dy * dy
    }
    /// `self += o * s`
    pub fn add_scaled(&mut self, o: Vec2, s: f32) {
        self.x += o.x * s;
        self.y += o.y * s;
    }
    pub fn add(&mut self, o: Vec2) {
        self.x += o.x;
        self.y += o.y;
    }
    pub fn scale(&mut self, s: f32) {
        self.x *= s;
        self.y *= s;
    }
    /// Normalises in place; the zero vector is left unchanged (as in Java).
    pub fn normalize(&mut self) {
        let d = self.len();
        if d != 0.0 {
            self.x /= d;
            self.y /= d;
        }
    }
    /// Counter-clockwise rotation by `deg` degrees (uses the truncated 0.01745 constant).
    pub fn rotate_deg(&mut self, deg: f32) {
        let r = (deg * VEC_DEG2RAD) as f64;
        let (c, s) = (r.cos() as f32, r.sin() as f32);
        let (x, y) = (self.x, self.y);
        self.x = x * c - y * s;
        self.y = x * s + y * c;
    }
}

/// `Q.a`
#[derive(Clone, Copy, Debug, Default)]
pub struct Circle {
    pub c: Vec2,
    pub r: f32,
}

/// `Q.e` — axis-aligned box stored as centre + half extents.
#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub c: Vec2,
    pub hx: f32,
    pub hy: f32,
}

/// `Q.c` — a box that can be switched on/off (barricade, checkpoint, parking spot…).
#[derive(Clone, Copy, Debug)]
pub struct Zone {
    pub active: bool,
    pub rect: Rect,
}

impl Zone {
    /// `new Q.c()` — active, zero size.
    pub fn new() -> Self {
        Self { active: true, rect: Rect::default() }
    }
    pub fn with_half(hx: f32, hy: f32) -> Self {
        Self { active: true, rect: Rect { c: Vec2::ZERO, hx, hy } }
    }
}

impl Default for Zone {
    fn default() -> Self {
        Self::new()
    }
}

/// `Q.b` — unit grid direction. Index: 0 = -y ("north"), 1 = +x, 2 = +y, 3 = -x.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridDir {
    pub x: i32,
    pub y: i32,
}

impl Default for GridDir {
    fn default() -> Self {
        Self { x: 1, y: 0 }
    }
}

impl GridDir {
    /// `Q.b.a()`
    pub fn index(self) -> i32 {
        if self.y < 0 {
            0
        } else if self.y > 0 {
            2
        } else if self.x < 0 {
            3
        } else {
            1
        }
    }
    /// `Q.b.b(rel)`: absolute index after turning by relative direction `rel`, wrapped to 0..=3.
    pub fn turned_index(self, rel: i32) -> i32 {
        let mut i = self.index() + rel;
        while i < 0 {
            i += 4;
        }
        while i > 3 {
            i -= 4;
        }
        i
    }
    /// `Q.b.c(i)`: set from an absolute index.
    pub fn set_index(&mut self, i: i32) {
        match i.rem_euclid(4) {
            0 => *self = GridDir { x: 0, y: -1 },
            1 => *self = GridDir { x: 1, y: 0 },
            2 => *self = GridDir { x: 0, y: 1 },
            _ => *self = GridDir { x: -1, y: 0 },
        }
    }
}

/// `Q.d.a` — Elo-style expected score of the player with rating `mine` against `opp`
/// (logistic base 10, scale 4000).
pub fn expected_score(mine: f32, opp: f32) -> f32 {
    1.0 / (10f64.powf(((opp - mine) / 4000.0) as f64) as f32 + 1.0)
}

/// `Q.d.b` — circle vs box overlap.
pub fn circle_rect(c: &Circle, r: &Rect) -> bool {
    let (cx, cy) = (c.c.x, c.c.y);
    if cx + c.r < r.c.x - r.hx || cx - c.r > r.c.x + r.hx {
        return false;
    }
    if cy + c.r < r.c.y - r.hy || cy - c.r > r.c.y + r.hy {
        return false;
    }
    let px = cx.clamp(r.c.x - r.hx, r.c.x + r.hx);
    let py = cy.clamp(r.c.y - r.hy, r.c.y + r.hy);
    c.c.dist_sq(px, py) < c.r * c.r
}

/// `Q.d.c` — box vs box overlap.
pub fn rect_rect(a: &Rect, b: &Rect) -> bool {
    !(a.c.x - a.hx > b.c.x + b.hx
        || a.c.x + a.hx < b.c.x - b.hx
        || a.c.y - a.hy > b.c.y + b.hy
        || a.c.y + a.hy < b.c.y - b.hy)
}
