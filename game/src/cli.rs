//! Native-only entry: command-line options, scripted runs and screenshot scenes.

use crate::game::Game;
use crate::keys::Keys;
use crate::{assets, boot, game_loop, persist};
use drdriving_core::mission::Mission;
use drdriving_core::save::SaveGame;
use macroquad::prelude::*;

struct Opts {
    shot: Option<String>,
    scene: String,
    seed: Option<i64>,
    wait: u32,
    autopilot: bool,
    script: String,
    shots: Vec<(f32, String)>,
    shot_dir: String,
    quit_after: Option<f32>,
    ephemeral: bool,
    no_traffic: bool,
    ff: u32,
    audio_test: bool,
    gui: i32,
    /// `Some(0)` = Online Match, `Some(n)` = friend channel n.
    online: Option<i32>,
    name: Option<String>,
}

fn parse() -> Opts {
    let mut c = Opts {
        shot: None,
        scene: "menu".into(),
        seed: None,
        wait: 3,
        autopilot: false,
        script: String::new(),
        shots: Vec::new(),
        shot_dir: ".".into(),
        quit_after: None,
        ephemeral: false,
        no_traffic: false,
        ff: 1,
        audio_test: false,
        gui: 0,
        online: None,
        name: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--shot" => c.shot = it.next(),
            "--scene" => c.scene = it.next().unwrap_or_default(),
            "--gui" => c.gui = it.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            "--seed" => c.seed = it.next().and_then(|s| s.parse().ok()),
            "--wait" => c.wait = it.next().and_then(|s| s.parse().ok()).unwrap_or(3),
            "--autopilot" => c.autopilot = true,
            "--ephemeral" => c.ephemeral = true,
            "--no-traffic" => c.no_traffic = true,
            "--audio-test" => c.audio_test = true,
            "--online" => c.online = Some(it.next().and_then(|s| s.parse().ok()).unwrap_or(0)),
            "--db" => {
                if let Some(u) = it.next() {
                    std::env::set_var("DRDRIVING_DB", u);
                }
            }
            "--name" => c.name = it.next(),
            "--ff" => c.ff = it.next().and_then(|s| s.parse().ok()).unwrap_or(1).max(1),
            "--script" => c.script = it.next().unwrap_or_default(),
            "--shot-dir" => c.shot_dir = it.next().unwrap_or_default(),
            "--quit-after" => c.quit_after = it.next().and_then(|s| s.parse().ok()),
            "--shots" => {
                c.shots = it
                    .next()
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|x| x.split_once(':').and_then(|(t, n)| Some((t.parse().ok()?, n.to_string()))))
                    .collect();
                c.shots.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            }
            "-h" | "--help" => {
                println!(
                    "drdriving [options]\n  --seed N               seed the mission generator\n  --autopilot            demo mode: the game plays itself\n  --script \"t:Key,..\"    press keys at given game times\n  --shots \"t:name,..\"    save screenshots at game times into --shot-dir\n  --quit-after SECONDS   exit after this much game time\n  --ff N                 N simulation steps per rendered frame\n  --no-traffic           hide AI traffic\n  --ephemeral            use a demo save, never write to disk\n  --shot FILE --scene S  one scene: menu garage store options carselect race race2 race3 highway ending pause dialog exit\n  --audio-test           report the audio backend\n  --online [CHANNEL]     start searching for an opponent at once (Online Match, or friend channel 100-999)\n  --db URL               multiplayer database (default: the original game's Firebase; or env DRDRIVING_DB)\n  --name NAME            player name shown to the opponent"
                );
                std::process::exit(0);
            }
            _ => {}
        }
    }
    c
}

/// A well-stocked save for screenshots (never written to disk).
fn demo_save() -> SaveGame {
    let mut s = SaveGame::zero_for_country("us");
    s.coin = 4200;
    s.gold = 35;
    s.driver_level = 3;
    s.driver_exp = 17000;
    for i in [1, 2, 3, 7] {
        s.has_car[i] = true;
    }
    s.upgrade[1] = [3, 2, 1, 0, 0, 2];
    s.car_damage[1] = 3000;
    s.record = [8120, 7000, 5400, 9100, 0, 0, 6000, 7800, 5100];
    s.mission_clear = [12, 5, 3, 4, 0, 0, 2, 3, 1];
    s.missions_clear_count = 30;
    s.travel_distance = 52_000;
    s.travel_second = 4200;
    s.max_speed = 31_000;
    s
}

pub async fn run() {
    let o = parse();
    let base = assets::find_assets();
    if o.ephemeral || o.shot.is_some() {
        persist::READ_ONLY.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let mut save = if o.shot.is_some() || o.ephemeral { demo_save() } else { persist::load() };
    save.gui_size = if o.gui > 0 { o.gui } else { save.gui_size };
    let seed = o.seed.unwrap_or_else(|| (macroquad::miniquad::date::now() * 1000.0) as i64);
    let mut game = boot(&base, save, seed).await;
    if o.audio_test {
        println!("audio backend: {}", game.audio.backend_name());
        game.audio.play(crate::audio::Snd::Horn, 1.0);
        for _ in 0..120 {
            next_frame().await;
        }
        return;
    }
    if let Some(n) = &o.name {
        game.lobby.name = n.clone();
    }
    game.auto = o.autopilot;
    game.no_traffic = o.no_traffic;
    if !o.script.is_empty() {
        game.keys = Keys::with_script(&o.script);
    }
    match o.online {
        Some(0) => game.start_quick(),
        Some(n) => game.start_channel(n),
        None => {}
    }
    if let Some(path) = &o.shot {
        screenshot(&mut game, &o, path).await;
        return;
    }
    let scripted = (o.autopilot || !o.script.is_empty() || !o.shots.is_empty() || o.quit_after.is_some()) && std::env::var_os("DRDRIVING_PERF").is_none();
    if !scripted {
        game_loop(&mut game).await;
        return;
    }
    let mut sim = 0.0f32;
    let mut shot_i = 0usize;
    loop {
        for _ in 0..o.ff {
            game.update(1.0 / 60.0);
            sim += 1.0 / 60.0;
            if shot_i < o.shots.len() && o.shots[shot_i].0 <= sim {
                break;
            }
        }
        game.draw();
        while shot_i < o.shots.len() && o.shots[shot_i].0 <= sim {
            let p = format!("{}/{}.png", o.shot_dir, o.shots[shot_i].1);
            get_screen_data().export_png(&p);
            shot_i += 1;
        }
        if game.quit || o.quit_after.map_or(false, |q| sim >= q) {
            break;
        }
        next_frame().await;
    }
}

async fn screenshot(game: &mut Game, o: &Opts, path: &str) {
    use drdriving_core::session::Phase;
    let sc = o.scene.as_str();
    match sc {
        "garage" => game.enter_mode(9),
        "store" => game.enter_mode(10),
        "options" => game.enter_mode(11),
        "carselect" => {
            game.world.car_mode = 1;
            game.enter_mode(1);
        }
        "dialog" => {
            game.enter_mode(9);
            game.set_dialog(true, 1, 1, 0);
        }
        "exit" => {
            game.enter_mode(8);
            game.enter_mode(13);
        }
        "channel" => {
            game.enter_mode(8);
            game.lobby.channel_text = "427".into();
            game.enter_lobby();
        }
        "name" => {
            game.enter_mode(11);
            game.edit_name();
        }
        "country" => {
            game.lobby.country = "IN".into();
            game.enter_mode(11);
            game.edit_country();
        }
        "toast" => {
            game.enter_mode(8);
            game.show_toast("Please input the same number with your friend between 100~999.");
        }
        "menu" => {}
        _ => {
            game.world.car_mode = 1;
            game.world.mission = if sc == "highway" { Mission::Highway } else { Mission::Speed };
            game.enter_mode(1);
            game.start_race();
            if let Some(s) = game.session.as_mut() {
                s.camera_mode = match sc {
                    "race2" => 2,
                    "race3" => 3,
                    _ => 4,
                };
            }
            game.no_traffic = sc == "race" || sc == "race2";
        }
    }
    let race = matches!(sc, "race" | "race2" | "race3" | "highway" | "ending" | "pause");
    let mut t = 0.0f32;
    for frame in 0..o.wait.max(1) {
        if race {
            if sc == "ending" && t > 4.0 {
                game.keys.inject(KeyCode::D);
            }
            game.update(1.0 / 60.0);
            if sc == "pause" && frame == o.wait / 2 {
                game.keys.inject(KeyCode::Escape);
            }
            t += 1.0 / 60.0;
        } else {
            game.update(1.0 / 60.0);
        }
        game.draw();
        if frame + 1 >= o.wait.max(1) {
            get_screen_data().export_png(path);
            let _ = Phase::Playing;
            return;
        }
        next_frame().await;
    }
}
