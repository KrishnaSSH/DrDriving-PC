//! On-screen "pads" (buttons, the steering wheel, pedals, gear slider): a direct port of
//! `tool.b` / `tool.n`. The original feeds them touch events; here mouse and touch both do.
//! Pad types: 1 = analog wheel, 2 = held button, 3 = click button, 4 = gear slider.

pub const MAX_PADS: usize = 64;
pub const PAD_ANALOG: i32 = 1;
pub const PAD_BUTTON: i32 = 2;
pub const PAD_CLICK: i32 = 3;
pub const PAD_GEAR: i32 = 4;

#[derive(Clone, Default)]
pub struct Pad {
    pub angle: f32,
    pub angle_last: f32,
    pub cpx: i32,
    pub cpy: i32,
    pub cx: i32,
    pub cy: i32,
    pub visible: bool,
    pub rx: f32,
    pub ry: f32,
    pub szx: f32,
    pub szy: f32,
    pub x: f32,
    pub y: f32,
    pub on: bool,
    pub active: bool,
    pub clicked: bool,
    /// menu.png sprite index drawn for this pad (-1 = none)
    pub texture: i32,
    pub kind: i32,
}

impl Pad {
    pub fn set_pad(&mut self, kind: i32, tex: i32, x: f32, y: f32, szx: f32, szy: f32) {
        self.kind = kind;
        self.texture = tex;
        self.x = x;
        self.y = y;
        self.rx = x;
        self.ry = y;
        self.szx = szx;
        self.szy = szy;
    }
}

pub struct Pads {
    pub p: Vec<Pad>,
    f_angle: f32,
    touch_count: i32,
    /// set when a click was consumed (the original plays `click1.ogg` then)
    pub click_pending: bool,
}

impl Pads {
    pub fn new() -> Pads {
        Pads { p: vec![Pad { texture: -1, ..Default::default() }; MAX_PADS], f_angle: 0.0, touch_count: 0, click_pending: false }
    }

    pub fn activate(&mut self, i: usize) {
        let n = &mut self.p[i];
        if n.active {
            return;
        }
        n.active = true;
        n.visible = true;
        n.on = false;
        n.clicked = false;
    }

    pub fn clear(&mut self) {
        for n in self.p.iter_mut() {
            n.active = false;
            n.visible = false;
            n.on = false;
            n.clicked = false;
            n.angle = 0.0;
        }
    }

    pub fn deactivate(&mut self, i: usize) {
        let n = &mut self.p[i];
        n.active = false;
        n.on = false;
        n.clicked = false;
    }

    pub fn deactivate_all(&mut self) {
        for n in self.p.iter_mut() {
            n.active = false;
            n.on = false;
            n.clicked = false;
            n.angle = 0.0;
        }
    }

    /// `GetClicked`: consume a click (and request the click sound).
    pub fn get_clicked(&mut self, i: usize) -> bool {
        let c = self.p[i].clicked;
        if c {
            self.click_pending = true;
        }
        self.p[i].clicked = false;
        c
    }

    pub fn hide(&mut self, i: usize) {
        let n = &mut self.p[i];
        n.active = false;
        n.visible = false;
        n.on = false;
        n.clicked = false;
    }

    pub fn show(&mut self, i: usize) {
        let n = &mut self.p[i];
        if n.visible {
            return;
        }
        n.active = true;
        n.visible = true;
        n.on = false;
        n.clicked = false;
    }

    fn wrap360(a: &mut f32) {
        while *a < 0.0 {
            *a += 360.0;
        }
        while *a > 360.0 {
            *a -= 360.0;
        }
    }

    pub fn on_began(&mut self, x: f32, y: f32) {
        self.touch_count += 1;
        for i in 0..self.p.len() {
            let n = &self.p[i];
            if !n.active {
                continue;
            }
            let (px, sx, py, sy) = (n.x, n.szx, n.y, n.szy);
            if !(x >= (px - sx) - 32.0 && x <= px + sx + 32.0 && y >= (py - sy) - 32.0 && y <= py + sy + 32.0) {
                continue;
            }
            if self.p[i].kind == PAD_ANALOG {
                let len = (x - px).abs() + (y - py).abs();
                if len < 24.0 {
                    self.p[i].clicked = true;
                }
                if len < 180.0 && len > 5.0 {
                    self.f_angle = (y - py).atan2(x - px).to_degrees();
                    Self::wrap360(&mut self.f_angle);
                    let fa = self.f_angle;
                    let n = &mut self.p[i];
                    if n.on {
                        let mut d = fa - n.angle_last;
                        if d > 180.0 {
                            d -= 360.0;
                        }
                        if d < -180.0 {
                            d += 360.0;
                        }
                        if d > 25.0 {
                            d = 25.0;
                        }
                        // (sic) the original clamps the negative side to +25 as well
                        let step = if d >= -25.0 { d } else { 25.0 };
                        n.angle = (n.angle + step).clamp(-540.0, 540.0);
                        n.angle_last = fa;
                    } else {
                        n.angle_last = fa;
                    }
                    n.on = true;
                }
            }
            let n = &mut self.p[i];
            let k = n.kind;
            if k == PAD_BUTTON || k == PAD_CLICK || k == PAD_GEAR {
                n.on = false;
                if x > px - sx && x < px + sx && y > py - sy && y < py + sy {
                    n.on = true;
                    n.cpx = (x - px) as i32;
                    n.cpy = (y - py) as i32;
                    n.cx = (x - px) as i32;
                    n.cy = (y - py) as i32;
                }
                if k == PAD_CLICK {
                    if n.on {
                        n.rx = px + 3.0;
                        n.ry = py + 3.0;
                    } else {
                        n.rx = px;
                        n.ry = py;
                    }
                }
            }
        }
    }

    pub fn on_ended(&mut self, x: f32, y: f32) {
        self.touch_count -= 1;
        for n in self.p.iter_mut() {
            if !n.active {
                continue;
            }
            let (px, sx, py, sy) = (n.x, n.szx, n.y, n.szy);
            if x >= (px - sx) - 50.0 && x <= px + sx + 50.0 && y >= (py - sy) - 50.0 && y <= py + sy + 50.0 {
                n.on = false;
                n.rx = px;
                n.ry = py;
                if x >= px - sx && x <= px + sx && y >= py - sy && y <= py + sy {
                    if n.kind == PAD_ANALOG {
                        n.clicked = false;
                    } else if n.kind == PAD_CLICK {
                        n.clicked = true;
                    }
                }
            }
        }
        if self.touch_count < 1 {
            self.touch_count = 0;
            for n in self.p.iter_mut() {
                n.on = false;
            }
        }
    }

    pub fn on_moved(&mut self, x: f32, y: f32) {
        for i in 0..self.p.len() {
            let n = &self.p[i];
            if !n.active {
                continue;
            }
            let (px, sx, py, sy) = (n.x, n.szx, n.y, n.szy);
            if !(x >= (px - sx) - 50.0 && x <= px + sx + 50.0 && y >= (py - sy) - 50.0 && y <= py + sy + 50.0) {
                continue;
            }
            if self.p[i].kind == PAD_ANALOG {
                let len = (x - px).abs() + (y - py).abs();
                if len < 200.0 && len > 5.0 {
                    self.f_angle = (y - py).atan2(x - px).to_degrees();
                    Self::wrap360(&mut self.f_angle);
                    let fa = self.f_angle;
                    let n = &mut self.p[i];
                    if n.on {
                        let mut d = fa - n.angle_last;
                        while d > 180.0 {
                            d -= 360.0;
                        }
                        while d < -180.0 {
                            d += 360.0;
                        }
                        let d = d.clamp(-50.0, 50.0);
                        n.angle = (n.angle + d).clamp(-540.0, 540.0);
                        n.angle_last = fa;
                    } else {
                        n.angle_last = fa;
                    }
                    n.on = true;
                }
            }
            let n = &mut self.p[i];
            let k = n.kind;
            if k == PAD_BUTTON || k == PAD_CLICK {
                if x <= px - sx || x >= px + sx {
                    n.on = false;
                } else if y <= py - sy || y >= py + sy {
                    n.on = false;
                } else {
                    n.on = true;
                    n.cx = (x - px) as i32;
                    n.cy = (y - py) as i32;
                }
                if k == PAD_CLICK {
                    if n.on {
                        n.rx = px + 3.0;
                        n.ry = py + 3.0;
                    } else {
                        n.rx = px;
                        n.ry = py;
                    }
                }
            }
            if k == PAD_GEAR {
                if x <= px - sx || x >= px + sx {
                    n.on = false;
                } else if y <= py - sy || y >= py + sy {
                    n.on = false;
                } else {
                    n.cx = (x - px) as i32;
                    n.cy = (y - py) as i32;
                }
            }
        }
    }
}
