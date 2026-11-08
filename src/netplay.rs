//! Sans-IO multiplayer match state machine.
//!
//! [`Match`] speaks the [`net`](crate::net) wire protocol exactly like `DrDrivingActivity` of the
//! Android app, but never touches the network: feed it decoded [`Message`]s with
//! [`Match::handle`], drain what it wants to say with [`Match::take_outbox`], and call
//! [`Match::tick`] with the elapsed time. [`crate::rtdb`] plugs it into Firebase.
//!
//! Message flow (creator = the player who made the room, joiner = the one who claimed it):
//!
//! ```text
//!   joiner   S, R <ver>     claimed the lobby: hello + version
//!   creator  R <ver>        answers S
//!   both     P …            after a matching R: profile (rating, car, country, name)
//!   creator  I …            after the joiner's P: course (mission + 20 turns + 40 traffic picks);
//!                           the creator starts its race at once, the joiner on receiving it
//!   both     U …            position updates while racing (every 3 frames)
//!   winner   O Y / loser O N   result (a crash or a forfeit is a loss: `O N`)
//!   both     A …            rematch request; when both sent one the creator sends a new `I`
//!   both     T              keep-alive after 3 s without sending; 12 s of silence = gone
//! ```
//!
//! Everything received from the peer is untrusted: floats must be finite and in range, names
//! are sanitised, and messages that do not fit the current stage are ignored.

use crate::mission::{Mission, MissionInfo};
use crate::net::{self, HandshakeAction, Message, PROTOCOL_VERSION};
use crate::rng::JavaRandom;

/// Seconds of silence after which the peer is considered gone (`lastPingReceivedTime`, 12 s).
pub const TIMEOUT: f32 = 12.0;
/// A keep-alive is sent when nothing else was sent for this long (`lastPingSentTime`, 3 s).
pub const PING_INTERVAL: f32 = 3.0;
/// Longest accepted player name (characters).
pub const MAX_NAME: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Made the room (`FirebaseCreateRoom`); generates the courses.
    Creator,
    /// Claimed a waiting lobby (`FirebaseJoinRoom`).
    Joiner,
}

/// What a player tells the other side about themselves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub rating: i32,
    pub car: i32,
    pub country: i32,
    pub name: String,
}

impl Profile {
    /// Clamp everything into the ranges the game can represent and strip anything that would
    /// break the space-separated protocol.
    pub fn sanitized(mut self) -> Profile {
        self.rating = self.rating.clamp(-10000, 10000);
        self.car = self.car.clamp(0, 27);
        self.country = self.country.clamp(0, 255);
        let clean: String = self.name.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
        // the wire format is split on single spaces and rejoined, so runs of spaces collapse
        let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
        self.name = if clean.is_empty() { "Player".to_string() } else { clean };
        self
    }
}

/// One position update from the peer (the `U` message).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Remote {
    pub time: f32,
    pub mission_distance: f32,
    pub cx: f32,
    pub cy: f32,
    pub vx: f32,
    pub vy: f32,
    pub angle: f32,
}

impl Remote {
    fn plausible(&self) -> bool {
        let f = [self.time, self.mission_distance, self.cx, self.cy, self.vx, self.vy, self.angle];
        f.iter().all(|v| v.is_finite())
            && self.cx.abs() < 1.0e6
            && self.cy.abs() < 1.0e6
            && self.mission_distance.abs() < 1.0e6
            && self.vx.abs() < 1000.0
            && self.vy.abs() < 1000.0
            && self.time.abs() < 1.0e6
    }

    fn to_message(self) -> Message {
        Message::Move {
            time: self.time,
            mission_distance: self.mission_distance,
            cx: self.cx,
            cy: self.cy,
            vx: self.vx,
            vy: self.vy,
            angle: self.angle,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    /// The room or the connection to it went away.
    Disconnected,
    /// Nothing heard from the peer for [`TIMEOUT`] seconds.
    Timeout,
    /// The peer runs a newer protocol: this build needs updating.
    LocalOutdated,
    /// The peer runs an older protocol.
    PeerOutdated,
    /// The peer sent something that cannot be valid.
    Protocol,
    /// We closed it ourselves.
    Left,
}

impl CloseReason {
    pub fn describe(self) -> &'static str {
        match self {
            CloseReason::Disconnected => "Opponent disconnected",
            CloseReason::Timeout => "Connection timed out",
            CloseReason::LocalOutdated => "Opponent has a newer version",
            CloseReason::PeerOutdated => "Opponent has an older version",
            CloseReason::Protocol => "Invalid data from opponent",
            CloseReason::Left => "Disconnected",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// In (or looking for) a lobby, nobody to talk to yet. A creator leaves this on its first `S`.
    Lobby,
    /// `R` sent / expected, waiting for the peer's `R`.
    Handshake,
    /// `P` sent, waiting for the peer's `P`.
    Profile,
    /// Profiles known; the creator is about to choose a course, the joiner waits for `I`.
    Course,
    Racing,
    /// This race has a result (either side finished); a rematch can be negotiated.
    Finished,
    Closed(CloseReason),
}

/// Something the UI may want to react to. State itself lives on the [`Match`].
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    PeerProfile(Profile),
    /// The course for the next race is known (`Match::course`).
    Course,
    /// The course is known: begin the countdown now.
    StartRace,
    Opponent(Remote),
    /// The peer's race ended. `opponent_won == false` means they crashed or forfeited.
    OpponentFinished { opponent_won: bool },
    RematchRequested,
    Closed(CloseReason),
}

pub struct Match {
    pub role: Role,
    /// Friendly (channel) matches never change the rating.
    pub friendly: bool,
    pub local: Profile,
    pub peer: Option<Profile>,
    pub stage: Stage,
    /// The course of the race that is about to start / is running.
    pub course: Option<MissionInfo>,
    pub local_rematch: bool,
    pub peer_rematch: bool,
    outbox: Vec<Message>,
    rng: JavaRandom,
    generator: MissionInfo,
    send_count: i32,
    read_count: i32,
    /// `mMultiPlayer`: pings and the silence timeout only run once the other side is known.
    active: bool,
    idle: f32,
    since_send: f32,
}

impl Match {
    /// A creator waiting in a lobby. `seed` drives the course generator.
    pub fn new(local: Profile, seed: i64, friendly: bool) -> Match {
        let mut rng = JavaRandom::new(seed);
        // the creator's "current" mission only decides which mission the *next* one differs from
        let start = Mission::from_index(rng.next_int_bound(9) as i32).unwrap_or(Mission::Speed);
        Match {
            role: Role::Creator,
            friendly,
            local: local.sanitized(),
            peer: None,
            stage: Stage::Lobby,
            course: None,
            local_rematch: false,
            peer_rematch: false,
            outbox: Vec::new(),
            rng,
            generator: MissionInfo::new(start),
            send_count: 0,
            read_count: 0,
            active: false,
            idle: 0.0,
            since_send: 0.0,
        }
    }

    /// The lobby claim succeeded: we are the joiner. Says hello (`S`) and sends our version.
    pub fn joined(&mut self) {
        if self.stage != Stage::Lobby {
            return;
        }
        self.role = Role::Joiner;
        self.stage = Stage::Handshake;
        self.active = true;
        self.idle = 0.0;
        self.since_send = 0.0;
        self.push(Message::Start);
        self.push(Message::Version(PROTOCOL_VERSION));
    }

    pub fn is_closed(&self) -> bool {
        matches!(self.stage, Stage::Closed(_))
    }

    pub fn close_reason(&self) -> Option<CloseReason> {
        match self.stage {
            Stage::Closed(r) => Some(r),
            _ => None,
        }
    }

    /// True once the other player is known (a joiner from the claim, a creator from the first `S`).
    pub fn has_peer(&self) -> bool {
        self.active
    }

    /// Everything queued for the peer since the last call.
    pub fn take_outbox(&mut self) -> Vec<Message> {
        std::mem::take(&mut self.outbox)
    }

    /// Queue a message. Like `sendMessage`, it is dropped while the peer has fallen too far
    /// behind (see [`net::may_send`]) and before the other side is known.
    fn push(&mut self, m: Message) {
        if !self.active || !net::may_send(self.send_count, self.read_count) {
            return;
        }
        self.send_count += 1;
        self.since_send = 0.0;
        self.outbox.push(m);
    }

    fn close(&mut self, reason: CloseReason, ev: &mut Vec<Event>) {
        if self.is_closed() {
            return;
        }
        self.stage = Stage::Closed(reason);
        ev.push(Event::Closed(reason));
    }

    /// The room vanished or the connection failed.
    pub fn on_link_closed(&mut self) -> Vec<Event> {
        let mut ev = Vec::new();
        self.close(CloseReason::Disconnected, &mut ev);
        ev
    }

    /// Leave on purpose (nothing more is queued afterwards).
    pub fn leave(&mut self) {
        let mut ev = Vec::new();
        self.close(CloseReason::Left, &mut ev);
    }

    /// Creator: pick the next course (`infoOnline.generate`), tell the joiner and start.
    fn send_course(&mut self, ev: &mut Vec<Event>) {
        let current = self.generator.mission;
        self.generator.generate(current, &mut self.rng);
        let info = self.generator.clone();
        self.push(Message::course_from(&info));
        self.course = Some(info);
        self.stage = Stage::Racing;
        ev.push(Event::Course);
        ev.push(Event::StartRace);
    }

    fn try_rematch(&mut self, ev: &mut Vec<Event>) {
        if self.stage == Stage::Finished && self.local_rematch && self.peer_rematch {
            self.local_rematch = false;
            self.peer_rematch = false;
            self.course = None;
            self.stage = Stage::Course;
            if self.role == Role::Creator {
                self.send_course(ev);
            }
        }
    }

    /// Process one message from the peer.
    pub fn handle(&mut self, msg: Message) -> Vec<Event> {
        let mut ev = Vec::new();
        if self.is_closed() {
            return ev;
        }
        self.idle = 0.0;
        self.read_count += 1;
        match msg {
            Message::Ping => {}
            Message::Start => {
                // the joiner arrived; answer with our version
                if !self.active {
                    self.active = true;
                    self.since_send = 0.0;
                    self.stage = Stage::Handshake;
                }
                self.push(Message::Version(PROTOCOL_VERSION));
            }
            Message::Version(v) => {
                if self.stage == Stage::Handshake {
                    match net::handshake(PROTOCOL_VERSION, v, self.friendly) {
                        HandshakeAction::Proceed => {
                            self.stage = Stage::Profile;
                            let p = self.local.clone();
                            self.push(Message::Profile { rating: p.rating, car: p.car, country: p.country, name: p.name });
                        }
                        HandshakeAction::LeaveAndUpdate => self.close(CloseReason::LocalOutdated, &mut ev),
                        HandshakeAction::PeerOutdated { .. } => self.close(CloseReason::PeerOutdated, &mut ev),
                    }
                }
            }
            Message::Profile { rating, car, country, name } => {
                if self.stage == Stage::Profile {
                    let p = Profile { rating, car, country, name }.sanitized();
                    self.peer = Some(p.clone());
                    ev.push(Event::PeerProfile(p));
                    self.stage = Stage::Course;
                    if self.role == Role::Creator {
                        self.send_course(&mut ev);
                    }
                }
            }
            Message::Course { mission, directions, aicar } => {
                if self.role == Role::Joiner && self.stage == Stage::Course {
                    let valid = Mission::from_index(mission as i32).filter(|_| {
                        directions.iter().all(|d| (0..4).contains(d)) && aicar.iter().all(|a| (0..3).contains(a))
                    });
                    match valid {
                        None => self.close(CloseReason::Protocol, &mut ev),
                        Some(m) => {
                            let mut info = MissionInfo::new(m);
                            info.directions = directions;
                            for (d, s) in info.aicar.iter_mut().zip(aicar.iter()) {
                                *d = *s as u8;
                            }
                            self.course = Some(info);
                            self.stage = Stage::Racing;
                            ev.push(Event::Course);
                            ev.push(Event::StartRace);
                        }
                    }
                }
            }
            Message::Move { time, mission_distance, cx, cy, vx, vy, angle } => {
                if self.stage == Stage::Racing {
                    let r = Remote { time, mission_distance, cx, cy, vx, vy, angle };
                    if r.plausible() {
                        ev.push(Event::Opponent(r));
                    }
                }
            }
            Message::GameOver { sender_won } => {
                if self.stage == Stage::Racing {
                    self.stage = Stage::Finished;
                    ev.push(Event::OpponentFinished { opponent_won: sender_won });
                    self.try_rematch(&mut ev);
                }
            }
            Message::Rematch { car, rating } => {
                if matches!(self.stage, Stage::Racing | Stage::Finished) && !self.peer_rematch {
                    self.peer_rematch = true;
                    if let Some(p) = self.peer.as_mut() {
                        *p = Profile { rating, car, country: p.country, name: p.name.clone() }.sanitized();
                    }
                    ev.push(Event::RematchRequested);
                    self.try_rematch(&mut ev);
                }
            }
        }
        ev
    }

    /// Advance timers: keep-alive and silence detection (both only once the peer is known).
    pub fn tick(&mut self, dt: f32) -> Vec<Event> {
        let mut ev = Vec::new();
        if self.is_closed() || !self.active {
            return ev;
        }
        self.idle += dt;
        self.since_send += dt;
        if self.idle > TIMEOUT {
            self.close(CloseReason::Timeout, &mut ev);
        } else if self.since_send >= PING_INTERVAL {
            self.push(Message::Ping);
        }
        ev
    }

    /// Racing: report our position. Silently dropped outside a race.
    pub fn send_move(&mut self, r: Remote) {
        if self.stage == Stage::Racing {
            self.push(r.to_message());
        }
    }

    /// Our race ended. `won == true` only for crossing the finish line first; a crash or a
    /// forfeit is `false`. Ignored unless a race is running (so it is sent at most once).
    pub fn finish(&mut self, won: bool) {
        if self.stage == Stage::Racing {
            self.stage = Stage::Finished;
            self.push(Message::GameOver { sender_won: won });
        }
    }

    /// After a race: ask for another one with the car we want to use.
    pub fn request_rematch(&mut self, car: i32, rating: i32) -> Vec<Event> {
        let mut ev = Vec::new();
        if self.stage == Stage::Finished && !self.local_rematch {
            self.local_rematch = true;
            self.local.car = car.clamp(0, 27);
            self.local.rating = rating.clamp(-10000, 10000);
            self.push(Message::Rematch { car: self.local.car, rating: self.local.rating });
            self.try_rematch(&mut ev);
        }
        ev
    }
}
