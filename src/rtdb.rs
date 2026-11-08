//! Matchmaking and messaging over the Firebase Realtime Database, the way the Android app does
//! it (`DrDrivingActivity`), spoken through the database's REST API so it runs on every
//! platform (native and browser) without a Firebase SDK. The app uses no Firebase
//! Authentication; neither does this.
//!
//! ```text
//! Waiting/<w>            { channel, status: 0|1, room }   open lobbies (push-id keys)
//! Gaming/<room>/Live     "yyyyMMddHHmmss"                 room exists; deleted = player left
//! Gaming/<room>/ToCreator/<n>   "<message>"               mailbox of the creator
//! Gaming/<room>/ToJoiner/<n>    "<message>"               mailbox of the joiner
//! ```
//!
//! * **Search** (`searchRoom`): list `Waiting`, take the first lobby with `status == 0` on our
//!   channel and claim it with a conditional write of `status` (0 -> 1, the transaction is an
//!   ETag compare-and-swap). Lost the race, or no lobby: create `Gaming/<room>` and post a lobby.
//! * **Messages**: the sender writes `<n> = "<text>"` into the peer's mailbox, the receiver
//!   reads its mailbox, handles the children in order and deletes them.
//! * **Presence**: deleting `Gaming/<room>` (what leaving does) makes `Live` vanish, which is
//!   how the other side learns that the player left.
//!
//! The database is reached through a [`Backend`] (an asynchronous HTTP client): the game crate
//! provides one for native builds and one for the browser, [`mem`] is an in-memory fake.
//! Nothing in here blocks: [`Connection::pump`] is called once per frame.

use crate::net::{self, Message};
use crate::netplay::{CloseReason, Event as NetEvent, Match, Profile, Remote, Stage};
use crate::rng::JavaRandom;
use crate::save::SaveGame;
use crate::session::{Event as RaceEvent, Session};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

/// The original game's database. Override it (own Firebase project, or a local test server)
/// with `DRDRIVING_DB` on desktop or `?db=` in the page URL.
pub const DEFAULT_DB: &str = "https://crucial-bonsai-821.firebaseio.com";

/// Position updates are sent every this many simulation frames (60 Hz -> 20 Hz).
pub const MOVE_EVERY: u32 = 3;

/// Without a single successful request for this long the connection counts as lost.
const NET_LOST: f32 = 10.0;
/// Gap between mailbox requests once the peer is known (two may be in flight, so updates are not
/// limited to one per round trip) / while waiting for one in a lobby.
const POLL_ACTIVE: f32 = 0.12;
const POLL_LOBBY: f32 = 0.25;
/// A joiner that hears nothing from the room it claimed for this long has hit an abandoned lobby
/// (creators answer within a second or two) and moves on to the next one.
const NO_ANSWER: f32 = 6.0;
/// Lobbies older than this are ghosts of players who quit: nobody waits that long (the original
/// gives up after 40 s). Measured with the local clock; if that is badly off the "no answer" rule
/// above still protects.
const LOBBY_MAX_AGE_MS: u64 = 120_000;
/// A creator looks at the lobby list this often, to join a lobby opened a moment before its own.
const RELIST_EVERY: f32 = 2.5;
/// How often a room's `Live` node is looked at.
const LIVE_EVERY: f32 = 2.0;
/// Lost lobby claims in a row before giving up searching and opening a lobby ourselves.
const MAX_CLAIM_LOSSES: u32 = 6;

// ------------------------------------------------------------------------------- backend

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Put,
    Patch,
    Delete,
}

#[derive(Clone, Debug)]
pub struct Request {
    pub method: Method,
    /// Database path without `.json`, e.g. `Gaming/-Nabc/ToJoiner`.
    pub path: String,
    /// JSON text.
    pub body: Option<String>,
    /// GET: also report the ETag of the value (for a following conditional write).
    pub want_etag: bool,
    /// PUT: only write if the value still has this ETag; otherwise the answer is HTTP 412.
    pub if_match: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Response {
    /// HTTP status; `0` = the request never got an answer (offline, DNS, TLS, CORS ...).
    pub status: u16,
    pub body: String,
    pub etag: Option<String>,
}

impl Response {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// An asynchronous HTTP client bound to one database URL. Requests are started, and their
/// answers collected later with [`Backend::poll`]; both must return immediately.
pub trait Backend {
    fn start(&mut self, req: Request) -> u64;
    fn poll(&mut self) -> Vec<(u64, Response)>;
}

fn req(method: Method, path: impl Into<String>, body: Option<String>) -> Request {
    Request { method, path: path.into(), body, want_etag: false, if_match: None }
}

// ------------------------------------------------------------------------------- helpers

const PUSH_CHARS: &[u8; 64] = b"-0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmnopqrstuvwxyz";

/// A Firebase push id: 8 characters of time (so ids sort chronologically) + 12 random ones.
pub fn push_id(unix_ms: u64, rng: &mut JavaRandom) -> String {
    let mut s = [0u8; 20];
    let mut t = unix_ms;
    for i in (0..8).rev() {
        s[i] = PUSH_CHARS[(t % 64) as usize];
        t /= 64;
    }
    for c in s.iter_mut().skip(8) {
        *c = PUSH_CHARS[rng.next_int_bound(64) as usize];
    }
    String::from_utf8(s.to_vec()).unwrap_or_default()
}

/// The children of a mailbox as `(counter, text)`, sorted by counter. The REST API returns
/// either an object or - when the keys look like array indices - an array with `null` holes.
fn mailbox_entries(v: &Value) -> Vec<(i64, String)> {
    let mut out: Vec<(i64, String)> = match v {
        Value::Object(m) => m.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.as_str()?.to_string()))).collect(),
        Value::Array(a) => a.iter().enumerate().filter_map(|(i, v)| Some((i as i64, v.as_str()?.to_string()))).collect(),
        _ => Vec::new(),
    };
    out.sort_by_key(|e| e.0);
    out
}

/// Creation time (unix ms) encoded in the first 8 characters of a push id.
fn push_time(key: &str) -> Option<u64> {
    if key.len() < 8 {
        return None;
    }
    let mut t = 0u64;
    for c in key.bytes().take(8) {
        t = t * 64 + PUSH_CHARS.iter().position(|&x| x == c)? as u64;
    }
    Some(t)
}

/// Not older than [`LOBBY_MAX_AGE_MS`] (keys that are not push ids count as fresh).
fn fresh(key: &str, now_ms: u64) -> bool {
    push_time(key).map_or(true, |t| now_ms.saturating_sub(t) <= LOBBY_MAX_AGE_MS)
}

/// Open lobbies on `channel` as `(key, room)`, oldest first (push ids sort by time).
fn open_lobbies(waiting: &Value, channel: i32, skip: &HashSet<String>) -> Vec<(String, String)> {
    let Some(map) = waiting.as_object() else { return Vec::new() };
    let mut out: Vec<(String, String)> = map
        .iter()
        .filter_map(|(key, e)| {
            let open = e.get("status").and_then(Value::as_f64) == Some(0.0);
            let same = e.get("channel").and_then(Value::as_f64) == Some(channel as f64);
            let room = e.get("room").and_then(Value::as_str)?;
            (open && same && !skip.contains(room)).then(|| (key.clone(), room.to_string()))
        })
        .collect();
    out.sort();
    out
}

/// The lobby to claim: the newest open one on our channel that is not a ghost. (The original takes
/// the oldest, which is the likeliest to be abandoned.)
fn pick_lobby(waiting: &Value, channel: i32, skip: &HashSet<String>, now_ms: u64) -> Option<(String, String)> {
    open_lobbies(waiting, channel, skip).into_iter().rev().find(|(k, _)| fresh(k, now_ms))
}

/// For a waiting creator: an open, fresh lobby that was created before ours (two players who
/// searched at the same moment each made a lobby; the younger one joins the older one).
fn pick_older(waiting: &Value, channel: i32, mine: &str, skip: &HashSet<String>, now_ms: u64) -> Option<(String, String)> {
    open_lobbies(waiting, channel, skip).into_iter().find(|(k, _)| k.as_str() < mine && fresh(k, now_ms))
}

fn http_error(status: u16) -> String {
    match status {
        0 => "Cannot reach the multiplayer server".into(),
        401 | 403 => format!("The multiplayer server refused access (HTTP {status})"),
        404 => "Multiplayer database not found (HTTP 404)".into(),
        s => format!("Multiplayer server error (HTTP {s})"),
    }
}

// ------------------------------------------------------------------------------- connection

/// Where the matchmaking handshake is. Every variant that waits for an answer carries its
/// request id.
enum Step {
    /// `GET Waiting`.
    Listing(u64),
    /// Reading `Waiting/<w>/status` (with its ETag), then writing 1 if it was 0.
    ClaimRead { req: u64, lobby: String, room: String },
    ClaimWrite { req: u64, lobby: String, room: String },
    /// `PUT Gaming/<room>`.
    MakeRoom(u64),
    /// `PUT Waiting/<lobby>`.
    PostLobby(u64),
    /// In a room (creator waiting for the joiner, or a running match).
    InRoom,
    /// A matchmaking request failed in a way that is worth repeating: do it again at `at`.
    Retry { at: f32, resume: Resume },
    Dead,
}

/// Which matchmaking request a [`Step::Retry`] repeats (the request itself is `stage_req`).
#[derive(Clone)]
enum Resume {
    Listing,
    ClaimRead { lobby: String, room: String },
    MakeRoom,
    PostLobby,
}

/// Network hiccups and server overload: asking again may work. A rejected request (4xx) will not.
fn transient(status: u16) -> bool {
    status == 0 || status == 429 || status >= 500
}

/// Matchmaking requests repeated this often in total before the connection gives up.
const MAX_RETRIES: u32 = 6;

#[derive(Default)]
struct Mail {
    pending: Vec<Message>,
    send_key: i64,
    /// Writes in flight (at most two) with what they carry, and batches to send again.
    sends: Vec<(u64, Vec<(i64, String)>)>,
    retry: Vec<Vec<(i64, String)>>,
    send_due: f32,
    /// Mailbox reads in flight (at most two).
    msgs: Vec<u64>,
    msg_due: f32,
    seen: HashSet<i64>,
    to_delete: Vec<i64>,
    del_req: Option<(u64, Vec<i64>)>,
    live_req: Option<u64>,
    live_due: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Looking for an open lobby / creating one.
    Searching,
    /// Our lobby is posted; nobody has claimed it yet.
    Waiting,
    /// An opponent is there; versions and profiles are being exchanged.
    Connecting,
    InMatch,
    Closed,
}

pub struct Connection {
    be: Box<dyn Backend>,
    pub m: Match,
    pub channel: i32,
    profile: Profile,
    seed: i64,
    friendly: bool,
    resets: i64,
    joined_at: f32,
    relist_req: Option<u64>,
    relist_due: f32,
    clock: f32,
    unix_ms: u64,
    rng: JavaRandom,
    step: Step,
    room: Option<String>,
    lobby: Option<String>,
    skipped: HashSet<String>,
    claim_losses: u32,
    stage_req: Option<Request>,
    retries: u32,
    mail: Mail,
    last_ok: f32,
    frame: u32,
    error: Option<String>,
}

impl Connection {
    /// Start looking for an opponent on `channel` (see [`net::quick_channel`] /
    /// [`net::friend_channel`]). Channels from [`net::FIRST_FRIEND_CHANNEL`] up are friendly
    /// matches that never change the rating. `unix_ms` is the current time (used for ids and the
    /// `Live` stamp; the core has no clock of its own, which keeps it usable in the browser).
    pub fn new(backend: Box<dyn Backend>, channel: i32, profile: Profile, seed: i64, unix_ms: u64) -> Connection {
        let friendly = channel >= net::FIRST_FRIEND_CHANNEL;
        let mut c = Connection {
            be: backend,
            m: Match::new(profile.clone(), seed, friendly),
            channel,
            profile,
            seed,
            friendly,
            resets: 0,
            joined_at: 0.0,
            relist_req: None,
            relist_due: 0.0,
            clock: 0.0,
            unix_ms,
            rng: JavaRandom::new(seed ^ 0x5DEE_CE66),
            step: Step::Dead,
            room: None,
            lobby: None,
            skipped: HashSet::new(),
            claim_losses: 0,
            stage_req: None,
            retries: 0,
            mail: Mail::default(),
            last_ok: 0.0,
            frame: 0,
            error: None,
        };
        c.step = c.list();
        c
    }

    pub fn status(&self) -> Status {
        if self.m.is_closed() || matches!(self.step, Step::Dead) {
            return Status::Closed;
        }
        match self.step {
            Step::InRoom => match self.m.stage {
                Stage::Lobby => Status::Waiting,
                Stage::Handshake | Stage::Profile | Stage::Course if self.m.course.is_none() => Status::Connecting,
                _ => Status::InMatch,
            },
            _ => Status::Searching,
        }
    }

    /// Why the connection ended, when the cause was the network or the server.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn close_reason(&self) -> Option<CloseReason> {
        self.m.close_reason()
    }

    /// The `Gaming` key of the room we are in.
    pub fn room_key(&self) -> Option<&str> {
        self.room.as_deref()
    }

    fn now_ms(&self) -> u64 {
        self.unix_ms + (self.clock * 1000.0) as u64
    }

    fn call(&mut self, r: Request) -> u64 {
        self.be.start(r)
    }

    /// A matchmaking request: remembered so that it can be repeated.
    fn stage_call(&mut self, r: Request) -> u64 {
        self.stage_req = Some(r.clone());
        self.call(r)
    }

    /// The request failed: repeat it shortly when that can help, else end the connection.
    fn retry_or_fail(&mut self, r: &Response, resume: Resume, ev: &mut Vec<NetEvent>) -> Step {
        if transient(r.status) && self.retries < MAX_RETRIES {
            self.retries += 1;
            Step::Retry { at: self.clock + 0.5 * self.retries as f32, resume }
        } else {
            self.fail(http_error(r.status), ev)
        }
    }

    // ---------------------------------------------------------------------- matchmaking

    fn list(&mut self) -> Step {
        Step::Listing(self.stage_call(req(Method::Get, "Waiting", None)))
    }

    /// `FirebaseCreateRoom`: make the room, then post a lobby pointing at it.
    fn make_room(&mut self) -> Step {
        let room = push_id(self.now_ms(), &mut self.rng);
        let live = net::live_stamp((self.now_ms() / 1000) as i64);
        self.room = Some(room.clone());
        let body = json!({ "Live": live }).to_string();
        Step::MakeRoom(self.stage_call(req(Method::Put, format!("Gaming/{room}"), Some(body))))
    }

    fn fail(&mut self, why: String, ev: &mut Vec<NetEvent>) -> Step {
        self.error = Some(why);
        ev.extend(self.m.on_link_closed());
        self.cleanup();
        Step::Dead
    }

    fn claim_lost(&mut self, room: String) -> Step {
        self.skipped.insert(room);
        self.claim_losses += 1;
        if self.claim_losses >= MAX_CLAIM_LOSSES {
            self.make_room()
        } else {
            self.list()
        }
    }

    fn claimed(&mut self, lobby: String, room: String) -> Step {
        // the lobby is ours: remove it from the list and say hello
        self.call(req(Method::Delete, format!("Waiting/{lobby}"), None));
        self.room = Some(room);
        self.m.joined();
        self.joined_at = self.clock;
        self.last_ok = self.clock;
        self.mail.live_due = self.clock + LIVE_EVERY;
        Step::InRoom
    }

    fn on_response(&mut self, id: u64, r: Response, ev: &mut Vec<NetEvent>) {
        let step = std::mem::replace(&mut self.step, Step::Dead);
        self.step = match step {
            Step::Listing(q) if q == id => {
                if !r.ok() {
                    self.retry_or_fail(&r, Resume::Listing, ev)
                } else {
                    let v: Value = serde_json::from_str(&r.body).unwrap_or(Value::Null);
                    match pick_lobby(&v, self.channel, &self.skipped, self.now_ms()) {
                        Some((lobby, room)) => {
                            let q = Request { want_etag: true, ..req(Method::Get, format!("Waiting/{lobby}/status"), None) };
                            Step::ClaimRead { req: self.stage_call(q), lobby, room }
                        }
                        None => self.make_room(),
                    }
                }
            }
            Step::ClaimRead { req: q, lobby, room } if q == id => {
                let open = r.ok() && r.body.trim().parse::<f64>() == Ok(0.0);
                if !r.ok() && transient(r.status) {
                    self.retry_or_fail(&r, Resume::ClaimRead { lobby, room }, ev)
                } else if !open {
                    // taken by someone else, withdrawn, or unreadable
                    self.claim_lost(room)
                } else {
                    // the original runs a transaction: write 1 only if it is still 0. Without an
                    // ETag (a proxy that strips it) fall back to a plain write.
                    let q = Request { if_match: r.etag.clone(), ..req(Method::Put, format!("Waiting/{lobby}/status"), Some("1".into())) };
                    Step::ClaimWrite { req: self.stage_call(q), lobby, room }
                }
            }
            Step::ClaimWrite { req: q, lobby, room } if q == id => {
                if r.ok() {
                    self.claimed(lobby, room)
                } else if r.status == 412 || r.status == 0 || r.status >= 500 {
                    self.claim_lost(room)
                } else {
                    let why = http_error(r.status);
                    self.fail(why, ev)
                }
            }
            Step::MakeRoom(q) if q == id => {
                if !r.ok() {
                    self.retry_or_fail(&r, Resume::MakeRoom, ev)
                } else {
                    let lobby = push_id(self.now_ms(), &mut self.rng);
                    let room = self.room.clone().unwrap_or_default();
                    let body = json!({ "channel": self.channel, "status": 0, "room": room }).to_string();
                    self.lobby = Some(lobby.clone());
                    Step::PostLobby(self.stage_call(req(Method::Put, format!("Waiting/{lobby}"), Some(body))))
                }
            }
            Step::PostLobby(q) if q == id => {
                if !r.ok() {
                    self.retry_or_fail(&r, Resume::PostLobby, ev)
                } else {
                    self.last_ok = self.clock;
                    self.mail.live_due = self.clock + LIVE_EVERY;
                    self.mail.msg_due = self.clock + POLL_LOBBY;
                    Step::InRoom
                }
            }
            Step::InRoom => self.mail_response(id, r, ev).unwrap_or(Step::InRoom),
            other => other, // an answer to something fire-and-forget
        };
    }

    // ---------------------------------------------------------------------- mailbox

    fn inbox(&self) -> &'static str {
        if self.m.role == crate::netplay::Role::Creator { "ToCreator" } else { "ToJoiner" }
    }

    fn outbox(&self) -> &'static str {
        if self.m.role == crate::netplay::Role::Creator { "ToJoiner" } else { "ToCreator" }
    }

    /// Start over with a fresh match (after giving up a lobby).
    fn reset_match(&mut self) {
        self.resets += 1;
        self.m = Match::new(self.profile.clone(), self.seed.wrapping_add(self.resets), self.friendly);
        self.mail = Mail::default();
        self.relist_req = None;
        self.relist_due = self.clock + RELIST_EVERY;
    }

    /// Leave our own lobby / room (not the connection) and claim someone else's lobby instead.
    fn merge_into(&mut self, lobby: String, room: String) -> Step {
        if let Some(l) = self.lobby.take() {
            self.call(req(Method::Delete, format!("Waiting/{l}"), None));
        }
        if let Some(r) = self.room.take() {
            self.call(req(Method::Delete, format!("Gaming/{r}"), None));
        }
        self.reset_match();
        let q = Request { want_etag: true, ..req(Method::Get, format!("Waiting/{lobby}/status"), None) };
        Step::ClaimRead { req: self.stage_call(q), lobby, room }
    }

    /// The room we joined never answered: forget it (and clear it away) and look for another lobby.
    fn abandon_room_and_search(&mut self) {
        if let Some(r) = self.room.take() {
            self.skipped.insert(r.clone());
            self.call(req(Method::Delete, format!("Gaming/{r}"), None));
        }
        self.reset_match();
        self.claim_losses = 0;
        self.step = self.list();
    }

    /// Handle the answer to a mailbox request. `Some(step)` = the connection moved on (it ended, or
    /// it switched to another lobby).
    fn mail_response(&mut self, id: u64, r: Response, ev: &mut Vec<NetEvent>) -> Option<Step> {
        let fatal = matches!(r.status, 401 | 403);
        if r.ok() {
            self.last_ok = self.clock;
        }
        if let Some(pos) = self.mail.msgs.iter().position(|&q| q == id) {
            self.mail.msgs.remove(pos);
            if r.ok() {
                let v: Value = serde_json::from_str(&r.body).unwrap_or(Value::Null);
                for (n, text) in mailbox_entries(&v) {
                    if !self.mail.seen.insert(n) {
                        continue; // already handled; its deletion is on the way
                    }
                    self.mail.to_delete.push(n);
                    // undecodable messages are ignored: a stray one must not end the match
                    if let Ok(msg) = Message::decode(&text) {
                        ev.extend(self.m.handle(msg));
                    }
                }
            } else if fatal {
                return Some(self.fail(http_error(r.status), ev));
            } else {
                self.mail.msg_due = self.clock + 0.4;
            }
        } else if self.mail.live_req == Some(id) {
            self.mail.live_req = None;
            self.mail.live_due = self.clock + LIVE_EVERY;
            if r.ok() && r.body.trim() == "null" {
                // our room is gone: the other player left (or it never existed)
                ev.extend(self.m.on_link_closed());
            } else if fatal {
                return Some(self.fail(http_error(r.status), ev));
            }
        } else if let Some(pos) = self.mail.sends.iter().position(|s| s.0 == id) {
            let (_, batch) = self.mail.sends.remove(pos);
            if !r.ok() {
                if fatal {
                    return Some(self.fail(http_error(r.status), ev));
                }
                self.mail.retry.push(batch);
                self.mail.send_due = self.clock + 0.25;
            }
        } else if self.mail.del_req.as_ref().map(|s| s.0) == Some(id) {
            let (_, keys) = self.mail.del_req.take().unwrap();
            if !r.ok() && !fatal {
                self.mail.to_delete.extend(keys);
            }
        } else if self.relist_req == Some(id) {
            self.relist_req = None;
            self.relist_due = self.clock + RELIST_EVERY;
            if r.ok() && self.m.role == crate::netplay::Role::Creator && self.m.stage == Stage::Lobby {
                if let Some(mine) = self.lobby.clone() {
                    let v: Value = serde_json::from_str(&r.body).unwrap_or(Value::Null);
                    if let Some((lobby, room)) = pick_older(&v, self.channel, &mine, &self.skipped, self.now_ms()) {
                        return Some(self.merge_into(lobby, room));
                    }
                }
            }
        }
        None
    }

    /// Hand the match's outgoing messages to the network and start whatever poll is due.
    fn service(&mut self) {
        if !matches!(self.step, Step::InRoom) || self.m.is_closed() {
            return;
        }
        let Some(room) = self.room.clone() else { return };
        let out = self.m.take_outbox();
        self.mail.pending.extend(out);

        // writes: up to two in flight, so updates are not limited to one per round trip
        while self.mail.sends.len() < 2 && self.clock >= self.mail.send_due {
            let Some(batch) = self.mail.retry.pop().or_else(|| self.next_batch()) else { break };
            let body: Map<String, Value> = batch.iter().map(|(k, t)| (k.to_string(), Value::String(t.clone()))).collect();
            let path = format!("Gaming/{room}/{}", self.outbox());
            let id = self.call(req(Method::Patch, path, Some(Value::Object(body).to_string())));
            self.mail.sends.push((id, batch));
        }
        // reads of our mailbox: two overlapping ones once a race is on
        let waiting = self.m.stage == Stage::Lobby;
        let (max_reads, gap) = if waiting { (1, POLL_LOBBY) } else { (2, POLL_ACTIVE) };
        if self.mail.msgs.len() < max_reads && self.clock >= self.mail.msg_due {
            let id = self.call(req(Method::Get, format!("Gaming/{room}/{}", self.inbox()), None));
            self.mail.msgs.push(id);
            self.mail.msg_due = self.clock + gap;
        }
        if self.mail.del_req.is_none() && !self.mail.to_delete.is_empty() {
            let keys = std::mem::take(&mut self.mail.to_delete);
            let body: Map<String, Value> = keys.iter().map(|k| (k.to_string(), Value::Null)).collect();
            let id = self.call(req(Method::Patch, format!("Gaming/{room}/{}", self.inbox()), Some(Value::Object(body).to_string())));
            self.mail.del_req = Some((id, keys));
        }
        if self.mail.live_req.is_none() && self.clock >= self.mail.live_due {
            let id = self.call(req(Method::Get, format!("Gaming/{room}/Live"), None));
            self.mail.live_req = Some(id);
        }
        // a creator that is still alone keeps an eye on the list of lobbies
        if waiting && self.lobby.is_some() && self.relist_req.is_none() && self.clock >= self.relist_due {
            self.relist_req = Some(self.call(req(Method::Get, "Waiting", None)));
        }
    }

    /// Everything queued so far as one write. Of several position updates only the newest is
    /// worth sending; the message counters stay contiguous.
    fn next_batch(&mut self) -> Option<Vec<(i64, String)>> {
        if self.mail.pending.is_empty() {
            return None;
        }
        let last_move = self.mail.pending.iter().rposition(|m| matches!(m, Message::Move { .. }));
        let pending = std::mem::take(&mut self.mail.pending);
        let mut batch = Vec::new();
        let mut dropped = 0;
        for (i, m) in pending.iter().enumerate() {
            if matches!(m, Message::Move { .. }) && Some(i) != last_move {
                dropped += 1;
                continue;
            }
            batch.push((self.mail.send_key, m.encode()));
            self.mail.send_key += 1;
        }
        // updates that are never sent must not count as sent, or the send throttle (which compares
        // with what the peer has received) would cut everything off after ~20 s of racing
        if dropped > 0 {
            self.m.forget_sent(dropped);
        }
        Some(batch)
    }

    // ---------------------------------------------------------------------- public driving

    /// Advance the connection. `dt` is the time since the previous call (seconds). Call it every
    /// frame; it never blocks.
    pub fn pump(&mut self, dt: f32) -> Vec<NetEvent> {
        let mut ev = Vec::new();
        if matches!(self.step, Step::Dead) {
            return ev;
        }
        self.clock += dt;
        for (id, r) in self.be.poll() {
            self.on_response(id, r, &mut ev);
            if matches!(self.step, Step::Dead) {
                break;
            }
        }
        if let Step::Retry { at, .. } = &self.step {
            if self.clock >= *at {
                if let (Step::Retry { resume, .. }, Some(q)) = (std::mem::replace(&mut self.step, Step::Dead), self.stage_req.clone()) {
                    let id = self.call(q);
                    self.step = match resume {
                        Resume::Listing => Step::Listing(id),
                        Resume::ClaimRead { lobby, room } => Step::ClaimRead { req: id, lobby, room },
                        Resume::MakeRoom => Step::MakeRoom(id),
                        Resume::PostLobby => Step::PostLobby(id),
                    };
                }
            }
        }
        if matches!(self.step, Step::InRoom) {
            ev.extend(self.m.tick(dt));
            if self.m.role == crate::netplay::Role::Joiner
                && self.m.stage == Stage::Handshake
                && !self.m.heard()
                && self.clock - self.joined_at > NO_ANSWER
            {
                // nobody is home: an abandoned lobby. Try the next one.
                self.abandon_room_and_search();
            } else {
                if self.clock - self.last_ok > NET_LOST {
                    self.error = Some("Connection to the server lost".into());
                    ev.extend(self.m.on_link_closed());
                }
                self.service();
            }
        }
        if self.m.is_closed() {
            self.cleanup();
        }
        ev
    }

    /// Delete our lobby and room (what `leaveRoom` does) and stop. Idempotent.
    fn cleanup(&mut self) {
        if let Some(l) = self.lobby.take() {
            self.call(req(Method::Delete, format!("Waiting/{l}"), None));
        }
        if let Some(r) = self.room.take() {
            self.call(req(Method::Delete, format!("Gaming/{r}"), None));
        }
        self.step = Step::Dead;
    }

    /// Leave: the other player finds out because the room disappears.
    pub fn leave(&mut self) {
        self.m.leave();
        self.cleanup();
    }

    /// Before `Session::frame` of an online race: apply the opponent's latest state and result.
    /// Returns the race events caused by the opponent finishing / leaving.
    pub fn pre_frame(&mut self, s: &mut Session, save: &mut SaveGame, dt: f32) -> Vec<RaceEvent> {
        let mut race = Vec::new();
        for e in self.pump(dt) {
            match e {
                NetEvent::Opponent(r) => s.world.apply_opponent_update(r.time, r.mission_distance, r.cx, r.cy, r.vx, r.vy, r.angle),
                NetEvent::OpponentFinished { opponent_won } => race.extend(s.opponent_finished(opponent_won, save)),
                // the opponent vanishing mid-race forfeits it
                NetEvent::Closed(_) => race.extend(s.opponent_finished(false, save)),
                _ => {}
            }
        }
        race
    }

    /// After `Session::frame`: send our position (20 Hz) and our result, if it just happened.
    pub fn post_frame(&mut self, s: &mut Session) {
        self.frame = self.frame.wrapping_add(1);
        if let Some(won) = s.local_finish.take() {
            self.m.finish(won);
        } else if self.frame % MOVE_EVERY == 0 && self.m.stage == Stage::Racing {
            let p = &s.world.player;
            self.m.send_move(Remote {
                time: s.world.travel_time,
                mission_distance: p.mission_distance,
                cx: p.cx,
                cy: p.cy,
                vx: p.v.x,
                vy: p.v.y,
                angle: p.angle,
            });
        }
        self.service();
    }

    /// Give up the running race (counts as a loss for us).
    pub fn forfeit(&mut self, s: &mut Session, save: &mut SaveGame) -> Vec<RaceEvent> {
        let ev = s.forfeit(save);
        if let Some(won) = s.local_finish.take() {
            self.m.finish(won);
            self.service();
        }
        ev
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.cleanup();
    }
}

// ------------------------------------------------------------------------------- fake database

/// An in-memory Realtime Database with the REST semantics this module relies on (PUT / PATCH /
/// DELETE, `null` = delete, empty objects vanish, array-looking objects come back as arrays,
/// ETags with `if-match`). Requests are answered on the next [`Backend::poll`]. Used by the
/// tests; also handy for trying the game offline.
pub mod mem {
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    pub struct MemDb {
        tree: Arc<Mutex<Value>>,
        fault: Arc<Mutex<Option<u16>>>,
        log: Arc<Mutex<Vec<String>>>,
    }

    fn segs(path: &str) -> Vec<&str> {
        path.split('/').filter(|s| !s.is_empty()).collect()
    }

    fn empty(v: &Value) -> bool {
        v.is_null() || v.as_object().map_or(false, |m| m.is_empty())
    }

    fn strip_nulls(v: Value) -> Value {
        match v {
            Value::Object(m) => {
                let m: Map<String, Value> = m.into_iter().map(|(k, v)| (k, strip_nulls(v))).filter(|(_, v)| !empty(v)).collect();
                Value::Object(m)
            }
            other => other,
        }
    }

    fn set_at(node: &mut Value, path: &[&str], v: Value) {
        if path.is_empty() {
            *node = strip_nulls(v);
            return;
        }
        if !node.is_object() {
            if empty(&v) {
                return;
            }
            *node = Value::Object(Map::new());
        }
        let map = node.as_object_mut().unwrap();
        let child = map.entry(path[0].to_string()).or_insert(Value::Null);
        set_at(child, &path[1..], v);
        if empty(child) {
            map.remove(path[0]);
        }
    }

    fn get_at<'a>(node: &'a Value, path: &[&str]) -> &'a Value {
        let mut cur = node;
        for p in path {
            match cur.get(*p) {
                Some(v) => cur = v,
                None => return &Value::Null,
            }
        }
        cur
    }

    /// The REST API shows objects whose keys are mostly 0..n as arrays.
    fn arrayify(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let idx: Option<Vec<usize>> = m.keys().map(|k| k.parse::<usize>().ok()).collect();
                if let Some(idx) = idx {
                    let max = idx.iter().copied().max().unwrap_or(0);
                    if !idx.is_empty() && max < 2 * idx.len() {
                        let mut a = vec![Value::Null; max + 1];
                        for (k, v) in m {
                            a[k.parse::<usize>().unwrap()] = arrayify(v);
                        }
                        return Value::Array(a);
                    }
                }
                Value::Object(m.iter().map(|(k, v)| (k.clone(), arrayify(v))).collect())
            }
            other => other.clone(),
        }
    }

    fn etag(v: &Value) -> String {
        let mut h = DefaultHasher::new();
        v.to_string().hash(&mut h);
        format!("{:016x}", h.finish())
    }

    impl MemDb {
        pub fn new() -> MemDb {
            MemDb { tree: Arc::new(Mutex::new(Value::Null)), ..Default::default() }
        }

        pub fn backend(&self) -> MemBackend {
            self.backend_with_latency(0)
        }

        /// A backend whose answers arrive `polls` calls of [`Backend::poll`] late (a round trip),
        /// like a real network; writes still take effect at once.
        pub fn backend_with_latency(&self, polls: u32) -> MemBackend {
            MemBackend { db: self.clone(), next: 1, ready: Vec::new(), cut: Arc::new(AtomicBool::new(false)), latency: polls }
        }

        /// Everything stored at `path`, as the REST API would return it.
        pub fn read(&self, path: &str) -> Value {
            let t = self.tree.lock().unwrap();
            get_at(&t, &segs(path)).clone()
        }

        /// Write directly (what another client would do).
        pub fn write(&self, path: &str, v: Value) {
            let mut t = self.tree.lock().unwrap();
            set_at(&mut t, &segs(path), v);
        }

        /// Answer every request with this HTTP status (`Some(0)` = network error, `Some(401)`
        /// = rules deny access) until reset with `None`.
        pub fn set_fault(&self, status: Option<u16>) {
            *self.fault.lock().unwrap() = status;
        }

        /// `METHOD path body` of every request seen so far.
        pub fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }

        fn handle(&self, r: &Request) -> Response {
            self.log.lock().unwrap().push(format!("{:?} {} {}", r.method, r.path, r.body.clone().unwrap_or_default()));
            if let Some(status) = *self.fault.lock().unwrap() {
                return Response { status, ..Default::default() };
            }
            let mut t = self.tree.lock().unwrap();
            let path = segs(&r.path);
            let here = |t: &Value| get_at(t, &path).clone();
            match r.method {
                Method::Get => {
                    let v = here(&t);
                    Response { status: 200, body: arrayify(&v).to_string(), etag: r.want_etag.then(|| etag(&v)) }
                }
                Method::Put => {
                    if let Some(want) = &r.if_match {
                        if *want != etag(&here(&t)) {
                            return Response { status: 412, ..Default::default() };
                        }
                    }
                    let v = serde_json::from_str(r.body.as_deref().unwrap_or("null")).unwrap_or(Value::Null);
                    set_at(&mut t, &path, v);
                    Response { status: 200, body: "null".into(), etag: None }
                }
                Method::Patch => {
                    let v: Value = serde_json::from_str(r.body.as_deref().unwrap_or("{}")).unwrap_or(Value::Null);
                    let Value::Object(m) = v else { return Response { status: 400, ..Default::default() } };
                    for (k, v) in m {
                        let mut p = path.clone();
                        p.push(&k);
                        set_at(&mut t, &p, v);
                    }
                    Response { status: 200, body: "null".into(), etag: None }
                }
                Method::Delete => {
                    set_at(&mut t, &path, Value::Null);
                    Response { status: 200, body: "null".into(), etag: None }
                }
            }
        }
    }

    pub struct MemBackend {
        db: MemDb,
        next: u64,
        /// Answers waiting for their delay (in polls) to run out.
        ready: Vec<(u64, Response, u32)>,
        cut: Arc<AtomicBool>,
        latency: u32,
    }

    impl MemBackend {
        /// A switch that makes this client (only) lose its network: requests are dropped and
        /// answered with status 0.
        pub fn network_switch(&self) -> Arc<AtomicBool> {
            self.cut.clone()
        }
    }

    impl Backend for MemBackend {
        fn start(&mut self, r: Request) -> u64 {
            let id = self.next;
            self.next += 1;
            let resp = if self.cut.load(Ordering::Relaxed) { Response::default() } else { self.db.handle(&r) };
            self.ready.push((id, resp, self.latency));
            id
        }

        fn poll(&mut self) -> Vec<(u64, Response)> {
            let mut out = Vec::new();
            let mut waiting = Vec::new();
            for (id, resp, left) in std::mem::take(&mut self.ready) {
                if left == 0 {
                    out.push((id, resp));
                } else {
                    waiting.push((id, resp, left - 1));
                }
            }
            self.ready = waiting;
            out
        }
    }
}
