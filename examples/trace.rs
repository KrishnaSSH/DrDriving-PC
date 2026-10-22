use drdriving_core::{mission::Mission, save::SaveGame, world::*};
fn main() {
    let save = SaveGame::default();
    let mut w = World::new(0, Some(5));
    w.mission = Mission::Speed;
    w.init_map(&save);
    w.prepare_race(&save);
    let input = Controls { accel: true, brake: false, steer: 0.0 };
    for i in 0..600 {
        w.update(1.0 / 60.0, mode::PLAYING, input);
        if i % 30 == 0 {
            println!("t={:5.2} y={:8.2} v={:6.2} dstY={} g_dy={:6.2} blocks={} coll={}", w.travel_time, w.player.cy, w.speed, w.dst.1 * 140, w.g_dy, w.blocks_passed, w.check_collision());
        }
        if w.advance_block(&save) { println!("advanced at t={:.2}", w.travel_time); break; }
    }
}
