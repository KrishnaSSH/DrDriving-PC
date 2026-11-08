//! Multiplayer: the protocol state machine in memory, matchmaking and messaging against an
//! in-memory Firebase Realtime Database, a scripted stand-in for the original Android client,
//! and two complete races.

use drdriving_core::economy::Outcome;
use drdriving_core::mission::Mission;
use drdriving_core::net::{self, Message, PROTOCOL_VERSION};
use drdriving_core::netplay::*;
use drdriving_core::rtdb::mem::MemDb;
use drdriving_core::rtdb::{Connection, Status};
use drdriving_core::save::SaveGame;
use drdriving_core::session::{Input, Phase, Session};
use drdriving_core::world::World;
use serde_json::{json, Value};

const NOW: u64 = 1_800_000_000_000;

fn profile(name: &str, car: i32) -> Profile {
    Profile { rating: 1200, car, country: 3, name: name.into() }
}

// ---------------------------------------------------------------------------- state machine

/// Deliver everything each side queued to the other until both are quiet.
fn shuttle(a: &mut Match, b: &mut Match) -> (Vec<Event>, Vec<Event>) {
    let (mut ea, mut eb) = (Vec::new(), Vec::new());
    for _ in 0..20 {
        let (oa, ob) = (a.take_outbox(), b.take_outbox());
        if oa.is_empty() && ob.is_empty() {
            break;
        }
        for m in oa {
            // everything goes through the text encoding, exactly as on the wire
            eb.extend(b.handle(Message::decode(&m.encode()).unwrap()));
        }
        for m in ob {
            ea.extend(a.handle(Message::decode(&m.encode()).unwrap()));
        }
    }
    (ea, eb)
}

/// (creator, joiner) after the lobby claim.
fn pair() -> (Match, Match) {
    let c = Match::new(profile("Creator", 4), 42, true);
    let mut j = Match::new(profile("Joiner", 7), 0, true);
    j.joined();
    (c, j)
}

#[test]
fn handshake_profiles_course_and_start() {
    let (mut c, mut j) = pair();
    assert_eq!(j.stage, Stage::Handshake);
    assert_eq!(c.stage, Stage::Lobby);
    // the joiner opens with S then R, like the app
    assert_eq!(j.take_outbox(), vec![Message::Start, Message::Version(PROTOCOL_VERSION)]);
    j.joined(); // a second claim is ignored
    assert!(j.take_outbox().is_empty());
    let mut j = {
        let mut j = Match::new(profile("Joiner", 7), 0, true);
        j.joined();
        j
    };
    let (ec, ej) = shuttle(&mut c, &mut j);
    assert_eq!(c.peer.as_ref().unwrap().name, "Joiner");
    assert_eq!(j.peer.as_ref().unwrap().car, 4);
    assert_eq!((c.stage, j.stage), (Stage::Racing, Stage::Racing));
    // the creator starts when it has the joiner's profile; the joiner when the course arrives
    assert!(ec.contains(&Event::StartRace) && ej.contains(&Event::StartRace));
    let (cc, cj) = (c.course.clone().unwrap(), j.course.clone().unwrap());
    assert_eq!((cc.mission, cc.directions, cc.aicar), (cj.mission, cj.directions, cj.aicar));
    assert!(!matches!(cc.mission, Mission::Fuel | Mission::Vip));
}

#[test]
fn exact_message_order_matches_the_original_client() {
    let (mut c, mut j) = pair();
    let (mut from_c, mut from_j) = (Vec::new(), Vec::new());
    for _ in 0..10 {
        let (oc, oj) = (c.take_outbox(), j.take_outbox());
        if oc.is_empty() && oj.is_empty() {
            break;
        }
        let tag = |m: &Message| m.encode().split(' ').next().unwrap().to_string();
        from_j.extend(oj.iter().map(tag));
        from_c.extend(oc.iter().map(tag));
        for m in oj {
            c.handle(m);
        }
        for m in oc {
            j.handle(m);
        }
    }
    // joiner: S, R, P.  creator: R (answering S), P (answering R), then a single I
    assert_eq!(from_j, ["S", "R", "P"]);
    assert_eq!(from_c, ["R", "P", "I"]);
}

#[test]
fn finish_and_rematch_gives_a_new_course() {
    let (mut c, mut j) = pair();
    shuttle(&mut c, &mut j);
    let first = c.course.clone().unwrap();
    j.finish(true);
    let (ec, _) = shuttle(&mut c, &mut j);
    assert!(ec.contains(&Event::OpponentFinished { opponent_won: true }));
    assert_eq!(c.stage, Stage::Finished);
    // a rematch needs both sides
    c.request_rematch(4, 1200);
    shuttle(&mut c, &mut j);
    assert_eq!(c.stage, Stage::Finished);
    j.request_rematch(9, 1300);
    let (ec, ej) = shuttle(&mut c, &mut j);
    assert!(ec.contains(&Event::StartRace) && ej.contains(&Event::StartRace));
    assert_eq!(c.peer.as_ref().unwrap().car, 9, "the rematch carries the new car");
    let second = c.course.clone().unwrap();
    assert_ne!(first.mission, second.mission, "the generator never repeats the previous mission");
    assert_eq!(second.directions, j.course.clone().unwrap().directions);
}

#[test]
fn a_crash_is_a_loss_and_only_reported_once() {
    let (mut c, mut j) = pair();
    shuttle(&mut c, &mut j);
    c.finish(false);
    c.finish(true); // ignored: already decided
    let (_, ej) = shuttle(&mut c, &mut j);
    assert_eq!(ej.iter().filter(|e| matches!(e, Event::OpponentFinished { .. })).count(), 1);
    assert!(ej.contains(&Event::OpponentFinished { opponent_won: false }));
}

#[test]
fn version_mismatch_closes() {
    let mut j = Match::new(profile("J", 1), 0, true);
    j.joined();
    let ev = j.handle(Message::Version(PROTOCOL_VERSION + 1));
    assert_eq!(ev, vec![Event::Closed(CloseReason::LocalOutdated)]);
    let mut j = Match::new(profile("J", 1), 0, true);
    j.joined();
    let ev = j.handle(Message::Version(PROTOCOL_VERSION - 1));
    assert_eq!(ev, vec![Event::Closed(CloseReason::PeerOutdated)]);
    assert!(j.handle(Message::Start).is_empty(), "a closed match ignores everything");
}

#[test]
fn hostile_input_is_rejected() {
    let (mut c, mut j) = pair();
    shuttle(&mut c, &mut j);
    // NaN / absurd positions never reach the world
    let bad = |cx: f32, vx: f32| Message::Move { time: 1.0, mission_distance: 1.0, cx, cy: 0.0, vx, vy: 0.0, angle: 0.0 };
    assert!(j.handle(bad(f32::NAN, 0.0)).is_empty());
    assert!(j.handle(bad(1.0e9, 0.0)).is_empty());
    assert!(j.handle(bad(0.0, f32::INFINITY)).is_empty());
    assert_eq!(j.handle(bad(5.0, 5.0)).len(), 1);
    // out-of-stage messages are ignored
    assert!(j.handle(Message::Version(PROTOCOL_VERSION)).is_empty());
    assert_eq!(j.stage, Stage::Racing);
    // an invalid course closes the match
    let mut j2 = Match::new(profile("J", 1), 0, true);
    j2.joined();
    j2.handle(Message::Version(PROTOCOL_VERSION));
    j2.handle(Message::Profile { rating: 0, car: 1, country: 0, name: "C".into() });
    let ev = j2.handle(Message::Course { mission: 99, directions: [0; 20], aicar: [0; 40] });
    assert_eq!(ev, vec![Event::Closed(CloseReason::Protocol)]);
    // a joiner never accepts a course while it is the creator and vice versa
    let (mut c, mut j) = pair();
    shuttle(&mut c, &mut j);
    j.finish(true);
    shuttle(&mut c, &mut j);
    assert!(c.handle(Message::Course { mission: 0, directions: [0; 20], aicar: [0; 40] }).is_empty());
}

#[test]
fn profiles_are_sanitised() {
    let p = Profile { rating: 99999, car: 500, country: -4, name: "  Evil\nName\twith a very long tail that goes on  ".into() }.sanitized();
    assert_eq!((p.rating, p.car, p.country), (10000, 27, 0));
    assert!(!p.name.contains(['\n', '\t']) && p.name.chars().count() <= MAX_NAME);
    assert_eq!(Profile { rating: 0, car: 0, country: 0, name: " \n ".into() }.sanitized().name, "Player");
    assert_eq!(Profile { rating: 0, car: 0, country: 0, name: "a   b".into() }.sanitized().name, "a b");
}

#[test]
fn silence_times_out_and_pings_keep_it_alive() {
    let (mut c, mut j) = pair();
    shuttle(&mut c, &mut j);
    // pings flow both ways (every 3 s), so neither side times out (12 s)
    for _ in 0..80 {
        c.tick(0.5);
        j.tick(0.5);
        shuttle(&mut c, &mut j);
    }
    assert!(!c.is_closed() && !j.is_closed());
    // but a peer that goes quiet does
    let mut closed = false;
    for _ in 0..30 {
        closed |= c.tick(0.5).contains(&Event::Closed(CloseReason::Timeout));
    }
    assert!(closed);
}

#[test]
fn a_lobby_never_times_out_or_pings() {
    // the creator waits for a joiner for as long as the user lets it
    let mut c = Match::new(profile("C", 1), 1, false);
    for _ in 0..200 {
        assert!(c.tick(1.0).is_empty());
    }
    assert!(c.take_outbox().is_empty() && !c.is_closed() && !c.has_peer());
}

#[test]
fn sending_stops_when_the_peer_falls_far_behind() {
    let (mut c, mut j) = pair();
    shuttle(&mut c, &mut j);
    c.take_outbox();
    let mv = Remote { time: 1.0, mission_distance: 1.0, cx: 0.0, cy: 0.0, vx: 0.0, vy: 0.0, angle: 0.0 };
    // while the peer has sent us five messages or fewer nothing is throttled ...
    for _ in 0..1000 {
        c.send_move(mv);
    }
    assert_eq!(c.take_outbox().len(), 1000);
    // ... but once it has sent more, we stop 300 messages ahead of what we have read
    for _ in 0..10 {
        c.handle(Message::Ping);
    }
    for _ in 0..1000 {
        c.send_move(mv);
    }
    assert!(c.take_outbox().len() < 400);
}

#[test]
fn channels() {
    assert_eq!(net::quick_channel(0, false), 0);
    assert_eq!(net::quick_channel(999, true), 1);
    assert_eq!(net::quick_channel(1000, false), 2);
    assert_eq!(net::quick_channel(-5000, true), 1);
    assert_eq!(net::quick_channel(9000, true), 3);
    assert_eq!(net::friend_channel(99), None);
    assert_eq!(net::friend_channel(100), Some(100));
    assert_eq!(net::friend_channel(999), Some(999));
    assert_eq!(net::friend_channel(1000), None);
}

// ---------------------------------------------------------------------------- matchmaking

fn conn(db: &MemDb, channel: i32, name: &str, seed: i64) -> Connection {
    Connection::new(Box::new(db.backend()), channel, profile(name, 1), seed, NOW)
}

/// Pump every connection (10 ms steps) until `done` or the step budget runs out.
fn run(cs: &mut [&mut Connection], mut done: impl FnMut(&[&mut Connection]) -> bool, steps: usize) -> bool {
    for _ in 0..steps {
        for c in cs.iter_mut() {
            c.pump(0.01);
        }
        if done(cs) {
            return true;
        }
    }
    false
}

fn racing(c: &Connection) -> bool {
    c.m.stage == Stage::Racing
}

#[test]
fn quick_match_end_to_end() {
    let db = MemDb::new();
    let mut a = conn(&db, 0, "Alice", 11);
    assert_eq!(a.status(), Status::Searching);
    assert!(run(&mut [&mut a], |c| c[0].status() == Status::Waiting, 100));

    // the lobby is exactly what the original writes
    let waiting = db.read("Waiting");
    let (lobby, entry) = waiting.as_object().unwrap().iter().next().unwrap();
    assert_eq!(entry["channel"], 0);
    assert_eq!(entry["status"], 0);
    let room = entry["room"].as_str().unwrap().to_string();
    assert_eq!(lobby.len(), 20, "a Firebase push id");
    let live = db.read(&format!("Gaming/{room}/Live"));
    assert_eq!(live.as_str().unwrap().len(), 14, "yyyyMMddHHmmss");

    let mut b = conn(&db, 0, "Bob", 22);
    assert!(run(&mut [&mut a, &mut b], |c| racing(c[0]) && racing(c[1]), 2000), "both reach the race");
    assert!(db.read("Waiting").is_null(), "the claimed lobby is removed");
    assert_eq!(a.m.peer.as_ref().unwrap().name, "Bob");
    assert_eq!(b.m.peer.as_ref().unwrap().name, "Alice");
    let (ca, cb) = (a.m.course.clone().unwrap(), b.m.course.clone().unwrap());
    assert_eq!((ca.mission, ca.directions, ca.aicar), (cb.mission, cb.directions, cb.aicar));
    assert!(!a.m.friendly && !b.m.friendly);
    assert_eq!(a.status(), Status::InMatch);

    // position updates flow both ways
    let mv = |x: f32| Remote { time: 2.0, mission_distance: x, cx: x, cy: 1.0, vx: 3.0, vy: 0.0, angle: 90.0 };
    let mut got = (None, None);
    a.m.send_move(mv(10.0));
    b.m.send_move(mv(20.0));
    run(
        &mut [&mut a, &mut b],
        |_| true,
        1, // flush
    );
    for _ in 0..300 {
        for (i, c) in [&mut a, &mut b].into_iter().enumerate() {
            for e in c.pump(0.01) {
                if let Event::Opponent(r) = e {
                    if i == 0 {
                        got.0 = Some(r.cx);
                    } else {
                        got.1 = Some(r.cx);
                    }
                }
            }
        }
    }
    assert_eq!(got, (Some(20.0), Some(10.0)));
    // consumed messages are deleted from the mailboxes
    for to in ["ToCreator", "ToJoiner"] {
        let left = db.read(&format!("Gaming/{room}/{to}"));
        let n = left.as_array().map(|a| a.len()).or_else(|| left.as_object().map(|o| o.len())).unwrap_or(0);
        assert!(n <= 2, "{to} keeps growing: {left}");
    }

    // leaving removes the room, which is how the other side finds out
    a.leave();
    assert!(db.read(&format!("Gaming/{room}")).is_null());
    let mut closed = false;
    for _ in 0..600 {
        closed |= b.pump(0.01).iter().any(|e| matches!(e, Event::Closed(_)));
    }
    assert!(closed && b.status() == Status::Closed);
}

#[test]
fn friend_channels_only_meet_each_other() {
    let db = MemDb::new();
    let mut quick = conn(&db, 0, "Quick", 1);
    let mut f123 = conn(&db, 123, "F123", 2);
    assert!(run(&mut [&mut quick, &mut f123], |c| c[0].status() == Status::Waiting && c[1].status() == Status::Waiting, 300));
    let mut f456 = conn(&db, 456, "F456", 3);
    assert!(run(&mut [&mut quick, &mut f123, &mut f456], |c| c[2].status() == Status::Waiting, 300));
    assert_eq!(db.read("Waiting").as_object().unwrap().len(), 3, "three separate lobbies");
    let mut other = conn(&db, 123, "Other", 4);
    assert!(run(&mut [&mut quick, &mut f123, &mut f456, &mut other], |c| racing(c[1]) && racing(c[3]), 2000));
    assert!(f123.m.friendly && other.m.friendly, "channels from 100 are friendly matches");
    assert_eq!(f123.m.peer.as_ref().unwrap().name, "Other");
    assert!(!racing(&quick) && !racing(&f456));
    assert_eq!(db.read("Waiting").as_object().unwrap().len(), 2);
}

#[test]
fn two_claimants_one_lobby_exactly_one_wins() {
    let db = MemDb::new();
    let mut a = conn(&db, 2, "Host", 1);
    assert!(run(&mut [&mut a], |c| c[0].status() == Status::Waiting, 100));
    // both start looking in the same instant: both see the open lobby and race to claim it
    let mut b = conn(&db, 2, "B", 2);
    let mut c = conn(&db, 2, "C", 3);
    assert!(run(&mut [&mut a, &mut b, &mut c], |cs| cs[1..].iter().any(|x| racing(x)) && cs[1..].iter().all(|x| x.status() == Status::Waiting || racing(x)), 3000));
    let in_a = [&b, &c].iter().filter(|x| x.m.stage == Stage::Racing).count();
    assert_eq!(in_a, 1, "one of them got the lobby");
    let loser = if racing(&b) { &c } else { &b };
    assert_eq!(loser.status(), Status::Waiting, "the loser opened its own lobby instead");
    assert_eq!(db.read("Waiting").as_object().unwrap().len(), 1);
}

#[test]
fn a_stale_lobby_is_skipped() {
    // a lobby whose room is already gone (its creator crashed): claiming it finds no room, and
    // the joiner carries on searching instead of getting stuck
    let db = MemDb::new();
    db.write("Waiting/-Nstale0000000000000", json!({"channel": 0, "status": 0, "room": "-Ngone"}));
    let mut b = conn(&db, 0, "B", 5);
    let mut seen_closed = false;
    for _ in 0..800 {
        seen_closed |= b.pump(0.01).iter().any(|e| matches!(e, Event::Closed(_)));
    }
    assert!(seen_closed, "the dead room is detected through the missing Live node");
    assert!(db.read("Waiting").is_null(), "and the stale lobby is cleaned up by the claim");
}

#[test]
fn permission_denied_and_offline_report_errors() {
    let db = MemDb::new();
    db.set_fault(Some(401));
    let mut a = conn(&db, 0, "A", 1);
    let mut ev = Vec::new();
    for _ in 0..10 {
        ev.extend(a.pump(0.01));
    }
    assert!(ev.iter().any(|e| matches!(e, Event::Closed(_))));
    assert!(a.error().unwrap().contains("401"), "{:?}", a.error());
    assert_eq!(a.status(), Status::Closed);

    // offline: it keeps trying for a while before giving up
    let db = MemDb::new();
    db.set_fault(Some(0));
    let mut a = conn(&db, 0, "A", 1);
    for _ in 0..500 {
        a.pump(0.01);
    }
    assert_eq!(a.status(), Status::Searching, "still retrying after 5 s");
    for _ in 0..1500 {
        a.pump(0.01);
    }
    assert!(a.error().unwrap().contains("reach"));
    assert_eq!(a.status(), Status::Closed);

    // a request the server rejects (HTTP 400, like print=silent + if-match once did) is not retried
    let db = MemDb::new();
    db.set_fault(Some(400));
    let mut a = conn(&db, 0, "A", 1);
    for _ in 0..5 {
        a.pump(0.01);
    }
    assert!(a.error().unwrap().contains("400"), "{:?}", a.error());
}

#[test]
fn a_short_outage_while_searching_is_survived() {
    let db = MemDb::new();
    db.set_fault(Some(0));
    let mut a = conn(&db, 0, "A", 1);
    for _ in 0..150 {
        a.pump(0.01); // fails, retries, fails ...
    }
    db.set_fault(None);
    assert!(run(&mut [&mut a], |c| c[0].status() == Status::Waiting, 1000), "the search recovers once the network is back");
    let mut b = conn(&db, 0, "B", 2);
    assert!(run(&mut [&mut a, &mut b], |c| racing(c[0]) && racing(c[1]), 2000));
}

#[test]
fn losing_the_network_mid_match_disconnects_after_ten_seconds() {
    let db = MemDb::new();
    let mut a = conn(&db, 0, "A", 1);
    assert!(run(&mut [&mut a], |c| c[0].status() == Status::Waiting, 100));
    let be = db.backend();
    let cut = be.network_switch();
    let mut b = Connection::new(Box::new(be), 0, profile("B", 2), 2, NOW);
    assert!(run(&mut [&mut a, &mut b], |c| racing(c[0]) && racing(c[1]), 2000));
    cut.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut closed_at = None;
    for i in 0..1500 {
        let ev = b.pump(0.01);
        if closed_at.is_none() && ev.iter().any(|e| matches!(e, Event::Closed(_))) {
            closed_at = Some(i);
        }
    }
    let i = closed_at.expect("b notices that it is offline");
    assert!((900..1200).contains(&i), "after about 10 s, not {}", i as f32 / 100.0);
    assert_eq!(b.error(), Some("Connection to the server lost"));
}

#[test]
fn silent_peer_times_out_after_twelve_seconds() {
    let db = MemDb::new();
    let mut a = conn(&db, 0, "A", 1);
    assert!(run(&mut [&mut a], |c| c[0].status() == Status::Waiting, 100));
    let mut b = conn(&db, 0, "B", 2);
    assert!(run(&mut [&mut a, &mut b], |c| racing(c[0]) && racing(c[1]), 2000));
    // b stops being pumped (a frozen client) but its room stays; a hears nothing
    let mut closed = None;
    for i in 0..2000 {
        if a.pump(0.01).contains(&Event::Closed(CloseReason::Timeout)) {
            closed = Some(i);
            break;
        }
    }
    let i = closed.expect("timeout");
    assert!((1150..1350).contains(&i), "12 s, got {}", i as f32 / 100.0);
}

#[test]
fn leaving_while_waiting_removes_lobby_and_room() {
    let db = MemDb::new();
    let mut a = conn(&db, 0, "A", 1);
    assert!(run(&mut [&mut a], |c| c[0].status() == Status::Waiting, 100));
    assert!(!db.read("Waiting").is_null() && !db.read("Gaming").is_null());
    a.leave();
    assert!(db.read("Waiting").is_null() && db.read("Gaming").is_null());
    // dropping a connection cleans up as well
    let mut b = conn(&db, 0, "B", 2);
    assert!(run(&mut [&mut b], |c| c[0].status() == Status::Waiting, 100));
    drop(b);
    assert!(db.read("Waiting").is_null() && db.read("Gaming").is_null());
}

// ---------------------------------------------------------------------------- the original app

/// Plays the part of the Android client by writing to the database directly, using the exact
/// paths and strings found in the decompiled `DrDrivingActivity`.
struct OriginalApp {
    db: MemDb,
    room: String,
    creator: bool,
    sent: usize,
}

impl OriginalApp {
    fn send(&mut self, text: &str) {
        let to = if self.creator { "ToJoiner" } else { "ToCreator" };
        self.db.write(&format!("Gaming/{}/{}/{}", self.room, to, self.sent), json!(text));
        self.sent += 1;
    }

    /// `onChildAdded` + removeValue.
    fn receive(&mut self) -> Vec<String> {
        let from = if self.creator { "ToCreator" } else { "ToJoiner" };
        let path = format!("Gaming/{}/{}", self.room, from);
        let v = self.db.read(&path);
        let mut out = Vec::new();
        if let Some(o) = v.as_object() {
            let mut keys: Vec<i64> = o.keys().map(|k| k.parse().unwrap()).collect();
            keys.sort();
            for k in keys {
                out.push(o[&k.to_string()].as_str().unwrap().to_string());
                self.db.write(&format!("{path}/{k}"), Value::Null);
            }
        }
        out
    }
}

#[test]
fn we_can_join_a_room_made_by_the_original_app() {
    let db = MemDb::new();
    // FirebaseCreateRoom: Gaming/<k> = {Live}, then Waiting/<w> = {channel, status, room}
    db.write("Gaming/-Nroom", json!({"Live": "20261108120000"}));
    db.write("Waiting/-Nlobby", json!({"channel": 2, "status": 0, "room": "-Nroom"}));
    let mut app = OriginalApp { db: db.clone(), room: "-Nroom".into(), creator: true, sent: 0 };
    let mut me = conn(&db, 2, "Me", 9);

    let mut inbox = Vec::new();
    for _ in 0..600 {
        me.pump(0.01);
        inbox.extend(app.receive());
        if inbox.len() >= 2 {
            break;
        }
    }
    assert_eq!(inbox, ["S", "R 69"], "we say hello first, like the app's joiner does");
    assert_eq!(db.read("Waiting/-Nlobby"), Value::Null, "the lobby is deleted by the joiner");
    // the original creator reacts to S with R, and to the joiner's R with its profile
    app.send("R 69");
    app.send("P 1500 6 81 Original Driver");
    // the app (creator) starts as soon as it has our P and sends the course
    let mut got_p = false;
    for _ in 0..600 {
        me.pump(0.01);
        for m in app.receive() {
            got_p |= m.starts_with("P 1200 1 3 Me");
        }
        if got_p {
            break;
        }
    }
    assert!(got_p, "we answer R with our profile");
    let mut info = drdriving_core::mission::MissionInfo::new(Mission::Highway);
    info.generate(Mission::Highway, &mut drdriving_core::rng::JavaRandom::new(5));
    app.send(&Message::course_from(&info).encode());
    let mut started = false;
    for _ in 0..600 {
        started |= me.pump(0.01).contains(&Event::StartRace);
        if started {
            break;
        }
    }
    assert!(started);
    let peer = me.m.peer.clone().unwrap();
    assert_eq!((peer.name.as_str(), peer.rating, peer.car, peer.country), ("Original Driver", 1500, 6, 81));
    assert_eq!(me.m.course.as_ref().unwrap().mission, info.mission);
    assert!(!me.m.friendly);
    // it drives, we see it; we crash, it sees "O N"
    app.send("U 1.5 20.0 3.5 40.25 0.0 11.0 180.0");
    let mut opp = None;
    for _ in 0..200 {
        for e in me.pump(0.01) {
            if let Event::Opponent(r) = e {
                opp = Some(r);
            }
        }
    }
    assert_eq!(opp.unwrap().cy, 40.25);
    me.m.finish(false);
    let mut out = Vec::new();
    for _ in 0..300 {
        me.pump(0.01);
        out.extend(app.receive());
    }
    assert!(out.contains(&"O N".to_string()), "{out:?}");
}

#[test]
fn the_original_app_can_join_our_room() {
    let db = MemDb::new();
    let mut me = conn(&db, 3, "Me", 4);
    assert!(run(&mut [&mut me], |c| c[0].status() == Status::Waiting, 100));
    // searchRoom + the claim transaction, done by hand
    let waiting = db.read("Waiting");
    let (lobby, entry) = waiting.as_object().unwrap().iter().next().unwrap();
    let room = entry["room"].as_str().unwrap().to_string();
    db.write(&format!("Waiting/{lobby}/status"), json!(1));
    db.write(&format!("Waiting/{lobby}"), Value::Null);
    let mut app = OriginalApp { db: db.clone(), room, creator: false, sent: 0 };
    app.send("S");
    app.send("R 69");
    let mut inbox = Vec::new();
    for _ in 0..800 {
        me.pump(0.01);
        inbox.extend(app.receive());
        if inbox.iter().any(|m| m.starts_with('P')) {
            break;
        }
    }
    assert_eq!(&inbox[..2], ["R 69", "P 1200 1 3 Me"], "the creator answers S with R, then R with its profile");
    // the real joiner sends its profile after the matching R
    app.send("P -300 2 7 Some Name");
    let mut got_course = None;
    for _ in 0..600 {
        me.pump(0.01);
        for m in app.receive() {
            if m.starts_with("I ") {
                got_course = Some(m);
            }
        }
        if got_course.is_some() {
            break;
        }
    }
    let course = got_course.expect("the creator sends I right after the joiner's P");
    assert_eq!(course.split(' ').count(), 62, "I + mission + 20 directions + 40 cars");
    assert_eq!(me.m.peer.as_ref().unwrap().name, "Some Name");
    // a version mismatch (newer app) ends it
    let mut me2 = conn(&db, 1, "Me2", 6);
    assert!(run(&mut [&mut me2], |c| c[0].status() == Status::Waiting, 100));
    let w = db.read("Waiting");
    let (lobby2, e2) = w.as_object().unwrap().iter().find(|(_, e)| e["channel"] == 1).unwrap();
    let mut app2 = OriginalApp { db: db.clone(), room: e2["room"].as_str().unwrap().into(), creator: false, sent: 0 };
    db.write(&format!("Waiting/{lobby2}"), Value::Null);
    app2.send("S");
    app2.send("R 70");
    let mut closed = false;
    for _ in 0..600 {
        closed |= me2.pump(0.01).contains(&Event::Closed(CloseReason::LocalOutdated));
        if closed {
            break;
        }
    }
    assert!(closed, "a creator that is older than its joiner closes the room");
}

// ---------------------------------------------------------------------------- full races

struct Side {
    conn: Connection,
    save: SaveGame,
    session: Option<Session>,
    world: Option<World>,
}

impl Side {
    fn start_race(&mut self, seed: i64) {
        let course = self.conn.m.course.clone().unwrap();
        let peer = self.conn.m.peer.clone().unwrap();
        let mut w = self.world.take().unwrap_or_else(|| World::new(0, Some(seed)));
        w.setup_online(&course, peer.car as usize, &peer.name, peer.rating, peer.country);
        w.prepare_race(&self.save);
        self.session = Some(Session::new_online(w, self.conn.m.friendly, peer.rating));
    }
}

/// Two players in a friend match, both with their race started. Pumps both connections.
fn matched(db: &MemDb) -> (Side, Side) {
    let mk = |conn| Side { conn, save: SaveGame::default(), session: None, world: None };
    let mut creator = mk(conn(db, 321, "Creator", 7));
    assert!(run(&mut [&mut creator.conn], |c| c[0].status() == Status::Waiting, 100));
    let mut joiner = mk(conn(db, 321, "Joiner", 8));
    assert!(run(&mut [&mut creator.conn, &mut joiner.conn], |c| racing(c[0]) && racing(c[1]), 3000), "match start");
    creator.start_race(1);
    joiner.start_race(2);
    (creator, joiner)
}

/// Step both sessions one frame each, like the game loop does.
fn race_frame(a: &mut Side, b: &mut Side, slow_b: bool) {
    for (i, side) in [&mut *a, &mut *b].into_iter().enumerate() {
        let Some(mut s) = side.session.take() else { continue };
        let _ = side.conn.pre_frame(&mut s, &mut side.save, 1.0 / 60.0);
        let (mut accel, brake, wheel) = drdriving_core::autopilot::drive(&s.world, side.save.right_driver);
        if i == 1 && slow_b {
            accel = false; // the joiner never moves: the creator must win
        }
        if s.phase == Phase::Playing {
            s.wheel_angle = wheel;
        }
        for c in s.world.cars.iter_mut() {
            c.visible = false; // empty roads: this test is about the netcode
        }
        s.frame(1.0 / 60.0, &Input { accel, brake, ..Default::default() }, &mut side.save);
        side.conn.post_frame(&mut s);
        side.session = Some(s);
    }
}

#[test]
fn full_race_over_the_database() {
    let db = MemDb::new();
    let (mut creator, mut joiner) = matched(&db);
    assert_eq!(creator.session.as_ref().unwrap().world.mission, joiner.session.as_ref().unwrap().world.mission);
    assert_eq!(joiner.session.as_ref().unwrap().world.opponent.name, "Creator");

    let rating_before = (creator.save.rating, joiner.save.rating);
    let mut saw_opponent_move = false;
    for _ in 0..(200 * 60) {
        race_frame(&mut creator, &mut joiner, true);
        let j = joiner.session.as_ref().unwrap();
        saw_opponent_move |= j.world.opponent.mission_distance > 50.0;
        if creator.session.as_ref().unwrap().is_over() && j.is_over() {
            break;
        }
    }
    assert!(saw_opponent_move, "the joiner should see the creator driving");
    let (cs, js) = (creator.session.as_ref().unwrap(), joiner.session.as_ref().unwrap());
    assert_eq!(cs.phase, Phase::Success, "creator finished first");
    assert_eq!(js.phase, Phase::Ending(Outcome::OppWin), "joiner lost to the opponent");
    assert!(cs.result.as_ref().unwrap().multi.as_ref().unwrap().won);
    assert!(!js.result.as_ref().unwrap().multi.as_ref().unwrap().won);
    assert_eq!((creator.save.rating, joiner.save.rating), rating_before, "friendly matches never change the rating");
    assert!(creator.save.coin > 0, "the winner is paid");
    assert_eq!(creator.save.win, 0, "friendly wins are not counted in the ranking");
    assert_eq!(creator.conn.m.stage, Stage::Finished);
    assert_eq!(joiner.conn.m.stage, Stage::Finished);
}

#[test]
fn ranked_race_changes_ratings() {
    let db = MemDb::new();
    let mk = |conn| Side { conn, save: SaveGame::default(), session: None, world: None };
    let mut creator = mk(conn(&db, 2, "Creator", 7));
    assert!(run(&mut [&mut creator.conn], |c| c[0].status() == Status::Waiting, 100));
    let mut joiner = mk(conn(&db, 2, "Joiner", 8));
    assert!(run(&mut [&mut creator.conn, &mut joiner.conn], |c| racing(c[0]) && racing(c[1]), 3000));
    creator.start_race(1);
    joiner.start_race(2);
    for _ in 0..(200 * 60) {
        race_frame(&mut creator, &mut joiner, true);
        if creator.session.as_ref().unwrap().is_over() && joiner.session.as_ref().unwrap().is_over() {
            break;
        }
    }
    assert!(creator.save.rating > 0, "the winner gains rating");
    assert!(joiner.save.rating < 0, "the loser loses rating");
    assert_eq!(creator.save.win, 1);
}

#[test]
fn rematch_then_leaving_mid_race_forfeits() {
    let db = MemDb::new();
    let (mut creator, mut joiner) = matched(&db);
    // the joiner forfeits during the countdown
    {
        let (mut s, save) = (joiner.session.take().unwrap(), &mut joiner.save);
        joiner.conn.forfeit(&mut s, save);
        assert_eq!(s.phase, Phase::Ending(Outcome::NoFuel));
        joiner.session = Some(s);
    }
    // events must go through the session (as in the game), so step frames rather than pump
    for _ in 0..3000 {
        race_frame(&mut creator, &mut joiner, true);
        if creator.session.as_ref().unwrap().is_over() {
            break;
        }
    }
    let cs = creator.session.as_ref().unwrap();
    assert_eq!(cs.phase, Phase::Success);
    assert!(cs.result.as_ref().unwrap().multi.as_ref().unwrap().won);

    // both ask for a rematch -> a fresh course, and the sides can race again
    creator.conn.m.request_rematch(1, 0);
    joiner.conn.m.request_rematch(2, 0);
    assert!(run(&mut [&mut creator.conn, &mut joiner.conn], |c| racing(c[0]) && racing(c[1]), 3000), "rematch start");
    creator.world = Some(creator.session.take().unwrap().world);
    joiner.world = Some(joiner.session.take().unwrap().world);
    creator.start_race(1);
    joiner.start_race(2);
    assert_eq!(creator.session.as_ref().unwrap().world.opponent.travel_time, 0.0);

    // now the joiner simply disappears mid-race: the creator wins by default
    joiner.conn.leave();
    drop(joiner);
    let mut n = 0;
    while !creator.session.as_ref().unwrap().is_over() && n < 3000 {
        let mut s = creator.session.take().unwrap();
        creator.conn.pre_frame(&mut s, &mut creator.save, 0.01);
        creator.session = Some(s);
        n += 1;
    }
    let cs = creator.session.as_ref().unwrap();
    assert_eq!(cs.phase, Phase::Success);
    assert!(cs.result.as_ref().unwrap().multi.as_ref().unwrap().won);
    assert!(db.read("Gaming").is_null());
}
