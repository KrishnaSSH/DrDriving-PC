//! A simple autopilot (also used as the game's `--autopilot` demo mode and by the end-to-end tests).
//! It keeps its lane, slows for junctions and turns in two phases: hold the approach line until the
//! car is deep enough in the junction box (the game advances to the next block 10 units before
//! the centre, and the medians run right up to that point), then arc into the exit lane.
//!
//! It does not avoid traffic, drift on purpose, or park — it is a driving demonstration and test
//! driver, not an AI opponent.

use crate::world::World;

/// Wheel angle (deg) that produces steering factor `u` (inverse of `steer_from_wheel`, sens 0).
pub fn wheel_for(u: f32) -> f32 {
    if u.abs() < 0.02 {
        return 0.0;
    }
    u.signum() * (u.abs().sqrt() * 620.0 - 80.0).clamp(0.0, 540.0)
}

fn angle_diff(a: f32, b: f32) -> f32 {
    let mut d = (a - b) % 360.0;
    if d > 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d
}

/// Autopilot. Keeps its lane, brakes for junctions, and — because the game advances to the next
/// block 10 units before the junction centre — turns in two phases: hold the approach line until
/// the car is deep enough in the junction box (so it doesn't clip the median), then arc into the
/// exit lane. Returns (accel, brake, desired wheel angle).
pub fn drive(w: &World, rd: bool) -> (bool, bool, f32) {
    let side = if rd { -1.0 } else { 1.0 };
    let lane = 4.8 * side;
    let perp = |v: (f32, f32)| (-v.1, v.0);
    let d = (w.curr_dir.x as f32, w.curr_dir.y as f32);
    let p = perp(d);
    let pos = (w.player.cx, w.player.cy);
    let dir_ang = d.0.atan2(-d.1).to_degrees();
    let herr = angle_diff(dir_ang, w.player.angle);
    let jc = (w.cur.0 as f32 * 140.0, w.cur.1 as f32 * 140.0);
    let near_j = ((pos.0 - jc.0).powi(2) + (pos.1 - jc.1).powi(2)).sqrt() < 22.0;
    // the post-flip turning phase ends once the car has cleared the junction box
    let turning_now = herr.abs() > 12.0 && near_j;

    let (aim, hold_line): ((f32, f32), bool);
    let mut target_speed = 9.0;
    if turning_now {
        // post-flip: `cur` is the junction being crossed, `d` the exit direction
        let j = (w.cur.0 as f32 * 140.0, w.cur.1 as f32 * 140.0);
        let ha = (w.player.angle / 90.0).round() * 90.0;
        let hv = (ha.to_radians().sin(), -ha.to_radians().cos());
        let ahead = (j.0 - pos.0) * hv.0 + (j.1 - pos.1) * hv.1;
        let thr = if w.rel_dir_cur == 2 { 2.5 } else { 4.5 };
        target_speed = 3.2;
        if ahead > thr && herr.abs() > 60.0 {
            // keep driving straight along the approach road
            let a_lane = perp(hv);
            let lat = (pos.0 - j.0) * a_lane.0 + (pos.1 - j.1) * a_lane.1;
            let correct = (side * 4.8 - lat).clamp(-2.0, 2.0);
            aim = (pos.0 + hv.0 * 10.0 + a_lane.0 * correct, pos.1 + hv.1 * 10.0 + a_lane.1 * correct);
            hold_line = true;
        } else {
            aim = (j.0 + d.0 * 22.0 + p.0 * lane, j.1 + d.1 * 22.0 + p.1 * lane);
            hold_line = false;
        }
    } else {
        let dst = (w.dst.0 as f32 * 140.0, w.dst.1 as f32 * 140.0);
        let along = (pos.0 - dst.0) * d.0 + (pos.1 - dst.1) * d.1; // negative before the junction
        let lat_target = (dst.0 + p.0 * lane, dst.1 + p.1 * lane);
        aim = (lat_target.0 + d.0 * (along + 18.0), lat_target.1 + d.1 * (along + 18.0));
        hold_line = true;
        if w.rel_dir_next != 0 && -along < 50.0 {
            target_speed = 4.0;
        }
    }
    let want = ((aim.0 - pos.0).atan2(-(aim.1 - pos.1))).to_degrees();
    let mut err = angle_diff(want, w.player.angle);
    if !hold_line && err.abs() > 100.0 {
        // near-180 degree error (U-turn): choose the side from where the exit lane lies
        let a = w.player.angle.to_radians();
        let lateral = (aim.0 - pos.0) * a.cos() + (aim.1 - pos.1) * a.sin();
        err = if lateral >= 0.0 { err.abs() } else { -err.abs() };
    }
    let v = w.speed.max(1.0);
    let omega = (2.4 * err).clamp(-130.0, 130.0);
    let u = omega / (w.player.wheel_factor * v);
    let accel = w.speed < target_speed;
    let brake = w.speed > target_speed + 2.5;
    (accel, brake, wheel_for(u.clamp(-1.0, 1.0)))
}
