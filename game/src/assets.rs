//! Loads every texture, model and font the game uses. Works on native (reads the extracted
//! `assets/` folder) and on the web (fetches `assets/...` relative to the page) through
//! macroquad's async file API.

use crate::model::Model;
use macroquad::file::load_file;
use macroquad::miniquad::{FilterMode, TextureWrap};
use macroquad::prelude::*;

pub struct Assets {
    pub ui: Texture2D,
    pub menu: Texture2D,
    pub background: Texture2D,
    pub logo: Texture2D,
    pub car_tex: Vec<Texture2D>,
    pub interior: Vec<Texture2D>,
    pub road_tex: Vec<Texture2D>,
    pub car_model: Vec<Model>,
    pub light_model: Vec<Model>,
    pub road_model: Vec<Model>,
    pub font: Option<Font>,
    pub mats: crate::scene::Materials,
    /// base path/URL the assets were loaded from (audio uses it too)
    pub base: String,
}

/// Where the extracted APK assets live. Native: `$DRDRIVING_ASSETS`, `./assets`, or any parent's
/// `assets/`. Web: `assets` next to the page.
pub fn find_assets() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        "assets".to_string()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::path::Path;
        if let Ok(p) = std::env::var("DRDRIVING_ASSETS") {
            return p;
        }
        let mut starts = vec![std::env::current_dir().unwrap_or_default()];
        if let Ok(exe) = std::env::current_exe() {
            if let Some(d) = exe.parent() {
                starts.push(d.to_path_buf());
            }
        }
        for s in starts {
            let mut d: Option<&Path> = Some(&s);
            while let Some(dir) = d {
                let cand = dir.join("assets");
                if cand.join("ui.png").exists() {
                    return cand.to_string_lossy().into_owned();
                }
                d = dir.parent();
            }
        }
        panic!("assets/ folder not found (set DRDRIVING_ASSETS to the extracted APK assets directory)");
    }
}

pub async fn bytes(base: &str, rel: &str) -> Vec<u8> {
    let path = format!("{base}/{rel}");
    load_file(&path).await.unwrap_or_else(|e| panic!("cannot load {path}: {e}"))
}

async fn texture(base: &str, rel: &str, repeat: bool, mip: bool) -> Texture2D {
    let b = bytes(base, rel).await;
    let img = Image::from_file_with_format(&b, None).unwrap_or_else(|e| panic!("bad image {rel}: {e}"));
    let t = Texture2D::from_image(&img);
    t.set_filter(FilterMode::Linear);
    if repeat {
        // the original relies on GL's default REPEAT: road, fence, tyre and sky UVs run outside 0..1
        let gl = unsafe { get_internal_gl() };
        gl.quad_context.texture_set_wrap(t.raw_miniquad_id(), TextureWrap::Repeat, TextureWrap::Repeat);
    }
    let _ = mip;
    t
}

/// Car texture whose baked shadow silhouette (opaque, near-black, hard edged) is turned into a soft
/// translucent shadow: alpha follows how dark each texel is, capped at ~45%.
async fn car_texture(base: &str, rel: &str, shadow: Option<[f32; 4]>) -> Texture2D {
    let b = bytes(base, rel).await;
    let mut img = Image::from_file_with_format(&b, None).unwrap_or_else(|e| panic!("bad image {rel}: {e}"));
    if let Some([u0, v0, u1, v1]) = shadow {
        let (iw, ih) = (img.width as usize, img.height as usize);
        let (w, h) = (iw as f32, ih as f32);
        let (x0, x1) = ((u0 * w).floor() as usize, ((u1 * w).ceil() as usize).min(iw));
        let (y0, y1) = ((v0 * h).floor() as usize, ((v1 * h).ceil() as usize).min(ih));
        const PAD: usize = 2;
        for y in y0.saturating_sub(PAD)..(y1 + PAD).min(ih) {
            for x in x0.saturating_sub(PAD)..(x1 + PAD).min(iw) {
                let inside = (y0..y1).contains(&y) && (x0..x1).contains(&x);
                let px = &mut img.bytes[(y * iw + x) * 4..][..4];
                if px[3] == 0 {
                    // keep transparent texels black so filtering can't pull in a light fringe
                    px.copy_from_slice(&[0, 0, 0, 0]);
                } else if inside {
                    let lum = (px[0] as f32 + px[1] as f32 + px[2] as f32) / 3.0;
                    let a = ((200.0 - lum) / 200.0).clamp(0.0, 1.0) * 0.45;
                    px.copy_from_slice(&[0, 0, 0, (a * 255.0) as u8]);
                }
            }
        }
    }
    let t = Texture2D::from_image(&img);
    t.set_filter(FilterMode::Linear);
    t
}

async fn model(base: &str, rel: &str) -> Model {
    let b = bytes(base, rel).await;
    Model::parse(&String::from_utf8_lossy(&b))
}

pub async fn load(base: &str, progress: &mut dyn FnMut(usize, usize, &Texture2D, &Option<Font>)) -> Assets {
    let car_png = |i: usize| match i {
        28 => "cars/AI_car01.png".to_string(),
        29 => "cars/AI_car02.png".to_string(),
        _ => format!("cars/car{i:02}.png"),
    };
    let car_obj = |i: usize| match i {
        28 => "cars/AI_car01.obj".to_string(),
        29 => "cars/AI_car02.obj".to_string(),
        _ => format!("cars/car{i:02}.obj"),
    };
    let light_obj = |i: usize| match i {
        28 => "cars/light_AI01.obj".to_string(),
        29 => "cars/light_AI02.obj".to_string(),
        _ => format!("cars/light{i:02}.obj"),
    };
    // index order is the one used by the original `Assets.roadTextures` / `roadModels`
    let road_png = [
        "road_road", "road_barricade", "check", "sky", "road_traffic_off", "road_traffic_on", "tiretrack", "tire", "driver", "light", "police", "vip",
        "parking_box", "road_block_1", "road_block_2",
    ];
    let road_obj = [
        "road_road", "road_barricade", "check", "sky", "road_traffic_off", "road_traffic_on", "tiretrack", "tire", "driver", "drivers", "police", "polices",
        "parking_off", "parking_on", "driver2", "driver2s", "vip", "vips", "road_block", "road_fence",
    ];
    let interior = |i: usize| if i >= 28 { "interior/interiorTruck.png".to_string() } else { format!("interior/interior{i:02}.png") };

    let total = 6 + 30 + road_png.len() + road_obj.len();
    let mut done = 0usize;

    let font = match load_file(&format!("{base}/fonts/Roboto-Regular.ttf")).await {
        Ok(b) => load_ttf_font_from_bytes(&b).ok(),
        Err(_) => None,
    };
    let logo = texture(base, "logo.png", false, false).await;
    macro_rules! step {
        () => {{
            done += 1;
            progress(done, total, &logo, &font);
            next_frame().await;
        }};
    }
    step!();
    let ui = texture(base, "ui.png", false, false).await;
    step!();
    let menu = texture(base, "menu.png", false, false).await;
    step!();
    let background = texture(base, "background.png", false, false).await;
    step!();

    let mut car_tex = Vec::new();
    let mut interior_tex = Vec::new();
    let mut car_model = Vec::new();
    let mut light_model = Vec::new();
    for i in 0..30 {
        let cm = model(base, &car_obj(i)).await;
        car_tex.push(car_texture(base, &car_png(i), cm.shadow_uv_rect()).await);
        interior_tex.push(texture(base, &interior(i), false, false).await);
        car_model.push(cm);
        light_model.push(model(base, &light_obj(i)).await);
        step!();
    }
    let mut road_tex = Vec::new();
    for n in road_png {
        road_tex.push(texture(base, &format!("envi/{n}.png"), true, true).await);
        step!();
    }
    let mut road_model = Vec::new();
    for n in road_obj {
        road_model.push(model(base, &format!("envi/{n}.obj")).await);
        step!();
    }

    Assets {
        ui,
        menu,
        background,
        logo,
        car_tex,
        interior: interior_tex,
        road_tex,
        car_model,
        light_model,
        road_model,
        font,
        mats: crate::scene::make_materials(),
        base: base.to_string(),
    }
}
