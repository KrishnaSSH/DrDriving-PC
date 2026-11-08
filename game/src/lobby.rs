//! Multiplayer front end: starting a search (Online Match / a friend channel), the search
//! overlay, pumping the network between races, starting an online race and the rematch / leave
//! buttons on the result screens.
//!
//! Matchmaking and messaging live in `drdriving_core::rtdb` (the original game's Firebase
//! Realtime Database); this file is only the glue and the screens:
//!
//! * **Online Match** searches the rating channel straight away, like the original: the menu stays
//!   usable and an hourglass spins next to the cancel button; after 40 s the search gives up.
//! * **vs Friend / Channel** open mode 14 where both players type the same number (100-999).

use crate::game::*;
use crate::ui_menu::class_for;
use crate::http;
use crate::persist;
use crate::sprites as S;
use drdriving_core::cars::PURCHASABLE_CARS;
use drdriving_core::economy;
use drdriving_core::net::{self, COUNTRY_NAMES};
use drdriving_core::netplay::{CloseReason, Event as NetEvent, Profile, MAX_NAME};
use drdriving_core::rtdb::Connection;
use macroquad::prelude::*;

/// `g_fSandWatchTime > 40`: a search that finds nobody gives up after this long.
const SEARCH_TIMEOUT: f32 = 40.0;
/// The cancel button only works after this long (the original needs the room to exist first).
const CANCEL_AFTER: f32 = 3.0;
/// Pause before a quick search starts over because its opponent vanished.
const RETRY_AFTER: f32 = 1.0;

/// An automatic search in progress (`g_bAutoMatching` + `g_fSandWatchTime`).
pub struct Search {
    pub time: f32,
    pub channel: i32,
    /// Online Match (rating channel) as opposed to a friend channel.
    pub quick: bool,
    /// Start over in this many seconds (the connection broke before a race began).
    retry: Option<f32>,
}

/// Enter / Escape pressed in the browser's invisible input.
enum Submit {
    None,
    Enter,
    Escape,
}

/// What the text dialog (mode 14) edits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edit {
    Channel,
    Name,
    /// The search box of the country picker.
    Search,
}

pub struct Lobby {
    /// Shown to the opponent and on the HUD.
    pub name: String,
    /// Digits typed on the channel screen.
    pub channel_text: String,
    pub edit: Edit,
    /// Two-letter country code whose flag the opponent sees (empty = none).
    pub country: String,
    /// The tab screen the dialog was opened from (and returns to).
    pub return_mode: i32,
    /// A message like Android's `Toast` at the bottom of the screen: (text, seconds left).
    pub toast: Option<(String, f32)>,
    /// Country picker: scroll offset (px), typed search text, and a drag in progress
    /// (start y, scroll at start, moved far enough to be a scroll rather than a tap).
    /// Text field editing: caret and selection anchor (char indices), a mouse drag in progress,
    /// time since the last edit (the caret blinks from it), held-key repeat timers, and the last
    /// click (time, count) for double / triple clicks.
    pub caret: usize,
    pub anchor: usize,
    pub field_drag: bool,
    pub blink: f32,
    rep: Vec<(KeyCode, f32)>,
    last_click: (f32, u8),
    pub pick_scroll: f32,
    pub pick_filter: String,
    pub pick_drag: Option<(f32, f32, bool)>,
    pub search: Option<Search>,
    /// Both ratings when the current race began (the result screen counts them to the new values).
    pub rating_before: i32,
    pub opp_rating_before: i32,
    bs_held: f32,
}

impl Default for Lobby {
    fn default() -> Self {
        let name = persist::pref_get("name")
            .or_else(|| std::env::var("DRDRIVING_NAME").ok())
            .or_else(|| std::env::var("USER").ok())
            .or_else(|| std::env::var("USERNAME").ok())
            .unwrap_or_else(|| "Player".into());
        Lobby {
            name: name.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect(),
            channel_text: persist::pref_get("channel").unwrap_or_default(),
            edit: Edit::Channel,
            country: persist::pref_get("country").unwrap_or_else(|| persist::locale_country().to_uppercase()).chars().filter(|c| c.is_ascii_alphabetic()).take(2).collect::<String>().to_uppercase(),
            return_mode: 8,
            toast: None,
            caret: 0,
            anchor: 0,
            field_drag: false,
            blink: 0.0,
            rep: Vec::new(),
            last_click: (-10.0, 0),
            pick_scroll: 0.0,
            pick_filter: String::new(),
            pick_drag: None,
            search: None,
            rating_before: 0,
            opp_rating_before: 0,
            bs_held: 0.0,
        }
    }
}

impl Game {
    // ------------------------------------------------------------------ starting / stopping

    pub fn searching(&self) -> bool {
        self.lobby.search.is_some()
    }

    /// The garage selection can be a truck left over from a Truck mission or an expired rental.
    fn fix_car(&mut self) {
        let cm = self.world.car_mode;
        if cm >= PURCHASABLE_CARS && self.save.car_rent[cm.min(31)] < 1 {
            self.world.car_mode = economy::best_car_id(&self.save);
        }
    }

    fn profile(&mut self) -> Profile {
        self.fix_car();
        Profile {
            rating: self.save.rating,
            car: self.world.car_mode as i32,
            country: net::country_index(&self.lobby.country),
            name: if self.lobby.name.trim().is_empty() { "Player".into() } else { self.lobby.name.clone() },
        }
    }

    fn seed(&self) -> i64 {
        (macroquad::miniquad::date::now() * 1000.0) as i64 ^ (self.clock.to_bits() as i64)
    }

    fn connect(&mut self, channel: i32) {
        let (profile, seed) = (self.profile(), self.seed());
        self.net = Some(Connection::new(http::backend(), channel, profile, seed, http::now_ms()));
    }

    /// `startQuickOnlineGame(0)`: look for an opponent of similar rating.
    pub fn start_quick(&mut self) {
        if self.net.is_some() || self.searching() {
            return;
        }
        let channel = net::quick_channel(self.save.rating, self.save.right_driver);
        self.begin_search(channel, true);
    }

    /// `startQuickOnlineGame(n)` for a friend channel.
    pub fn start_channel(&mut self, n: i32) {
        if self.net.is_some() || self.searching() {
            return;
        }
        let Some(channel) = net::friend_channel(n) else { return };
        persist::pref_set("channel", &channel.to_string());
        persist::pref_set("name", &self.lobby.name);
        self.begin_search(channel, false);
    }

    fn begin_search(&mut self, channel: i32, quick: bool) {
        self.online_notice.clear();
        self.connect(channel);
        self.lobby.search = Some(Search { time: 0.0, channel, quick, retry: None });
        if self.mode == 14 {
            self.enter_mode(8);
        }
    }

    /// Stop everything online: the connection (which removes our lobby / room) and the search.
    pub fn leave_online(&mut self) {
        if let Some(mut c) = self.net.take() {
            c.leave();
        }
        self.lobby.search = None;
        self.online_notice.clear();
        self.forfeit_armed = 0.0;
        self.world.clear_online();
    }

    /// Give up the search; `why` is shown in a dialog on a tab screen.
    pub fn cancel_search(&mut self, why: Option<(&str, &str)>) {
        if let Some(mut c) = self.net.take() {
            c.leave();
        }
        self.lobby.search = None;
        if let Some((a, b)) = why {
            self.notice(a, b);
        }
    }

    /// A message dialog over the tab screens (no-op elsewhere).
    pub fn notice(&mut self, a: &str, b: &str) {
        if !matches!(self.mode, 8..=11) {
            return;
        }
        self.dialog = Dialog { question: false, action: 14, str1: a.to_string(), str2: b.to_string(), ..Default::default() };
        self.enter_mode(12);
    }

    // ------------------------------------------------------------------ text dialog (mode 14)

    /// `dialogChannel()`: ask for the channel number. Like the original it first leaves any room.
    pub fn enter_lobby(&mut self) {
        if self.searching() {
            self.cancel_search(None);
        }
        self.open_text_dialog(Edit::Channel);
    }

    /// Settings -> Name.
    pub fn edit_name(&mut self) {
        self.open_text_dialog(Edit::Name);
    }

    /// Settings -> Country: a scrollable list of countries with their flags (mode 15).
    pub fn edit_country(&mut self) {
        while get_char_pressed().is_some() {}
        self.lobby.return_mode = if matches!(self.mode, 8..=11) { self.mode } else { 11 };
        self.lobby.pick_filter.clear();
        self.lobby.pick_drag = None;
        self.lobby.edit = Edit::Search;
        self.lobby.caret = 0;
        self.lobby.anchor = 0;
        self.lobby.blink = 0.0;
        self.enter_mode(15);
        text_input::open("", 20, false);
        // start with the current country in view
        let rows = self.pick_rows();
        let cur = self.lobby.country.clone();
        let i = rows.iter().position(|&r| COUNTRY_NAMES[r].0 == cur).unwrap_or(0);
        self.lobby.pick_scroll = (i as f32 * PICK_ROW - 3.0 * PICK_ROW).max(0.0);
    }

    fn open_text_dialog(&mut self, edit: Edit) {
        // drop any characters typed before the dialog opened
        while get_char_pressed().is_some() {}
        self.lobby.edit = edit;
        // like a browser's address bar, the current text starts out selected
        let n = self.field().chars().count();
        self.lobby.anchor = 0;
        self.lobby.caret = n;
        self.lobby.blink = 0.0;
        self.lobby.return_mode = if matches!(self.mode, 8..=11) { self.mode } else { 8 };
        self.enter_mode(14);
        // in the browser a real HTML input is shown on top (phones have no keyboard for the canvas)
        let (_, initial, max, numeric) = self.dialog_text();
        text_input::open(&initial, max, numeric);
    }

    fn dialog_text(&self) -> (&'static str, String, u32, bool) {
        match self.lobby.edit {
            Edit::Channel => ("Please input the channel number.", self.lobby.channel_text.clone(), 3, true),
            Edit::Name => ("Please input your name.", self.lobby.name.clone(), MAX_NAME as u32, false),
            Edit::Search => ("", self.lobby.pick_filter.clone(), 20, false),
        }
    }

    fn close_text_dialog(&mut self) {
        text_input::close();
        let m = self.lobby.return_mode;
        self.enter_mode(m);
    }

    pub fn show_toast(&mut self, text: &str) {
        self.lobby.toast = Some((text.to_string(), 3.0));
    }

    /// OK: apply what was typed.
    fn accept_text(&mut self) {
        match self.lobby.edit {
            Edit::Name => {
                let n = self.lobby.name.split_whitespace().collect::<Vec<_>>().join(" ");
                self.lobby.name = if n.is_empty() { "Player".into() } else { n };
                persist::pref_set("name", &self.lobby.name);
                self.close_text_dialog();
            }
            Edit::Channel => {
                let t = self.lobby.channel_text.clone();
                match t.parse::<i32>().ok().filter(|_| t.len() == 3).and_then(net::friend_channel) {
                    Some(n) => {
                        text_input::close();
                        self.start_channel(n)
                    }
                    None => {
                        // the original dismisses the dialog and shows this as a toast
                        self.close_text_dialog();
                        self.show_toast("Please input the same number with your friend between 100~999.");
                    }
                }
            }
            Edit::Search => {}
        }
    }

    fn field(&mut self) -> &mut String {
        match self.lobby.edit {
            Edit::Channel => &mut self.lobby.channel_text,
            Edit::Name => &mut self.lobby.name,
            Edit::Search => &mut self.lobby.pick_filter,
        }
    }

    pub fn process_lobby(&mut self) {
        match self.sync_web_input() {
            Submit::Enter => {
                self.accept_text();
                return;
            }
            Submit::Escape => {
                self.close_text_dialog();
                return;
            }
            Submit::None => {}
        }
        if self.pads.get_clicked(sp::CH_CANCEL) {
            self.close_text_dialog();
            return;
        }
        if self.pads.get_clicked(sp::CH_PLAY) || self.keys.pressed(KeyCode::Enter) {
            self.accept_text();
        }
    }

    // ---- text editing: the field behaves like a browser's address bar

    fn web_input_active(&self) -> bool {
        let (l, t, w, h) = self.field_px();
        text_input::snapshot(l, t, w, h).is_some()
    }

    /// The game's field in window pixels (where the invisible browser input is laid).
    fn field_px(&self) -> (f32, f32, f32, f32) {
        let (l, r, cy, size) = self.field_rect();
        let half = size * 0.75 + 5.0;
        (self.g.sx(l, r - l, cy - half, 2.0 * half), self.g.oy + (cy - half) * self.g.k, (r - l) * self.g.k, 2.0 * half * self.g.k)
    }

    /// Browser build: copy the invisible input's text and selection into the game's field (and
    /// filter it); report Enter / Escape pressed in it. Does nothing on other platforms.
    fn sync_web_input(&mut self) -> Submit {
        let (l, t, w, h) = self.field_px();
        let Some(s) = text_input::snapshot(l, t, w, h) else { return Submit::None };
        let filtered: String = s.value.chars().filter(|&c| self.accepts(c)).take(self.max_len()).collect();
        let n = filtered.chars().count();
        if filtered != s.value {
            text_input::set(&filtered, n, n);
        }
        let (anchor, caret) = (s.anchor.min(n), s.caret.min(n));
        if filtered != *self.field() || anchor != self.lobby.anchor || caret != self.lobby.caret {
            self.lobby.blink = 0.0;
        }
        *self.field() = filtered;
        self.lobby.anchor = anchor;
        self.lobby.caret = caret;
        match s.state {
            1 => Submit::Enter,
            2 => Submit::Escape,
            _ => Submit::None,
        }
    }

    fn accepts(&self, c: char) -> bool {
        match self.lobby.edit {
            Edit::Channel => c.is_ascii_digit(),
            Edit::Name | Edit::Search => !c.is_control(),
        }
    }

    fn max_len(&self) -> usize {
        match self.lobby.edit {
            Edit::Channel => 3,
            Edit::Name => MAX_NAME,
            Edit::Search => 20,
        }
    }

    /// Mouse wheel notches this frame (positive = away from you), whatever units the platform
    /// reports (Windows and browsers use multiples of 100-120, others small fractions).
    pub fn wheel_notches() -> f32 {
        let y = mouse_wheel().1;
        let n = if y.abs() >= 20.0 { y / 120.0 } else { y };
        n.clamp(-3.0, 3.0)
    }

    fn sel(&self) -> (usize, usize) {
        (self.lobby.anchor.min(self.lobby.caret), self.lobby.anchor.max(self.lobby.caret))
    }

    fn put_caret(&mut self, pos: usize, extend: bool) {
        self.lobby.caret = pos;
        if !extend {
            self.lobby.anchor = pos;
        }
        self.lobby.blink = 0.0;
    }

    /// Replace the selection with `s` (what the field accepts, as far as it fits).
    fn insert_text(&mut self, s: &str) {
        let (lo, hi) = self.sel();
        let max = self.max_len();
        let mut v: Vec<char> = self.field().chars().collect();
        let (lo, hi) = (lo.min(v.len()), hi.min(v.len()));
        let ins: Vec<char> = s.chars().filter(|&c| self.accepts(c)).collect();
        let room = max.saturating_sub(v.len() - (hi - lo));
        let ins = &ins[..ins.len().min(room)];
        v.splice(lo..hi, ins.iter().copied());
        *self.field() = v.iter().collect();
        self.put_caret(lo + ins.len(), false);
    }

    fn delete_range(&mut self, lo: usize, hi: usize) {
        let mut v: Vec<char> = self.field().chars().collect();
        let (lo, hi) = (lo.min(v.len()), hi.min(v.len()));
        v.drain(lo..hi);
        *self.field() = v.iter().collect();
        self.put_caret(lo, false);
    }

    fn selected_text(&mut self) -> String {
        let (lo, hi) = self.sel();
        self.field().chars().skip(lo).take(hi - lo).collect()
    }

    /// True on the first frame a key is down and then repeatedly while it stays down.
    fn key_repeat(&mut self, k: KeyCode) -> bool {
        if !is_key_down(k) {
            self.lobby.rep.retain(|e| e.0 != k);
            return false;
        }
        let dt = self.dt;
        match self.lobby.rep.iter_mut().find(|e| e.0 == k) {
            None => {
                self.lobby.rep.push((k, 0.0));
                true
            }
            Some(e) => {
                e.1 += dt;
                if e.1 > 0.4 {
                    e.1 -= 0.035;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn word_left(v: &[char], mut p: usize) -> usize {
        while p > 0 && !v[p - 1].is_alphanumeric() {
            p -= 1;
        }
        while p > 0 && v[p - 1].is_alphanumeric() {
            p -= 1;
        }
        p
    }

    fn word_right(v: &[char], mut p: usize) -> usize {
        while p < v.len() && !v[p].is_alphanumeric() {
            p += 1;
        }
        while p < v.len() && v[p].is_alphanumeric() {
            p += 1;
        }
        p
    }

    /// Keyboard editing of the field: typing, caret keys with Shift / Ctrl, Home / End, Delete,
    /// select all, copy / cut / paste.
    fn type_text(&mut self) {
        self.lobby.blink += self.dt;
        if self.web_input_active() {
            return; // the browser's input is doing the editing
        }
        let ctrl = self.keys.down(KeyCode::LeftControl) || self.keys.down(KeyCode::RightControl) || self.keys.down(KeyCode::LeftSuper) || self.keys.down(KeyCode::RightSuper);
        let shift = self.keys.down(KeyCode::LeftShift) || self.keys.down(KeyCode::RightShift);
        while let Some(c) = get_char_pressed() {
            if !ctrl && !c.is_control() {
                self.insert_text(&c.to_string());
            }
        }
        let len = self.field().chars().count();
        if ctrl {
            if self.keys.pressed(KeyCode::A) {
                self.lobby.anchor = 0;
                self.lobby.caret = len;
                self.lobby.blink = 0.0;
            }
            if self.keys.pressed(KeyCode::C) || self.keys.pressed(KeyCode::X) {
                let t = self.selected_text();
                if !t.is_empty() {
                    macroquad::miniquad::window::clipboard_set(&t);
                    if self.keys.pressed(KeyCode::X) {
                        let (lo, hi) = self.sel();
                        self.delete_range(lo, hi);
                    }
                }
            }
            if self.keys.pressed(KeyCode::V) {
                if let Some(t) = macroquad::miniquad::window::clipboard_get() {
                    self.insert_text(&t);
                }
            }
        }
        let v: Vec<char> = self.field().chars().collect();
        let (lo, hi) = self.sel();
        let has_sel = lo != hi;
        if self.key_repeat(KeyCode::Left) {
            let p = if ctrl { Self::word_left(&v, self.lobby.caret) } else if has_sel && !shift { lo } else { self.lobby.caret.saturating_sub(1) };
            self.put_caret(p, shift);
        }
        if self.key_repeat(KeyCode::Right) {
            let p = if ctrl { Self::word_right(&v, self.lobby.caret) } else if has_sel && !shift { hi } else { (self.lobby.caret + 1).min(v.len()) };
            self.put_caret(p, shift);
        }
        if self.keys.pressed(KeyCode::Home) {
            self.put_caret(0, shift);
        }
        if self.keys.pressed(KeyCode::End) {
            self.put_caret(v.len(), shift);
        }
        if self.key_repeat(KeyCode::Backspace) {
            if has_sel {
                self.delete_range(lo, hi);
            } else if self.lobby.caret > 0 {
                let from = if ctrl { Self::word_left(&v, self.lobby.caret) } else { self.lobby.caret - 1 };
                self.delete_range(from, self.lobby.caret);
            }
        }
        if self.key_repeat(KeyCode::Delete) {
            if has_sel {
                self.delete_range(lo, hi);
            } else if self.lobby.caret < v.len() {
                let to = if ctrl { Self::word_right(&v, self.lobby.caret) } else { self.lobby.caret + 1 };
                self.delete_range(self.lobby.caret, to);
            }
        }
    }

    // ---- the field with the mouse

    /// The input plate of the dialog (or the picker's search box): (left, right, centre y, text
    /// size) in virtual units.
    fn field_rect(&self) -> (f32, f32, f32, f32) {
        let sh = self.g.sh;
        match self.lobby.edit {
            Edit::Search => (220.0, 580.0, Self::pick_geometry(sh).0 + 54.0, 21.0),
            _ => (250.0, 550.0, sh * 0.3 + 86.0, 27.0),
        }
    }

    fn caret_x(&self, v: &[char], i: usize) -> f32 {
        let size = self.field_rect().3;
        let all: String = v.iter().collect();
        let left = 400.0 - self.text.width(&self.g, &all, size) / 2.0;
        let pre: String = v.iter().take(i).collect();
        left + self.text.width(&self.g, &pre, size)
    }

    fn index_at(&mut self, x: f32) -> usize {
        let v: Vec<char> = self.field().chars().collect();
        (0..=v.len())
            .min_by(|&a, &b| (self.caret_x(&v, a) - x).abs().partial_cmp(&(self.caret_x(&v, b) - x).abs()).unwrap())
            .unwrap_or(0)
    }

    /// A press on the field: place the caret (Shift extends), double click selects the word, triple
    /// click everything; dragging selects.
    pub fn field_began(&mut self, x: f32, y: f32) {
        let (l, r, cy, _) = self.field_rect();
        if !(l..=r).contains(&x) || (y - cy).abs() > 22.0 {
            return;
        }
        let shift = self.keys.down(KeyCode::LeftShift) || self.keys.down(KeyCode::RightShift);
        let i = self.index_at(x);
        let (t, n) = self.lobby.last_click;
        let n = if self.clock - t < 0.4 { n + 1 } else { 1 };
        self.lobby.last_click = (self.clock, n);
        let v: Vec<char> = self.field().chars().collect();
        match n {
            1 => {
                self.put_caret(i, shift);
                self.lobby.field_drag = true;
            }
            2 => {
                let (a, b) = (Self::word_left(&v, (i + 1).min(v.len())), Self::word_right(&v, i.saturating_sub(1)));
                self.lobby.anchor = a.min(i);
                self.lobby.caret = b.max(i);
            }
            _ => {
                self.lobby.anchor = 0;
                self.lobby.caret = v.len();
            }
        }
        self.lobby.blink = 0.0;
    }

    pub fn field_moved(&mut self, x: f32) {
        if self.lobby.field_drag {
            let i = self.index_at(x);
            self.put_caret(i, true);
        }
    }

    // ------------------------------------------------------------------ the network each frame

    /// The search timer, its cancel button and the retry after a lost opponent.
    fn tick_search(&mut self) {
        let dt = self.dt;
        if let Some((_, t)) = self.lobby.toast.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                self.lobby.toast = None;
            }
        }
        let Some(s) = self.lobby.search.as_mut() else { return };
        s.time += dt;
        let (time, channel) = (s.time, s.channel);
        let mut restart = false;
        if let Some(r) = s.retry.as_mut() {
            *r -= dt;
            if *r <= 0.0 {
                s.retry = None;
                restart = true;
            }
        }
        if self.pads.get_clicked(sp::CANCEL_SEARCH) && time > CANCEL_AFTER {
            self.cancel_search(None);
            return;
        }
        if time > SEARCH_TIMEOUT {
            self.cancel_search(Some(("No opponent found.", "Please try again later.")));
            return;
        }
        if restart && self.net.is_none() {
            self.connect(channel);
        }
    }

    /// Runs every frame. During a live race `race_frame` owns the connection instead.
    pub fn pump_net(&mut self) {
        self.tick_search();
        let racing = self.session.as_ref().map_or(false, |s| s.online.is_some() && !s.is_over());
        if racing {
            return;
        }
        let dt = self.dt;
        let Some(conn) = self.net.as_mut() else { return };
        let events = conn.pump(dt);
        let closed = conn.m.is_closed();
        let reason = conn.m.close_reason().unwrap_or(CloseReason::Disconnected);
        let mut ended = None;
        for e in events {
            match e {
                NetEvent::StartRace => {
                    self.start_online_race();
                    return;
                }
                NetEvent::Closed(r) => ended = Some(r),
                _ => {}
            }
        }
        // a close that happened during a race was already reported to the session
        if ended.is_none() && closed {
            ended = Some(reason);
        }
        if let Some(r) = ended {
            self.net_closed(r);
        }
    }

    fn net_closed(&mut self, reason: CloseReason) {
        let error = self.net.as_ref().and_then(|c| c.error().map(|s| s.to_string()));
        if std::env::var_os("DRDRIVING_TRACE").is_some() {
            eprintln!("[{:7.1}s] network closed: {reason:?} {error:?}", self.clock);
        }
        self.net = None; // dropping the connection removes our room
        let Some(quick) = self.lobby.search.as_ref().map(|s| s.quick) else {
            self.online_notice = error.unwrap_or_else(|| reason.describe().to_string());
            return;
        };
        match (error, reason) {
            (Some(e), _) => {
                let hint = if e.contains("reach") || e.contains("lost") { "Check your connection and try again." } else { "Please try again later." };
                self.cancel_search(Some((&e, hint)))
            }
            (None, CloseReason::LocalOutdated) => self.cancel_search(Some(("Opponent has a newer version.", "Please update the game."))),
            // the opponent vanished before the race: keep looking (Online Match only)
            (None, _) if quick => {
                if let Some(s) = self.lobby.search.as_mut() {
                    s.retry = Some(RETRY_AFTER);
                }
            }
            (None, CloseReason::PeerOutdated) => self.cancel_search(Some(("Match canceled:", "opponent has an old version."))),
            (None, _) => self.cancel_search(Some(("Your opponent left.", ""))),
        }
    }

    /// The course is agreed: build the world for it and start the countdown.
    fn start_online_race(&mut self) {
        let Some(conn) = &self.net else { return };
        let (Some(course), Some(peer)) = (conn.m.course.clone(), conn.m.peer.clone()) else { return };
        let friendly = conn.m.friendly;
        if self.session.is_some() {
            self.leave_race(); // coming from the previous race's result screen
        }
        self.fix_car();
        self.lobby.search = None;
        self.online_notice.clear();
        self.forfeit_armed = 0.0;
        self.lobby.rating_before = self.save.rating;
        self.lobby.opp_rating_before = peer.rating;
        self.world.setup_online(&course, peer.car as usize, &peer.name, peer.rating, peer.country);
        self.start_race_with(Some((friendly, peer.rating)));
    }

    // ------------------------------------------------------------------ result screens

    pub fn is_online_race(&self) -> bool {
        self.session.as_ref().map_or(false, |s| s.online.is_some())
    }

    /// Result-screen buttons of an online race: RACE asks for a rematch (the next race starts
    /// when both sides did), BACK leaves. Returns false when this is not an online race.
    pub fn online_result_buttons(&mut self) -> bool {
        if !self.is_online_race() {
            return false;
        }
        if self.pads.get_clicked(sp::RACE) {
            self.fix_car();
            let (car, rating) = (self.world.car_mode as i32, self.save.rating);
            match self.net.as_mut() {
                Some(c) if !c.m.is_closed() => {
                    c.m.request_rematch(car, rating);
                }
                _ => {
                    // nobody to race any more
                    self.leave_race();
                    self.leave_online();
                    self.enter_mode(8);
                }
            }
        } else if self.pads.get_clicked(sp::BACK_TO_MENU) {
            self.leave_race();
            self.leave_online();
            self.enter_mode(8);
        }
        true
    }

    // ------------------------------------------------------------------ rendering

    /// `renderSandWatch`: while searching, an hourglass spins on the cancel button (after 0.5 s).
    /// That is all the original shows; the menu stays usable.
    pub fn render_search_overlay(&mut self) {
        let Some(s) = &self.lobby.search else { return };
        if s.time < 0.5 {
            return;
        }
        let (time, pad) = (s.time, self.pads.p[sp::CANCEL_SEARCH].clone());
        self.g.f(&self.a.menu, S::MENU[62], pad.rx, pad.ry, time * 180.0);
    }

    /// The input dialog in the game's own style (like "Are you sure to exit?"): the tab stays
    /// visible under a dark veil, a title, a plate with the typed text and the OK / cancel icons.
    pub fn render_lobby(&mut self) {
        self.g.anchor = true;
        self.g.band = (tab_y(self.g.sh) - 88.0, tab_y(self.g.sh) + 88.0);
        self.g.lthr = 360.0;
        match self.lobby.return_mode {
            11 => self.render_tab_option(),
            _ => self.render_tab_profile(),
        }
        self.render_screen_pad(true);
        self.g.anchor = false; // the dialog is centred, nothing sticks to the screen edges
        let (sh, cw) = (self.g.sh, 400.0);
        let blink = ((self.lobby.blink * 2.0) as i32) % 2 == 0;
        let (title, value, _, _) = self.dialog_text();
        let hint = match self.lobby.edit {
            Edit::Channel => "Use the same number as your friend (100~999)",
            Edit::Name => "Shown to your opponent (up to 16 characters)",
            Edit::Search => "",
        };
        let a = (self.mode_time * 5.0).min(0.85);
        let g = &mut self.g;
        g.set_tint(Color::new(a, a, a, a));
        g.e(&self.a.ui, S::BLACK, cw, sh / 2.0, g.vw / 2.0, sh / 2.0);
        g.clear_tint();
        self.text.d(g, title, cw, sh * 0.3, 0, 27.0);
        g.set_tint(Color::new(0.75, 0.75, 0.75, 1.0));
        self.text.d(g, hint, cw, sh * 0.3 + 32.0, 0, 18.0);
        g.clear_tint();
        self.draw_field(&value, "", fy_plate(sh), blink);
        self.render_screen_pad(false);
    }

    // ------------------------------------------------------------------ country picker (mode 15)

    /// A text field in the game's style: dark plate, green text, selection highlight and caret.
    /// `_cy` is unused for geometry (the plate position comes from [`Self::field_rect`]).
    fn draw_field(&mut self, value: &str, placeholder: &str, _cy: f32, blink: bool) {
        let (l, r, cy, size) = self.field_rect();
        let v: Vec<char> = value.chars().collect();
        let n = v.len();
        let (lo, hi) = (self.lobby.anchor.min(self.lobby.caret).min(n), self.lobby.anchor.max(self.lobby.caret).min(n));
        let (x_lo, x_hi, x_caret) = (self.caret_x(&v, lo), self.caret_x(&v, hi), self.caret_x(&v, self.lobby.caret.min(n)));
        let half_h = size * 0.75 + 5.0;
        let g = &mut self.g;
        g.set_tint(Color::new(1.0, 1.0, 1.0, 0.6));
        g.e(&self.a.ui, S::BLACK, (l + r) / 2.0, cy, (r - l) / 2.0, half_h);
        g.clear_tint();
        if lo != hi {
            g.set_tint(Color::new(0.57, 0.816, 0.098, 0.55));
            g.e(&self.a.ui, S::WHITE_DOT, (x_lo + x_hi) / 2.0, cy, (x_hi - x_lo) / 2.0, size * 0.55);
            g.clear_tint();
        }
        if value.is_empty() {
            g.set_tint(Color::new(0.6, 0.6, 0.6, 1.0));
            self.text.d(g, placeholder, 400.0, cy, 0, size);
        } else {
            g.set_tint(COLOR_GREEN);
            self.text.d(g, value, 400.0, cy, 0, size);
        }
        if lo == hi && blink {
            g.set_tint(COLOR_GREEN);
            g.e(&self.a.ui, S::WHITE_DOT, x_caret, cy, 1.0, size * 0.55);
        }
        g.clear_tint();
    }

    /// Indices into `COUNTRY_NAMES` that match the typed search.
    fn pick_rows(&self) -> Vec<usize> {
        let f = self.lobby.pick_filter.to_lowercase();
        (0..COUNTRY_NAMES.len()).filter(|&i| f.is_empty() || COUNTRY_NAMES[i].1.to_lowercase().contains(&f) || COUNTRY_NAMES[i].0.to_lowercase() == f).collect()
    }

    /// Card edges and the list area: (card top, card bottom, list top, list bottom).
    fn pick_geometry(sh: f32) -> (f32, f32, f32, f32) {
        let (top, bottom) = (sh * 0.06, sh * 0.94);
        (top, bottom, top + 78.0, bottom - 46.0)
    }

    pub fn pick_cancel_pos(sh: f32) -> f32 {
        sh * 0.94 - 24.0
    }

    fn pick_max_scroll(&self, rows: usize) -> f32 {
        let (_, _, y0, y1) = Self::pick_geometry(self.g.sh);
        (rows as f32 * PICK_ROW - (y1 - y0)).max(0.0)
    }

    pub fn pick_began(&mut self, y: f32) {
        self.lobby.pick_drag = Some((y, self.lobby.pick_scroll, false));
    }

    pub fn pick_moved(&mut self, y: f32) {
        let n = self.pick_rows().len();
        let max = self.pick_max_scroll(n);
        if let Some((y0, s0, moved)) = self.lobby.pick_drag.as_mut() {
            if (y - *y0).abs() > 8.0 {
                *moved = true;
            }
            if *moved {
                self.lobby.pick_scroll = (*s0 + (*y0 - y)).clamp(0.0, max);
            }
        }
    }

    /// A tap (a press and release that did not scroll) picks the row under the finger.
    pub fn pick_ended(&mut self, x: f32, y: f32) {
        let Some((_, _, moved)) = self.lobby.pick_drag.take() else { return };
        if moved || !(120.0..=680.0).contains(&x) {
            return;
        }
        let (_, _, y0, y1) = Self::pick_geometry(self.g.sh);
        if y < y0 || y > y1 {
            return;
        }
        let rows = self.pick_rows();
        let i = ((y - y0 + self.lobby.pick_scroll) / PICK_ROW) as usize;
        if let Some(&r) = rows.get(i) {
            self.choose_country(COUNTRY_NAMES[r].0);
        }
    }

    fn choose_country(&mut self, code: &str) {
        text_input::close();
        self.lobby.country = code.to_string();
        persist::pref_set("country", code);
        let m = self.lobby.return_mode;
        self.enter_mode(m);
    }

    pub fn process_picker(&mut self) {
        // the search box is a normal text field (typing, selection, shortcuts, mouse)
        let before = self.lobby.pick_filter.clone();
        let sub = self.sync_web_input();
        self.type_text();
        let changed = self.lobby.pick_filter != before;
        let rows = self.pick_rows();
        if changed {
            self.lobby.pick_scroll = 0.0;
        }
        let max = self.pick_max_scroll(rows.len());
        let mut d = -Self::wheel_notches() * 2.0 * PICK_ROW;
        for (key, step) in [(KeyCode::Down, PICK_ROW), (KeyCode::Up, -PICK_ROW), (KeyCode::PageDown, 6.0 * PICK_ROW), (KeyCode::PageUp, -6.0 * PICK_ROW)] {
            if self.keys.pressed(key) {
                d += step;
            }
        }
        self.lobby.pick_scroll = (self.lobby.pick_scroll + d).clamp(0.0, max);
        if matches!(sub, Submit::Escape) {
            text_input::close();
            let m = self.lobby.return_mode;
            self.enter_mode(m);
            return;
        }
        if self.keys.pressed(KeyCode::Enter) || matches!(sub, Submit::Enter) {
            if let Some(&r) = rows.first() {
                if !self.lobby.pick_filter.is_empty() {
                    self.choose_country(COUNTRY_NAMES[r].0);
                    return;
                }
            }
        }
        if self.pads.get_clicked(sp::CH_PICK_CANCEL) {
            text_input::close();
            let m = self.lobby.return_mode;
            self.enter_mode(m);
        }
    }

    pub fn render_picker(&mut self) {
        self.g.anchor = true;
        self.g.band = (tab_y(self.g.sh) - 88.0, tab_y(self.g.sh) + 88.0);
        self.g.lthr = 360.0;
        self.render_tab_option();
        self.render_screen_pad(true);
        self.g.anchor = false;
        let (sh, cw) = (self.g.sh, 400.0);
        let (top, bottom, y0, y1) = Self::pick_geometry(sh);
        let rows = self.pick_rows();
        let scroll = self.lobby.pick_scroll;
        let filter = self.lobby.pick_filter.clone();
        let cur = self.lobby.country.clone();
        let max = self.pick_max_scroll(rows.len());
        let a = (self.mode_time * 5.0).min(0.85);
        let g = &mut self.g;
        g.set_tint(Color::new(a, a, a, a));
        g.e(&self.a.ui, S::BLACK, cw, sh / 2.0, g.vw / 2.0, sh / 2.0);
        g.clear_tint();
        self.text.e(g, "Select your country", cw, top + 20.0, 0, 27.0, 280.0);
        let blink = ((self.lobby.blink * 2.0) as i32) % 2 == 0;
        let value = filter.clone();
        self.draw_field(&value, "Type to search", 0.0, blink);
        let g = &mut self.g;
        for (n, &r) in rows.iter().enumerate() {
            let y = y0 + n as f32 * PICK_ROW - scroll + PICK_ROW / 2.0;
            if y - PICK_ROW / 2.0 < y0 - 1.0 || y + PICK_ROW / 2.0 > y1 + 1.0 {
                continue;
            }
            let (code, name) = COUNTRY_NAMES[r];
            let selected = code == cur;
            if selected {
                g.set_tint(Color::new(0.57, 0.816, 0.098, 0.3));
                g.e(&self.a.ui, S::WHITE_DOT, cw, y, 270.0, PICK_ROW / 2.0 - 1.0);
                g.clear_tint();
            }
            g.e(&self.a.world, S::flag(net::country_index(code)), 165.0, y, 0.8, 0.8);
            if selected {
                g.set_tint(COLOR_GREEN);
            }
            self.text.e(g, name, 200.0, y, 1, 21.0, 230.0);
            g.set_tint(Color::new(0.65, 0.65, 0.65, 1.0));
            self.text.e(g, code, 665.0, y, 2, 17.0, 40.0);
            g.clear_tint();
        }
        if rows.is_empty() {
            self.text.d(g, "No country found", cw, (y0 + y1) / 2.0, 0, 21.0);
        } else if scroll < max {
            g.set_tint(COLOR_GREEN);
            self.text.d(g, "v", 665.0, y1 + 10.0, 0, 16.0);
            g.clear_tint();
        }
        self.render_screen_pad(false);
    }

    /// The mouse cursor: a pointing hand over anything clickable, the text I-beam over the input
    /// field of a dialog, the arrow elsewhere. (Touch screens have no cursor.)
    pub fn update_cursor(&mut self) {
        use macroquad::miniquad::{window::set_mouse_cursor, CursorIcon};
        let mut want = 0u8;
        if !self.touch_seen && !matches!(self.mode, 0 | 2 | 3) {
            let (mx, my) = mouse_position();
            let (x, y) = self.g.to_virtual(mx, my);
            let sh = self.g.sh;
            let (fl, fr, fcy, _) = self.field_rect();
            let field = matches!(self.mode, 14 | 15) && (fl..=fr).contains(&x) && (y - fcy).abs() <= 22.0;
            let (_, _, y0, y1) = Self::pick_geometry(sh);
            let list = self.mode == 15 && (120.0..=680.0).contains(&x) && y >= y0 && y <= y1;
            let pad = self.pads.p.iter().any(|p| p.active && p.kind == crate::pad::PAD_CLICK && (x - p.x).abs() <= p.szx && (y - p.y).abs() <= p.szy);
            want = if field {
                2
            } else if pad || list {
                1
            } else {
                0
            };
        }
        if want != self.cursor {
            self.cursor = want;
            set_mouse_cursor(match want {
                1 => CursorIcon::Pointer,
                2 => CursorIcon::Text,
                _ => CursorIcon::Default,
            });
        }
    }

    /// A message at the bottom of the screen, like Android's `Toast`.
    pub fn render_toast(&mut self) {
        let Some((text, t)) = self.lobby.toast.clone() else { return };
        let (sh, a) = (self.g.sh, (t * 2.0).clamp(0.0, 1.0));
        let g = &mut self.g;
        g.set_tint(Color::new(0.25, 0.25, 0.25, 0.9 * a));
        g.e(&self.a.ui, S::WHITE_DOT, 400.0, sh - 70.0, 330.0, 18.0);
        g.set_tint(Color::new(1.0, 1.0, 1.0, a));
        self.text.d(g, &text, 400.0, sh - 70.0, 0, 17.0);
        g.clear_tint();
    }

    /// `renderMultiEnding`: both players' flag, name, class badge and rating (the ratings count
    /// from the old value to the new one between 1.5 s and 2.5 s), the car's condition, and the
    /// rematch / "opponent left" message.
    pub fn render_multi_result(&mut self, win: bool) {
        let (sh, cw, t) = (self.g.sh, 400.0, self.mode_time);
        {
            let g = &mut self.g;
            g.e(&self.a.ui, S::UI[19], cw, sh - 35.0, g.vw / 2.0, 35.0);
        }
        self.render_gold(0, 0);
        let Some(s) = &self.session else { return };
        let opp_now = s.result.as_ref().and_then(|r| r.multi.as_ref()).map_or(s.world.opponent.rating, |m| m.opponent_rating);
        let car = s.world.car_mode;
        let (opp, country_opp) = (s.world.opponent.name.clone(), s.world.opponent.country);
        let country_me = net::country_index(&self.lobby.country);
        let (me_last, opp_last) = (self.lobby.rating_before, self.lobby.opp_rating_before);
        let me_now = self.save.rating;
        let lerp = |a: i32, b: i32| if t < 1.5 { a } else if t < 2.5 { a + ((t - 1.5) * (b - a) as f32) as i32 } else { b };
        let (me_r, opp_r) = (lerp(me_last, me_now), lerp(opp_last, opp_now));
        let damage = self.save.car_damage[car.min(31)];
        let cost = economy::repair_cost(car, damage);
        let msg = match self.net.as_ref() {
            None => Some("Opponent has left the game."),
            Some(c) if c.m.is_closed() => Some("Opponent has left the game."),
            Some(c) if c.m.local_rematch => Some("Requesting for rematch."),
            Some(c) if c.m.peer_rematch => Some("Opponent wants rematch."),
            _ => None,
        };
        let my_name = if self.lobby.name.chars().count() > 1 { self.lobby.name.clone() } else { "Me".to_string() };
        let opp_name = if opp.chars().count() > 1 { opp } else { "Opponent".to_string() };
        let f7 = 0.38 * sh;
        let (g, ui, m) = (&mut self.g, &self.a.ui, &self.a.menu);
        if t > 1.0 {
            self.bf.c(g, ui, if win { "YOU WON!" } else { "YOU LOST!" }, cw, sh - 35.0);
            self.bf.c(g, ui, &format!("{}", opp_r + 10000), 162.0, f7 + 24.0);
            self.bf.c(g, ui, &format!("{}", me_r + 10000), 662.0, f7 + 24.0);
            if t > 2.0 {
                let cond = 10000 - damage;
                let y = (sh * 0.7) as i32 as f32;
                self.bf.b(g, ui, "Condition", 301.0, y - 10.0, 0.5);
                self.bf.f(g, ui, &format!("{}%", cond / 100), 480.0, y - 10.0, 0.5);
                if cond < 9000 {
                    self.bf.b(g, ui, "Repair Cost", 301.0, y + 10.0, 0.5);
                    self.bf.f(g, ui, &format!("{}Coin", cost), 480.0, y + 10.0, 0.5);
                } else {
                    self.bf.b(g, ui, "No need to repair", 301.0, y + 10.0, 0.5);
                }
                g.d(ui, S::UI[9], 299.0, y, 91.0, 3.0, 1);
                g.d(ui, S::UI[if cond < 9000 { 11 } else { 10 }], 300.0, y, cond as f32 * 0.009, 2.0, 1);
                g.h(ui, S::UI[7], 496.0, y);
            }
            g.e(m, S::CLASS[class_for(opp_r)], 104.0, f7 + 24.0, 0.25, 0.25);
            g.e(m, S::CLASS[class_for(me_r)], 604.0, f7 + 24.0, 0.25, 0.25);
            g.h(&self.a.world, S::flag(country_opp), 150.0, f7 - 34.0);
            g.h(&self.a.world, S::flag(country_me), 650.0, f7 - 34.0);
            self.text.e(g, &opp_name, 150.0, f7 - 2.0, 0, 24.0, 140.0);
            self.text.e(g, &my_name, 650.0, f7 - 2.0, 0, 24.0, 140.0);
        }
        if let (true, Some(msg)) = (t > 2.0 && t.fract() < 0.7, msg) {
            self.text.d(g, msg, cw, 0.2 * sh, 0, 27.0);
        }
        self.render_screen_pad(false);
    }
}

fn fy_plate(sh: f32) -> f32 {
    sh * 0.3 + 86.0
}

/// Height of a row of the country picker.
pub const PICK_ROW: f32 = 42.0;

/// The browser's invisible text input (see `web/index.html`): it is only there to bring up the
/// phone keyboard and to give the game a browser's selection / shortcut handling; the game draws
/// the field itself. Elsewhere these do nothing and the game edits the text itself.
mod text_input {
    pub struct Snap {
        pub value: String,
        pub anchor: usize,
        pub caret: usize,
        /// 0 = open, 1 = Enter pressed, 2 = Escape pressed
        pub state: u32,
    }

    #[cfg(target_arch = "wasm32")]
    extern "C" {
        fn dr_text_open(value: *const u8, value_len: u32, max: u32, numeric: u32);
        fn dr_text_close();
        fn dr_text_pos(l: f32, t: f32, w: f32, h: f32);
        fn dr_text_state() -> u32;
        fn dr_text_active() -> u32;
        fn dr_text_value(dst: *mut u8, cap: u32) -> i32;
        fn dr_text_sel(which: u32) -> i32;
        fn dr_text_set(value: *const u8, value_len: u32, start: u32, end: u32);
    }

    #[cfg(target_arch = "wasm32")]
    pub fn open(value: &str, max: u32, numeric: bool) {
        unsafe { dr_text_open(value.as_ptr(), value.len() as u32, max, numeric as u32) };
    }
    #[cfg(target_arch = "wasm32")]
    pub fn close() {
        unsafe { dr_text_close() };
    }
    /// Place the input over the game's field (window px) and read its text and selection.
    #[cfg(target_arch = "wasm32")]
    pub fn snapshot(l: f32, t: f32, w: f32, h: f32) -> Option<Snap> {
        unsafe {
            if dr_text_active() == 0 {
                return None;
            }
            dr_text_pos(l, t, w, h);
            let mut buf = vec![0u8; 256];
            let n = dr_text_value(buf.as_mut_ptr(), buf.len() as u32).max(0) as usize;
            buf.truncate(n);
            let (start, end, back) = (dr_text_sel(0).max(0) as usize, dr_text_sel(1).max(0) as usize, dr_text_sel(2) == 1);
            let (anchor, caret) = if back { (end, start) } else { (start, end) };
            Some(Snap { value: String::from_utf8_lossy(&buf).into_owned(), anchor, caret, state: dr_text_state() })
        }
    }
    #[cfg(target_arch = "wasm32")]
    pub fn set(value: &str, start: usize, end: usize) {
        unsafe { dr_text_set(value.as_ptr(), value.len() as u32, start as u32, end as u32) };
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(_: &str, _: u32, _: bool) {}
    #[cfg(not(target_arch = "wasm32"))]
    pub fn close() {}
    #[cfg(not(target_arch = "wasm32"))]
    pub fn snapshot(_: f32, _: f32, _: f32, _: f32) -> Option<Snap> {
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set(_: &str, _: usize, _: usize) {}
}
