use drdriving_core::cars::*;
use drdriving_core::economy::*;
use drdriving_core::math::*;
use drdriving_core::mission::*;
use drdriving_core::net::*;
use drdriving_core::rng::JavaRandom;
use drdriving_core::save::SaveGame;
use drdriving_core::world::*;

#[test]
fn java_random_matches_jdk() {
    // Well-known JDK outputs for `new Random(42)`.
    let mut r = JavaRandom::new(42);
    assert_eq!(r.next_int(), -1170105035);
    assert_eq!(r.next_int(), 234785527);
    // power-of-two and non-power-of-two bounds both stay in range and are deterministic
    let mut a = JavaRandom::new(7);
    let mut b = JavaRandom::new(7);
    for bound in [2, 3, 4, 8, 9, 40, 1000] {
        let x = a.next_int_bound(bound);
        assert!((0..bound).contains(&x));
        assert_eq!(x, b.next_int_bound(bound));
    }
    // Random(0).nextInt(10) == 0 in the JDK
    assert_eq!(JavaRandom::new(0).next_int_bound(10), 0);
}

#[test]
fn car_table_matches_decompiled_values() {
    assert_eq!(CAR_COUNT, 30);
    let k = car(0);
    assert_eq!((k.name, k.ability, k.price, k.repair_cost), ("KOS", [10, 10, 10, 40, 10, 20], 0, 100));
    assert_eq!(car(1).price_kind(), Price::Coins(2000));
    assert_eq!(car(7).price_kind(), Price::Gold(100));
    assert_eq!(car(15).price_kind(), Price::Gold(500));
    assert_eq!(car(29).name, "TRUCK");
    assert!((car(22).half_length - 2.9).abs() < 1e-6);
    for (i, c) in CAR_TABLE.iter().enumerate() {
        assert_eq!(c.no, i);
    }
}

#[test]
fn save_roundtrip_and_quirks() {
    let mut s = SaveGame::zero_for_country("GB");
    assert!(!s.right_driver && s.show_yard, "new saves start left-side whatever the locale");
    assert!(!SaveGame::zero_for_country("kr").right_driver);
    s.coin = 1234;
    s.gold = 56;
    s.has_car[3] = true;
    s.upgrade[3] = [1, 2, 3, 4, 5, 10];
    s.car_rent[17] = 12345;
    s.record[8] = 777;
    s.achievement[5] = true;
    s.rating = -250;
    let json = s.to_json();
    let back = SaveGame::from_json_strict(&json).unwrap();
    assert_eq!(s, back);

    // bad version
    let bad = json.replace("\"version\":\"1.0\"", "\"version\":\"2.0\"");
    assert!(SaveGame::from_json_strict(&bad).is_err());

    // out-of-range rent is zeroed, mission records are clamped on load
    let tweaked = json.replace("\"iCarRent17\":12345", "\"iCarRent17\":99999").replace("\"iRecord0\":0", "\"iRecord0\":50000");
    let t = SaveGame::from_json_strict(&tweaked).unwrap();
    assert_eq!(t.car_rent[17], 0);
    assert_eq!(t.record[0], 10000);

    // a missing key aborts the load but keeps what was read so far (Java swallows the exception)
    let broken = json.replace("\"iMB\"", "\"iXX\"");
    let l = SaveGame::from_json_lenient(&broken);
    assert_eq!(l.coin, 1234);
    assert!(SaveGame::from_json_strict(&broken).is_err());

    // blank string = fresh save
    assert_eq!(SaveGame::from_json_lenient("   "), SaveGame::default());
}

#[test]
fn levels_and_classes() {
    assert_eq!(base_exp_for_level(0), 0);
    assert_eq!(base_exp_for_level(1), 4096);
    assert_eq!(base_exp_for_level(2), 4096 + 5120);
    assert_eq!(level_for_exp(0.0), 0);
    assert_eq!(level_for_exp(4095.0), 0);
    assert_eq!(level_for_exp(4096.0), 1);
    assert_eq!(level_for_exp(9215.0), 1);
    assert_eq!(level_for_exp(9216.0), 2);
    assert_eq!(level_for_exp(1.0e9), 9);
    assert_eq!(class_for_rating(-1), 0);
    assert_eq!(class_for_rating(999), 1);
    assert_eq!(class_for_rating(5999), 5);
    assert_eq!(class_for_rating(6000), 6);
}

#[test]
fn shop_rules() {
    let mut s = SaveGame::default();
    s.coin = 2500;
    assert_eq!(buy_car(&mut s, 0), Err(ShopError::InvalidCar));
    assert_eq!(buy_car(&mut s, 5), Err(ShopError::NotEnough)); // 9000 coins
    assert_eq!(buy_car(&mut s, 1), Ok(()));
    assert_eq!(s.coin, 500);
    assert!(s.has_car[1]);
    assert_eq!(buy_car(&mut s, 1), Err(ShopError::AlreadyOwned));
    // gold car
    assert_eq!(buy_car(&mut s, 7), Err(ShopError::NotEnough));
    s.gold = 150;
    assert_eq!(buy_car(&mut s, 7), Ok(()));
    assert_eq!(s.gold, 50);
    // upgrades: cost = ability*10 + level*50 - 50, floor 50; resets damage; cap at level 10
    s.coin = 100_000;
    s.driver_level = 9;
    s.car_damage[0] = 5000;
    assert_eq!(upgrade_cost(&s, 0, 3), Some(40 * 10 - 50)); // KOS fuel = 40
    assert_eq!(upgrade_cost(&s, 0, 0), Some(50)); // 10*10-50 = 50
    assert_eq!(upgrade_car(&mut s, 0, 0), Ok(()));
    assert_eq!(s.car_damage[0], 0);
    assert_eq!(upgrade_cost(&s, 0, 0), Some(100));
    for _ in 0..9 {
        upgrade_car(&mut s, 0, 0).unwrap();
    }
    assert_eq!(s.upgrade[0][0], 10);
    assert_eq!(upgrade_car(&mut s, 0, 0), Err(ShopError::MaxUpgrade));
    assert_eq!(upgrade_cost(&s, 20, 0), None); // not rented
    // rental
    s.gold = 5;
    assert_eq!(rent_car(&mut s, 16), Err(ShopError::NotEnough)); // KYUBIN rents for 10 gold
}

#[test]
fn rental_and_repair() {
    let mut s = SaveGame::default();
    s.gold = 10;
    assert_eq!(rent_car(&mut s, 16), Ok(()));
    assert_eq!((s.gold, s.car_rent[16]), (0, RENT_DISTANCE));
    assert_eq!(repair_cost(3, 0), 0);
    assert_eq!(repair_cost(3, 500), 240 * 1000 / 40000); // clamps damage up to 1000
    assert_eq!(repair_cost(3, 20000), 240 * 9000 / 40000);
    assert!(apply_purchase(&mut s, "drdriving.gold600"));
    assert_eq!((s.gold, s.bought_gold), (600, 600));
    assert!(!apply_purchase(&mut s, "nope"));
}

#[test]
fn mission_results_and_damage() {
    let mut s = SaveGame::default();
    let run = RunStats {
        mission: Some(Mission::Speed),
        car: 0,
        travel_distance: 1000.0,
        drift_distance: 50.0,
        travel_time: 60.0,
        score: 80.0,
        speed_max: 30.0,
        ..Default::default()
    };
    let r = complete_mission_success(&mut s, &run);
    assert_eq!(r.record, 8000);
    assert!(r.new_record);
    assert_eq!(r.reward_coins, (80.0f32 * 0.3 + 10.0 + 7.0) as i32); // 41
    assert_eq!(s.mission_clear[0], 1);
    assert_eq!(s.driver_exp, r.reward_coins * 15);
    assert_eq!(s.car_damage[0], (0.05 * 1000.0 + 50.0) as i32);
    assert_eq!(s.max_speed, 30000);
    // low score still pays the minimum reward of 25
    let mut s2 = SaveGame::default();
    let low = RunStats { mission: Some(Mission::Speed), score: 1.0, ..Default::default() };
    assert_eq!(complete_mission_success(&mut s2, &low).reward_coins, 25);

    // truck: no damage; accident on a normal car: >= 321
    let mut s3 = SaveGame::default();
    let truck = RunStats { mission: Some(Mission::Truck), car: 29, travel_distance: 5000.0, ..Default::default() };
    finish_mission(&mut s3, &truck, Outcome::Success);
    assert_eq!(s3.car_damage[29], 0);
    let crash = RunStats { mission: Some(Mission::Speed), car: 0, speed: 0.0, ..Default::default() };
    finish_mission(&mut s3, &crash, Outcome::Accident);
    assert_eq!(s3.car_damage[0], 321);

    // achievement: 10 km of travel unlocks #0 (+200 coins)
    let mut s4 = SaveGame::default();
    let far = RunStats { mission: Some(Mission::Speed), travel_distance: 10000.0, ..Default::default() };
    let unlocked = finish_mission(&mut s4, &far, Outcome::Success);
    assert!(unlocked.contains(&0));
    assert!(s4.achievement[0]);
    assert_eq!(s4.coin, 200);
}

#[test]
fn rating_math() {
    assert!((expected_score(1000.0, 1000.0) - 0.5).abs() < 1e-6);
    assert!(expected_score(2000.0, 0.0) > 0.7);
    // equal ratings at 0: d = 100; winner gets 210-100 + 10 + 10 = 130, loser (opp) loses 100.
    let mut s = SaveGame::default();
    let run = RunStats { mission: Some(Mission::Speed), travel_distance: 0.0, ..Default::default() };
    let res = finish_multiplayer(&mut s, &run, Outcome::Success, 0, false);
    assert!(res.won);
    assert_eq!(s.rating, 130);
    assert_eq!(res.opponent_rating, -100);
    assert_eq!(s.win, 1);
    assert!(res.gold_bonus);
    assert_eq!(s.gold, 1);
    // 5 coins minimum win reward + 200 for the 'Multiplayer First Win' achievement
    assert_eq!(s.coin, 5 + 200);
    assert!(s.achievement[18]);
    // a loss
    let mut s2 = SaveGame::default();
    let res2 = finish_multiplayer(&mut s2, &run, Outcome::OppWin, 0, false);
    assert!(!res2.won);
    assert_eq!(s2.rating, -100);
    assert_eq!(res2.opponent_rating, 0 + 100 + 30); // opp < 1000: +10 +10 +10
    // friendly matches do not touch ratings
    let mut s3 = SaveGame::default();
    finish_multiplayer(&mut s3, &run, Outcome::Success, 0, true);
    assert_eq!((s3.rating, s3.win), (0, 0));
    assert_eq!(matchmaking_channel(0), 100);
    assert_eq!(matchmaking_channel(-10000), 0);
}

#[test]
fn route_generation_rules() {
    for seed in 0..50 {
        let mut rng = JavaRandom::new(seed);
        for m in Mission::ALL {
            let mut info = MissionInfo::new(m);
            info.generate_directions(&mut rng);
            match m {
                Mission::Highway | Mission::Lane => assert!(info.directions.iter().all(|&d| d == 0)),
                Mission::Parking => assert!(info.directions.iter().all(|&d| d == 1 || d == 3)),
                _ => {
                    for w in info.directions.windows(2) {
                        assert_ne!(w[0], w[1], "same turn twice in a row for {m:?}");
                    }
                    assert_ne!(info.directions[0], 0, "first step never equals the initial 'straight'");
                }
            }
            assert!(info.aicar.iter().all(|&a| a < 3));
        }
        // generate(): never Fuel/VIP, never the current mission
        let mut info = MissionInfo::new(Mission::Speed);
        for _ in 0..30 {
            let cur = info.mission;
            info.generate(cur, &mut rng);
            assert_ne!(info.mission, cur);
            assert!(info.mission != Mission::Fuel && info.mission != Mission::Vip);
        }
    }
}

#[test]
fn grid_dir_and_geometry() {
    let mut d = GridDir::default();
    for i in 0..4 {
        d.set_index(i);
        assert_eq!(d.index(), i);
    }
    d.set_index(1);
    assert_eq!(d.turned_index(1), 2);
    assert_eq!(d.turned_index(3), 0);
    assert_eq!(java_round(2.5), 3);
    assert_eq!(java_round(-2.5), -2);
    assert_eq!(java_round(-0.4), 0);

    let rect = Rect { c: Vec2::new(0.0, 0.0), hx: 1.0, hy: 1.0 };
    assert!(circle_rect(&Circle { c: Vec2::new(1.5, 0.0), r: 1.0 }, &rect));
    assert!(!circle_rect(&Circle { c: Vec2::new(3.0, 0.0), r: 1.0 }, &rect));
    // corner: centre 1.2,1.2 radius 0.5 -> dist to corner = 0.283 <0.5 hit; radius .2 miss
    assert!(circle_rect(&Circle { c: Vec2::new(1.2, 1.2), r: 0.5 }, &rect));
    assert!(!circle_rect(&Circle { c: Vec2::new(1.2, 1.2), r: 0.28 }, &rect));
    let a = Rect { c: Vec2::new(0.0, 0.0), hx: 1.0, hy: 1.0 };
    let b = Rect { c: Vec2::new(1.9, 0.0), hx: 1.0, hy: 1.0 };
    let c = Rect { c: Vec2::new(2.5, 0.0), hx: 1.0, hy: 1.0 };
    assert!(rect_rect(&a, &b));
    assert!(!rect_rect(&a, &c));
}

#[test]
fn protocol_roundtrip() {
    let msgs = vec![
        Message::Start,
        Message::Ping,
        Message::Version(PROTOCOL_VERSION),
        Message::Profile { rating: -120, car: 5, country: 82, name: "Mr Kim Jr".into() },
        Message::Rematch { car: 3, rating: 1500 },
        Message::GameOver { sender_won: true },
        Message::GameOver { sender_won: false },
        Message::Move { time: 12.5, mission_distance: 100.25, cx: 1960.0, cy: 2000.5, vx: 0.0, vy: 15.0, angle: 180.0 },
    ];
    for m in msgs {
        assert_eq!(Message::decode(&m.encode()).unwrap(), m, "{}", m.encode());
    }
    let mut rng = JavaRandom::new(1);
    let mut info = MissionInfo::new(Mission::Speed);
    info.generate_directions(&mut rng);
    let course = Message::course_from(&info);
    let enc = course.encode();
    assert_eq!(enc.split(' ').count(), 62); // tag + mission + 20 + 40
    assert_eq!(Message::decode(&enc).unwrap(), course);

    assert_eq!(Message::decode(""), Err(ParseError::Empty));
    assert!(matches!(Message::decode("Z 1"), Err(ParseError::UnknownTag(_))));
    assert!(Message::decode("R x").is_err());
    assert!(Message::decode("U 1 2 3").is_err());
    // Java parses Float.toString output such as "1.0E-4"
    assert!(matches!(Message::decode("U 1.0E-4 0 0 0 0 0 0"), Ok(Message::Move { .. })));
}

#[test]
fn matchmaking_rules() {
    assert_eq!(handshake(69, 69, false), HandshakeAction::Proceed);
    assert_eq!(handshake(69, 70, false), HandshakeAction::LeaveAndUpdate);
    assert_eq!(handshake(69, 68, true), HandshakeAction::PeerOutdated { notify: true });
    assert!(must_update(70, 69));
    assert!(!must_update(69, 69));
    assert_eq!(claim_lobby(Some(0)), (Claim::Won, Some(1)));
    assert_eq!(claim_lobby(Some(1)).0, Claim::Lost);
    assert_eq!(claim_lobby(None).0, Claim::Retry);
    assert!(may_send(10, 0));
    assert!(may_send(1000, 5)); // still in the warm-up window
    assert!(!may_send(1000, 6));
    assert!(may_send(306, 6));
    assert_eq!(live_stamp(0), "19700101090000");
    assert_eq!(live_stamp(1_700_000_000), "20231115071320");
}

fn fresh_world(m: Mission, seed: i64) -> (World, SaveGame) {
    let mut save = SaveGame::default();
    save.right_driver = false;
    let mut w = World::new(0, Some(seed));
    w.mission = m;
    w.init_map(&save);
    w.prepare_race(&save);
    (w, save)
}

#[test]
fn world_setup_values() {
    let (w, _) = fresh_world(Mission::Speed, 1);
    // KOS, no upgrades, no damage
    assert!((w.engine_power - (10.0 * 0.029 + 2.9)).abs() < 1e-5);
    assert!((w.brake_power - (10.0 * 0.08 + 7.0)).abs() < 1e-5);
    assert!((w.tire_power - (10.0 * 0.12 + 15.5)).abs() < 1e-5);
    assert!((w.fuel_consume - 60.0 / (40.0 + 20.0)).abs() < 1e-5);
    assert_eq!(w.gas_left, 500.0);
    assert_eq!(w.blocks_for_mission, 6);
    // starts in cell (14,20) heading +y (angle 180), 30 units in, 4.8 left of the centre line
    assert!((w.player.cx - (14.0 * 140.0 - 4.8)).abs() < 1e-3);
    assert!((w.player.cy - (20.0 * 140.0 + 30.0)).abs() < 1e-3);
    assert_eq!(w.player.angle, 180.0);
    assert_eq!(w.dst, (14, 21));
    let (f, _) = fresh_world(Mission::Fuel, 1);
    assert_eq!(f.gas_left, 100.0);
    let (t, _) = fresh_world(Mission::Truck, 1);
    assert_eq!(t.car_mode, 29);
}

#[test]
fn damage_weakens_the_car() {
    let mut save = SaveGame::default();
    save.car_damage[0] = 9000;
    let mut w = World::new(0, Some(3));
    w.mission = Mission::Speed;
    w.prepare_race(&save);
    let f = (11000 - 9000) as f32 * 1e-4; // 0.2
    assert!((w.engine_power - (10.0 * f * 0.029 + 2.9)).abs() < 1e-5);
    let mut ok = SaveGame::default();
    ok.car_damage[0] = 1000; // threshold: no penalty at exactly 1000
    let mut w2 = World::new(0, Some(3));
    w2.prepare_race(&ok);
    assert!((w2.engine_power - (10.0 * 0.029 + 2.9)).abs() < 1e-5);
}

#[test]
fn driving_straight_accelerates_and_advances_blocks() {
    let (mut w, save) = fresh_world(Mission::Speed, 5);
    let input = Controls { accel: true, brake: false, steer: 0.0 };
    let mut advanced = false;
    for _ in 0..(60 * 12) {
        w.update(1.0 / 60.0, mode::PLAYING, input);
        assert!(!w.check_collision(), "went off-road driving straight at t={}", w.travel_time);
        if w.advance_block(&save) {
            advanced = true;
            w.find_next_mission(&save);
            break;
        }
    }
    assert!(w.speed > 5.0, "speed {}", w.speed);
    assert!(w.travel_distance > 80.0, "travelled {}", w.travel_distance);
    assert!(advanced, "never reached the first intersection");
    assert_eq!(w.blocks_passed, 1);
    assert_eq!(w.cur, (14, 21));
    assert!(w.gas_left < 500.0 && w.gas_left > 0.0);
    assert!(w.score < 135.0);
}

#[test]
fn braking_stops_and_steering_off_road_crashes() {
    let (mut w, _) = fresh_world(Mission::Speed, 5);
    let go = Controls { accel: true, brake: false, steer: 0.0 };
    for _ in 0..120 {
        w.update(1.0 / 60.0, mode::PLAYING, go);
    }
    let fast = w.speed;
    assert!(fast > 3.0);
    let stop = Controls { accel: false, brake: true, steer: 0.0 };
    for _ in 0..600 {
        w.update(1.0 / 60.0, mode::PLAYING, stop);
    }
    assert!(w.speed < 0.5 && w.player.v.len() < 0.5, "speed after braking {}", w.speed);

    // hold full lock to one side at speed: the car must leave the road and register a crash
    let (mut c, _) = fresh_world(Mission::Speed, 5);
    let mut crashed = false;
    for i in 0..600 {
        let steer = if i > 60 { 1.0 } else { 0.0 };
        c.update(1.0 / 60.0, mode::PLAYING, Controls { accel: true, brake: false, steer });
        if c.check_accident() {
            crashed = true;
            break;
        }
    }
    assert!(crashed);
    assert!(c.driver.active);
}

#[test]
fn out_of_fuel_and_time_limit() {
    let (mut w, _) = fresh_world(Mission::Fuel, 2);
    let go = Controls { accel: true, brake: false, steer: 0.0 };
    for _ in 0..(60 * 150) {
        w.update(1.0 / 60.0, mode::PLAYING, go);
        if w.gas_left < 0.0 {
            break;
        }
    }
    assert!(w.gas_left < 0.0, "100 units of fuel should run out well before the 180 s limit");
    // broken-brake mission ignores brake input
    let (mut b, _) = fresh_world(Mission::BrokenBrake, 2);
    for _ in 0..120 {
        b.update(1.0 / 60.0, mode::PLAYING, go);
    }
    let v = b.speed;
    for _ in 0..30 {
        b.update(1.0 / 60.0, mode::PLAYING, Controls { accel: false, brake: true, steer: 0.0 });
    }
    assert!(b.speed > v * 0.5, "brakes must not work in BrokenBrake");
}

use drdriving_core::economy::Outcome;
use drdriving_core::session::*;

fn new_session(m: Mission, seed: i64, save: &SaveGame) -> Session {
    let mut w = World::new(0, Some(seed));
    w.mission = m;
    w.init_map(save);
    w.prepare_race(save);
    Session::new(w)
}

#[test]
fn traffic_spawns_and_cars_move() {
    let mut save = SaveGame::default();
    let mut s = new_session(Mission::Highway, 11, &save);
    let hold = Input { accel: true, ..Default::default() };
    let mut max_cars = 0;
    for _ in 0..(60 * 20) {
        s.frame(1.0 / 60.0, &hold, &mut save);
        max_cars = max_cars.max(s.world.cars.iter().filter(|c| c.visible).count());
        if s.is_over() {
            break;
        }
    }
    assert!(max_cars >= 6, "expected traffic on a highway, saw {max_cars}");
    // traffic never uses the player's own model
    assert!(s.world.cars.iter().filter(|c| c.visible).all(|c| c.kind != s.world.car_mode));
    // visible cars have sane lanes and headings
    for c in s.world.cars.iter().filter(|c| c.visible) {
        assert!((0..3).contains(&c.lane));
        assert!([0, 90, 180, 270].contains(&c.angle));
        assert!(c.v.len() <= c.v_max + 1e-3);
    }
}

#[test]
fn traffic_brakes_for_obstacles() {
    use drdriving_core::traffic::TrafficCar;
    let mut w = World::new(0, Some(1));
    let save = SaveGame::default();
    w.init_map(&save);
    let mut dir = GridDir::default();
    dir.set_index(2); // +y
    let mut a = TrafficCar::default();
    a.set(0, 5, 14, 20, dir, 0.0, 0, 4, 36, 36, false);
    let mut b = TrafficCar::default();
    b.set(1, 6, 14, 20, dir, 6.0, 0, 4, 0, 0, false); // parked right in front, same lane, vmax 0
    b.v_max = 0.0;
    let cars = vec![a.clone(), b.clone()];
    let mut moving = a.clone();
    let y0 = moving.rec.c.y;
    for _ in 0..120 {
        moving.update(1.0 / 60.0, &w.player, &w.barricades, &cars);
    }
    assert!(moving.rec.c.y - y0 < 8.0, "should stop short of the car ahead (moved {})", moving.rec.c.y - y0);
    assert!(moving.v.len() < 1.0);
}

#[test]
fn session_start_sequence_and_crash() {
    let mut save = SaveGame::default();
    let mut s = new_session(Mission::Speed, 3, &save);
    assert_eq!(s.phase, Phase::Starting);
    let mut got_go = false;
    for _ in 0..200 {
        let ev = s.frame(1.0 / 60.0, &Input::default(), &mut save);
        if ev.contains(&Event::Go) {
            got_go = true;
            break;
        }
    }
    assert!(got_go && s.phase == Phase::Playing);
    // full right lock at speed leaves the road -> crash ends the run with damage + stats
    let mut crashed = false;
    for _ in 0..(60 * 15) {
        let input = Input { accel: true, steer: 1, ..Default::default() };
        let ev = s.frame(1.0 / 60.0, &input, &mut save);
        if ev.contains(&Event::Crash) {
            crashed = true;
            break;
        }
    }
    assert!(crashed);
    assert_eq!(s.phase, Phase::Ending(Outcome::Accident));
    assert_eq!(save.accident_no, 1);
    assert!(save.car_damage[0] >= 321);
    assert!(s.result.as_ref().unwrap().success.is_none());
    assert!(!s.can_continue());
    s.frame(2.0, &Input::default(), &mut save);
    assert!(s.can_continue());
}

#[test]
fn pause_blinkers_gears_and_wheel() {
    let mut save = SaveGame::default();
    let mut s = new_session(Mission::Speed, 4, &save);
    let ev = s.frame(0.016, &Input { pause: true, ..Default::default() }, &mut save);
    assert!(ev.contains(&Event::Paused(true)));
    assert_eq!(s.phase, Phase::Paused);
    let t = s.world.travel_time;
    s.frame(1.0, &Input::default(), &mut save);
    assert_eq!(s.world.travel_time, t, "world is frozen while paused");
    s.frame(0.016, &Input { pause: true, ..Default::default() }, &mut save);
    assert_eq!(s.phase, Phase::Playing);
    // blinker toggles and sound events
    let ev = s.frame(0.016, &Input { blinker_left: true, ..Default::default() }, &mut save);
    assert_eq!(s.world.blinker, -1);
    assert!(ev.contains(&Event::BlinkerSound(true)));
    s.frame(0.016, &Input { blinker_left: true, ..Default::default() }, &mut save);
    assert_eq!(s.world.blinker, 0);
    // gear change only below 5 units/s
    s.frame(0.016, &Input { gear: Some(1), ..Default::default() }, &mut save);
    assert_eq!(s.world.gear, 1);
    // holding steer turns the wheel to the limit; steering curve is symmetric and monotonic
    for _ in 0..120 {
        s.frame(0.016, &Input { steer: 1, ..Default::default() }, &mut save);
        if s.phase != Phase::Playing {
            break;
        }
    }
    assert!(s.wheel_angle > 100.0 || s.phase != Phase::Playing);
    assert!(steer_from_wheel(540.0, 0) > 0.99 && steer_from_wheel(540.0, 0) <= 1.0);
    assert_eq!(steer_from_wheel(0.1, 0), 0.0);
    assert!((steer_from_wheel(-200.0, 0) + steer_from_wheel(200.0, 0)).abs() < 1e-6);
    assert!(steer_from_wheel(200.0, 4) > steer_from_wheel(200.0, 0), "higher setting = more sensitive steering");
}

#[test]
fn angle_history_lags_the_car() {
    use drdriving_core::world::AngleHistory;
    let mut h = AngleHistory::default();
    for i in 0..20 {
        h.insert(0.016, 180.0 + i as f32); // slowly turning
    }
    let lagged = h.last_angle(0.09);
    assert!(lagged < 199.0 && lagged > 190.0, "{lagged}");
}

#[test]
fn shop_extras() {
    let mut s = SaveGame::default();
    s.coin = 10_000;
    s.driver_level = 0;
    s.car_damage[0] = 3000;
    assert_eq!(upgrade_car(&mut s, 0, 0), Ok(()));
    assert_eq!(upgrade_car(&mut s, 0, 0), Err(ShopError::LowDriverLevel)); // level 1 needs driver level >= 1
    s.driver_level = 1;
    assert_eq!(upgrade_car(&mut s, 0, 0), Ok(()));
    s.car_damage[2] = 5000;
    s.has_car[2] = true;
    let cost = repair_cost(2, 5000);
    let before = s.coin;
    assert_eq!(repair_car(&mut s, 2), Ok(cost));
    assert_eq!(s.coin, before - cost);
    assert_eq!(repair_car(&mut s, 2), Err(ShopError::NothingToRepair));
    s.gold = 5;
    assert_eq!(buy_coin(&mut s, EXCHANGE_RATES[1].0, EXCHANGE_RATES[1].1), Ok(()));
    assert_eq!(s.gold, 0);
}
