//! One race, from the start lights to the result screen (`drdriving.d`: `frameMoveObject`,
//! `gameFrameMove`, `onSuccessMission`). It owns a [`World`], turns keyboard-style input into
//! the steering/throttle the physics wants, reproduces the wheel mechanics (a physical wheel
//! that must be turned and that self-centres while the car moves), blinker rules, bonus-coin
//! rules and end-of-race detection, and reports what happened as [`Event`]s so the front end
//! only has to render and play sounds.

use crate::economy::{self, MultiResult, Outcome, SuccessResult};
use crate::mission::Mission;
use crate::save::SaveGame;
use crate::world::{mode, Controls, World, GEAR_D};

/// The steering wheel turns lock to lock over ±540°.
pub const WHEEL_MAX: f32 = 540.0;
/// How fast a held steering key turns the wheel (deg/s). The original needs a finger drag on a
/// touch wheel; this value takes ~0.8 s to reach full lock.
pub const WHEEL_KEY_RATE: f32 = 700.0;
/// Countdown before the race goes live (`g_fModeTime > 2.4`).
pub const START_DELAY: f32 = 2.4;
/// Delay before the success/ending screens accept input (2.5 s / 1.5 s in the original).
pub const SUCCESS_INPUT_DELAY: f32 = 2.5;
pub const ENDING_INPUT_DELAY: f32 = 1.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Starting,
    Playing,
    Paused,
    Success,
    Ending(Outcome),
}

/// Per-frame input. "Pressed" fields are edge-triggered (true for one frame only).
#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub accel: bool,
    pub brake: bool,
    /// -1 = left, 0 = none, 1 = right (held).
    pub steer: i8,
    /// The wheel is being held (touch/mouse drag or a steering key): don't self-centre it.
    pub wheel_held: bool,
    pub horn: bool,
    pub blinker_left: bool,
    pub blinker_right: bool,
    pub hazard: bool,
    pub camera: bool,
    pub gear: Option<i32>,
    pub mirror_back: bool,
    pub mirror_left: bool,
    pub mirror_right: bool,
    pub pause: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoinReason {
    LaneChange,
    SubMission,
    TurnSignal,
    Turn,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Horn,
    BlinkerSound(bool),
    BrakeSound,
    DriftStart,
    DriftEnd,
    Coin(CoinReason),
    /// 0 = ok, 1 = good, 2 = perfect turn.
    TurnRating(u8),
    Crash,
    OutOfFuel,
    Won,
    Paused(bool),
    Go,
}

#[derive(Clone, Debug)]
pub struct RaceResult {
    pub outcome: Outcome,
    pub success: Option<SuccessResult>,
    pub unlocked: Vec<usize>,
    /// Set for online races: who won and what it paid (see [`economy::finish_multiplayer`]).
    pub multi: Option<MultiResult>,
}

/// Settings of an online race against a remote opponent.
#[derive(Clone, Copy, Debug)]
pub struct OnlineRace {
    /// Friendly matches do not change the rating.
    pub friendly: bool,
    pub opp_rating: i32,
}

pub struct Session {
    pub world: World,
    pub phase: Phase,
    pub phase_time: f32,
    pub wheel_angle: f32,
    prev_steer: f32,
    was_brake: bool,
    /// 2 = far chase, 3 = near chase, 4 = driver's seat.
    pub camera_mode: i32,
    pub mirror_back: bool,
    pub mirror_left: bool,
    pub mirror_right: bool,
    pub result: Option<RaceResult>,
    /// `Some` for a race against a remote player (no pausing, results go through
    /// [`economy::finish_multiplayer`]).
    pub online: Option<OnlineRace>,
    /// Set when *this* player's race ended by their own doing: `Some(true)` = crossed the
    /// finish line, `Some(false)` = crashed / ran out of fuel / forfeited. The network layer
    /// takes it to tell the opponent; it is never set by the opponent's result.
    pub local_finish: Option<bool>,
}

/// Wheel angle (degrees) → steering factor (-1..1), including the sensitivity option (-4..4).
pub fn steer_from_wheel(angle: f32, sensitivity: i32) -> f32 {
    let k = (sensitivity * 10) as f32;
    if angle > 0.2 {
        let f = (angle + k + 80.0) / (k + 620.0);
        f * f
    } else if angle < -0.2 {
        let f = ((-angle) + k + 80.0) / (k + 620.0);
        -(f * f)
    } else {
        0.0
    }
}

impl Session {
    /// Start a race: `world` must already have had `prepare_race` called.
    pub fn new(world: World) -> Self {
        Self {
            world,
            phase: Phase::Starting,
            phase_time: 0.0,
            wheel_angle: 0.0,
            prev_steer: 0.0,
            was_brake: false,
            camera_mode: 4,
            mirror_back: false,
            mirror_left: false,
            mirror_right: false,
            result: None,
            online: None,
            local_finish: None,
        }
    }

    /// Start an online race: `world` must have had [`World::setup_online`] and `prepare_race`.
    pub fn new_online(world: World, friendly: bool, opp_rating: i32) -> Self {
        let mut s = Self::new(world);
        s.online = Some(OnlineRace { friendly, opp_rating });
        s
    }

    pub fn mode_number(&self) -> i32 {
        match self.phase {
            Phase::Starting => mode::STARTING,
            Phase::Playing => mode::PLAYING,
            Phase::Paused => 4,
            Phase::Success => mode::SUCCESS,
            // losing to the opponent lets the car coast to a stop, like a win does
            Phase::Ending(Outcome::OppWin) => mode::MULTI_ENDING,
            Phase::Ending(_) => 6,
        }
    }

    pub fn is_over(&self) -> bool {
        matches!(self.phase, Phase::Success | Phase::Ending(_))
    }

    /// True once the result screen may be dismissed.
    pub fn can_continue(&self) -> bool {
        match self.phase {
            Phase::Success => self.phase_time > SUCCESS_INPUT_DELAY,
            Phase::Ending(_) => self.phase_time > ENDING_INPUT_DELAY,
            _ => false,
        }
    }

    fn set_phase(&mut self, p: Phase) {
        self.phase = p;
        self.phase_time = 0.0;
    }

    /// `frameMoveObject`: apply controls, run the physics, then the blinker/lane bookkeeping.
    fn move_objects(&mut self, dt: f32, input: &Input, save: &mut SaveGame, ev: &mut Vec<Event>) {
        if input.horn {
            ev.push(Event::Horn);
        }
        if input.camera {
            self.camera_mode += 1;
            if self.camera_mode > 4 {
                self.camera_mode = 2;
            }
        }
        {
            let w = &mut self.world;
            if input.hazard {
                let was = w.emergency;
                w.emergency = !was;
                if !was {
                    ev.push(Event::BlinkerSound(true));
                } else if w.blinker == 0 {
                    ev.push(Event::BlinkerSound(false));
                }
            }
            for (pressed, dir) in [(input.blinker_left, -1), (input.blinker_right, 1)] {
                if pressed {
                    if w.blinker == 0 {
                        w.blinker = dir;
                        ev.push(Event::BlinkerSound(true));
                    } else {
                        w.blinker = 0;
                        if !w.emergency {
                            ev.push(Event::BlinkerSound(false));
                        }
                    }
                }
            }
            if let Some(g) = input.gear {
                if w.speed < 5.0 {
                    w.gear = g;
                }
            }
        }

        // wheel: keys turn it, otherwise it self-centres at a speed-dependent rate
        let playing = self.phase == Phase::Playing;
        if playing && input.steer != 0 {
            self.wheel_angle =
                (self.wheel_angle + input.steer as f32 * WHEEL_KEY_RATE * dt).clamp(-WHEEL_MAX, WHEEL_MAX);
        }
        let steer = if playing { steer_from_wheel(self.wheel_angle, save.wheel_sensitivity) } else { 0.0 };

        let mode_no = self.mode_number();
        let was_brake = self.was_brake;
        self.was_brake = input.brake;
        let w = &mut self.world;
        let drifting_before = w.drifting;
        let lane_before = w.current_lane();
        w.update(dt, mode_no, Controls { accel: input.accel, brake: input.brake, steer });
        if w.mission == Mission::BrokenBrake && input.brake && !was_brake && playing {
            // the original plays a "brakes failed" sound whenever the pedal goes down
            ev.push(Event::BrakeSound);
        }
        if !drifting_before && w.drifting {
            ev.push(Event::DriftStart);
        }
        if drifting_before && !w.drifting {
            ev.push(Event::DriftEnd);
        }

        // moving to an adjacent lane while signalling toward it pays one coin
        let lane_after = w.current_lane();
        let rd = save.right_driver;
        let b = w.blinker;
        if b != 0 && !w.now_turning && lane_before != lane_after {
            let ok = if lane_before < lane_after {
                (!rd && b > 0) || (rd && b < 0)
            } else {
                (!rd && b < 0) || (rd && b > 0)
            };
            if ok {
                w.coins_acquired += 1;
                ev.push(Event::Coin(CoinReason::LaneChange));
            }
        }

        if input.steer == 0 && !input.wheel_held {
            let rate = w.speed.min(10.0) * 80.0 * dt;
            if self.wheel_angle > rate {
                self.wheel_angle -= rate;
            } else if self.wheel_angle < -rate {
                self.wheel_angle += rate;
            } else {
                self.wheel_angle = 0.0;
            }
        }

        // the indicator cancels itself once the wheel comes back through centre
        if self.prev_steer > 0.1 && steer < 0.1 && w.blinker > 0 {
            w.blinker = 0;
            if !w.emergency {
                ev.push(Event::BlinkerSound(false));
            }
        }
        if self.prev_steer < -0.1 && steer > -0.1 && w.blinker < 0 {
            w.blinker = 0;
            if !w.emergency {
                ev.push(Event::BlinkerSound(false));
            }
        }
        self.prev_steer = steer;
    }

    fn finish(&mut self, outcome: Outcome, save: &mut SaveGame) {
        let run = self.world.run_stats();
        let (unlocked, multi) = match self.online {
            Some(o) => {
                let m = economy::finish_multiplayer(save, &run, outcome, o.opp_rating, o.friendly);
                (m.unlocked.clone(), Some(m))
            }
            None => (economy::finish_mission(save, &run, outcome), None),
        };
        if self.online.is_some() && matches!(outcome, Outcome::Accident | Outcome::NoFuel) {
            self.local_finish = Some(false);
        }
        self.result = Some(RaceResult { outcome, success: None, unlocked, multi });
        self.set_phase(Phase::Ending(outcome));
    }

    /// Online race: the opponent's result arrived. `opponent_won == false` (they crashed,
    /// forfeited or left) makes this player the winner. No-op once this race is decided or
    /// outside an online race.
    pub fn opponent_finished(&mut self, opponent_won: bool, save: &mut SaveGame) -> Vec<Event> {
        let mut ev = Vec::new();
        if self.online.is_none() || self.is_over() {
            return ev;
        }
        if opponent_won {
            self.finish(Outcome::OppWin, save);
        } else {
            let o = self.online.unwrap();
            let run = self.world.run_stats();
            let m = economy::finish_multiplayer(save, &run, Outcome::OppLose, o.opp_rating, o.friendly);
            self.result = Some(RaceResult { outcome: Outcome::OppLose, success: None, unlocked: m.unlocked.clone(), multi: Some(m) });
            self.set_phase(Phase::Success);
            ev.push(Event::Won);
        }
        ev
    }

    /// Online race: give up. Counts as a loss; the car just stops where it is.
    pub fn forfeit(&mut self, save: &mut SaveGame) -> Vec<Event> {
        if self.online.is_none() || self.is_over() {
            return Vec::new();
        }
        let o = self.online.unwrap();
        let run = self.world.run_stats();
        let m = economy::finish_multiplayer(save, &run, Outcome::NoFuel, o.opp_rating, o.friendly);
        self.local_finish = Some(false);
        self.result = Some(RaceResult { outcome: Outcome::NoFuel, success: None, unlocked: m.unlocked.clone(), multi: Some(m) });
        self.set_phase(Phase::Ending(Outcome::NoFuel));
        Vec::new()
    }

    /// Advance one frame. Returns the events that happened during it.
    pub fn frame(&mut self, dt: f32, input: &Input, save: &mut SaveGame) -> Vec<Event> {
        let mut ev = Vec::new();
        self.phase_time += dt;
        match self.phase {
            Phase::Paused => {
                if input.pause {
                    self.set_phase(Phase::Playing);
                    ev.push(Event::Paused(false));
                }
                return ev;
            }
            Phase::Ending(Outcome::OppWin) => {
                self.move_objects(dt, input, save, &mut ev);
                return ev;
            }
            Phase::Ending(_) => return ev,
            Phase::Success => {
                // the car coasts to a stop under automatic braking
                self.move_objects(dt, input, save, &mut ev);
                return ev;
            }
            Phase::Starting | Phase::Playing => {}
        }

        // ---- gameFrameMove
        self.move_objects(dt, input, save, &mut ev);

        if input.pause && self.online.is_none() {
            self.set_phase(Phase::Paused);
            ev.push(Event::Paused(true));
            return ev;
        }
        if input.mirror_back && self.camera_mode > 2 {
            self.mirror_back = !self.mirror_back;
        }
        if input.mirror_left && self.camera_mode == 4 {
            self.mirror_left = !self.mirror_left;
        }
        if input.mirror_right && self.camera_mode == 4 {
            self.mirror_right = !self.mirror_right;
        }

        if self.phase == Phase::Playing {
            if self.world.check_accident() {
                save.accident_no += 1;
                self.world.driver.active = true;
                ev.push(Event::Crash);
                self.finish(Outcome::Accident, save);
                return ev;
            }
            let w = &mut self.world;
            if w.speed < 0.1 && (w.gas_left < 0.0 || w.travel_time > 180.0) {
                w.driver.active = false;
                ev.push(Event::OutOfFuel);
                self.finish(Outcome::NoFuel, save);
                return ev;
            }
        }

        let w = &mut self.world;
        if w.complete_sub_mission() {
            ev.push(Event::Coin(CoinReason::SubMission));
        }
        if w.sub_mission_lane < 0 {
            w.make_sub_mission();
        }
        let was_turning = w.now_turning;
        if w.advance_block(save) {
            w.find_next_mission(save);
        }

        // finishing a turn: score it by how quickly it was taken relative to speed
        if was_turning && !w.now_turning {
            let t = w.travel_time - w.turn_start_time;
            if t < 6.0 {
                let mut score = (((w.speed * 0.25) + 13.0) - (t * 5.0) - w.engine_power) as i32;
                let mut rel = w.rel_dir_cur;
                if save.right_driver {
                    if rel == 3 {
                        rel = 1;
                    } else if rel == 1 {
                        rel = 3;
                    }
                }
                if rel == 3 {
                    score += 1;
                } else if rel == 2 {
                    score += 7;
                }
                if score > 1 {
                    let rate = if score > 7 {
                        w.perfect_turns += 1;
                        2
                    } else if score > 4 {
                        1
                    } else {
                        0
                    };
                    ev.push(Event::TurnRating(rate));
                    if rate > 0 {
                        w.coins_acquired += 1;
                        ev.push(Event::Coin(CoinReason::Turn));
                    }
                }
            }
        }
        // signalling the right way as the turn starts is worth a coin
        if !was_turning && w.now_turning && w.blinker != 0 {
            let b = w.blinker;
            let rd = save.right_driver;
            let good = (b < 0 && w.rel_dir_next == 3)
                || (b > 0 && w.rel_dir_next == 1)
                || (w.rel_dir_next == 2 && ((rd && b > 0) || (!rd && b < 0)));
            if good {
                w.coins_acquired += 1;
                ev.push(Event::Coin(CoinReason::TurnSignal));
            }
        }

        if w.pass_check_point() {
            let run = w.run_stats();
            if let Some(o) = self.online {
                let m = economy::finish_multiplayer(save, &run, Outcome::Success, o.opp_rating, o.friendly);
                self.result = Some(RaceResult { outcome: Outcome::Success, success: None, unlocked: m.unlocked.clone(), multi: Some(m) });
                self.local_finish = Some(true);
            } else {
                let success = economy::complete_mission_success(save, &run);
                let unlocked = success.unlocked.clone();
                self.result = Some(RaceResult { outcome: Outcome::Success, success: Some(success), unlocked, multi: None });
            }
            self.set_phase(Phase::Success);
            self.world.on_change_success();
            ev.push(Event::Won);
            return ev;
        }

        if self.phase == Phase::Starting && self.phase_time > START_DELAY {
            self.set_phase(Phase::Playing);
            ev.push(Event::Go);
        }
        let _ = GEAR_D;
        ev
    }
}
