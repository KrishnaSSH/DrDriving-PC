//! Progression, shop, rewards, rating and achievements — the arithmetic scattered through
//! `drdriving.d` (`buyCar`, `rentCar`, `iGetUpgradeCost`, `onSuccessMission`,
//! `onMissionFinished`, `onMultiFinished`, …) collected into pure functions.

use crate::cars::{car, ABILITY_DURABILITY, PLAYER_CARS, PURCHASABLE_CARS};
use crate::math::expected_score;
use crate::mission::Mission;
use crate::save::SaveGame;

pub const RENT_DISTANCE: i32 = 30000;
pub const MAX_DAMAGE: i32 = 9000;
/// Highest upgrade level per ability: the Java requires the current level to be <= 9.
pub const MAX_UPGRADE: i32 = 10;

// ---------------------------------------------------------------- levels

/// `iGetBaseExpForLevel` — total XP needed to *reach* `level`.
pub fn base_exp_for_level(level: i32) -> i32 {
    let (mut total, mut step) = (0i32, 4096i32);
    for _ in 0..level {
        total += step;
        step = (step * 5) / 4;
    }
    total
}

/// `iGetLevelForExp` — level 0..=9 for an XP value (quirk preserved: 10 steps, caps at 9).
pub fn level_for_exp(exp: f32) -> i32 {
    let mut threshold = 4096i32;
    let mut step = 4096i32;
    for level in 0..10 {
        if exp < threshold as f32 {
            return level;
        }
        step = (step * 5) / 4;
        threshold += step;
    }
    9
}

/// `iGetClass` — rating bracket 0..=6 shown as a class badge.
pub fn class_for_rating(rating: i32) -> i32 {
    match rating {
        r if r < 0 => 0,
        r if r < 1000 => 1,
        r if r < 2000 => 2,
        r if r < 3000 => 3,
        r if r < 4000 => 4,
        r if r < 6000 => 5,
        _ => 6,
    }
}

// ---------------------------------------------------------------- shop

#[derive(Debug, PartialEq, Eq)]
pub enum ShopError {
    InvalidCar,
    AlreadyOwned,
    NotEnough,
    NotUpgradable,
    MaxUpgrade,
    /// The upgrade level may not exceed the driver level.
    LowDriverLevel,
    NothingToRepair,
}

fn owns_or_rents(s: &SaveGame, c: usize) -> bool {
    if c < PURCHASABLE_CARS {
        s.has_car[c]
    } else {
        s.car_rent[c] > 0
    }
}

fn pay(s: &mut SaveGame, price: i32) -> Result<(), ShopError> {
    if price < 0 {
        let cost = -price;
        if cost > s.coin {
            return Err(ShopError::NotEnough);
        }
        s.coin -= cost;
    } else {
        // Java: `price <= 0 || price > gold` -> refuse. A zero price cannot be bought.
        if price <= 0 || price > s.gold {
            return Err(ShopError::NotEnough);
        }
        s.gold -= price;
    }
    Ok(())
}

/// `buyCar(i)` — only cars 1..16. Negative price = coins, positive = gold.
pub fn buy_car(s: &mut SaveGame, c: usize) -> Result<(), ShopError> {
    if c < 1 || c >= PURCHASABLE_CARS {
        return Err(ShopError::InvalidCar);
    }
    if s.has_car[c] {
        return Err(ShopError::AlreadyOwned);
    }
    pay(s, car(c).price)?;
    s.has_car[c] = true;
    Ok(())
}

/// `rentCar(i)` — cars 16..28, 30000 distance units, damage reset.
pub fn rent_car(s: &mut SaveGame, c: usize) -> Result<(), ShopError> {
    if !(PURCHASABLE_CARS..PLAYER_CARS).contains(&c) {
        return Err(ShopError::InvalidCar);
    }
    if s.has_car[c] || s.car_rent[c] > 0 {
        return Err(ShopError::AlreadyOwned);
    }
    pay(s, car(c).price)?;
    s.car_rent[c] = RENT_DISTANCE;
    s.car_damage[c] = 0;
    Ok(())
}

/// `buyCoin(coins, goldCost)` — gold -> coin exchange (rates are UI-defined in the original).
pub fn buy_coin(s: &mut SaveGame, coins: i32, gold_cost: i32) -> Result<(), ShopError> {
    if coins >= 1 && gold_cost >= 1 && s.gold >= gold_cost {
        s.gold -= gold_cost;
        s.coin += coins;
        Ok(())
    } else {
        Err(ShopError::NotEnough)
    }
}

/// `iGetUpgradeCost(car, ability)`; `None` where the Java returns -1.
pub fn upgrade_cost(s: &SaveGame, c: usize, ability: usize) -> Option<i32> {
    if c >= PLAYER_CARS || ability > 5 || !owns_or_rents(s, c) {
        return None;
    }
    let cost = car(c).ability[ability] * 10 + s.upgrade[c][ability] * 50 - 50;
    Some(cost.max(50))
}

/// `upgradeCar` — pays in coins, raises the upgrade level (max 10) and, as a side effect
/// present in the original, **resets that car's damage to 0**.
pub fn upgrade_car(s: &mut SaveGame, c: usize, ability: usize) -> Result<(), ShopError> {
    let cost = upgrade_cost(s, c, ability).ok_or(ShopError::NotUpgradable)?;
    if s.upgrade[c][ability] >= MAX_UPGRADE {
        return Err(ShopError::MaxUpgrade);
    }
    if s.upgrade[c][ability] > s.driver_level {
        return Err(ShopError::LowDriverLevel);
    }
    if cost > s.coin {
        return Err(ShopError::NotEnough);
    }
    s.coin -= cost;
    s.upgrade[c][ability] += 1;
    s.car_damage[c] = 0;
    Ok(())
}

/// `iGetRepairCost` for a car with accumulated `damage`.
pub fn repair_cost(c: usize, damage: i32) -> i32 {
    if damage == 0 {
        return 0;
    }
    let d = damage.clamp(1000, 9000);
    (car(c).repair_cost * d) / 40000
}

/// Gold -> coin exchange offers in the store: `(coins, gold_cost)`.
pub const EXCHANGE_RATES: [(i32, i32); 3] = [(200, 2), (600, 5), (1500, 10)];

/// `processCarSelect` repair button: pays `repair_cost` coins and zeroes the damage.
pub fn repair_car(s: &mut SaveGame, c: usize) -> Result<i32, ShopError> {
    if s.car_damage[c] <= 0 {
        return Err(ShopError::NothingToRepair);
    }
    let cost = repair_cost(c, s.car_damage[c]);
    if s.coin < cost {
        return Err(ShopError::NotEnough);
    }
    s.coin -= cost;
    s.car_damage[c] = 0;
    Ok(cost)
}

/// Gold awarded by a Play Billing SKU after consumption.
pub fn gold_for_sku(sku: &str) -> Option<i32> {
    match sku {
        "drdriving.gold200" => Some(200),
        "drdriving.gold600" => Some(600),
        "drdriving.gold1500" => Some(1500),
        _ => None,
    }
}

/// Applies a consumed purchase (`onConsumeResponse`).
pub fn apply_purchase(s: &mut SaveGame, sku: &str) -> bool {
    match gold_for_sku(sku) {
        Some(g) => {
            s.gold += g;
            s.bought_gold += g;
            true
        }
        None => false,
    }
}

// ---------------------------------------------------------------- achievements

#[derive(Clone, Copy, Debug)]
pub struct Achievement {
    pub id: usize,
    /// Which statistic is compared (see [`achievement_stat`]).
    pub kind: i32,
    pub required: i32,
    pub coin_reward: i32,
    pub description: &'static str,
}

macro_rules! ach {
    ($id:expr, $k:expr, $req:expr, $rew:expr, $d:expr) => {
        Achievement { id: $id, kind: $k, required: $req, coin_reward: $rew, description: $d }
    };
}

pub const ACHIEVEMENTS: [Achievement; 24] = [
    ach!(0, 0, 10000, 200, "Travel Distance 10+ km"),
    ach!(1, 0, 100000, 500, "Travel Distance 100+ km"),
    ach!(2, 1, 1000, 200, "Drift Length 1+ km"),
    ach!(3, 1, 10000, 1000, "Drift Length 10+ km"),
    ach!(4, 2, 10, 200, "Mission Clear 10+"),
    ach!(5, 2, 100, 500, "Mission Clear 100+"),
    ach!(6, 3, 10, 200, "Speed Mission Clear 10+"),
    ach!(7, 4, 10, 200, "Highway Mission Clear 10+"),
    ach!(8, 5, 10, 200, "Drift Mission Clear 10+"),
    ach!(9, 6, 10, 200, "Fuel Efficiency Mission Clear 10+"),
    ach!(10, 7, 10, 200, "VIP Escort Mission Clear 10+"),
    ach!(11, 8, 10, 200, "Speed Parking Mission Clear 10+"),
    ach!(12, 9, 10, 200, "Lane Submission Clear 10+"),
    ach!(13, 9, 100, 500, "Lane Submission Clear 100+"),
    ach!(14, 10, 10, 200, "Perfect Turn 10+"),
    ach!(15, 10, 100, 500, "Perfect Turn 100+"),
    ach!(16, 11, 100, 200, "Maximum Speed 100+ km/h"),
    ach!(17, 12, 10, 1000, "Driver Level 10"),
    ach!(18, 13, 1, 200, "Multiplayer First Win"),
    ach!(19, 13, 10, 500, "Multiplayer Win 10+"),
    ach!(20, 13, 100, 1000, "Multiplayer Win 100+"),
    ach!(21, 14, 10, 200, "Broken Brake Mission Clear 10+"),
    ach!(22, 15, 10, 200, "Truck Mission Clear 10+"),
    ach!(23, 16, 10, 200, "Lane Mission Clear 10+"),
];

/// The statistic an achievement kind compares against (the `switch` in `onMissionFinished`).
/// Kinds 3..=8 and 14..=16 map to `mission_clear[0..=5]` / `[6..=8]`.
pub fn achievement_stat(s: &SaveGame, kind: i32) -> i32 {
    match kind {
        0 => s.travel_distance,
        1 => s.drift_distance,
        2 => s.missions_clear_count,
        3 => s.mission_clear[0],
        4 => s.mission_clear[1],
        5 => s.mission_clear[2],
        6 => s.mission_clear[3],
        7 => s.mission_clear[4],
        8 => s.mission_clear[5],
        9 => s.sub_mission,
        10 => s.perfect_turn,
        11 => (s.max_speed as f32 * 0.0036) as i32,
        12 => s.driver_level + 1,
        13 => s.win,
        14 => s.mission_clear[6],
        15 => s.mission_clear[7],
        16 => s.mission_clear[8],
        _ => 0,
    }
}

// ---------------------------------------------------------------- mission results

/// Statistics of one finished run, taken from the world (`g_f*` fields of `drdriving.l`).
#[derive(Clone, Debug, Default)]
pub struct RunStats {
    pub mission: Option<Mission>,
    pub car: usize,
    pub travel_distance: f32,
    pub drift_distance: f32,
    pub travel_time: f32,
    pub coins_acquired: i32,
    pub sub_missions: i32,
    pub perfect_turns: i32,
    pub speed_max: f32,
    /// Speed at the moment of the crash (used only for accident damage).
    pub speed: f32,
    pub score: f32,
}

/// Why a run ended (`n.DEF_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Outcome {
    Success = 0,
    Accident = 1,
    NoFuel = 2,
    OppWin = 3,
    OppLose = 4,
}

/// `onMissionFinished(code)` — folds a run into the lifetime statistics, applies car damage /
/// rental wear and awards newly reached achievements. Returns the ids of newly unlocked
/// achievements (the Java UI shows only the last one).
pub fn finish_mission(s: &mut SaveGame, run: &RunStats, outcome: Outcome) -> Vec<usize> {
    s.travel_distance = (s.travel_distance as f32 + run.travel_distance) as i32;
    s.drift_distance = (s.drift_distance as f32 + run.drift_distance) as i32;
    s.travel_second = (s.travel_second as f32 + run.travel_time) as i32;
    s.coin += run.coins_acquired;
    s.sub_mission += run.sub_missions;
    s.perfect_turn += run.perfect_turns;
    if run.speed_max * 1000.0 > s.max_speed as f32 {
        s.max_speed = (run.speed_max * 1000.0) as i32;
    }

    if run.mission != Some(Mission::Truck) {
        let c = run.car;
        s.car_damage[c] += (0.05 * run.travel_distance + run.drift_distance) as i32;
        if outcome == Outcome::Accident {
            let durability = car(c).ability[ABILITY_DURABILITY] + 10 + s.upgrade[c][ABILITY_DURABILITY];
            let mut d = ((run.speed * 3000.0) / durability as f32 + 300.0) as i32;
            if d < 321 {
                d = 321;
            }
            s.car_damage[c] += d;
        }
        if s.car_damage[c] > MAX_DAMAGE {
            s.car_damage[c] = MAX_DAMAGE;
        }
        if c >= PURCHASABLE_CARS {
            let left = (s.car_rent[c] as f32 - run.travel_distance) as i32;
            s.car_rent[c] = left;
            if left < 0 {
                s.car_rent[c] = 0;
                s.car_damage[c] = 0;
            }
        }
    }

    let mut unlocked = Vec::new();
    for a in ACHIEVEMENTS.iter() {
        if s.achievement[a.id] {
            continue;
        }
        if achievement_stat(s, a.kind) >= a.required {
            s.coin += a.coin_reward;
            s.achievement[a.id] = true;
            unlocked.push(a.id);
        }
    }
    unlocked
}

/// Result of a successful single-player mission.
#[derive(Clone, Debug, Default)]
pub struct SuccessResult {
    pub reward_coins: i32,
    pub record: i32,
    pub new_record: bool,
    pub previous_record: i32,
    pub unlocked: Vec<usize>,
}

/// `onSuccessMission()`.
pub fn complete_mission_success(s: &mut SaveGame, run: &RunStats) -> SuccessResult {
    let mission = run.mission.expect("success needs a mission");
    let mut record = (100.0 * run.score) as i32;
    if record < 1 {
        record = 1;
    }
    let mut reward = (run.score * 0.3 + run.travel_distance * 0.01 + 7.0) as i32;
    if reward < 25 {
        reward = 25;
    }
    s.coin += reward;
    s.driver_exp += reward * 15;
    s.driver_level = level_for_exp(s.driver_exp as f32);
    s.missions_clear_count += 1;
    s.mission_clear[mission.index()] += 1;

    // `n.set(mission, record)`
    let prev = s.record[mission.index()];
    let new_record = if prev > 0 { record > prev } else { true };
    if new_record {
        s.record[mission.index()] = record;
    }
    let unlocked = finish_mission(s, run, Outcome::Success);
    SuccessResult { reward_coins: reward, record, new_record, previous_record: prev, unlocked }
}

// ---------------------------------------------------------------- multiplayer ending

#[derive(Clone, Debug, Default)]
pub struct MultiResult {
    pub won: bool,
    pub opponent_rating: i32,
    pub gold_bonus: bool,
    pub unlocked: Vec<usize>,
}

/// `onMultiFinished(code)`. `outcome` must be one of Success/OppLose (win) or
/// Accident/NoFuel/OppWin (loss). `friendly` matches skip rating changes.
pub fn finish_multiplayer(
    s: &mut SaveGame,
    run: &RunStats,
    outcome: Outcome,
    mut opp_rating: i32,
    friendly: bool,
) -> MultiResult {
    let won = matches!(outcome, Outcome::Success | Outcome::OppLose);

    if won {
        let mut c = (run.travel_distance * 0.06) as i32;
        if c < 5 {
            c = 5;
        }
        s.coin += c;
        s.driver_exp += c * 5;
        s.driver_level = level_for_exp(s.driver_exp as f32);
    }

    if !friendly {
        let mine = s.rating;
        let mut d = (expected_score(mine as f32, opp_rating as f32) * 200.0 + 0.5) as i32;
        d = d.clamp(1, 199);
        if won {
            let mut gain = 200 - d;
            opp_rating -= gain;
            if mine < 4000 {
                gain = 210 - d;
            }
            if mine < 2000 {
                gain += 10;
            }
            if mine < 1000 {
                gain += 10;
            }
            s.rating += gain;
            s.win += 1;
        } else {
            s.rating -= d;
            let o = opp_rating;
            let mut od = d;
            if o < 4000 {
                od += 10;
            }
            if o < 2000 {
                od += 10;
            }
            if o < 1000 {
                od += 10;
            }
            opp_rating = o + od;
            s.lose += 1;
        }
    }

    s.rating = s.rating.clamp(-10000, 10000);
    opp_rating = opp_rating.clamp(-10000, 10000);
    if s.win > 100000 {
        s.win = 100000;
    }

    let mut gold_bonus = false;
    if won && s.mb < 10000 && (outcome == Outcome::Success || run.travel_distance > 500.0) {
        s.mb += 1;
        s.gold += 1;
        gold_bonus = true;
    }

    let unlocked = finish_mission(s, run, outcome);
    MultiResult { won, opponent_rating: opp_rating, gold_bonus, unlocked }
}

/// Matchmaking "channel" derived from the rating (`((rating + 10000) / 1000) * 10`), 10..=200.
/// Right-hand-drive players use a separate `rchannel` namespace.
pub fn matchmaking_channel(rating: i32) -> i32 {
    ((rating + 10000) / 1000) * 10
}

/// `iGetBestCarId()` — the highest-numbered car the player owns or rents (0 if only the starter).
pub fn best_car_id(s: &SaveGame) -> usize {
    (1..PLAYER_CARS).rev().find(|&i| owns_or_rents(s, i)).unwrap_or(0)
}
