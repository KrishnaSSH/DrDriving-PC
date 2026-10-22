//! End-to-end: an autopilot drives complete missions through the public `Session` API, so route
//! generation, junction logic, steering, checkpoints and the reward path are exercised together.

use drdriving_core::economy::Outcome;
use drdriving_core::mission::Mission;
use drdriving_core::save::SaveGame;
use drdriving_core::session::*;
use drdriving_core::world::*;

use drdriving_core::autopilot::{drive};

fn run_mission(mission: Mission, seed: i64, traffic: bool, max_secs: f32) -> (Phase, Session, SaveGame) {
    let mut save = SaveGame::default();
    let mut world = World::new(0, Some(seed));
    world.mission = mission;
    world.init_map(&save);
    world.prepare_race(&save);
    let mut s = Session::new(world);
    let dt = 1.0 / 60.0;
    for _ in 0..(max_secs / dt) as usize {
        if !traffic {
            for c in s.world.cars.iter_mut() {
                c.visible = false;
            }
        }
        let (accel, brake, wheel) = drive(&s.world, save.right_driver);
        if s.phase == Phase::Playing {
            s.wheel_angle = wheel;
        }
        s.frame(dt, &Input { accel, brake, ..Default::default() }, &mut save);
        if s.is_over() {
            break;
        }
    }
    (s.phase, s, save)
}

#[test]
fn autopilot_completes_speed_missions_on_empty_roads() {
    let mut wins = 0;
    for k in 0..10 {
        let seed = 1000 + k * 7919; // spread seeds: Java's Random is correlated for consecutive small seeds
        let (phase, s, save) = run_mission(Mission::Speed, seed, false, 170.0);
        if phase == Phase::Success {
            wins += 1;
            let res = s.result.as_ref().unwrap();
            assert_eq!(res.outcome, Outcome::Success);
            let sr = res.success.as_ref().unwrap();
            assert!(sr.reward_coins >= 25);
            assert_eq!(save.mission_clear[0], 1);
            assert!(save.record[0] > 0);
            assert!(s.world.blocks_passed > s.world.blocks_for_mission);
        } else {
            eprintln!("seed {seed}: {:?} after {:.0}s blocks={} pos=({:.0},{:.0}) dst={:?}", phase, s.world.travel_time, s.world.blocks_passed, s.world.player.cx, s.world.player.cy, s.world.dst);
        }
    }
    assert!(wins >= 8, "autopilot should finish nearly every empty-road run, got {wins}/10");
}

#[test]
fn autopilot_completes_other_missions() {
    // missions with straight/oriented finishes that the controller can handle on empty roads
    for (m, seed) in [(Mission::Highway, 3), (Mission::Fuel, 5), (Mission::BrokenBrake, 6)] {
        let (phase, s, save) = run_mission(m, seed, false, 175.0);
        assert_eq!(phase, Phase::Success, "{m:?} should be winnable: t={:.0} blocks={}", s.world.travel_time, s.world.blocks_passed);
        assert_eq!(save.mission_clear[m.index()], 1);
        assert!(save.coin >= 25);
    }
    // Drift needs 40 units of deliberate drifting before the finish opens: the bot never drifts,
    // so the checkpoint must stay closed (no accidental completion).
    let (phase, s, _) = run_mission(Mission::Drift, 4, false, 175.0);
    assert!(matches!(phase, Phase::Playing | Phase::Ending(_)));
    assert!(!s.world.checkpoint.active || s.world.drift_distance > 40.0);
}

#[test]
fn autopilot_with_traffic_never_panics_and_usually_survives() {
    let mut finished = 0;
    for seed in 20..28 {
        let (phase, _s, _) = run_mission(Mission::Speed, seed, true, 120.0);
        if matches!(phase, Phase::Success) {
            finished += 1;
        }
    }
    eprintln!("with traffic: {finished}/8 reached the finish without crashing");
}

#[test]
fn parking_spot_accepts_aligned_car_in_park_only() {
    let save = SaveGame::default();
    let mut w = World::new(0, Some(9));
    w.mission = Mission::Parking;
    w.init_map(&save);
    w.prepare_race(&save);
    // force the finish phase: pretend two blocks have been driven and look at the spot
    w.blocks_passed = w.blocks_for_mission + 1;
    w.find_next_mission(&save);
    assert!(w.parking.active, "parking spot should exist after the required blocks");
    let spot = w.parking.rect.c;
    let along_y = w.curr_dir.x == 0;
    let good = if along_y { 180.0 } else { 90.0 };
    w.player.cx = spot.x;
    w.player.cy = spot.y;
    w.player.angle = good;
    w.gear = GEAR_D;
    assert!(!w.pass_check_point(), "must be in P to finish");
    assert!(w.in_parking_lot, "inside the box and aligned");
    w.gear = GEAR_P;
    assert!(w.pass_check_point(), "aligned, centred and in park should complete");
    // sideways across the road must not count
    w.player.angle = if along_y { 90.0 } else { 180.0 };
    assert!(!w.pass_check_point());
    // too far from the spot centre
    w.player.angle = good;
    w.player.cx = spot.x + 1.5;
    assert!(!w.pass_check_point());
}
