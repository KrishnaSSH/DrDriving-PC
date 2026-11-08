//! Multiplayer wire protocol and matchmaking rules (`DrDrivingActivity`).
//!
//! Only the *logic* is here — message encoding/decoding, the version handshake, the lobby
//! claim transaction, channel selection and the send throttle. [`crate::netplay`] runs the
//! protocol and [`crate::rtdb`] carries it over the Firebase Realtime Database exactly like the
//! original: each message is written as `Gaming/<room>/ToCreator|ToJoiner/<counter> = "<text>"`
//! and the receiver deletes the child after reading it.

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

/// First lobby channel that is a user-typed "friend" number (100..=999). Smaller channels are
/// the automatic rating buckets and never change hands between friends.
pub const FIRST_FRIEND_CHANNEL: i32 = 100;

/// Matchmaking channel of "Online Match" (`startQuickOnlineGame`): players rated under 1000
/// meet in channels 0/1, everyone else in 2/3; left-hand-traffic drivers are 0/2, right-hand 1/3.
pub fn quick_channel(rating: i32, right_driver: bool) -> i32 {
    (if rating < 1000 { 0 } else { 2 }) + right_driver as i32
}

/// A friend match: both players type the same number 100..=999.
pub fn friend_channel(n: i32) -> Option<i32> {
    (FIRST_FRIEND_CHANNEL..1000).contains(&n).then_some(n)
}

/// Flag index (`framework.b.a`) of an ISO country code, as sent in the profile message `P`.
const COUNTRY_FLAGS: &[(&str, i32)] = &[
    ("IN", 95), ("TR", 213), ("KR", 192), ("ID", 96), ("BR", 31), ("EG", 65), ("US", 222), ("PH", 166),
    ("RU", 173), ("AZ", 17), ("SA", 179), ("ES", 193), ("MY", 125), ("RS", 182), ("RO", 172), ("TW", 204),
    ("CL", 44), ("AR", 11), ("DE", 80), ("CO", 47), ("JO", 106), ("PK", 159), ("MX", 132), ("CA", 38),
    ("BA", 29), ("KG", 111), ("KW", 110), ("KZ", 107), ("LV", 113), ("SE", 200), ("HK", 92), ("UY", 223),
    ("BD", 20), ("DZ", 3), ("AE", 219), ("OM", 157), ("PL", 167), ("IL", 101), ("TH", 207), ("LB", 114),
    ("HU", 93), ("BH", 19), ("LK", 194), ("AO", 6), ("AT", 16), ("FR", 76), ("NL", 147), ("AU", 15),
    ("BG", 33), ("GB", 220), ("PY", 164), ("QA", 170), ("VE", 227), ("ZA", 191), ("CR", 53), ("DO", 62),
    ("LT", 119), ("PE", 165), ("BY", 22), ("CZ", 58), ("GR", 83), ("IQ", 98), ("IS", 94), ("LA", 112),
    ("LU", 120), ("SG", 185), ("SI", 187), ("TJ", 205), ("TN", 212), ("UA", 218), ("IR", 97), ("SD", 197),
    ("BE", 23), ("CH", 201), ("CN", 45), ("DK", 59), ("FI", 75), ("NZ", 148), ("PT", 168), ("LY", 117),
    ("AL", 2), ("AS", 4), ("AD", 5), ("AI", 7), ("AQ", 8), ("AG", 9), ("AM", 12), ("AW", 13), ("BS", 18),
    ("BB", 21), ("BZ", 24), ("BJ", 25), ("BM", 26), ("BT", 27), ("BO", 28), ("BW", 30), ("BN", 32),
    ("BF", 34), ("BI", 35), ("KH", 36), ("CM", 37), ("CV", 39), ("KY", 41), ("CF", 42), ("TD", 43),
    ("KM", 49), ("CG", 50), ("CD", 51), ("CK", 52), ("CI", 54), ("HR", 55), ("CU", 56), ("CY", 57),
    ("DJ", 60), ("DM", 61), ("EC", 64), ("SV", 66), ("GQ", 68), ("ER", 69), ("EE", 70), ("ET", 71),
    ("FO", 73), ("FJ", 74), ("GP", 76), ("GA", 77), ("GM", 78), ("GE", 79), ("GH", 81), ("GI", 82),
    ("GD", 84), ("GU", 85), ("GT", 86), ("GN", 87), ("GW", 88), ("GY", 89), ("HT", 90), ("HN", 91),
    ("IE", 99), ("JM", 103), ("KE", 108), ("KI", 109), ("LS", 115), ("LR", 116), ("LI", 118), ("MO", 121),
    ("MK", 122), ("MG", 123), ("MW", 124), ("MV", 126), ("ML", 127), ("MH", 129), ("MR", 130), ("MU", 131),
    ("FM", 133), ("MD", 134), ("MC", 135), ("MN", 136), ("ME", 137), ("MS", 138), ("MZ", 140), ("MM", 141),
    ("NA", 142), ("NR", 144), ("NP", 145), ("NI", 149), ("NE", 150), ("NG", 151), ("PW", 160), ("PS", 161),
    ("PA", 162), ("PR", 169), ("RW", 174), ("LC", 175), ("WS", 176), ("SM", 177), ("ST", 178), ("SN", 181),
    ("SC", 183), ("SL", 184), ("SK", 186), ("SB", 188), ("SO", 190), ("KN", 195), ("VC", 196), ("SR", 198),
    ("SZ", 199), ("SY", 202), ("PF", 203), ("TZ", 206), ("TL", 208), ("TG", 209), ("TO", 210), ("TT", 211),
    ("TM", 214), ("TC", 215), ("TV", 216), ("UG", 217), ("UZ", 224), ("VU", 225), ("VA", 226), ("VN", 228),
    ("VG", 229), ("VI", 230), ("EH", 232), ("YE", 233), ("ZM", 234), ("ZW", 235), ("AF", 236), ("UK", 220),
    ("EN", 222), ("ZH", 45), ("DX", 220), ("FA", 97), ("RE", 76), ("CS", 58), ("LO", 112), ("XX", 222),
    ("HI", 95), ("EU", 72), ("NC", 76), ("GL", 231),
];

/// The `iCountry` value for a two-letter country code (case-insensitive); 0 when unknown.
pub fn country_index(code: &str) -> i32 {
    if code.len() < 2 {
        return 0;
    }
    COUNTRY_FLAGS.iter().find(|(c, _)| c.eq_ignore_ascii_case(code)).map_or(0, |&(_, i)| i)
}

/// Countries offered by the picker: (ISO code, name), sorted by name. Every code here has a flag.
pub const COUNTRY_NAMES: &[(&str, &str)] = &[
    ("AF", "Afghanistan"),
    ("AL", "Albania"),
    ("DZ", "Algeria"),
    ("AD", "Andorra"),
    ("AO", "Angola"),
    ("AI", "Anguilla"),
    ("AQ", "Antarctica"),
    ("AG", "Antigua & Barbuda"),
    ("AR", "Argentina"),
    ("AM", "Armenia"),
    ("AW", "Aruba"),
    ("AU", "Australia"),
    ("AT", "Austria"),
    ("AZ", "Azerbaijan"),
    ("BS", "Bahamas"),
    ("BH", "Bahrain"),
    ("BD", "Bangladesh"),
    ("BB", "Barbados"),
    ("BY", "Belarus"),
    ("BE", "Belgium"),
    ("BZ", "Belize"),
    ("BJ", "Benin"),
    ("BM", "Bermuda"),
    ("BT", "Bhutan"),
    ("BO", "Bolivia"),
    ("BA", "Bosnia & Herzegovina"),
    ("BW", "Botswana"),
    ("BR", "Brazil"),
    ("GB", "Britain (UK)"),
    ("BN", "Brunei"),
    ("BG", "Bulgaria"),
    ("BF", "Burkina Faso"),
    ("BI", "Burundi"),
    ("KH", "Cambodia"),
    ("CM", "Cameroon"),
    ("CA", "Canada"),
    ("CV", "Cape Verde"),
    ("KY", "Cayman Islands"),
    ("CF", "Central African Rep."),
    ("TD", "Chad"),
    ("CL", "Chile"),
    ("CN", "China"),
    ("CO", "Colombia"),
    ("KM", "Comoros"),
    ("CD", "Congo (Dem. Rep.)"),
    ("CG", "Congo (Rep.)"),
    ("CK", "Cook Islands"),
    ("CR", "Costa Rica"),
    ("HR", "Croatia"),
    ("CU", "Cuba"),
    ("CY", "Cyprus"),
    ("CZ", "Czech Republic"),
    ("CI", "Côte d’Ivoire"),
    ("DK", "Denmark"),
    ("DJ", "Djibouti"),
    ("DM", "Dominica"),
    ("DO", "Dominican Republic"),
    ("TL", "East Timor"),
    ("EC", "Ecuador"),
    ("EG", "Egypt"),
    ("SV", "El Salvador"),
    ("GQ", "Equatorial Guinea"),
    ("ER", "Eritrea"),
    ("EE", "Estonia"),
    ("SZ", "Eswatini (Swaziland)"),
    ("ET", "Ethiopia"),
    ("EU", "European Union"),
    ("FO", "Faroe Islands"),
    ("FJ", "Fiji"),
    ("FI", "Finland"),
    ("FR", "France"),
    ("PF", "French Polynesia"),
    ("GA", "Gabon"),
    ("GM", "Gambia"),
    ("GE", "Georgia"),
    ("DE", "Germany"),
    ("GH", "Ghana"),
    ("GI", "Gibraltar"),
    ("GR", "Greece"),
    ("GL", "Greenland"),
    ("GD", "Grenada"),
    ("GP", "Guadeloupe"),
    ("GU", "Guam"),
    ("GT", "Guatemala"),
    ("GN", "Guinea"),
    ("GW", "Guinea-Bissau"),
    ("GY", "Guyana"),
    ("HT", "Haiti"),
    ("HN", "Honduras"),
    ("HK", "Hong Kong"),
    ("HU", "Hungary"),
    ("IS", "Iceland"),
    ("IN", "India"),
    ("ID", "Indonesia"),
    ("IR", "Iran"),
    ("IQ", "Iraq"),
    ("IE", "Ireland"),
    ("IL", "Israel"),
    ("JM", "Jamaica"),
    ("JO", "Jordan"),
    ("KZ", "Kazakhstan"),
    ("KE", "Kenya"),
    ("KI", "Kiribati"),
    ("KR", "Korea (South)"),
    ("KW", "Kuwait"),
    ("KG", "Kyrgyzstan"),
    ("LA", "Laos"),
    ("LV", "Latvia"),
    ("LB", "Lebanon"),
    ("LS", "Lesotho"),
    ("LR", "Liberia"),
    ("LY", "Libya"),
    ("LI", "Liechtenstein"),
    ("LT", "Lithuania"),
    ("LU", "Luxembourg"),
    ("MO", "Macau"),
    ("MG", "Madagascar"),
    ("MW", "Malawi"),
    ("MY", "Malaysia"),
    ("MV", "Maldives"),
    ("ML", "Mali"),
    ("MH", "Marshall Islands"),
    ("MR", "Mauritania"),
    ("MU", "Mauritius"),
    ("MX", "Mexico"),
    ("FM", "Micronesia"),
    ("MD", "Moldova"),
    ("MC", "Monaco"),
    ("MN", "Mongolia"),
    ("ME", "Montenegro"),
    ("MS", "Montserrat"),
    ("MZ", "Mozambique"),
    ("MM", "Myanmar (Burma)"),
    ("NA", "Namibia"),
    ("NR", "Nauru"),
    ("NP", "Nepal"),
    ("NL", "Netherlands"),
    ("NC", "New Caledonia"),
    ("NZ", "New Zealand"),
    ("NI", "Nicaragua"),
    ("NE", "Niger"),
    ("NG", "Nigeria"),
    ("MK", "North Macedonia"),
    ("OM", "Oman"),
    ("PK", "Pakistan"),
    ("PW", "Palau"),
    ("PS", "Palestine"),
    ("PA", "Panama"),
    ("PY", "Paraguay"),
    ("PE", "Peru"),
    ("PH", "Philippines"),
    ("PL", "Poland"),
    ("PT", "Portugal"),
    ("PR", "Puerto Rico"),
    ("QA", "Qatar"),
    ("RO", "Romania"),
    ("RU", "Russia"),
    ("RW", "Rwanda"),
    ("RE", "Réunion"),
    ("AS", "Samoa (American)"),
    ("WS", "Samoa (western)"),
    ("SM", "San Marino"),
    ("ST", "Sao Tome & Principe"),
    ("SA", "Saudi Arabia"),
    ("SN", "Senegal"),
    ("RS", "Serbia"),
    ("SC", "Seychelles"),
    ("SL", "Sierra Leone"),
    ("SG", "Singapore"),
    ("SK", "Slovakia"),
    ("SI", "Slovenia"),
    ("SB", "Solomon Islands"),
    ("SO", "Somalia"),
    ("ZA", "South Africa"),
    ("ES", "Spain"),
    ("LK", "Sri Lanka"),
    ("KN", "St Kitts & Nevis"),
    ("LC", "St Lucia"),
    ("VC", "St Vincent"),
    ("SD", "Sudan"),
    ("SR", "Suriname"),
    ("SE", "Sweden"),
    ("CH", "Switzerland"),
    ("SY", "Syria"),
    ("TW", "Taiwan"),
    ("TJ", "Tajikistan"),
    ("TZ", "Tanzania"),
    ("TH", "Thailand"),
    ("TG", "Togo"),
    ("TO", "Tonga"),
    ("TT", "Trinidad & Tobago"),
    ("TN", "Tunisia"),
    ("TR", "Turkey"),
    ("TM", "Turkmenistan"),
    ("TC", "Turks & Caicos Is"),
    ("TV", "Tuvalu"),
    ("UG", "Uganda"),
    ("UA", "Ukraine"),
    ("AE", "United Arab Emirates"),
    ("US", "United States"),
    ("UY", "Uruguay"),
    ("UZ", "Uzbekistan"),
    ("VU", "Vanuatu"),
    ("VA", "Vatican City"),
    ("VE", "Venezuela"),
    ("VN", "Vietnam"),
    ("VG", "Virgin Islands (UK)"),
    ("VI", "Virgin Islands (US)"),
    ("EH", "Western Sahara"),
    ("YE", "Yemen"),
    ("ZM", "Zambia"),
    ("ZW", "Zimbabwe"),
];
