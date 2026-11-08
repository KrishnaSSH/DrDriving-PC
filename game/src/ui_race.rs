//! Race-side rendering: car select, the in-race HUD (`renderObject`), pause, success and ending.

use crate::game::*;
use crate::scene::{self, View};
use crate::gfx::Spr;
use crate::sprites as S;
use drdriving_core::economy;
use drdriving_core::mission::Mission;
use drdriving_core::world::World;
use macroquad::prelude::*;

const BLACKISH: Color = Color::new(0.0, 0.0, 0.0, 1.0);

impl Game {
    fn draw_world_ref(&self) -> &World {
        match &self.session {
            Some(s) => &s.world,
            None => &self.world,
        }
    }

    fn draw_rt(&self, rt: &RenderTarget, left: f32, top: f32, w: f32, h: f32) {
        let g = &self.g;
        draw_texture_ex(
            &rt.texture,
            g.sx(left, w, top, h),
            g.oy + top * g.k,
            WHITE,
            DrawTextureParams { dest_size: Some(vec2(w * g.k, h * g.k)), flip_y: true, ..Default::default() },
        );
    }

    // ------------------------------------------------------------------ car select (mode 1)

    pub fn render_car_select(&mut self) {
        clear_background(BLACKISH);
        let view = scene::carselect_view(&self.world, self.mode_time * 0.1, self.save.right_driver, &self.g);
        scene::render_world(&self.a, &self.world, self.save.right_driver, &view, &self.drifts);
        set_default_camera();
        self.render_gold(0, 0);
        let sh = self.g.sh;
        let (cw, w) = (400.0, &self.world);
        let g = &mut self.g;
        let ui = &self.a.ui;
        g.e(ui, S::UI[19], cw, sh - 35.0, g.vw / 2.0, 35.0);
        // (render_gold above is drawn first so the bar sits underneath the pads but over the scene)
        let m = w.mission.index();
        g.h(ui, S::mission_icon(m), 306.0, sh - 35.0);
        let rec = self.save.record[m];
        let rs = if rec > 0 { format!("{}", rec) } else { "N/A".to_string() };
        self.bf.b(g, ui, MISSION_NAMES[m], 348.0, ((sh - 35.0) - 12.0) + 2.0, 0.875);
        self.bf.b(g, ui, &format!("RECORD {}", rs), 348.0, ((sh - 35.0) + 12.0) + 2.0, 0.6);
        let c = w.car_mode;
        let i6 = 10000 - self.save.car_damage[c];
        let i7 = (sh * 0.7) as i32;
        let f5 = (i7 - 9) as f32;
        self.bf.b(g, ui, "Condition", 301.0, f5, 0.5);
        self.bf.f(g, ui, &format!("{}%", i6 / 100), 480.0, f5, 0.5);
        if i6 < 9000 {
            let f6 = (i7 + 9) as f32;
            self.bf.b(g, ui, "Repair Cost", 301.0, f6, 0.5);
            self.bf.f(g, ui, &format!("{}Coin", economy::repair_cost(c, self.save.car_damage[c])), 480.0, f6, 0.5);
        } else {
            self.bf.b(g, ui, "No need to repair", 301.0, (i7 + 9) as f32, 0.5);
        }
        let f7 = i7 as f32;
        g.d(ui, S::UI[9], 299.0, f7, 91.0, 3.0, 1);
        g.d(ui, S::UI[if i6 < 9000 { 11 } else { 10 }], 300.0, f7, i6 as f32 * 0.009, 2.0, 1);
        let rp = &self.pads.p[sp::REPAIR];
        g.h(ui, S::UI[7], rp.rx + 96.0, rp.ry);
        g.h(ui, S::UI[6], cw, sh * 0.25);
        self.bf.d(g, ui, CLASS_NAMES[c % 4], cw, (sh * 0.25) - 24.0, 0.5);
        g.set_tint(COLOR_RED);
        self.bf.c(g, ui, drdriving_core::cars::car(c).name, cw, sh * 0.25);
        g.clear_tint();
        self.render_screen_pad(false);
    }

    // ------------------------------------------------------------------ race (modes 2..6)

    pub fn render_race(&mut self) {
        let Some(s) = &self.session else { return };
        let rd = self.save.right_driver;
        let mode = self.mode;
        let cam4 = self.camera_mode == 4;
        let gear_active = self.pads.p[sp::GEAR].active;
        let w = &s.world;
        // mirrors (only through the driver's seat view): rendered into small targets
        let reverse = w.gear == 1;
        let show_back = cam4 && gear_active && (reverse || s.mirror_back);
        let show_left = cam4 && gear_active && (reverse || s.mirror_left);
        let show_right = cam4 && gear_active && (reverse || s.mirror_right);
        if show_back {
            scene::render_mirror(&self.a, w, rd, 0, &self.targets.back, &self.drifts);
        }
        if show_left {
            scene::render_mirror(&self.a, w, rd, 1, &self.targets.left, &self.drifts);
        }
        if show_right {
            scene::render_mirror(&self.a, w, rd, 2, &self.targets.right, &self.drifts);
        }
        let view: View = match mode {
            5 if matches!(w.mission, Mission::Vip | Mission::Parking) => scene::ending_view(w, self.mode_time, &self.g),
            5 => scene::success_view(w, self.success_cam, &self.g),
            6 => scene::ending_view(w, self.mode_time, &self.g),
            _ if cam4 => scene::driver_view(w, rd, &self.g),
            _ => scene::chase_view(w, self.camera_mode, &self.g),
        };
        clear_background(BLACKISH);
        scene::render_world(&self.a, w, rd, &view, &self.drifts);
        set_default_camera();

        // mirror frames + pictures
        let ui = &self.a.ui;
        if show_back {
            self.g.e(ui, S::BLACK, 400.0, 38.0, 128.0, 40.0);
            self.draw_rt(&self.targets.back, 275.0, -25.0, 250.0, 100.0);
        }
        if show_left {
            self.g.e(ui, S::BLACK, 212.5, 50.0, 40.0, 27.5);
            self.draw_rt(&self.targets.left, 175.0, 25.0, 75.0, 50.0);
        }
        if show_right {
            self.g.e(ui, S::BLACK, 587.5, 50.0, 40.0, 27.5);
            self.draw_rt(&self.targets.right, 550.0, 25.0, 75.0, 50.0);
        }
        self.render_hud();
    }

    fn render_hud(&mut self) {
        let Some(s) = &self.session else { return };
        let w = &s.world;
        let g = &mut self.g;
        let ui = &self.a.ui;
        let sh = g.sh;
        let (cw, ch) = (400.0, sh / 2.0);
        let mode = self.mode;
        let cam4 = self.camera_mode == 4;
        let rd = self.save.right_driver;
        let yard = self.save.show_yard;

        // gear selector (only when its pad is live and the car is slow)
        let gp = &self.pads.p[sp::GEAR];
        if gp.active && w.speed < 10.0 {
            let (f7, f8) = (gp.x, gp.y);
            let f9 = (if gp.on { gp.cy as f32 } else { (w.gear * 40 - 60) as f32 }).clamp(-60.0, 60.0);
            g.h(ui, S::GEAR[0], f7 - 2.0, f8 - 10.0);
            g.h(ui, S::GEAR[1], 1.0 + f7, (f9 + f8) - 8.0);
            let f10 = f7 - 26.0;
            g.h(ui, S::GEAR[if w.gear == 0 { 2 } else { 3 }], f10, (f8 - 60.0) - 5.0);
            g.h(ui, S::GEAR[if w.gear == 1 { 4 } else { 5 }], f10, (f8 - 20.0) - 5.0);
            g.h(ui, S::GEAR[if w.gear == 2 { 6 } else { 7 }], f10, (f8 + 20.0) - 5.0);
            g.h(ui, S::GEAR[if w.gear == 3 { 8 } else { 9 }], f10, (f8 + 60.0) - 5.0);
        }
        // dashboard + needles
        if cam4 && matches!(mode, 2 | 3 | 4) {
            let tex = &self.a.interior[w.player.kind];
            g.h(tex, S::INTERIOR[0], cw, sh - 56.0);
            if g.shift > 0.5 {
                // extend the dashboard to the screen edges with its outermost columns
                let d = S::INTERIOR[0];
                let top = (sh - 56.0) - d.h / 2.0;
                g.pin.set(Some(0.0));
                g.blit(tex, Spr { x: d.x, y: d.y, w: 6.0, h: d.h }, 0.0, top, g.shift + 1.0, d.h, false, 0.0);
                g.blit(tex, Spr { x: d.x + d.w - 6.0, y: d.y, w: 6.0, h: d.h }, 800.0 + g.shift - 1.0, top, g.shift + 1.0, d.h, false, 0.0);
                g.pin.set(None);
            }
            let sp_ = w.speed * (if yard { 0.6213 } else { 1.0 }) * 4.86 + 45.0;
            g.f(ui, S::INTERIOR[1], cw - 139.0, sh - 52.0, sp_);
            g.f(ui, S::INTERIOR[1], cw + 134.0, sh - 52.0, 315.0 - (w.gas_left / w.gas_initial) * 88.0);
        }
        // navigation + coin stack + blinkers + timer (driving only)
        if mode == 3 {
            let (dx, dy, cx, cy) = (w.g_dx, w.g_dy, w.g_cx, w.g_cy);
            let ahead = w.rel_dir_next;
            let dist = if w.dst.0 == w.cur.0 { dy as i32 } else { dx as i32 };
            if !w.parking.active && cx + cy > 20.0 && dx < 90.0 && dy < 90.0 && ahead > 0 && dist > 12 {
                let nav = S::NAVI[(ahead - 1) as usize];
                if ahead == 2 && rd {
                    g.e(ui, nav, cw, 48.0, -1.0, 1.0);
                } else {
                    g.h(ui, nav, cw, 48.0);
                }
                if yard {
                    self.bf.e(g, ui, &format!("{}yard", (((dist - 9) as f32 * 1.093613) / 10.0) as i32 * 10), cw + 45.0, 106.0);
                } else {
                    self.bf.e(g, ui, &format!("{}m", ((dist - 9) / 10) * 10), cw + 32.0, 106.0);
                }
            }
            let mut n = w.coins_acquired;
            if n > 0 {
                if self.moving_coin.visible {
                    n -= 1;
                    g.h(ui, S::COIN, self.moving_coin.x + 780.0, ((ch - 10.0) - (n * 5) as f32) + self.moving_coin.y);
                }
                for i in 0..n {
                    g.h(ui, S::COIN, 780.0, (ch - 10.0) - (i * 5) as f32);
                }
            }
            if cam4 {
                let on = (w.travel_time - ((w.travel_time / 0.84) as i32 as f32) * 0.84) < 0.42;
                if w.emergency {
                    g.h(ui, S::EMERGENCY, 398.0, sh - 74.0);
                }
                if (w.blinker != 0 || w.emergency) && on {
                    if w.emergency || w.blinker < 0 {
                        g.h(ui, S::BLINKER_L, 334.0, sh - 74.0);
                    }
                    if w.emergency || w.blinker > 0 {
                        g.h(ui, S::BLINKER_R, 462.0, sh - 74.0);
                    }
                }
            }
            if self.effect_turn.visible {
                let f5 = if cam4 { sh - 140.0 } else { sh - 80.0 };
                g.h(ui, S::TURN[self.effect_turn.rate as usize], self.effect_turn.x, f5 + self.effect_turn.y);
            }
            if w.sub_mission_lane >= 0 && !w.parking.active {
                // box and marker must share one anchor offset or the marker drifts out of the box
                g.pin.set(Some(g.off(600.0, 40.0)));
                g.h(ui, S::NAVI[3], 600.0, 40.0);
                let off = (if rd { -1.0 } else { 1.0 }) * 60.0 * (w.sub_mission_lane - 1) as f32 + 600.0;
                g.h(ui, S::NAVI[4], off, 40.0);
                g.pin.set(None);
            }
            let t = ((18000.0 - w.travel_time * 100.0) as i32).max(0);
            let (min, rest) = (t / 6000, t % 6000);
            let (sec, hun) = (rest / 100, rest % 100);
            self.bf.a(g, ui, &format!("{}:{:02}.{:02}", min, sec, hun), 120.0, 12.0);
        }
        // online: who is ahead, and the forfeit prompt
        if w.online && matches!(mode, 2 | 3) {
            // `((int)((mine - theirs) * 0.2)) * 5`, 0 until we have driven 10 m
            let mut gap = ((w.player.mission_distance - w.opponent.mission_distance) * 0.2) as i32 * 5;
            if w.travel_distance < 10.0 {
                gap = 0;
            }
            let unit = if self.save.show_yard { "yd " } else { "m " };
            let side = if w.player.mission_distance > w.opponent.mission_distance { "ahead" } else { "behind" };
            self.bf.e(g, ui, &format!("{}{}{}", gap.abs(), unit, side), 230.0, 36.0);
            if self.forfeit_armed > 0.0 {
                self.text.d(g, if self.touch_seen { "Tap the menu button again to forfeit" } else { "Press Esc again to forfeit" }, cw, sh * 0.3, 0, 24.0);
            }
        }
        // left-hand mission gauge
        if mode == 3 || mode == 2 {
            let fx = if self.save.left_wheel { 800.0 - 40.0 } else { 40.0 };
            g.e(ui, S::GAUGE[0], fx, ch - 16.0, 1.0, 0.6);
            let m = w.mission;
            let mi = m.index();
            let hidden = (m == Mission::Drift && w.drift_distance < 40.0) || (m == Mission::Lane && w.sub_mission_count < 12);
            if !hidden {
                let full = if matches!(mi, 0 | 1 | 2 | 5 | 8) { 135.0 } else { 150.0 };
                let sc = w.score;
                if sc > 0.0 {
                    g.e(ui, S::GAUGE[1], fx, ((ch - 16.0) + 72.0) - (((sc * 120.0) * 0.6) / full), 1.0, ((sc * 1.2) * 0.6) / full);
                }
                if self.save.record[mi] > 0 {
                    g.h(ui, S::GAUGE[2], fx, ((ch - 16.0) + 72.0) - ((((w.score_last * 2.0) * 120.0) * 0.6) / full));
                }
                let label = match m {
                    Mission::Fuel => "Fuel",
                    Mission::Vip => "Comfort",
                    _ => "Time",
                };
                self.bf.d(g, ui, label, fx, ch - 100.0, 0.625);
            }
            if matches!(m, Mission::Drift | Mission::Lane) {
                let f26 = if m == Mission::Drift { w.drift_distance * 3.0 } else { (w.sub_mission_count * 10) as f32 };
                g.e(ui, S::GAUGE[0], fx, ch - 16.0, 1.0, 0.6);
                g.e(ui, S::GAUGE[3], fx, ((ch - 16.0) + 72.0) - (f26 * 0.6), 1.0, f26 * 0.01 * 0.6);
                self.bf.d(g, ui, if m == Mission::Drift { "Drift" } else { "Lane" }, fx, ch - 112.0, 0.625);
            }
        }
        if cam4 && matches!(mode, 2 | 3 | 4) {
            g.e(ui, if yard { S::MPH } else { S::KMPH }, 261.0, sh - 19.0, 0.8, 0.8);
        }
        // wheel and pedals (interior atlas)
        let tex = &self.a.interior[w.player.kind];
        let wp = &self.pads.p[sp::WHEEL];
        if wp.visible {
            g.b(tex, S::WHEEL, wp.x, wp.y, self.wheel_zoom, self.wheel_zoom, wp.angle);
        }
        let ap = &self.pads.p[sp::ACCEL];
        if ap.visible {
            if ap.on || self.kb_accel {
                g.e(tex, S::ACCEL, ap.x, ap.y + 11.0, 1.0, 0.8);
            } else {
                g.h(tex, S::ACCEL, ap.x, ap.y);
            }
        }
        let bp = &self.pads.p[sp::BRAKE];
        if bp.visible {
            if bp.on || self.kb_brake {
                g.e(tex, S::BRAKE, bp.x, bp.y + 7.0, 1.0, 0.8);
            } else {
                g.h(tex, S::BRAKE, bp.x, bp.y);
            }
        }
        let cp = &self.pads.p[sp::CAMERA];
        if cp.visible {
            g.h(ui, S::CAMERA, cp.rx, cp.ry);
        }
        if mode == 2 && w.online {
            g.h(ui, S::UI[21], cw, ch + 50.0); // plate behind the opponent's name
        }
        if mode == 2 && self.mode_time > 0.6 {
            let i = ((4.0 - ((self.mode_time * 5.0) / 3.0)) as i32).clamp(0, 2) as usize;
            g.h(ui, S::COUNT[i], cw, ch - 78.0);
            g.h(ui, S::UI[5], cw, ch + 10.0);
            self.bf.c(g, ui, MISSION_NAMES[w.mission.index()], cw, ch + 10.0);
        }
        if mode == 2 && w.online {
            // opponent's name and flag under the countdown
            let name = if w.opponent.name.chars().count() > 1 { w.opponent.name.as_str() } else { "Opponent" };
            let half = self.text.d(g, name, cw, ch + 50.0, 0, 24.0);
            g.e(&self.a.world, S::flag(w.opponent.country), cw - 40.0 - half, ch + 50.0, 0.75, 0.75);
        }
        if mode == 3 && w.parking.active {
            self.text.d(g, if w.in_parking_lot { "Change gear to P" } else { "Park at the yellow zone" }, cw, sh - 92.0, 0, 27.0);
        }
    }

    pub fn render_pause(&mut self) {
        let sh = self.g.sh;
        let (cw, ch) = (400.0, sh / 2.0);
        let a = self.mode_time.min(0.3);
        let g = &mut self.g;
        g.set_tint(Color::new(1.0, 1.0, 1.0, a));
        g.e(&self.a.ui, S::BLACK, cw, ch, g.vw / 2.0, ch);
        g.clear_tint();
        g.e(&self.a.ui, S::BLACK, cw, sh - 35.0, g.vw / 2.0, 35.0);
        self.render_gold(0, 0);
        if ((self.mode_time * 2.0) as i32) % 3 != 0 {
            self.bf.c(&self.g, &self.a.ui, "Game Paused", cw, sh - 35.0);
        }
        self.render_screen_pad(false);
    }

    pub fn render_success(&mut self) {
        if self.is_online_race() {
            self.render_multi_result(true);
            return;
        }
        self.render_gold(0, 0);
        if self.mode_time <= 2.5 {
            return;
        }
        let sh = self.g.sh;
        let cw = 400.0;
        let f7 = (sh - 340.0).max(15.0);
        let g = &mut self.g;
        let ui = &self.a.ui;
        g.h(ui, S::UI[20], 600.0, sh - 150.0);
        for i in 0..4 {
            g.e(ui, S::UI[5], 400.0, f7 + 30.0 * i as f32, 1.0, 0.84);
        }
        g.e(ui, S::UI[19], cw, sh - 35.0, g.vw / 2.0, 35.0);
        let r = &self.result;
        self.bf.d(g, ui, if r.new_record { "New Record!" } else { "Mission Success" }, cw, sh - 35.0, 1.2);
        self.bf.b(g, ui, "Last Record", 280.0, f7, 0.75);
        self.bf.f(g, ui, &format!("{}", r.last_record), 520.0, f7, 0.75);
        self.bf.b(g, ui, "Mission Score", 280.0, f7 + 30.0, 0.75);
        self.bf.f(g, ui, &format!("{}", r.record), 520.0, f7 + 30.0, 0.75);
        self.bf.b(g, ui, "Mission Reward", 280.0, f7 + 60.0, 0.75);
        self.bf.f(g, ui, &format!("{}Coin", r.reward), 520.0, f7 + 60.0, 0.75);
        let last = r.last_exp as f32;
        let exp = self.save.driver_exp as f32;
        let f13 = (last + ((self.mode_time - 3.0) * 0.5) * (exp - last)).clamp(last, exp.max(last));
        let lvl = economy::level_for_exp(f13);
        let base = economy::base_exp_for_level(lvl) as f32;
        let next = economy::base_exp_for_level(lvl + 1) as f32;
        self.bf.b(g, ui, &format!("Driver Level {}", lvl + 1), 280.0, f7 + 90.0, 0.75);
        let mut pct = (((f13 - base) * 10000.0) / (next - base)) as i32;
        if lvl > 8 {
            pct = 0;
        }
        self.bf.f(g, ui, &format!("{}.{:02}%", pct / 100, pct % 100), 520.0, f7 + 90.0, 0.75);
        self.render_screen_pad(false);
    }

    pub fn render_ending(&mut self) {
        if self.is_online_race() {
            self.render_multi_result(false);
            return;
        }
        let sh = self.g.sh;
        let (cw, ch) = (400.0, sh / 2.0);
        let t = self.mode_time;
        let driver = self.draw_world_ref().driver.active;
        let mut f3 = if driver {
            if t < 0.2 {
                1.0
            } else if t < 2.2 {
                1.1 - t * 0.5
            } else if t < 3.0 {
                0.0
            } else {
                ((t * 0.05) - 0.15).min(0.6)
            }
        } else {
            (0.1 * t).min(0.6)
        };
        if f3 > 0.99 {
            f3 = 1.0;
        }
        let g = &mut self.g;
        g.set_tint(Color::new(1.0, 1.0, 1.0, f3));
        g.e(&self.a.ui, S::BLACK, cw, ch, g.vw / 2.0, ch);
        g.clear_tint();
        g.e(&self.a.ui, S::UI[19], cw, sh - 35.0, g.vw / 2.0, 35.0);
        self.render_gold(0, 0);
        if self.mode_time > 1.2 {
            self.bf.d(&self.g, &self.a.ui, "Mission Failed", cw, sh - 35.0, 1.2);
        }
        self.render_screen_pad(false);
    }

    // ------------------------------------------------------------------ dispatch

    pub fn draw(&mut self) {
        self.set_anchor();
        self.g.layout();
        clear_background(BLACKISH);
        match self.mode {
            0 => self.render_loading(100),
            1 => self.render_car_select(),
            2 | 3 => self.render_race(),
            4 => {
                self.render_race();
                self.render_pause();
            }
            5 => {
                self.render_race();
                self.render_success();
            }
            6 => {
                self.render_race();
                self.render_ending();
            }
            8 => {
                self.render_tab_profile();
                self.render_screen_pad(false);
            }
            9 => {
                self.render_tab_cargo();
                self.render_screen_pad(false);
            }
            10 => {
                self.render_tab_store();
                self.render_screen_pad(false);
            }
            11 => {
                self.render_tab_option();
                self.render_screen_pad(false);
            }
            12 => self.render_dialog(),
            13 => self.render_exit(),
            14 => self.render_lobby(),
            15 => self.render_picker(),
            _ => {}
        }
        if matches!(self.mode, 8..=12) {
            self.render_search_overlay();
        }
        self.render_toast();
        if self.help {
            self.render_help();
        }
        // letter-box bars when the window is narrower than 4:3
        let (sw, sh) = (screen_width(), screen_height());
        if self.g.oy > 0.5 {
            draw_rectangle(0.0, 0.0, sw, self.g.oy, BLACK);
            draw_rectangle(0.0, sh - self.g.oy, sw, self.g.oy, BLACK);
        }
    }

    /// Keyboard reference (toggled with H outside a race) — the original has no keyboard.
    fn render_help(&mut self) {
        let g = &mut self.g;
        let sh = g.sh;
        g.set_tint(Color::new(1.0, 1.0, 1.0, 0.8));
        g.e(&self.a.ui, S::BLACK, 400.0, sh / 2.0, g.vw / 2.0, sh / 2.0);
        g.clear_tint();
        let lines = [
            "KEYBOARD",
            "W / Up: accelerate     S / Down: brake",
            "A D / Left Right: steer     1 2 3 4: gear P R N D",
            "Q / E: indicators     H: hazard     Space: horn",
            "C: camera     B / Z / X: mirrors     Esc / P: pause",
            "Menus: Tab / F1-F4 pages, Space: start, Enter: OK, Esc: back",
            "Garage / Store / Options: 1-6 tap the cards, Left/Right scroll",
            "(Mouse and touch work like the original.)   H: close",
        ];
        for (i, l) in lines.iter().enumerate() {
            self.text.d(g, l, 400.0, 70.0 + i as f32 * 34.0, 0, if i == 0 { 30.0 } else { 20.0 });
        }
    }
}
