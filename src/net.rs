//! Multiplayer wire protocol and matchmaking rules (`DrDrivingActivity`).
//!
//! Only the *logic* is here — message encoding/decoding, the version handshake, the lobby
//! claim transaction and the send throttle. There is deliberately **no Firebase client**; plug
//! this into whatever transport you like. The original writes each message as
//! `<room>/ToCreator|ToJoiner/<counter> = "<text>"` in Firebase Realtime Database and the
//! receiver deletes the child after reading it.

use crate::mission::MissionInfo;

/// The protocol version the 1.73 client advertises in `R` (`verCode`). It is a hard-coded
/// constant in the app, independent of the APK `versionCode` (73).
pub const PROTOCOL_VERSION: i32 = 69;

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// `S` — hello / start.
    Start,
    /// `R <version>` — version handshake.
    Version(i32),
    /// `P <rating> <car> <country> <name…>`
    Profile { rating: i32, car: i32, country: i32, name: String },
    /// `A <car> <rating>` — rematch request / acceptance.
    Rematch { car: i32, rating: i32 },
    /// `I <mission> <20 dirs> <40 aicar>` — host-chosen course.
    Course { mission: i8, directions: [i8; 20], aicar: [i8; 40] },
    /// `U <time> <missionDist> <cx> <cy> <vx> <vy> <angle>` — position update.
    Move { time: f32, mission_distance: f32, cx: f32, cy: f32, vx: f32, vy: f32, angle: f32 },
    /// `O Y` / `O N` — sender finished; `true` = the *sender* won.
    GameOver { sender_won: bool },
    /// `T` — keep-alive.
    Ping,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    UnknownTag(String),
    BadField(&'static str),
    Short,
}

fn num<T: std::str::FromStr>(parts: &[&str], i: usize, what: &'static str) -> Result<T, ParseError> {
    parts.get(i).ok_or(ParseError::Short)?.parse::<T>().map_err(|_| ParseError::BadField(what))
}

impl Message {
    pub fn encode(&self) -> String {
        match self {
            Message::Start => "S".into(),
            Message::Version(v) => format!("R {v}"),
            Message::Profile { rating, car, country, name } => format!("P {rating} {car} {country} {name}"),
            Message::Rematch { car, rating } => format!("A {car} {rating}"),
            Message::Course { mission, directions, aicar } => {
                let mut s = format!("I {mission}");
                for d in directions {
                    s.push(' ');
                    s.push_str(&d.to_string());
                }
                for a in aicar {
                    s.push(' ');
                    s.push_str(&a.to_string());
                }
                s
            }
            Message::Move { time, mission_distance, cx, cy, vx, vy, angle } => {
                format!("U {time} {mission_distance} {cx} {cy} {vx} {vy} {angle}")
            }
            Message::GameOver { sender_won } => format!("O {}", if *sender_won { "Y" } else { "N" }),
            Message::Ping => "T".into(),
        }
    }

    pub fn decode(s: &str) -> Result<Message, ParseError> {
        let parts: Vec<&str> = s.split(' ').collect();
        match *parts.first().ok_or(ParseError::Empty)? {
            "" => Err(ParseError::Empty),
            "S" => Ok(Message::Start),
            "T" => Ok(Message::Ping),
            "R" => Ok(Message::Version(num(&parts, 1, "version")?)),
            "A" => Ok(Message::Rematch { car: num(&parts, 1, "car")?, rating: num(&parts, 2, "rating")? }),
            "O" => match parts.get(1) {
                Some(&"Y") => Ok(Message::GameOver { sender_won: true }),
                Some(_) => Ok(Message::GameOver { sender_won: false }), // Java: anything but "Y" is a loss
                None => Err(ParseError::Short),
            },
            "P" => {
                let name = if parts.len() > 4 { parts[4..].join(" ") } else { String::new() };
                if parts.len() < 5 {
                    return Err(ParseError::Short);
                }
                Ok(Message::Profile {
                    rating: num(&parts, 1, "rating")?,
                    car: num(&parts, 2, "car")?,
                    country: num(&parts, 3, "country")?,
                    name,
                })
            }
            "I" => {
                let mission: i8 = num(&parts, 1, "mission")?;
                let mut directions = [0i8; 20];
                let mut aicar = [0i8; 40];
                for i in 0..20 {
                    directions[i] = num(&parts, i + 2, "direction")?;
                }
                for i in 0..40 {
                    aicar[i] = num(&parts, i + 22, "aicar")?;
                }
                Ok(Message::Course { mission, directions, aicar })
            }
            "U" => Ok(Message::Move {
                time: num(&parts, 1, "time")?,
                mission_distance: num(&parts, 2, "dist")?,
                cx: num(&parts, 3, "cx")?,
                cy: num(&parts, 4, "cy")?,
                vx: num(&parts, 5, "vx")?,
                vy: num(&parts, 6, "vy")?,
                angle: num(&parts, 7, "angle")?,
            }),
            other => Err(ParseError::UnknownTag(other.to_string())),
        }
    }

    /// Build the `I` message for a freshly generated course.
    pub fn course_from(info: &MissionInfo) -> Message {
        let mut aicar = [0i8; 40];
        for (d, s) in aicar.iter_mut().zip(info.aicar.iter()) {
            *d = *s as i8;
        }
        Message::Course { mission: info.mission as i32 as i8, directions: info.directions, aicar }
    }
}

/// What to do after receiving the peer's `R <version>`.
#[derive(Debug, PartialEq, Eq)]
pub enum HandshakeAction {
    /// Same version: proceed (send profile).
    Proceed,
    /// Peer is newer: leave the room and send the user to the store to update.
    LeaveAndUpdate,
    /// Peer is older: cancel (only surfaced to the user for friendly matches).
    PeerOutdated { notify: bool },
}

pub fn handshake(local: i32, remote: i32, friendly_match: bool) -> HandshakeAction {
    if local == remote {
        HandshakeAction::Proceed
    } else if local < remote {
        HandshakeAction::LeaveAndUpdate
    } else {
        HandshakeAction::PeerOutdated { notify: friendly_match }
    }
}

/// Server-side Remote Config gate: matchmaking is refused when the published
/// `iAppVersion` exceeds the local protocol version.
pub fn must_update(remote_config_version: i32, local: i32) -> bool {
    remote_config_version > local
}

/// Outcome of the lobby-claim transaction on `Waiting/<key>/status`.
#[derive(Debug, PartialEq, Eq)]
pub enum Claim {
    /// Status was 0 (open): write 1 and the claim succeeds.
    Won,
    /// Status was already 1, or any other value: abort the transaction.
    Lost,
    /// Value not loaded yet: retry with the real value (Firebase's null-first pattern).
    Retry,
}

pub fn claim_lobby(status: Option<i64>) -> (Claim, Option<i64>) {
    match status {
        None => (Claim::Retry, None),
        Some(0) => (Claim::Won, Some(1)),
        Some(_) => (Claim::Lost, status),
    }
}

/// Send throttle: stop writing if the peer has fallen too far behind. The original sends
/// while `read_count <= 5 || send_count <= read_count + 300`.
pub fn may_send(send_count: i32, read_count: i32) -> bool {
    read_count <= 5 || send_count <= read_count + 300
}

/// Room liveness stamp written to `Gaming/<key>/Live`: `yyyyMMddHHmmss` in GMT+9. Pass a UTC
/// unix time; the +9h shift is applied here.
pub fn live_stamp(unix_utc: i64) -> String {
    let t = unix_utc + 9 * 3600;
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    // civil-from-days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}{m:02}{d:02}{:02}{:02}{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}
