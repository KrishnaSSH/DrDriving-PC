mod assets;
mod audio;
mod game;
mod gfx;
mod keys;
mod model;
mod pad;
mod persist;
mod scene;
mod sprites;
mod ui_menu;
mod ui_race;

use game::Game;
use macroquad::prelude::*;

fn conf() -> Conf {
    audio::install_panic_hook();
    Conf {
        window_title: "Dr. Driving".to_string(),
        window_width: 1280,
        window_height: 720,
        high_dpi: true,
        sample_count: 4,
        ..Default::default()
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod cli;

#[macroquad::main(conf)]
async fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        cli::run().await;
    }
    #[cfg(target_arch = "wasm32")]
    {
        run_web().await;
    }
}

/// Load everything (drawing the original loading screen), return a ready `Game`.
pub async fn boot(base: &str, save: drdriving_core::save::SaveGame, seed: i64) -> Game {
    let mut g = gfx::Gfx::new();
    let cur = (seed.unsigned_abs() % 24) as usize;
    let a = assets::load(base, &mut |done, total, logo, font| {
        g.layout();
        clear_background(BLACK);
        let text = gfx::GlText { font: font.clone() };
        ui_menu::draw_loading(&mut g, logo, &text, (done * 100 / total) as i32, cur);
    })
    .await;
    let audio = audio::Audio::load(base).await;
    Game::new(a, audio, save, seed)
}

/// One fixed-step frame loop shared by native and web.
pub async fn game_loop(game: &mut Game) {
    let mut acc = 0.0f32;
    let perf = std::env::var_os("DRDRIVING_PERF").is_some();
    let (mut t_up, mut t_dr, mut frames) = (0.0f64, 0.0f64, 0u32);
    loop {
        acc += get_frame_time().min(0.1);
        let t0 = macroquad::miniquad::date::now();
        let mut n = 0;
        while acc >= 1.0 / 60.0 && n < 5 {
            game.update(1.0 / 60.0);
            acc -= 1.0 / 60.0;
            n += 1;
        }
        let t1 = macroquad::miniquad::date::now();
        game.draw();
        if perf {
            t_up += t1 - t0;
            t_dr += macroquad::miniquad::date::now() - t1;
            frames += 1;
            if frames == 120 {
                eprintln!("perf: update {:.2} ms/frame, draw(cpu) {:.2} ms/frame, fps {}", t_up * 1000.0 / 120.0, t_dr * 1000.0 / 120.0, get_fps());
                (t_up, t_dr, frames) = (0.0, 0.0, 0);
            }
        }
        if game.quit {
            break;
        }
        next_frame().await;
    }
    game.persist();
}

#[cfg(target_arch = "wasm32")]
async fn run_web() {
    let seed = (macroquad::miniquad::date::now() * 1000.0) as i64;
    let save = persist::load();
    let mut game = boot(&assets::find_assets(), save, seed).await;
    game_loop(&mut game).await;
    // the browser tab can't "quit": just stop
    loop {
        next_frame().await;
    }
}
