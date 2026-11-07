//! The game front end: a port of `drdriving.d` (UI state machine) on top of `drdriving_core`.
//! Modes follow the original numbering: 0 loading, 1 car select, 2 countdown, 3 driving, 4 pause,
//! 5 success, 6 ending, 8 profile, 9 garage, 10 store, 11 options, 12 dialog, 13 exit.

use crate::assets::Assets;
use crate::audio::{Audio, Snd};
use crate::gfx::{BFont, Gfx, GlText};
use crate::keys::Keys;
use crate::pad::*;
use crate::persist;
use crate::scene::{DriftMark, DriftMarks};
use drdriving_core::cars::{car, PLAYER_CARS, PURCHASABLE_CARS};
use drdriving_core::economy::{self, Outcome, ShopError};
use drdriving_core::mission::Mission;
use drdriving_core::save::SaveGame;
use drdriving_core::session::{CoinReason, Event, Input, Phase, Session};
use drdriving_core::world::World;
use macroquad::prelude::*;

// ---- pad indices (same roles as the original g_sp*Idx fields)
pub mod sp {
    pub const WHEEL: usize = 0;
    pub const ACCEL: usize = 1;
    pub const BRAKE: usize = 2;
    pub const GEAR: usize = 3;
    pub const CAMERA: usize = 4;
    pub const BLINKER_L: usize = 5;
    pub const BLINKER_R: usize = 6;
    pub const EMERGENCY: usize = 7;
    pub const MENU: usize = 8;
    pub const MIRROR_BACK: usize = 9;
    pub const MIRROR_LEFT: usize = 10;
    pub const MIRROR_RIGHT: usize = 11;
    pub const RACE: usize = 12;
    pub const REPAIR: usize = 13;
    pub const BACK_TO_MENU: usize = 14;
    pub const MISSION_L: usize = 15;
    pub const MISSION_R: usize = 16;
    pub const CAR_L: usize = 17;
    pub const CAR_R: usize = 18;
    pub const LEADERBOARDS: usize = 19;
    pub const ACHIEVEMENTS: usize = 20;
    pub const CANCEL_SEARCH: usize = 21;
    pub const MENU_MAIN: usize = 22;
    pub const CARGO: usize = 23;
    pub const STORE: usize = 24;
    pub const OPTIONS: usize = 25;
    pub const STAGE_START: usize = 26;
    pub const PROFILE_L: usize = 27;
    pub const PROFILE_R: usize = 28;
    pub const SCROLL: usize = 29;
    pub const ONLINE: usize = 30;
    pub const ONLINE_FRIEND: usize = 31;
    pub const ONLINE_CHANNEL: usize = 32;
    pub const BUY_CAR: usize = 33;
    pub const RENT_CAR: usize = 34;
    pub const DIALOG_OK: usize = 35;
    pub const DIALOG_CANCEL: usize = 36;
    pub const DIALOG_CONFIRM: usize = 37;
    pub const FPS: usize = 38;
    pub const EXIT_OK: usize = 39;
    pub const EXIT_CANCEL: usize = 40;
}

pub const MISSION_NAMES: [&str; 9] = ["SPEED", "HIGHWAY", "DRIFT", "FUEL", "VIP ESCORT", "PARKING", "BROKEN BRAKE", "TRUCK", "LANE"];
pub const CLASS_NAMES: [&str; 4] = ["A CLASS", "B CLASS", "C CLASS", "S CLASS"];
pub const ABILITY_NAMES: [&str; 6] = ["Engine", "Brake", "Tire", "Fuel", "Comfort", "Durability"];
pub const APP_NAMES: [&str; 24] = [
    "Dr. Pipe 2", "Dr. Driving", "Dr. Janggi", "Dr. Reversi", "Dr. Gomoku", "Dr. Chess", "Dr. Computer", "Dr. Checkers", "Dr. Shogi", "Dr. Rocket", "Dr. Pair",
    "Dr. Xiangqi", "Dr. Parking 4", "Dr. Sudoku", "Dr. 1010", "Dr. Unblock", "Dr. Link", "Dr. Block", "Dr. Solitaire", "Dr. FreeCell", "Dr. Spider", "Dr. 2048",
    "Dr. Dominoes", "Dr. Driving 2",
];

pub const COLOR_GREEN: Color = Color::new(0.57, 0.816, 0.098, 1.0);
pub const COLOR_RED: Color = Color::new(0.914, 0.316, 0.055, 1.0);

#[derive(Default, Clone)]
pub struct Cargo {
    pub car: usize,
    pub ability: [f32; 6],
    pub upgrade: [f32; 6],
}

#[derive(Default, Clone)]
pub struct Scroll {
    pub scroll: f32,
    pub drag: f32,
}

#[derive(Default, Clone)]
pub struct Dialog {
    pub question: bool,
    pub action: i32,
    pub info1: i32,
    pub info2: i32,
    pub str1: String,
    pub str2: String,
}

/// Values captured before a mission finishes (the original's `n result` + counters).
#[derive(Default, Clone)]
pub struct ResultInfo {
    pub last_coin: i32,
    pub last_exp: i32,
    pub last_gold: i32,
    pub reward: i32,
    pub new_record: bool,
    pub last_record: i32,
    pub record: i32,
}

#[derive(Default)]
pub struct Fx {
    pub visible: bool,
    pub alive: f32,
    pub x: f32,
    pub y: f32,
    pub rate: i32,
}

pub struct Targets {
    pub back: RenderTarget,
    pub left: RenderTarget,
    pub right: RenderTarget,
}

pub struct Game {
    pub a: Assets,
    pub audio: Audio,
    pub save: SaveGame,
    pub world: World,
    pub session: Option<Session>,
    pub g: Gfx,
    pub pads: Pads,
    pub bf: BFont,
    pub bfb: BFont,
    pub text: GlText,
    pub keys: Keys,

    pub mode: i32,
    pub last_mode: i32,
    pub mode_time: f32,
    pub app_time: f32,
    pub dt: f32,

    pub cargo: Cargo,
    pub tab_cargo: Scroll,
    pub tab_option: Scroll,
    pub tab_store: Scroll,
    pub profile_view: i32,
    pub fling: f32,
    pub dialog: Dialog,
    pub result: ResultInfo,
    pub render_coin: f64,
    pub render_gold: f64,
    pub moving_coin: Fx,
    pub moving_gold: Fx,
    pub effect_turn: Fx,
    pub camera_mode: i32,
    pub prev_wheel: f32,
    pub snap_coin: i32,
    pub snap_exp: i32,
    pub snap_gold: i32,
    pub success_cam: Vec3,
    pub drifts: DriftMarks,
    pub targets: Targets,
    pub cur_app: usize,
    pub help: bool,
    pub kb_accel: bool,
    pub kb_brake: bool,

    pub auto: bool,
    pub no_traffic: bool,
    pub quit: bool,
    // pointer state
    touch_seen: bool,
    mouse_down: bool,
    last_mouse: (f32, f32),
    drag_samples: Vec<(f32, f32)>,
    pub last_sh: f32,
    pub wheel_zoom: f32,
    pub clock: f32,
}

fn rt(w: u32, h: u32) -> RenderTarget {
    let t = render_target(w, h);
    t.texture.set_filter(FilterMode::Linear);
    t
}

impl Game {
    pub fn new(a: Assets, audio: Audio, save: SaveGame, seed: i64) -> Game {
        let car0 = economy::best_car_id(&save);
        let mut world = World::new(car0, Some(seed));
        world.init_map(&save);
        let g = Gfx::new();
        let font = a.font.clone();
        let mut game = Game {
            a,
            audio,
            cargo: Cargo { car: car0, ability: [10.0; 6], upgrade: [0.0; 6] },
            save,
            world,
            session: None,
            g,
            pads: Pads::new(),
            bf: BFont::ui(),
            bfb: BFont::menu(),
            text: GlText { font },
            keys: Keys::default(),
            mode: 8,
            last_mode: 8,
            mode_time: 0.0,
            app_time: 0.0,
            dt: 1.0 / 60.0,
            tab_cargo: Scroll { scroll: -340.0, drag: 0.0 },
            tab_option: Scroll { scroll: -130.0, drag: 0.0 },
            tab_store: Scroll { scroll: -130.0, drag: 0.0 },
            profile_view: 0,
            fling: 0.0,
            dialog: Dialog::default(),
            result: ResultInfo::default(),
            render_coin: 0.0,
            render_gold: 0.0,
            moving_coin: Fx::default(),
            moving_gold: Fx::default(),
            effect_turn: Fx::default(),
            camera_mode: 4,
            prev_wheel: 0.0,
            snap_coin: 0,
            snap_exp: 0,
            snap_gold: 0,
            success_cam: Vec3::ZERO,
            drifts: DriftMarks::default(),
            targets: Targets { back: rt(500, 200), left: rt(300, 200), right: rt(300, 200) },
            cur_app: 1,
            help: false,
            kb_accel: false,
            kb_brake: false,
            auto: false,
            no_traffic: false,
            quit: false,
            touch_seen: false,
            mouse_down: false,
            last_mouse: (0.0, 0.0),
            drag_samples: Vec::new(),
            last_sh: 0.0,
            wheel_zoom: 1.0,
            clock: 0.0,
        };
        game.render_coin = game.save.coin as f64;
        game.render_gold = game.save.gold as f64;
        game.g.scale = Gfx::scale_for(game.save.gui_size);
        game.layout_pads();
        game.enter_mode(8);
        game
    }

    // ------------------------------------------------------------------ layout

    /// `onSurfaceChanged`: place every pad (virtual coordinates).
    pub fn layout_pads(&mut self) {
        self.g.layout();
        let sh = self.g.sh;
        self.last_sh = sh;
        let (sw, cw, ch) = (800.0f32, 400.0f32, sh / 2.0);
        let mut z = (sw / 800.0) * 4.0 / 4.0; // wheel zoom is derived from the physical width in the original
        z = (z * 4.0 / 4.0).clamp(0.75, 1.25);
        self.wheel_zoom = z.sqrt();
        if ch < 180.0 {
            self.wheel_zoom *= 0.9;
        }
        let z = self.wheel_zoom;
        let p = &mut self.pads.p;
        p[sp::WHEEL].set_pad(PAD_ANALOG, -1, (800.0 - 16.0) - 104.0 * z, sh - z * 101.0, 150.0, 150.0);
        p[sp::ACCEL].set_pad(PAD_BUTTON, -1, 152.0, sh - 64.0, 50.0, 110.0);
        p[sp::BRAKE].set_pad(PAD_BUTTON, -1, 48.0, sh - 42.0, 52.0, 66.0);
        p[sp::GEAR].set_pad(PAD_GEAR, -1, sw - 64.0, ch - 108.0, 80.0, 100.0);
        p[sp::CAMERA].set_pad(PAD_CLICK, -1, 31.0, 24.0, 60.0, 50.0);
        p[sp::BLINKER_L].set_pad(PAD_CLICK, -1, 326.0, sh - 74.0, 34.0, 34.0);
        p[sp::BLINKER_R].set_pad(PAD_CLICK, -1, 470.0, sh - 74.0, 34.0, 34.0);
        p[sp::EMERGENCY].set_pad(PAD_CLICK, -1, 398.0, sh - 74.0, 34.0, 34.0);
        p[sp::MENU].set_pad(PAD_CLICK, -1, cw, 4.0 * sh / 8.0, 90.0, sh / 8.0);
        p[sp::MIRROR_BACK].set_pad(PAD_CLICK, -1, cw, 60.0, 120.0, 60.0);
        p[sp::MIRROR_LEFT].set_pad(PAD_CLICK, -1, cw - 200.0, 60.0, 45.0, 60.0);
        p[sp::MIRROR_RIGHT].set_pad(PAD_CLICK, -1, cw + 200.0, 60.0, 45.0, 60.0);
        p[sp::RACE].set_pad(PAD_CLICK, 29, 768.0, sh - 35.0, 60.0, 48.0);
        p[sp::REPAIR].set_pad(PAD_CLICK, -1, cw, (sh * 0.7).floor(), 140.0, 35.0);
        p[sp::BACK_TO_MENU].set_pad(PAD_CLICK, 12, 32.0, sh - 35.0, 40.0, 48.0);
        p[sp::MISSION_L].set_pad(PAD_CLICK, 23, 250.0, sh - 35.0, 80.0, 40.0);
        p[sp::MISSION_R].set_pad(PAD_CLICK, 24, 550.0, sh - 35.0, 80.0, 40.0);
        p[sp::CAR_L].set_pad(PAD_CLICK, 18, 180.0, sh * 0.5, 60.0, 45.0);
        p[sp::CAR_R].set_pad(PAD_CLICK, 19, 620.0, sh * 0.5, 60.0, 45.0);
        p[sp::LEADERBOARDS].set_pad(PAD_CLICK, 15, 40.0, sh * 0.35 + 50.0, 32.0, 35.0);
        p[sp::ACHIEVEMENTS].set_pad(PAD_CLICK, 14, 112.0, sh * 0.35 + 50.0, 32.0, 35.0);
        p[sp::CANCEL_SEARCH].set_pad(PAD_CLICK, -1, 520.0, sh - 23.0, 23.0, 23.0);
        p[sp::MENU_MAIN].set_pad(PAD_CLICK, 4, 50.0, sh - 24.0, 48.0, 48.0);
        p[sp::CARGO].set_pad(PAD_CLICK, 5, 150.0, sh - 24.0, 48.0, 48.0);
        p[sp::STORE].set_pad(PAD_CLICK, 6, 250.0, sh - 24.0, 48.0, 48.0);
        p[sp::OPTIONS].set_pad(PAD_CLICK, 7, 350.0, sh - 24.0, 48.0, 48.0);
        p[sp::STAGE_START].set_pad(PAD_CLICK, 8, 720.0, sh - 35.0, 80.0, 40.0);
        p[sp::PROFILE_L].set_pad(PAD_CLICK, 18, 120.0, sh * 0.7 + 2.0, 60.0, 50.0);
        p[sp::PROFILE_R].set_pad(PAD_CLICK, 19, 680.0, sh * 0.7 + 2.0, 60.0, 50.0);
        p[sp::SCROLL].set_pad(PAD_CLICK, -1, 400.0, sh * 0.35, 400.0, 100.0);
        p[sp::ONLINE].set_pad(PAD_CLICK, -1, 270.0, sh * 0.35, 100.0, 85.0);
        p[sp::ONLINE_FRIEND].set_pad(PAD_CLICK, -1, 480.0, sh * 0.35, 100.0, 85.0);
        p[sp::ONLINE_CHANNEL].set_pad(PAD_CLICK, -1, 690.0, sh * 0.35, 100.0, 85.0);
        p[sp::BUY_CAR].set_pad(PAD_CLICK, 45, cw, sh * 0.7 + 2.0, 100.0, 35.0);
        p[sp::RENT_CAR].set_pad(PAD_CLICK, 46, cw, sh * 0.7 + 2.0, 100.0, 35.0);
        p[sp::DIALOG_OK].set_pad(PAD_CLICK, 28, cw + 150.0, ch + 40.0, 100.0, 35.0);
        p[sp::DIALOG_CANCEL].set_pad(PAD_CLICK, 27, cw - 150.0, ch + 40.0, 100.0, 35.0);
        p[sp::DIALOG_CONFIRM].set_pad(PAD_CLICK, 28, cw, ch + 40.0, 100.0, 35.0);
        p[sp::FPS].set_pad(PAD_CLICK, -1, sw - 24.0, 24.0, 24.0, 24.0);
        p[sp::EXIT_OK].set_pad(PAD_CLICK, 28, cw + 150.0, sh * 0.53 + 80.0, 110.0, 40.0);
        p[sp::EXIT_CANCEL].set_pad(PAD_CLICK, 27, cw - 150.0, sh * 0.53 + 80.0, 110.0, 40.0);
        self.adjust_wheel_position();
    }

    /// `adjustWheelPosition`: mirror the controls when the wheel is on the left, or for tilt mode.
    pub fn adjust_wheel_position(&mut self) {
        let z = self.wheel_zoom;
        let s = &self.save;
        let p = &mut self.pads.p;
        if !s.accel_disabled {
            p[sp::ACCEL].x = 800.0 - 80.0;
            p[sp::BRAKE].x = 80.0;
            p[sp::GEAR].x = 800.0 - 64.0;
            p[sp::CAMERA].x = 40.0;
            p[sp::CAMERA].rx = 40.0;
        } else if !s.left_wheel {
            p[sp::WHEEL].x = (800.0 - 16.0) - z * 104.0;
            p[sp::ACCEL].x = 152.0;
            p[sp::BRAKE].x = 48.0;
            p[sp::GEAR].x = 800.0 - 64.0;
            p[sp::CAMERA].x = 40.0;
            p[sp::CAMERA].rx = 40.0;
        } else {
            p[sp::WHEEL].x = z * 104.0 + 16.0;
            p[sp::ACCEL].x = 800.0 - 48.0;
            p[sp::BRAKE].x = 800.0 - 152.0;
            p[sp::GEAR].x = 64.0;
            p[sp::CAMERA].x = 800.0 - 40.0;
            p[sp::CAMERA].rx = 800.0 - 40.0;
        }
        for i in [sp::WHEEL, sp::ACCEL, sp::BRAKE, sp::GEAR] {
            p[i].rx = p[i].x;
        }
    }

    // ------------------------------------------------------------------ sound helpers

    pub fn sfx(&mut self, s: Snd, vol: f32) {
        self.audio.enabled = !self.save.sound_disabled;
        self.audio.play(s, vol);
    }

    fn update_music(&mut self) {
        // the original keeps the music looping through menus and races alike
        let want = !self.save.music_disabled;
        self.audio.enabled = true;
        self.audio.set_loop(Snd::Music, want, 0.2);
    }

    pub fn stop_race_sounds(&mut self) {
        for s in [Snd::Engine, Snd::Blinker, Snd::Drift] {
            self.audio.set_loop(s, false, 0.0);
        }
    }

    // ------------------------------------------------------------------ pointer input

    /// Mouse and touch events -> pad events, plus the fling gesture for the scrolling tabs.
    fn pointer_input(&mut self) {
        let k = self.g.k;
        let _ = k;
        let ts = touches();
        if !ts.is_empty() {
            self.touch_seen = true;
        }
        if self.touch_seen {
            for t in ts {
                let (x, y) = self.g.to_virtual(t.position.x, t.position.y);
                match t.phase {
                    TouchPhase::Started => self.ptr_began(x, y),
                    TouchPhase::Moved => self.ptr_moved(x, y),
                    TouchPhase::Ended | TouchPhase::Cancelled => self.ptr_ended(x, y),
                    TouchPhase::Stationary => {}
                }
            }
            return;
        }
        let (mx, my) = mouse_position();
        let (x, y) = self.g.to_virtual(mx, my);
        if is_mouse_button_pressed(MouseButton::Left) {
            self.mouse_down = true;
            self.last_mouse = (x, y);
            self.ptr_began(x, y);
        } else if self.mouse_down && is_mouse_button_down(MouseButton::Left) {
            if (x - self.last_mouse.0).abs() > 0.01 || (y - self.last_mouse.1).abs() > 0.01 {
                self.last_mouse = (x, y);
                self.ptr_moved(x, y);
            }
        }
        if self.mouse_down && is_mouse_button_released(MouseButton::Left) {
            self.mouse_down = false;
            self.ptr_ended(x, y);
        }
    }

    fn ptr_began(&mut self, x: f32, y: f32) {
        if self.mode == 0 {
            return;
        }
        self.drag_samples.clear();
        self.drag_samples.push((self.clock, x));
        self.pads.on_began(x, y);
    }
    fn ptr_moved(&mut self, x: f32, y: f32) {
        self.drag_samples.push((self.clock, x));
        if self.drag_samples.len() > 12 {
            self.drag_samples.remove(0);
        }
        self.pads.on_moved(x, y);
    }
    fn ptr_ended(&mut self, x: f32, y: f32) {
        // `GestureDetector.onFling`: only the scrolling tabs listen for it
        if matches!(self.mode, 9 | 10 | 11) {
            if let (Some(first), Some(last)) = (self.drag_samples.iter().find(|s| self.clock - s.0 < 0.12), self.drag_samples.last()) {
                let dt = (last.0 - first.0).max(1.0 / 60.0);
                let v_virtual = (last.1 - first.1) / dt;
                let v_phys = v_virtual * 2.4; // the original's units are device pixels of a ~1920 px screen
                if v_phys.abs() > 130.0 {
                    self.fling = v_phys;
                }
            }
        }
        self.drag_samples.clear();
        self.pads.on_ended(x, y);
    }

    // ------------------------------------------------------------------ modes

    /// `changeGameMode`: switch mode and set up which pads are live.
    pub fn enter_mode(&mut self, m: i32) {
        if std::env::var_os("DRDRIVING_TRACE").is_some() {
            eprintln!("[{:7.1}s] mode {} -> {}", self.clock, self.mode, m);
        }
        self.fling = 0.0;
        self.mode_time = 0.0;
        self.last_mode = self.mode;
        self.mode = m;
        let p = &mut self.pads;
        match m {
            0 => self.cur_app = (self.world.rng.next_int_bound(24)) as usize,
            1 => {
                p.clear();
                for i in [sp::REPAIR, sp::MISSION_L, sp::MISSION_R, sp::CAR_L, sp::CAR_R, sp::RACE, sp::BACK_TO_MENU] {
                    p.activate(i);
                }
                let cm = self.world.car_mode;
                if cm >= PURCHASABLE_CARS && self.save.car_rent[cm.min(31)] < 1 {
                    self.world.car_mode = economy::best_car_id(&self.save);
                }
                self.world.init_map(&self.save);
            }
            2 => {
                p.clear();
                self.adjust_wheel_position();
                if self.save.accel_disabled {
                    self.pads.activate(sp::WHEEL);
                }
                for i in [
                    sp::ACCEL, sp::BRAKE, sp::GEAR, sp::CAMERA, sp::MIRROR_BACK, sp::MIRROR_LEFT, sp::MIRROR_RIGHT, sp::BLINKER_L, sp::BLINKER_R, sp::EMERGENCY,
                    sp::MENU,
                ] {
                    self.pads.activate(i);
                }
                self.sfx(Snd::Start, 0.3);
            }
            3 => {
                if self.save.accel_disabled {
                    p.activate(sp::WHEEL);
                }
                for i in [sp::ACCEL, sp::BRAKE, sp::GEAR, sp::CAMERA, sp::MENU, sp::BLINKER_L, sp::BLINKER_R, sp::EMERGENCY] {
                    p.activate(i);
                }
                p.deactivate(sp::RACE);
                p.deactivate(sp::BACK_TO_MENU);
            }
            4 => {
                for i in [sp::WHEEL, sp::ACCEL, sp::BRAKE, sp::GEAR, sp::MENU, sp::BLINKER_L, sp::BLINKER_R, sp::EMERGENCY] {
                    p.deactivate(i);
                }
                p.hide(sp::CAMERA);
                p.activate(sp::RACE);
                p.activate(sp::BACK_TO_MENU);
                self.stop_race_sounds();
            }
            5 => {
                self.stop_race_sounds();
                self.pads.clear();
            }
            6 => {
                self.stop_race_sounds();
                self.pads.clear();
            }
            8 => {
                p.clear();
                for i in [
                    sp::PROFILE_L, sp::PROFILE_R, sp::ONLINE, sp::ONLINE_FRIEND, sp::ONLINE_CHANNEL, sp::LEADERBOARDS, sp::ACHIEVEMENTS, sp::CARGO, sp::STORE,
                    sp::OPTIONS, sp::STAGE_START, sp::CANCEL_SEARCH,
                ] {
                    p.activate(i);
                }
            }
            9 => {
                p.clear();
                for i in [sp::SCROLL, sp::PROFILE_L, sp::PROFILE_R] {
                    p.activate(i);
                }
                let c = self.cargo.car;
                if c < 16 {
                    p.hide(sp::RENT_CAR);
                    if self.save.has_car[c] {
                        p.hide(sp::BUY_CAR);
                    } else {
                        p.show(sp::BUY_CAR);
                    }
                } else if self.save.car_rent[c] > 0 {
                    p.hide(sp::BUY_CAR);
                    p.hide(sp::RENT_CAR);
                } else {
                    p.show(sp::RENT_CAR);
                }
                for i in [sp::MENU_MAIN, sp::STORE, sp::OPTIONS, sp::STAGE_START, sp::CANCEL_SEARCH] {
                    p.activate(i);
                }
            }
            10 => {
                p.clear();
                for i in [sp::SCROLL, sp::MENU_MAIN, sp::CARGO, sp::OPTIONS, sp::STAGE_START, sp::CANCEL_SEARCH] {
                    p.activate(i);
                }
            }
            11 => {
                p.clear();
                for i in [sp::SCROLL, sp::FPS, sp::MENU_MAIN, sp::CARGO, sp::STORE, sp::STAGE_START, sp::CANCEL_SEARCH] {
                    p.activate(i);
                }
            }
            12 => p.deactivate_all(),
            13 => {
                p.deactivate_all();
                self.cur_app = ((self.cur_app + 3) + self.world.rng.next_int_bound(17) as usize) % 22;
            }
            _ => {}
        }
    }

    /// `setDialog(question, action, info1, info2)`
    pub fn set_dialog(&mut self, question: bool, action: i32, info1: i32, info2: i32) {
        let mut d = Dialog { question, action, info1, info2, ..Default::default() };
        if question {
            match action {
                0 => d.str1 = "Would you like to buy car?".into(),
                1 => d.str1 = "Would you like to upgrade?".into(),
                2 => {
                    d.str1 = "Would you like to buy Coin?".into();
                    d.str2 = format!("Price : {} Gold", info2);
                }
                3 => d.str1 = "Would you like to purchase Gold?".into(),
                4 => {
                    d.str1 = "Not enough Coin.".into();
                    d.str2 = "Would you like to buy Coin?".into();
                }
                5 => {
                    d.str1 = "Not enough Gold.".into();
                    d.str2 = "Would you like to purchase Gold?".into();
                }
                6 => d.str1 = "Would you like to rent car?".into(),
                _ => {}
            }
        } else {
            match action {
                0 => d.str1 = "Not enough Gold.".into(),
                1 => d.str1 = "You can not upgrade anymore.".into(),
                2 => {
                    d.str1 = "Not enough driver level".into();
                    d.str2 = "to upgrade.".into();
                }
                // port-specific notices for features that need Google/online services
                10 => d.str1 = "Online play is not available in this version.".into(),
                11 => d.str1 = "Purchases are not available in this version.".into(),
                12 => {
                    d.str1 = "Achievements".into();
                    let n = self.save.achievement.iter().filter(|&&b| b).count();
                    d.str2 = format!("{} / {} unlocked", n, economy::ACHIEVEMENTS.len());
                }
                13 => {
                    d.str1 = "Leaderboards are not available".into();
                    d.str2 = "without Google Play Games.".into();
                }
                _ => {}
            }
        }
        self.dialog = d;
        self.enter_mode(12);
    }

    // ------------------------------------------------------------------ the frame

    pub fn persist(&self) {
        persist::store(&self.save);
    }

    /// Edge-anchor HUD / menu-corner elements on race and tab screens (see `Gfx::off`).
    pub fn set_anchor(&mut self) {
        let menu = matches!(self.mode, 8..=13);
        self.g.anchor = menu || matches!(self.mode, 1..=6);
        let y = self.g.sh * 0.35;
        // dialogs sit on top of a tab: their own buttons (at 250/550) stay centred
        self.g.lthr = if matches!(self.mode, 8..=11) { 360.0 } else { 250.0 };
        self.g.band = if menu { (y - 88.0, y + 88.0) } else { (0.0, 0.0) };
    }

    /// Advance one fixed 1/60 s step.
    pub fn update(&mut self, dt: f32) {
        self.set_anchor();
        self.keys.tick(dt);
        self.clock += dt;
        self.dt = dt;
        self.mode_time += dt;
        self.app_time += dt;
        if (self.g.sh - self.last_sh).abs() > 0.01 {
            self.layout_pads();
        }
        self.pointer_input();
        self.keyboard_to_pads();
        self.update_music();
        if self.auto {
            self.autopilot_menu();
        }
        match self.mode {
            1 => self.process_car_select(),
            2 | 3 => self.race_frame(),
            4 => self.process_pause(),
            5 => {
                self.race_frame();
                self.process_success();
            }
            6 => self.process_ending(),
            8 => self.process_tab_profile(),
            9 => self.process_tab_cargo(),
            10 => self.process_tab_store(),
            11 => self.process_tab_option(),
            12 => self.process_dialog(),
            13 => self.process_exit(),
            _ => {}
        }
        // shared menu-bar buttons (active only in the tab modes and the car-select flow)
        if self.pads.get_clicked(sp::STAGE_START) {
            self.world.change_mission_random();
            if self.mode == 9 && self.cargo.car < 28 {
                let c = self.cargo.car;
                let owned = if c < 16 { self.save.has_car[c] } else { self.save.car_rent[c] > 0 };
                if owned {
                    self.world.car_mode = c;
                }
            }
            self.enter_mode(1);
        } else if self.pads.get_clicked(sp::MENU_MAIN) {
            self.enter_mode(8);
        } else if self.pads.get_clicked(sp::CARGO) {
            self.enter_mode(9);
        } else if self.pads.get_clicked(sp::STORE) {
            self.enter_mode(10);
        } else if self.pads.get_clicked(sp::OPTIONS) {
            self.enter_mode(11);
        }
        if self.pads.click_pending {
            self.pads.click_pending = false;
            self.sfx(Snd::Click, 0.4);
        }
        // easing for the coin counters
        if self.moving_coin.visible {
            self.moving_coin.alive += dt;
            if self.moving_coin.alive > 0.5 {
                self.moving_coin.visible = false;
            } else {
                let t = self.moving_coin.alive;
                self.moving_coin.x = 200.0 * t - 100.0;
                self.moving_coin.y = (t - 0.3) * (t - 0.3) * 3000.0 - 120.0;
            }
        }
        if self.moving_gold.visible {
            self.moving_gold.alive += dt;
            if self.moving_gold.alive > 1.0 {
                self.moving_gold.visible = false;
            }
        }
        if self.effect_turn.visible {
            self.effect_turn.alive += dt;
            let t = self.effect_turn.alive;
            if t > 2.0 {
                self.effect_turn.visible = false;
            } else if t > 0.5 {
                self.effect_turn.y = 0.0;
            } else {
                self.effect_turn.y = (t - 0.4) * (t - 0.4) * 2000.0 - 20.0;
            }
        }
    }

    fn autopilot_menu(&mut self) {
        match self.mode {
            8 | 9 | 10 | 11 if self.mode_time > 1.0 => self.keys.inject(KeyCode::Space),
            1 if self.mode_time > 1.5 => self.keys.inject(KeyCode::Enter),
            _ => {}
        }
    }

    // ------------------------------------------------------------------ keyboard -> pads

    /// Maps the keyboard onto the same actions the touch pads trigger.
    fn keyboard_to_pads(&mut self) {
        let k = &self.keys;
        let pressed = |c: KeyCode| k.pressed(c);
        let press = |pads: &mut Pads, i: usize| {
            if pads.p[i].active {
                pads.p[i].clicked = true;
            }
        };
        if pressed(KeyCode::F1) && self.mode >= 8 && self.mode <= 11 && !self.keys.down(KeyCode::LeftShift) {
            // F1 = help overlay everywhere
        }
        if pressed(KeyCode::H) && !matches!(self.mode, 2 | 3) {
            self.help = !self.help;
        }
        match self.mode {
            8 | 9 | 10 | 11 => {
                if pressed(KeyCode::Tab) {
                    let n = [8, 9, 10, 11].iter().position(|&m| m == self.mode).unwrap();
                    let n = if k.down(KeyCode::LeftShift) { (n + 3) % 4 } else { (n + 1) % 4 };
                    self.enter_mode([8, 9, 10, 11][n]);
                    return;
                }
                for (key, m) in [(KeyCode::F1, 8), (KeyCode::F2, 9), (KeyCode::F3, 10), (KeyCode::F4, 11)] {
                    if pressed(key) && self.mode != m {
                        self.enter_mode(m);
                        return;
                    }
                }
                if pressed(KeyCode::Space) {
                    press(&mut self.pads, sp::STAGE_START);
                }
                if pressed(KeyCode::Escape) {
                    self.enter_mode(13);
                    return;
                }
                if self.mode == 8 {
                    if pressed(KeyCode::Enter) {
                        press(&mut self.pads, sp::STAGE_START);
                    }
                    if pressed(KeyCode::Left) {
                        press(&mut self.pads, sp::PROFILE_L);
                    }
                    if pressed(KeyCode::Right) {
                        press(&mut self.pads, sp::PROFILE_R);
                    }
                }
                if matches!(self.mode, 9..=11) {
                    // arrows scroll the card strip by one card
                    let step = 210.0;
                    let (lo, hi) = match self.mode {
                        9 => (-340.0, 360.0),
                        _ => (-130.0, 360.0),
                    };
                    let sc = match self.mode {
                        9 => &mut self.tab_cargo,
                        10 => &mut self.tab_store,
                        _ => &mut self.tab_option,
                    };
                    if self.mode != 9 {
                        if pressed(KeyCode::Left) {
                            sc.scroll = (sc.scroll - step).clamp(lo, hi);
                        }
                        if pressed(KeyCode::Right) {
                            sc.scroll = (sc.scroll + step).clamp(lo, hi);
                        }
                    }
                }
                if self.mode == 9 {
                    if pressed(KeyCode::Left) {
                        press(&mut self.pads, sp::PROFILE_L);
                    }
                    if pressed(KeyCode::Right) {
                        press(&mut self.pads, sp::PROFILE_R);
                    }
                    if pressed(KeyCode::Enter) {
                        press(&mut self.pads, sp::BUY_CAR);
                        press(&mut self.pads, sp::RENT_CAR);
                    }
                }
            }
            12 => {
                if pressed(KeyCode::Enter) || pressed(KeyCode::Y) || pressed(KeyCode::Space) {
                    press(&mut self.pads, sp::DIALOG_OK);
                    press(&mut self.pads, sp::DIALOG_CONFIRM);
                }
                if pressed(KeyCode::Escape) || pressed(KeyCode::N) {
                    press(&mut self.pads, sp::DIALOG_CANCEL);
                    press(&mut self.pads, sp::DIALOG_CONFIRM);
                }
            }
            13 => {
                if pressed(KeyCode::Enter) || pressed(KeyCode::Y) {
                    press(&mut self.pads, sp::EXIT_OK);
                }
                if pressed(KeyCode::Escape) || pressed(KeyCode::N) {
                    press(&mut self.pads, sp::EXIT_CANCEL);
                }
            }
            1 => {
                if pressed(KeyCode::Left) || pressed(KeyCode::A) {
                    press(&mut self.pads, sp::CAR_L);
                }
                if pressed(KeyCode::Right) || pressed(KeyCode::D) {
                    press(&mut self.pads, sp::CAR_R);
                }
                if pressed(KeyCode::Up) || pressed(KeyCode::W) {
                    press(&mut self.pads, sp::MISSION_L);
                }
                if pressed(KeyCode::Down) || pressed(KeyCode::S) {
                    press(&mut self.pads, sp::MISSION_R);
                }
                if pressed(KeyCode::R) {
                    press(&mut self.pads, sp::REPAIR);
                }
                if pressed(KeyCode::Enter) || pressed(KeyCode::Space) {
                    press(&mut self.pads, sp::RACE);
                }
                if pressed(KeyCode::Escape) {
                    press(&mut self.pads, sp::BACK_TO_MENU);
                }
            }
            4 => {
                if pressed(KeyCode::Escape) || pressed(KeyCode::P) || pressed(KeyCode::Enter) {
                    press(&mut self.pads, sp::RACE);
                }
                if pressed(KeyCode::Q) {
                    press(&mut self.pads, sp::BACK_TO_MENU);
                }
            }
            5 | 6 => {
                if pressed(KeyCode::Enter) || pressed(KeyCode::Space) {
                    press(&mut self.pads, sp::RACE);
                }
                if pressed(KeyCode::Escape) {
                    press(&mut self.pads, sp::BACK_TO_MENU);
                }
            }
            _ => {}
        }
    }

    /// 1..6 keys act like tapping the n-th visible card in a tab strip.
    pub fn card_key(&self) -> Option<usize> {
        for (i, k) in [KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4, KeyCode::Key5, KeyCode::Key6].iter().enumerate() {
            if self.keys.pressed(*k) {
                return Some(i);
            }
        }
        None
    }

    // ------------------------------------------------------------------ car select (mode 1)

    fn process_car_select(&mut self) {
        let dt = self.dt.min(0.1);
        let p = &mut self.world.player;
        if p.cy < 1984.0 {
            let step = (1984.0 - p.cy) * dt;
            p.cy += step;
            p.ang_distance += step * p.ang_velocity;
            p.cell_x = drdriving_core::math::java_round(p.cx / 140.0);
            p.cell_y = drdriving_core::math::java_round(p.cy / 140.0);
        }
        if self.pads.get_clicked(sp::BACK_TO_MENU) {
            self.enter_mode(8);
            return;
        }
        if self.pads.get_clicked(sp::CAR_L) {
            self.world.change_car(-1, &self.save);
            return;
        }
        if self.pads.get_clicked(sp::CAR_R) {
            self.world.change_car(1, &self.save);
            return;
        }
        if self.pads.get_clicked(sp::REPAIR) {
            match economy::repair_car(&mut self.save, self.world.car_mode) {
                Ok(_) => self.persist(),
                Err(_) => {}
            }
            return;
        }
        if self.pads.get_clicked(sp::MISSION_L) {
            self.world.change_mission(-1);
            return;
        }
        if self.pads.get_clicked(sp::MISSION_R) {
            self.world.change_mission(1);
            return;
        }
        if self.pads.get_clicked(sp::RACE) {
            self.start_race();
        }
    }

    /// Leave car select and start the countdown.
    pub fn start_race(&mut self) {
        self.cargo.car = self.world.car_mode;
        let mut w = std::mem::replace(&mut self.world, World::new(0, Some(1)));
        w.prepare_race(&self.save);
        self.drifts.clear();
        self.result = ResultInfo::default();
        self.session = Some(Session::new(w));
        self.camera_mode = 4;
        self.prev_wheel = 0.0;
        self.enter_mode(2);
    }

    // ------------------------------------------------------------------ race (modes 2, 3, 5)

    fn race_frame(&mut self) {
        let Some(mut s) = self.session.take() else {
            self.enter_mode(8);
            return;
        };
        let dt = self.dt;
        let k = &self.keys;
        let down = |a: KeyCode, b: KeyCode| k.down(a) || k.down(b);
        let hit = |a: KeyCode| k.pressed(a);

        // gear pad (slider) and keyboard
        let gp = self.pads.p[sp::GEAR].clone();
        let mut gear = None;
        if gp.on && gp.active && s.world.speed < 5.0 {
            gear = Some(if gp.cy < -46 {
                0
            } else if gp.cy < 4 {
                1
            } else if gp.cy < 46 {
                2
            } else {
                3
            });
        }
        for (key, g) in [(KeyCode::Key1, 0), (KeyCode::Key2, 1), (KeyCode::Key3, 2), (KeyCode::Key4, 3)] {
            if hit(key) {
                gear = Some(g);
            }
        }
        // wheel: drag (pad) or keys
        let steer_dir = down(KeyCode::Right, KeyCode::D) as i32 - down(KeyCode::Left, KeyCode::A) as i32;
        let wheel_pad = self.pads.p[sp::WHEEL].clone();
        let mut wheel = wheel_pad.angle;
        let playing = s.phase == Phase::Playing;
        if steer_dir != 0 && playing {
            wheel = (wheel + steer_dir as f32 * drdriving_core::session::WHEEL_KEY_RATE * dt).clamp(-540.0, 540.0);
        }
        s.wheel_angle = wheel;
        let wheel_held = (wheel_pad.on && wheel_pad.active) || steer_dir != 0;

        let wheel_click = self.pads.p[sp::WHEEL].clicked && self.pads.p[sp::WHEEL].active;
        self.pads.p[sp::WHEEL].clicked = false;
        self.kb_accel = down(KeyCode::Up, KeyCode::W);
        self.kb_brake = down(KeyCode::Down, KeyCode::S);
        let mut input = Input {
            accel: (self.pads.p[sp::ACCEL].on && self.pads.p[sp::ACCEL].active) || down(KeyCode::Up, KeyCode::W),
            brake: (self.pads.p[sp::BRAKE].on && self.pads.p[sp::BRAKE].active) || down(KeyCode::Down, KeyCode::S),
            steer: 0,
            wheel_held,
            horn: wheel_click || hit(KeyCode::Space),
            blinker_left: self.pads.get_clicked(sp::BLINKER_L) || hit(KeyCode::Q),
            blinker_right: self.pads.get_clicked(sp::BLINKER_R) || hit(KeyCode::E),
            hazard: self.pads.get_clicked(sp::EMERGENCY) || hit(KeyCode::H),
            camera: self.pads.get_clicked(sp::CAMERA) || hit(KeyCode::C),
            gear,
            mirror_back: self.pads.get_clicked(sp::MIRROR_BACK) || hit(KeyCode::B),
            mirror_left: self.pads.get_clicked(sp::MIRROR_LEFT) || hit(KeyCode::Z),
            mirror_right: self.pads.get_clicked(sp::MIRROR_RIGHT) || hit(KeyCode::X),
            pause: false,
        };
        if matches!(s.phase, Phase::Starting | Phase::Playing) && (self.pads.get_clicked(sp::MENU) || hit(KeyCode::Escape) || hit(KeyCode::P)) {
            input.pause = true;
        }
        if self.auto {
            let (accel, brake, wheel) = drdriving_core::autopilot::drive(&s.world, self.save.right_driver);
            input.accel = accel;
            input.brake = brake;
            input.wheel_held = false;
            if s.phase == Phase::Playing {
                s.wheel_angle = wheel;
            }
        }
        if self.no_traffic {
            for c in s.world.cars.iter_mut() {
                c.visible = false;
            }
        }
        if matches!(s.phase, Phase::Starting | Phase::Playing) {
            self.snap_coin = self.save.coin;
            self.snap_exp = self.save.driver_exp;
            self.snap_gold = self.save.gold;
        }
        let was_over = s.is_over();
        let events = s.frame(dt, &input, &mut self.save);
        // the session may have re-centred the wheel
        self.pads.p[sp::WHEEL].angle = s.wheel_angle;
        self.camera_mode = s.camera_mode;
        if s.world.drifting {
            let p = &s.world.player;
            self.drifts.push(DriftMark { x: p.x, y: p.y, angle: 180.0 - p.angle });
        }
        let sound_on = !self.save.sound_disabled;
        for e in events {
            match e {
                Event::Horn => self.sfx(Snd::Horn, 1.0),
                Event::BrakeSound => self.sfx(Snd::Brake, 0.2),
                Event::Crash => self.sfx(Snd::Crash, 0.1),
                Event::Coin(r) => {
                    if sound_on {
                        self.sfx(Snd::Submission, 1.0);
                    }
                    self.moving_coin.visible = true;
                    self.moving_coin.alive = 0.0;
                    let _ = r == CoinReason::Turn;
                }
                Event::TurnRating(r) => {
                    self.effect_turn.visible = true;
                    self.effect_turn.alive = 0.0;
                    self.effect_turn.x = 400.0;
                    self.effect_turn.y = self.g.sh - 120.0;
                    self.effect_turn.rate = r as i32;
                }
                Event::Won => {
                    self.sfx(Snd::Clap, 1.0);
                    self.success_cam = crate::scene::success_pos(&s.world);
                }
                Event::Go => {}
                Event::Paused(true) => {}
                Event::Paused(false) => {}
                Event::OutOfFuel | Event::BlinkerSound(_) | Event::DriftStart | Event::DriftEnd => {}
            }
        }
        // looping sounds follow the world state
        {
            let w = &s.world;
            self.audio.enabled = sound_on;
            let live = matches!(s.phase, Phase::Starting | Phase::Playing);
            self.audio.set_loop(Snd::Engine, live, 0.9);
            self.audio.loop_volume(Snd::Engine, 0.35 + 0.6 * (w.speed / 25.0).min(1.0));
            self.audio.set_loop(Snd::Blinker, live && (w.blinker != 0 || w.emergency), 0.5);
            self.audio.set_loop(Snd::Drift, live && w.drifting, 0.5);
        }
        let new_phase = s.phase;
        let over_now = !was_over && s.is_over();
        let mut result = None;
        if over_now {
            // capture what the result screens show
            result = s.result.clone();
        }
        self.session = Some(s);
        // mode transitions follow the session phase
        let want = match new_phase {
            Phase::Starting => 2,
            Phase::Playing => 3,
            Phase::Paused => 4,
            Phase::Success => 5,
            Phase::Ending(_) => 6,
        };
        if want != self.mode && (self.mode != 5 || want == 5) {
            self.enter_mode(want);
        }
        if over_now {
            if let Some(r) = result {
                self.result.last_coin = self.snap_coin;
                self.result.last_exp = self.snap_exp;
                self.result.last_gold = self.snap_gold;
                if let Some(sr) = &r.success {
                    self.result.reward = sr.reward_coins;
                    self.result.new_record = sr.new_record;
                    self.result.last_record = sr.previous_record;
                    self.result.record = sr.record;
                }
                if r.outcome == Outcome::Accident {
                    // handled by the session (accident counter)
                }
            }
            self.persist();
        }
    }

    pub fn auto_input_clear(&mut self) {}

    fn process_pause(&mut self) {
        // resume / abandon (the session itself is frozen while paused)
        if self.pads.get_clicked(sp::RACE) {
            if let Some(mut s) = self.session.take() {
                let mut dummy = Input { pause: true, ..Default::default() };
                dummy.wheel_held = true;
                s.frame(0.0, &dummy, &mut self.save);
                self.session = Some(s);
            }
            self.enter_mode(3);
        } else if self.pads.get_clicked(sp::BACK_TO_MENU) {
            self.leave_race();
            self.world.change_mission_random();
            self.enter_mode(1);
        }
    }

    fn leave_race(&mut self) {
        self.stop_race_sounds();
        if let Some(s) = self.session.take() {
            self.world = s.world;
        }
        self.persist();
    }

    fn process_success(&mut self) {
        // buttons appear 2.5 s after the finish (processSuccess)
        let ready = self.mode_time > 2.5;
        if ready {
            if !self.pads.p[sp::RACE].active {
                self.pads.activate(sp::RACE);
            }
            if !self.pads.p[sp::BACK_TO_MENU].active {
                self.pads.activate(sp::BACK_TO_MENU);
            }
        }
        if self.pads.get_clicked(sp::RACE) {
            self.leave_race();
            self.world.change_mission_random();
            self.enter_mode(1);
        } else if self.pads.get_clicked(sp::BACK_TO_MENU) {
            self.leave_race();
            self.enter_mode(8);
        }
        if self.auto && ready {
            self.pads.p[sp::RACE].clicked = true;
        }
    }

    fn process_ending(&mut self) {
        // buttons appear 1.5 s after the crash (processEnding)
        let ready = self.mode_time > 1.5;
        if ready {
            if !self.pads.p[sp::RACE].active {
                self.pads.activate(sp::RACE);
            }
            if !self.pads.p[sp::BACK_TO_MENU].active {
                self.pads.activate(sp::BACK_TO_MENU);
            }
        }
        if self.pads.get_clicked(sp::RACE) {
            self.leave_race();
            self.enter_mode(1);
        } else if self.pads.get_clicked(sp::BACK_TO_MENU) {
            self.leave_race();
            self.enter_mode(8);
        }
        if self.auto && ready {
            self.pads.p[sp::RACE].clicked = true;
        }
    }

    fn process_exit(&mut self) {
        if self.pads.get_clicked(sp::EXIT_OK) {
            self.persist();
            self.quit = true;
        }
        if self.pads.get_clicked(sp::EXIT_CANCEL) {
            let m = self.last_mode;
            self.enter_mode(m);
        }
        if self.mode_time > 0.15 {
            self.pads.activate(sp::EXIT_OK);
            self.pads.activate(sp::EXIT_CANCEL);
        }
    }

    // ------------------------------------------------------------------ shop actions (dialogs)

    fn shop_notice(&mut self, e: ShopError) {
        let _ = e;
    }

    fn process_dialog(&mut self) {
        if !self.pads.get_clicked(sp::DIALOG_OK) {
            if self.pads.get_clicked(sp::DIALOG_CANCEL) {
                let m = self.last_mode;
                self.enter_mode(m);
                return;
            }
            if self.pads.get_clicked(sp::DIALOG_CONFIRM) {
                let m = self.last_mode;
                self.enter_mode(m);
                return;
            }
            let t = self.mode_time;
            if t > 0.2 {
                if self.dialog.question {
                    self.pads.activate(sp::DIALOG_OK);
                    self.pads.activate(sp::DIALOG_CANCEL);
                } else if t > 3.0 {
                    let m = self.last_mode;
                    self.enter_mode(m);
                } else {
                    self.pads.activate(sp::DIALOG_CONFIRM);
                }
            }
            return;
        }
        let d = self.dialog.clone();
        let back = self.last_mode;
        match d.action {
            0 => {
                let _ = economy::buy_car(&mut self.save, d.info1 as usize).map_err(|e| self.shop_notice(e));
                self.world.car_mode = if self.save.has_car[d.info1 as usize] { d.info1 as usize } else { self.world.car_mode };
                self.persist();
                self.enter_mode(back);
            }
            1 => {
                let _ = economy::upgrade_car(&mut self.save, d.info1 as usize, d.info2 as usize);
                self.persist();
                self.enter_mode(back);
            }
            2 => {
                let _ = economy::buy_coin(&mut self.save, d.info1, d.info2);
                self.persist();
                self.enter_mode(back);
            }
            3 => {
                // real-money purchase: no billing in this build
                self.enter_mode(back);
                self.set_dialog(false, 11, 0, 0);
            }
            4 => {
                self.tab_store.scroll = -130.0;
                self.enter_mode(10);
            }
            5 => {
                self.tab_store.scroll = 520.0;
                self.enter_mode(10);
            }
            6 => {
                let _ = economy::rent_car(&mut self.save, d.info1 as usize);
                self.world.car_mode = if self.save.car_rent[d.info1 as usize] > 0 { d.info1 as usize } else { self.world.car_mode };
                self.persist();
                self.enter_mode(back);
            }
            _ => self.enter_mode(back),
        }
        // keep the garage card in sync with what was just bought/rented
        let c = self.cargo.car;
        if c < PLAYER_CARS {
            self.cargo.car = c;
        }
    }
}

// ---- tiny helpers shared with the UI modules

pub fn car_status(save: &SaveGame, c: usize) -> String {
    let info = car(c);
    if save.has_car[c] && c < 16 {
        "Owned".to_string()
    } else if c < 16 || save.car_rent[c] <= 0 {
        if info.price < 0 {
            format!("{} Coin", -info.price)
        } else {
            format!("{} Gold", info.price)
        }
    } else if save.car_rent[c] < 1000 {
        format!("{}m left", save.car_rent[c])
    } else {
        format!("{}km left", save.car_rent[c] / 1000)
    }
}

pub fn mission_of(m: Mission) -> usize {
    m.index()
}
