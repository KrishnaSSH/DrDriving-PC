//! Menu screens: background, profile, garage, store, options, dialog, exit, loading and the
//! generic pad-sprite drawing (`renderScreenPad`). Coordinates and sprite indices are the
//! original's.

use crate::game::*;
use crate::sprites as S;
use drdriving_core::cars::car;
use drdriving_core::economy;
use macroquad::prelude::*;

const DARK: Color = Color::new(0.05, 0.05, 0.05, 1.0);

impl Game {
    // ------------------------------------------------------------------ processing

    fn scroll_drag(&mut self, which: u8, lo: f32, hi: f32) {
        let wheel = Self::wheel_notches();
        let pad = self.pads.p[sp::SCROLL].clone();
        let sc = match which {
            0 => &mut self.tab_cargo,
            1 => &mut self.tab_option,
            _ => &mut self.tab_store,
        };
        if pad.on {
            self.fling = self.fling * 0.5 + ((sc.drag - pad.cpx as f32) + pad.cx as f32) * 80.0;
            sc.drag = (pad.cpx - pad.cx) as f32;
        } else {
            sc.scroll -= self.fling * 0.01;
            // the mouse wheel moves the strip half a card per notch
            sc.scroll = (sc.scroll - wheel * 105.0).clamp(lo, hi);
            sc.drag = 0.0;
        }
    }

    fn fling_decay(&mut self) {
        let f = self.fling;
        self.fling = if f > 100.0 || f < -100.0 { f * 0.95 } else { 0.0 };
    }

    /// Returns the tapped card index (relative to the first ability/offer card) if the scroll
    /// pad was clicked (not dragged), or a card chosen with the keyboard.
    fn scroll_tap(&mut self, which: u8, lo: f32, hi: f32) -> Option<i32> {
        let key = self.card_key();
        if self.pads.p[sp::SCROLL].clicked {
            self.pads.p[sp::SCROLL].clicked = false;
            let pad = self.pads.p[sp::SCROLL].clone();
            let sc = match which {
                0 => &mut self.tab_cargo,
                1 => &mut self.tab_option,
                _ => &mut self.tab_store,
            };
            let d = (pad.cpx - pad.cx) as f32;
            sc.scroll = (sc.scroll + d).clamp(lo, hi);
            if d > -10.0 && d < 10.0 {
                return Some(((sc.scroll as i32 + pad.cpx + 1555) / 210) - 5);
            }
        }
        key.map(|k| k as i32)
    }

    pub fn process_tab_cargo(&mut self) {
        self.fling_decay();
        let c = self.cargo.car;
        for i in 0..6 {
            self.cargo.ability[i] += (car(c).ability[i] as f32 - self.cargo.ability[i]) * 0.1;
            self.cargo.upgrade[i] += (self.save.upgrade[c][i] as f32 - self.cargo.upgrade[i]) * 0.1;
        }
        self.scroll_drag(0, -340.0, 360.0);
        if let Some(i) = self.scroll_tap(0, -340.0, 360.0) {
            if (0..6).contains(&i) {
                let i = i as usize;
                let lvl = self.save.upgrade[c][i];
                if lvl > 9 {
                    self.set_dialog(false, 1, 0, 0);
                    return;
                }
                if lvl > self.save.driver_level {
                    self.set_dialog(false, 2, 0, 0);
                    return;
                }
                match economy::upgrade_cost(&self.save, c, i) {
                    None => return,
                    Some(cost) if cost > self.save.coin => {
                        self.set_dialog(true, 4, 0, 0);
                        return;
                    }
                    Some(_) => {
                        self.set_dialog(true, 1, c as i32, i as i32);
                        return;
                    }
                }
            }
        }
        if self.pads.get_clicked(sp::BUY_CAR) && !self.save.has_car[c] {
            let price = car(c).price;
            let (have, need) = if price < 0 { (self.save.coin, -price) } else { (self.save.gold, price) };
            if need <= have {
                self.set_dialog(true, 0, c as i32, 0);
            } else if price < 0 {
                self.set_dialog(true, 4, 0, 0);
            } else {
                self.set_dialog(true, 5, 0, 0);
            }
            return;
        }
        if self.pads.get_clicked(sp::RENT_CAR) && self.save.car_rent[c] < 1 && c >= 16 {
            let price = car(c).price;
            let (have, need) = if price < 0 { (self.save.coin, -price) } else { (self.save.gold, price) };
            if need <= have {
                self.set_dialog(true, 6, c as i32, 0);
            } else if price < 0 {
                self.set_dialog(true, 4, 0, 0);
            } else {
                self.set_dialog(true, 5, 0, 0);
            }
            return;
        }
        let mut delta = 0i32;
        if self.pads.get_clicked(sp::PROFILE_L) {
            delta = -1;
        }
        if self.pads.get_clicked(sp::PROFILE_R) {
            delta = 1;
        }
        if delta != 0 {
            let n = 28i32;
            self.cargo.car = ((self.cargo.car as i32 + delta + n) % n) as usize;
            let c = self.cargo.car;
            let owned = self.save.has_car[c] || self.save.car_rent[c] > 0;
            if owned {
                self.pads.hide(sp::BUY_CAR);
                self.pads.hide(sp::RENT_CAR);
            } else if c < 16 {
                self.pads.show(sp::BUY_CAR);
                self.pads.hide(sp::RENT_CAR);
            } else {
                self.pads.show(sp::RENT_CAR);
                self.pads.hide(sp::BUY_CAR);
            }
        }
    }

    pub fn process_tab_store(&mut self) {
        self.fling_decay();
        self.scroll_drag(2, -130.0, 360.0);
        if let Some(i) = self.scroll_tap(2, -130.0, 360.0) {
            let gold = self.save.gold;
            match i {
                0 | 1 | 2 => {
                    let (coins, g) = economy::EXCHANGE_RATES[i as usize];
                    if gold < g {
                        self.set_dialog(false, 0, 0, 0);
                    } else {
                        self.set_dialog(true, 2, coins, g);
                    }
                }
                3 | 4 | 5 => {
                    if gold <= 1800 && self.save.bought_gold <= 12000 {
                        let n = [200, 600, 1500][(i - 3) as usize];
                        self.set_dialog(true, 3, n, 0);
                    }
                }
                _ => {}
            }
        }
    }

    pub fn process_tab_option(&mut self) {
        self.fling_decay();
        if let Some(i) = self.scroll_tap(1, -130.0, 990.0) {
            let s = &mut self.save;
            match i {
                0 => s.music_disabled = !s.music_disabled,
                1 => s.sound_disabled = !s.sound_disabled,
                2 => {
                    if !s.accel_disabled {
                        s.accel_disabled = true;
                        s.left_wheel = false;
                    } else if s.left_wheel {
                        s.accel_disabled = false;
                        s.left_wheel = false;
                    } else {
                        s.left_wheel = true;
                    }
                    self.adjust_wheel_position();
                }
                3 => {
                    s.wheel_sensitivity += 1;
                    if s.wheel_sensitivity > 4 {
                        s.wheel_sensitivity = -4;
                    }
                }
                4 => s.right_driver = !s.right_driver,
                5 => s.show_yard = !s.show_yard,
                6 => {
                    s.gui_size = (s.gui_size + 1) % 3;
                    self.g.scale = crate::gfx::Gfx::scale_for(s.gui_size);
                    self.last_sh = 0.0;
                }
                7 => {
                    self.edit_name();
                    return;
                }
                8 => {
                    self.edit_country();
                    return;
                }
                _ => return,
            }
            self.persist();
        }
        self.scroll_drag(1, -130.0, 990.0);
    }

    pub fn process_tab_profile(&mut self) {
        // Online Match searches straight away; vs Friend and Channel both ask for a channel number
        if self.pads.get_clicked(sp::ONLINE) {
            if !self.searching() {
                self.start_quick();
            }
            return;
        }
        if self.pads.get_clicked(sp::ONLINE_FRIEND) || self.pads.get_clicked(sp::ONLINE_CHANNEL) {
            self.enter_lobby();
            return;
        }
        if self.pads.get_clicked(sp::PROFILE_L) {
            self.profile_view = (self.profile_view + 2) % 3;
        }
        if self.pads.get_clicked(sp::PROFILE_R) {
            self.profile_view = (self.profile_view + 1) % 3;
        }
        if self.pads.get_clicked(sp::ACHIEVEMENTS) {
            self.set_dialog(false, 12, 0, 0);
            return;
        }
        if self.pads.get_clicked(sp::LEADERBOARDS) {
            self.set_dialog(false, 13, 0, 0);
        }
    }

    /// `setProfileString`: (names, values) of the three rows for the current page.
    fn profile_strings(&self) -> ([String; 3], [String; 3]) {
        let s = &self.save;
        let dist = |m: i32, lim: i32| -> String {
            if s.show_yard {
                let y = (m as f32 * 1.093613) as i32;
                if y < lim {
                    format!("{}Yard", y)
                } else {
                    format!("{}Mile", (y as f64 * 5.68e-4) as i32)
                }
            } else if m < 10000 {
                format!("{}m", m)
            } else {
                format!("{}km", (m as f64 * 0.001) as i32)
            }
        };
        match self.profile_view {
            1 => (
                ["Accidents".into(), "Travel Distance".into(), "Drift Length".into()],
                [format!("{}", s.accident_no), dist(s.travel_distance, 10000), dist(s.drift_distance, 1000)],
            ),
            2 => {
                let max = if s.show_yard { format!("{}mph", (s.max_speed as f32 * 0.002236936) as i32) } else { format!("{}km/h", (s.max_speed as f32 * 0.0036) as i32) };
                let avg = if s.show_yard {
                    format!("{}mph", if s.travel_second > 0 { ((s.travel_distance as f32 * 2.236936) / s.travel_second as f32) as i32 } else { 0 })
                } else {
                    format!("{}km/h", if s.travel_second > 0 { ((s.travel_distance as f32 * 3.6) / s.travel_second as f32) as i32 } else { 0 })
                };
                (["Perfect Turn".into(), "Max Speed".into(), "Avg Speed".into()], [format!("{}", s.perfect_turn), max, avg])
            }
            _ => (
                ["Driver Level".into(), "Mission Clear".into(), "Win/Lose".into()],
                [format!("{}", s.driver_level + 1), format!("{}", s.missions_clear_count), format!("{} / {}", s.win, s.lose)],
            ),
        }
    }

    // ------------------------------------------------------------------ rendering

    pub fn render_background(&mut self, mode: i32) {
        let g = &self.g;
        let sh = g.sh;
        let c = (g.vw / 800.0).max(sh / 640.0).max(1.0);
        g.e(&self.a.background, S::BACKGROUND, g.cw(), sh - 320.0 * c, c, c);
        g.pin.set(Some(0.0));
        g.d(&self.a.menu, S::MENU[3], 0.0, sh - 12.0, g.vw / 524.0, 1.0, 1);
        g.pin.set(None);
        let (idx, x) = match mode {
            8 => (4, 50.0),
            9 => (5, 150.0),
            10 => (6, 250.0),
            11 => (7, 350.0),
            _ => (0, 0.0),
        };
        if idx != 0 {
            g.e(&self.a.menu, S::MENU[idx], x, sh - 29.0, 1.25, 1.25);
        }
        self.render_gold_for_menu();
    }

    fn ease_counters(&mut self) {
        let dt = self.dt as f64;
        self.render_coin += (self.save.coin as f64 - self.render_coin) * 2.0 * dt;
        self.render_gold += (self.save.gold as f64 - self.render_gold) * 2.0 * dt;
    }

    pub fn render_gold_for_menu(&mut self) {
        self.ease_counters();
        let f5 = (tab_y(self.g.sh) - 96.0).max(16.0);
        let g = &mut self.g;
        g.h(&self.a.menu, S::MENU[25], 726.0, f5);
        g.h(&self.a.menu, S::MENU[26], 74.0, f5);
        g.set_tint(COLOR_GREEN);
        self.bfb.f(g, &self.a.menu, &format!("{}", (self.render_coin + 0.5) as i64), 790.0, f5, 0.75);
        g.set_tint(COLOR_RED);
        self.bfb.f(g, &self.a.menu, &format!("{}", (self.render_gold + 0.5) as i64), 138.0, f5, 0.75);
        g.clear_tint();
    }

    /// `renderGold`: the in-race / result screens' counters (ui.png plates, small font).
    pub fn render_gold(&mut self, gold: i32, coin: i32) {
        self.ease_counters();
        let _ = (gold, coin);
        let g = &mut self.g;
        g.h(&self.a.ui, S::UI[3], 726.0, 16.0);
        g.h(&self.a.ui, S::UI[4], 74.0, 16.0);
        g.set_tint(COLOR_GREEN);
        self.bf.f(g, &self.a.ui, &format!("{}", (self.render_coin + 0.5) as i64), 790.0, 16.0, 0.75);
        g.set_tint(COLOR_RED);
        self.bf.f(g, &self.a.ui, &format!("{}", (self.render_gold + 0.5) as i64), 138.0, 16.0, 0.75);
        g.clear_tint();
    }

    pub fn render_tab_profile(&mut self) {
        self.render_background(8);
        let sh = self.g.sh;
        let f3 = tab_y(sh);
        let f4 = self.pads.p[sp::PROFILE_L].y;
        let cw = 400.0;
        let g = &self.g;
        let m = &self.a.menu;
        g.e(m, S::MENU[11], cw, (f3 - 86.0) + 14.0, g.vw / 100.0, 0.5);
        g.e(m, S::MENU[11], cw, (86.0 + f3) - 14.0, g.vw / 100.0, 0.5);
        let f6 = ((sh / 2.0) * 0.1 + 15.0).clamp(25.0, 50.0);
        let (f7, f8) = (f4 - f6, f4 + f6);
        for y in [f7, f4, f8] {
            g.h(m, S::MENU[39], cw, y);
        }
        let on = self.pads.p[sp::ONLINE].clone();
        g.h(m, S::MENU[2], (on.x - 210.0) - 10.0, on.y);
        g.h(m, S::MENU[42], on.rx - 10.0, on.ry);
        g.h(m, S::MENU[41], (on.rx - 22.0) + 6.0, (on.y + 72.0) - 14.0);
        let fr = self.pads.p[sp::ONLINE_FRIEND].clone();
        g.h(m, S::MENU[43], fr.rx - 10.0, fr.ry);
        g.h(m, S::MENU[41], (fr.rx - 22.0) + 6.0, (fr.y + 72.0) - 14.0);
        let ch = self.pads.p[sp::ONLINE_CHANNEL].clone();
        g.h(m, S::MENU[44], ch.rx - 10.0, ch.ry);
        g.h(m, S::MENU[41], (ch.rx - 22.0) + 6.0, (ch.y + 72.0) - 14.0);
        g.h(m, S::MENU[2], (ch.x + 210.0) - 10.0, ch.y);
        let rating = self.save.rating;
        g.e(m, S::CLASS[class_for(rating)], 76.0, f3 - 44.0, 0.5, 0.5);
        let (names, vals) = self.profile_strings();
        let f9 = cw + 82.0;
        for (v, y) in vals.iter().zip([f7, f4, f8]) {
            self.bfb.d(g, m, v, f9, y, 0.8);
        }
        let g = &mut self.g;
        g.set_tint(COLOR_RED);
        self.bfb.d(g, m, &format!("{}", rating + 10000), 76.0, f3 + 2.0, 1.25);
        g.set_tint(COLOR_GREEN);
        let f12 = cw - 82.0;
        for (n, y) in names.iter().zip([f7, f4, f8]) {
            self.text.e(g, n, f12, y, 0, 21.0, 100.0);
        }
        g.clear_tint();
        let on = self.pads.p[sp::ONLINE].clone();
        self.text.e(g, "Online Match", on.rx - 15.0, (on.ry + 72.0) - 14.0, 0, 21.0, 100.0);
        let fr = self.pads.p[sp::ONLINE_FRIEND].clone();
        self.text.e(g, "vs Friend", fr.rx - 15.0, (fr.ry + 72.0) - 14.0, 0, 21.0, 100.0);
        let ch = self.pads.p[sp::ONLINE_CHANNEL].clone();
        self.text.e(g, "Channel", ch.rx - 15.0, (ch.ry + 72.0) - 14.0, 0, 21.0, 100.0);
    }

    /// Card strip common parts: card background + label plate, visible-range test. Returns x.
    fn card_x(scroll: f32, i: i32) -> f32 {
        -scroll + (i * 210) as f32
    }

    pub fn render_tab_cargo(&mut self) {
        self.render_background(9);
        // 3D car, drawn over the background (camera viewport 800x500 from the bottom)
        crate::scene::render_cur_car(&self.a, &self.g, self.cargo.car);
        let y = self.pads.p[sp::SCROLL].y;
        let f9 = self.tab_cargo.scroll + self.tab_cargo.drag;
        let c = self.cargo.car;
        let g = &mut self.g;
        let m = &self.a.menu;
        let info = car(c);
        for i in -4..10i32 {
            let f10 = Self::card_x(f9, i);
            if f10 < -100.0 - g.shift {
                continue;
            }
            if f10 > 1010.0 + g.shift {
                break;
            }
            g.h(m, S::MENU[2], f10 - 8.0, y);
            if i > -2 && i < 6 {
                g.h(m, S::MENU[41], (f10 - 20.5) + 5.0, (y + 72.0) - 14.0);
            }
            if i == -1 {
                self.bfb.d(g, m, CLASS_NAMES[c % 4], f10, y - 26.0, 0.5);
                g.h(m, S::MENU[40], f10, y);
                let status = car_status(&self.save, c);
                self.bfb.d(g, m, &status, f10 - 18.0, (y + 72.0) - 14.0, 0.75);
                g.set_tint(COLOR_RED);
                self.bfb.d(g, m, info.name, f10, y, 1.0);
                g.clear_tint();
            }
            if i > -1 && i < 6 {
                let u = i as usize;
                if let Some(cost) = economy::upgrade_cost(&self.save, c, u) {
                    let t = if self.save.upgrade[c][u] < 10 { format!("{} Coin", cost) } else { "MAX".to_string() };
                    self.bfb.d(g, m, &t, f10 - 18.0, (y + 72.0) - 14.0, 0.75);
                }
                self.bfb.f(g, m, &format!("{}", info.ability[u]), f10 + 69.0, y - 62.0, 0.625);
                g.h(m, S::MENU[u + 47], f10, y);
                let f12 = f10 - 92.0;
                g.d(m, S::MENU[9], f12, y - 50.0, 80.5, 2.0, 1);
                g.d(m, S::MENU[60], f12, y - 50.0, self.cargo.ability[u], 2.0, 1);
                g.d(m, S::MENU[10], f12 + self.cargo.ability[u] * 2.0 + 1.0, y - 50.0, self.cargo.upgrade[u], 2.0, 1);
                g.set_tint(COLOR_RED);
                self.bfb.f(g, m, &format!("+{}", self.save.upgrade[c][u]), f10 + 80.0, y + 30.0, 1.1);
                g.clear_tint();
            }
        }
        g.e(m, S::MENU[11], 400.0, (y - 86.0) + 14.0, g.vw / 100.0, 0.5);
        g.e(m, S::MENU[11], 400.0, (y + 86.0) - 14.0, g.vw / 100.0, 0.5);
        for i in 0..6i32 {
            let f15 = Self::card_x(f9, i);
            if f15 >= -100.0 - g.shift {
                if f15 > 1010.0 + g.shift {
                    break;
                }
                self.text.e(g, ABILITY_NAMES[i as usize], f15 - 92.0, y - 62.0, 1, 18.0, 64.0);
            }
        }
    }

    pub fn render_tab_store(&mut self) {
        self.render_background(10);
        let y = self.pads.p[sp::SCROLL].y;
        let f4 = self.tab_store.scroll + self.tab_store.drag;
        let g = &mut self.g;
        let m = &self.a.menu;
        let items: [(usize, &str); 6] =
            [(54, "200 Coin"), (56, "600 Coin"), (58, "1500 Coin"), (55, "200 Gold"), (57, "600 Gold"), (59, "1500 Gold")];
        for i in -4..6i32 {
            let f5 = Self::card_x(f4, i);
            if f5 < -100.0 - g.shift {
                continue;
            }
            if f5 > 1010.0 + g.shift {
                break;
            }
            g.h(m, S::MENU[2], f5 - 8.0, y);
            if i > -1 {
                g.h(m, S::MENU[41], (f5 - 20.5) + 5.0, (y + 72.0) - 14.0);
                g.h(m, S::MENU[11], f5 - 6.0, (y - 57.0) + 10.0);
                let (spr, txt) = items[i as usize];
                g.h(m, S::MENU[spr], f5, y);
                self.bfb.d(g, m, txt, f5 - 6.0, y - 59.0, 0.7);
            }
        }
        g.e(m, S::MENU[11], 400.0, (y - 86.0) + 14.0, g.vw / 100.0, 0.5);
        g.e(m, S::MENU[11], 400.0, (y + 86.0) - 14.0, g.vw / 100.0, 0.5);
        let labels = ["2 Gold", "5 Gold", "10 Gold", "", "", ""];
        for i in 0..6i32 {
            let f9 = Self::card_x(f4, i);
            if f9 >= -100.0 - g.shift {
                if f9 > 1010.0 + g.shift {
                    break;
                }
                self.text.e(g, labels[i as usize], f9 - 14.0, (y + 72.0) - 14.0, 0, 21.0, 100.0);
            }
        }
    }

    pub fn render_tab_option(&mut self) {
        self.render_background(11);
        let y = self.pads.p[sp::SCROLL].y;
        let f3 = self.tab_option.scroll + self.tab_option.drag;
        let s = self.save.clone();
        let g = &mut self.g;
        let m = &self.a.menu;
        let sh = g.sh;
        for i in -4..12i32 {
            let f4 = Self::card_x(f3, i);
            if f4 < -100.0 - g.shift {
                continue;
            }
            if f4 > 1010.0 + g.shift {
                break;
            }
            g.h(m, S::MENU[2], f4 - 8.0, y);
            if i > -1 && i < 9 {
                g.h(m, S::MENU[41], (f4 - 20.5) + 5.0, (y + 72.0) - 14.0);
            }
            let (lab_y, lab_x) = ((y + 72.0) - 14.0, f4 - 18.0);
            match i {
                0 => {
                    g.h(m, S::MENU[35], f4 - 10.0, y - 12.0);
                    g.h(m, S::MENU[if s.music_disabled { 16 } else { 20 }], f4 + 22.0, y - 12.0);
                    self.bfb.d(g, m, if s.music_disabled { "Off" } else { "On" }, lab_x, lab_y, 0.75);
                }
                1 => {
                    g.h(m, S::MENU[34], f4 - 10.0, y - 12.0);
                    g.h(m, S::MENU[if s.sound_disabled { 16 } else { 20 }], f4 + 22.0, y - 12.0);
                    self.bfb.d(g, m, if s.sound_disabled { "Off" } else { "On" }, lab_x, lab_y, 0.75);
                }
                2 => {
                    let mode;
                    if !s.accel_disabled {
                        g.h(m, S::MENU[31], f4, y - 12.0);
                        mode = 2;
                    } else if s.left_wheel {
                        g.e(m, S::MENU[32], f4, y - 12.0, -1.0, 1.0);
                        mode = 1;
                    } else {
                        g.h(m, S::MENU[32], f4, y - 12.0);
                        mode = 0;
                    }
                    let f6 = f4 - 18.0;
                    g.h(m, S::MENU[if mode == 0 { 21 } else { 22 }], f6 - 16.0, lab_y);
                    g.h(m, S::MENU[if mode == 1 { 21 } else { 22 }], f6, lab_y);
                    g.h(m, S::MENU[if mode == 2 { 21 } else { 22 }], f6 + 16.0, lab_y);
                }
                3 => {
                    g.h(m, S::MENU[33], f4, y - 12.0);
                    self.bfb.d(g, m, &format!("{}", s.wheel_sensitivity + 5), lab_x, lab_y, 0.75);
                }
                4 => {
                    if s.right_driver {
                        g.e(m, S::MENU[30], f4, y - 12.0, -1.0, 1.0);
                    } else {
                        g.h(m, S::MENU[30], f4, y - 12.0);
                    }
                    let f7 = f4 - 18.0;
                    g.h(m, S::MENU[if s.right_driver { 22 } else { 21 }], f7 - 8.0, lab_y);
                    g.h(m, S::MENU[if s.right_driver { 21 } else { 22 }], f7 + 8.0, lab_y);
                }
                5 => {
                    self.bfb.c(g, m, if s.show_yard { "Mile/Yard" } else { "KM/Meter" }, f4, y - 12.0);
                    let f8 = f4 - 18.0;
                    g.h(m, S::MENU[if s.show_yard { 22 } else { 21 }], f8 - 8.0, lab_y);
                    g.h(m, S::MENU[if s.show_yard { 21 } else { 22 }], f8 + 8.0, lab_y);
                }
                6 => {
                    let name = ["Large", "Medium", "Small"][s.gui_size.clamp(0, 2) as usize];
                    self.bfb.c(g, m, "GUI Size", f4, y - 12.0);
                    self.bfb.d(g, m, name, lab_x, lab_y, 0.75);
                }
                7 => {
                    // multiplayer player name (not in the original, which used the Play Games name)
                    self.bfb.c(g, m, "Name", f4, y - 12.0);
                    let n = self.lobby.name.clone();
                    self.text.e(g, &n, lab_x, lab_y, 0, 21.0, 80.0);
                }
                8 => {
                    // country: the flag the opponent sees
                    let code = self.lobby.country.clone();
                    let idx = drdriving_core::net::country_index(&code);
                    g.h(&self.a.world, S::flag(idx), f4, y - 12.0);
                    self.bfb.d(g, m, if code.is_empty() { "---" } else { &code }, lab_x, lab_y, 0.75);
                }
                _ => {}
            }
        }
        g.e(m, S::MENU[11], 400.0, (y - 86.0) + 14.0, g.vw / 100.0, 0.5);
        g.e(m, S::MENU[11], 400.0, (y + 86.0) - 14.0, g.vw / 100.0, 0.5);
        g.set_tint(COLOR_GREEN);
        self.bfb.d(g, m, "Designer / Programmer", 400.0 - 140.0, (sh * 0.68 + tab_dy(sh)) - 14.0, 0.75);
        self.bfb.d(g, m, "Graphic Artists", 400.0 - 140.0, (sh * 0.72 + tab_dy(sh)) + 14.0, 0.75);
        g.set_tint(DARK);
        self.bfb.d(g, m, "Sangha An", 400.0 + 140.0, (sh * 0.68 + tab_dy(sh)) - 14.0, 0.75);
        self.bfb.d(g, m, "Hyunchan Ko", 400.0 + 140.0, (sh * 0.72 + tab_dy(sh)) + 14.0, 0.75);
        g.clear_tint();
    }

    /// `renderScreenPad(z2)`: draw the menu-atlas sprite of every visible pad (`z2` = draw the
    /// inactive ones, used under dialogs).
    pub fn render_screen_pad(&mut self, z2: bool) {
        let t = self.app_time;
        let g = &self.g;
        let m = &self.a.menu;
        for n in self.pads.p.iter() {
            if !n.visible || n.texture < 0 {
                continue;
            }
            if z2 == n.active {
                continue;
            }
            if n.texture == 8 {
                // animated "go" arrows
                let i4 = (((t * 0.4) - ((t * 0.4) as i32) as f32) * 35.0) as i32;
                let (c3, c4, c5) = match i4 {
                    0 => (66usize, 67usize, 67usize),
                    1 => (65, 66, 67),
                    2 => (67, 65, 66),
                    3 => (67, 67, 65),
                    _ => (67, 67, 67),
                };
                g.h(m, S::MENU[c3], n.rx - 48.0, n.ry);
                g.h(m, S::MENU[c4], n.rx, n.ry);
                g.h(m, S::MENU[c5], n.rx + 48.0, n.ry);
            } else {
                g.h(m, S::MENU[n.texture as usize], n.rx, n.ry);
            }
        }
    }

    pub fn render_dialog(&mut self) {
        self.g.lthr = 360.0;
        match self.last_mode {
            9 => self.render_tab_cargo(),
            10 => self.render_tab_store(),
            8 => self.render_tab_profile(),
            11 => self.render_tab_option(),
            _ => {}
        }
        self.render_screen_pad(true);
        self.g.lthr = 250.0;
        let (cw, ch) = (400.0, self.g.sh / 2.0);
        let g = &mut self.g;
        g.set_tint(Color::new(1.0, 1.0, 1.0, 0.4));
        g.e(&self.a.ui, S::BLACK, cw, ch, g.vw / 2.0, ch);
        g.clear_tint();
        let f7 = ((self.mode_time * 2.0) + 0.6).clamp(0.6, 1.0);
        g.e(&self.a.ui, S::BLACK, cw, ch, f7 * 400.0, f7 * 80.0);
        if self.mode_time > 0.2 {
            let (s1, s2) = (self.dialog.str1.clone(), self.dialog.str2.clone());
            self.text.d(g, &s1, cw, ch - 50.0, 0, 27.0);
            self.text.d(g, &s2, cw, ch - 10.0, 0, 27.0);
        }
        self.render_screen_pad(false);
    }

    pub fn render_exit(&mut self) {
        self.g.lthr = 360.0;
        match self.last_mode {
            8 => self.render_tab_profile(),
            9 => self.render_tab_cargo(),
            10 => self.render_tab_store(),
            11 => self.render_tab_option(),
            _ => {}
        }
        self.render_screen_pad(true);
        self.g.lthr = 250.0;
        let (cw, ch, sh) = (400.0, self.g.sh / 2.0, self.g.sh);
        let g = &mut self.g;
        let f3 = (self.mode_time * 5.0).min(0.85);
        g.set_tint(Color::new(f3, f3, f3, f3));
        g.e(&self.a.ui, S::BLACK, cw, ch, g.vw / 2.0, ch);
        g.clear_tint();
        self.text.d(g, "Are you sure to exit?", cw, sh * 0.3, 0, 27.0);
        self.render_screen_pad(false);
    }

    pub fn render_loading(&mut self, progress: i32) {
        let cur = self.cur_app;
        draw_loading(&mut self.g, &self.a.logo, &self.text, progress, cur);
    }
}

/// `renderLoading`: SUD logo, promo strip and the two-colour progress bar.
pub fn draw_loading(g: &mut crate::gfx::Gfx, lg: &Texture2D, text: &crate::gfx::GlText, progress: i32, cur_app: usize) {
    let (cw, ch, sh) = (400.0, g.sh / 2.0, g.sh);
    g.e(lg, S::LOADING_WHITE, cw, ch, cw, ch);
    g.h(lg, S::SUD, cw, ch);
    g.h(lg, crate::gfx::Spr { x: 10.0, y: 950.0, w: 302.0, h: 72.0 }, 151.0, 36.0);
    let i = cur_app % 24;
    let icon = crate::gfx::Spr { x: ((i % 13) * 78 + 6) as f32, y: ((i / 13) * 78 + 6) as f32, w: 72.0, h: 72.0 };
    g.e(lg, icon, 39.0, 36.0, 0.75, 0.75);
    g.set_tint(Color::new(0.22, 0.64, 0.89, 1.0));
    g.d(lg, S::LOADING_WHITE, cw - 128.0, sh * 0.82, 128.0, 3.0, 1);
    g.set_tint(Color::new(0.95, 0.67, 0.0, 1.0));
    g.d(lg, S::LOADING_WHITE, cw - 128.0, sh * 0.82, progress as f32 * 1.28, 3.0, 1);
    g.set_tint(Color::new(0.0, 0.0, 0.0, 1.0));
    text.d(g, "SUD Games", 71.0, 20.0, 1, 18.0);
    text.d(g, APP_NAMES[i], 71.0, 48.0, 1, 22.0);
    g.clear_tint();
}

pub fn class_for(rating: i32) -> usize {
    economy::class_for_rating(rating) as usize
}
