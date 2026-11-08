//! Save-game model (`drdriving.o`) with a JSON format compatible with the original app,
//! so a save exported from the Android game (SharedPreferences key `game`, or Firestore
//! `Users/<id>.game`) loads here and vice-versa.
//!
//! Differences from the Java class:
//! * The Java app keeps `iCoin/iGold/iWin/iLose/iRating` offset by a random per-process
//!   number `rs` purely to hinder memory scanners. That is a runtime-only trick that never
//!   reaches the JSON, so it is not ported: values here are plain.
//! * The `*C` shadow fields (`iCoinC`, `iGoldC`) existed only for that trick and are dropped.

use serde_json::{json, Map, Value};

pub const SERIAL_VERSION: &str = "1.0";
pub const MAX_SAVE_CAR: usize = 32;
pub const MAX_ACHIEVEMENT: usize = 32;
pub const MISSION_TYPES: usize = 9;
pub const MAX_ABILITY: usize = 6;

#[derive(Clone, Debug, PartialEq)]
pub struct SaveGame {
    pub not_first: bool,
    pub music_disabled: bool,
    pub sound_disabled: bool,
    pub left_wheel: bool,
    pub right_driver: bool,
    pub accel_disabled: bool,
    pub show_yard: bool,
    /// GUI size: 0 = large (phone-like), 1 = medium, 2 = small (spacious, the default).
    pub gui_size: i32,
    pub rate_rewarded: bool,

    pub driver_level: i32,
    pub driver_exp: i32,
    pub wheel_sensitivity: i32,
    pub coin: i32,
    pub gold: i32,
    pub bought_gold: i32,

    pub has_car: [bool; MAX_SAVE_CAR],
    pub car_damage: [i32; MAX_SAVE_CAR],
    pub car_rent: [i32; MAX_SAVE_CAR],
    /// Per car, per ability upgrade level (0..=15, stored as a 4-bit nibble).
    pub upgrade: [[i32; MAX_ABILITY]; MAX_SAVE_CAR],

    pub record: [i32; MISSION_TYPES],
    pub mission_clear: [i32; MISSION_TYPES],
    pub achievement: [bool; MAX_ACHIEVEMENT],

    pub travel_distance: i32,
    pub drift_distance: i32,
    pub accident_no: i32,
    pub missions_clear_count: i32,
    pub travel_second: i32,
    pub max_speed: i32,
    pub perfect_turn: i32,
    pub sub_mission: i32,
    pub cs: i32,

    pub win: i32,
    pub lose: i32,
    pub rating: i32,
    /// Number of "multiplayer bonus" gold pieces already awarded (cap 10000).
    pub mb: i32,
}

#[derive(Debug)]
pub struct LoadError(pub String);
impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "save load error: {}", self.0)
    }
}
impl std::error::Error for LoadError {}

impl Default for SaveGame {
    /// `zero()` without the country heuristics (see [`SaveGame::zero_for_country`]).
    fn default() -> Self {
        let mut has_car = [false; MAX_SAVE_CAR];
        has_car[0] = true;
        Self {
            not_first: false,
            music_disabled: false,
            sound_disabled: false,
            left_wheel: false,
            right_driver: false,
            accel_disabled: true,
            show_yard: false,
            gui_size: 2,
            rate_rewarded: false,
            driver_level: 0,
            driver_exp: 0,
            wheel_sensitivity: 0,
            coin: 0,
            gold: 0,
            bought_gold: 0,
            has_car,
            car_damage: [0; MAX_SAVE_CAR],
            car_rent: [0; MAX_SAVE_CAR],
            upgrade: [[0; MAX_ABILITY]; MAX_SAVE_CAR],
            record: [0; MISSION_TYPES],
            mission_clear: [0; MISSION_TYPES],
            achievement: [false; MAX_ACHIEVEMENT],
            travel_distance: 0,
            drift_distance: 0,
            accident_no: 0,
            missions_clear_count: 0,
            travel_second: 0,
            max_speed: 0,
            perfect_turn: 0,
            sub_mission: 0,
            cs: 0,
            win: 0,
            lose: 0,
            rating: 0,
            mb: 0,
        }
    }
}

/// ISO-3166 alpha-2 codes (lower-case) that the original treats as right-hand-drive.
#[allow(dead_code)]
const RIGHT_DRIVER: &[&str] = &[
    "ai", "ag", "au", "bs", "bd", "bb", "bm", "bt", "bw", "bn", "ky", "cx", "ck", "cy", "dm", "fk", "fj", "gd",
    "gy", "hk", "in", "id", "ie", "jm", "jp", "je", "ke", "ki", "ls", "mo", "mw", "my", "mv", "mt", "mu", "ms",
    "mz", "np", "nz", "pk", "pg", "sg", "sb", "za", "lk", "th", "tt", "ug", "gb", "zm", "zw",
];

impl SaveGame {
    /// `zero()`: fresh save with locale-dependent defaults. This port always starts with the
    /// steering wheel on the left (the original picked the right side for the countries in
    /// [`RIGHT_DRIVER`]; Settings still lets the player switch). GB and US show imperial units.
    pub fn zero_for_country(country: &str) -> Self {
        let mut s = Self::default();
        let c = country.to_ascii_lowercase();
        if c == "kr" {
            return s;
        }
        if c == "gb" || c == "us" {
            s.show_yard = true;
        }
        s
    }

    pub fn rating_value(&self) -> i32 {
        self.rating
    }

    // ---------------------------------------------------------------- JSON

    pub fn to_json(&self) -> String {
        let mut m = Map::new();
        m.insert("version".into(), json!(SERIAL_VERSION));
        m.insert("bNotFirst".into(), json!(self.not_first));
        m.insert("musicDisabled".into(), json!(self.music_disabled));
        m.insert("soundDisabled".into(), json!(self.sound_disabled));
        m.insert("bLeftWheel".into(), json!(self.left_wheel));
        m.insert("bAccelDisabled".into(), json!(self.accel_disabled));
        m.insert("bRightDriver".into(), json!(self.right_driver));
        m.insert("bShowYard".into(), json!(self.show_yard));
        m.insert("iGuiSize".into(), json!(self.gui_size.clamp(0, 2)));
        m.insert("bRateRewarded".into(), json!(self.rate_rewarded));
        m.insert("iDriverLevel".into(), json!(self.driver_level));
        m.insert("iDriverExp".into(), json!(self.driver_exp));
        m.insert("iWheelSensitivity".into(), json!(self.wheel_sensitivity.clamp(-4, 4)));
        m.insert("iCoin".into(), json!(self.coin));
        m.insert("iGold".into(), json!(self.gold));
        m.insert("iBoughtGold".into(), json!(self.bought_gold));
        for i in 0..MAX_SAVE_CAR {
            // the Java forces car 0 to be owned on every save
            let has = if i == 0 { true } else { self.has_car[i] };
            m.insert(format!("bHasCar{i}"), json!(has));
            m.insert(format!("iCarDamage{i}"), json!(self.car_damage[i]));
            let mut packed = 0i32;
            for a in 0..MAX_ABILITY {
                packed += (self.upgrade[i][a] & 15) << (a * 4);
            }
            m.insert(format!("iUpgrade{i}"), json!(packed));
            m.insert(format!("iCarRent{i}"), json!(self.car_rent[i]));
        }
        for i in 0..MISSION_TYPES {
            m.insert(format!("iRecord{i}"), json!(self.record[i]));
            m.insert(format!("iMissionClear{i}"), json!(self.mission_clear[i]));
        }
        m.insert("iTravelDistance".into(), json!(self.travel_distance));
        m.insert("iDriftDistance".into(), json!(self.drift_distance));
        m.insert("iAccidentNo".into(), json!(self.accident_no));
        m.insert("iMissionsClearCount".into(), json!(self.missions_clear_count));
        m.insert("iTravelSecond".into(), json!(self.travel_second));
        m.insert("iMaxSpeed".into(), json!(self.max_speed));
        m.insert("iPerfectTurn".into(), json!(self.perfect_turn));
        m.insert("iSubMission".into(), json!(self.sub_mission));
        for i in 0..MAX_ACHIEVEMENT {
            m.insert(format!("bAchievement{i}"), json!(self.achievement[i]));
        }
        m.insert("iCS".into(), json!(self.cs));
        m.insert("iWin".into(), json!(self.win));
        m.insert("iLose".into(), json!(self.lose));
        m.insert("iRating".into(), json!(self.rating));
        m.insert("iMB".into(), json!(self.mb));
        Value::Object(m).to_string()
    }

    /// Parses a save. Mirrors the Java behaviour precisely: starts from a fresh `zero()`
    /// state, an empty/blank string yields defaults, a wrong `version` is an error, and a
    /// **missing key aborts the load but keeps everything read so far** (the Java swallows the
    /// exception). Use [`SaveGame::from_json_strict`] if you want that to be an error.
    pub fn from_json_lenient(s: &str) -> SaveGame {
        let mut save = SaveGame::default();
        let _ = save.load_into(s);
        save
    }

    pub fn from_json_strict(s: &str) -> Result<SaveGame, LoadError> {
        let mut save = SaveGame::default();
        save.load_into(s)?;
        Ok(save)
    }

    fn load_into(&mut self, s: &str) -> Result<(), LoadError> {
        if s.trim().is_empty() {
            return Ok(());
        }
        let v: Value = serde_json::from_str(s).map_err(|e| LoadError(e.to_string()))?;
        let o = v.as_object().ok_or_else(|| LoadError("not an object".into()))?;
        let geti = |k: &str| -> Result<i32, LoadError> {
            o.get(k)
                .and_then(Value::as_i64)
                .map(|n| n as i32)
                .ok_or_else(|| LoadError(format!("missing/invalid int `{k}`")))
        };
        let getb = |k: &str| -> Result<bool, LoadError> {
            o.get(k).and_then(Value::as_bool).ok_or_else(|| LoadError(format!("missing/invalid bool `{k}`")))
        };

        let version = o.get("version").and_then(Value::as_str).ok_or_else(|| LoadError("no version".into()))?;
        if version != SERIAL_VERSION {
            return Err(LoadError(format!("Unexpected loot format {version}")));
        }

        self.not_first = getb("bNotFirst")?;
        self.music_disabled = getb("musicDisabled")?;
        self.sound_disabled = getb("soundDisabled")?;
        self.left_wheel = getb("bLeftWheel")?;
        self.right_driver = getb("bRightDriver")?;
        self.show_yard = getb("bShowYard")?;
        self.rate_rewarded = getb("bRateRewarded")?;
        self.gui_size = o.get("iGuiSize").and_then(Value::as_i64).unwrap_or(2).clamp(0, 2) as i32;
        self.driver_level = geti("iDriverLevel")?;
        self.driver_exp = geti("iDriverExp")?;
        self.wheel_sensitivity = geti("iWheelSensitivity")?;
        self.coin = geti("iCoin")?;
        self.gold = geti("iGold")?;
        self.bought_gold = geti("iBoughtGold")?;
        self.travel_distance = geti("iTravelDistance")?;
        self.drift_distance = geti("iDriftDistance")?;
        self.accident_no = geti("iAccidentNo")?;
        self.missions_clear_count = geti("iMissionsClearCount")?;
        self.travel_second = geti("iTravelSecond")?;
        self.max_speed = geti("iMaxSpeed")?;
        self.perfect_turn = geti("iPerfectTurn")?;
        self.sub_mission = geti("iSubMission")?;
        self.cs = geti("iCS")?;

        for i in 0..6 {
            self.record[i] = geti(&format!("iRecord{i}"))?;
            self.mission_clear[i] = geti(&format!("iMissionClear{i}"))?;
        }
        // record clamps applied after the first six are read
        for (i, cap) in [(0, 10000), (1, 10000), (2, 10000), (3, 12000), (4, 12000), (5, 10000)] {
            self.record[i] = self.record[i].min(cap);
        }
        for i in 0..MAX_ACHIEVEMENT {
            self.achievement[i] = getb(&format!("bAchievement{i}"))?;
        }
        for i in 0..MAX_SAVE_CAR {
            self.has_car[i] = getb(&format!("bHasCar{i}"))?;
            self.car_damage[i] = geti(&format!("iCarDamage{i}"))?;
            let packed = geti(&format!("iUpgrade{i}"))?;
            for a in 0..MAX_ABILITY {
                let sh = a * 4;
                self.upgrade[i][a] = ((15i32 << sh) & packed) >> sh;
            }
        }
        self.has_car[0] = true;
        self.accel_disabled = getb("bAccelDisabled")?;
        self.win = geti("iWin")?;
        self.lose = geti("iLose")?;
        self.rating = geti("iRating")?;
        for i in 0..MAX_SAVE_CAR {
            let mut r = geti(&format!("iCarRent{i}"))?;
            if r < 0 || r > 30000 {
                r = 0;
            }
            self.car_rent[i] = r;
        }
        self.mb = geti("iMB")?;
        self.record[6] = self.record[6].min(12000);
        for i in 6..MISSION_TYPES {
            self.record[i] = geti(&format!("iRecord{i}"))?;
            self.mission_clear[i] = geti(&format!("iMissionClear{i}"))?;
        }
        Ok(())
    }
}
