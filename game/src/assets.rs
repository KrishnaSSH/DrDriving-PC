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
    /// `world.png`: the 237 country flags shown in online races
    pub world: Texture2D,
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

thread_local! {
    /// Redraws the loading screen. Web only: while a file is being fetched no frame is drawn, and
    /// the browser shows those frames as black flashes.
    static LOADING_DRAW: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

pub fn set_loading_draw(f: Option<Box<dyn Fn()>>) {
    LOADING_DRAW.with(|d| *d.borrow_mut() = f);
}

/// Runs `fut` to completion; on the web the loading screen is redrawn every frame meanwhile.
#[cfg(target_arch = "wasm32")]
async fn present_while<T: 'static + std::any::Any>(fut: impl std::future::Future<Output = T> + 'static + Send) -> T {
    let co = macroquad::experimental::coroutines::start_coroutine(fut);
    while !co.is_done() {
        clear_background(BLACK);
        LOADING_DRAW.with(|d| {
            if let Some(f) = d.borrow().as_ref() {
                f();
            }
        });
        next_frame().await;
    }
    co.retrieve().expect("coroutine finished")
}

/// `load_sound_from_bytes` (on the web the decode takes several frames) without black flashes.
pub async fn decode_sound(data: &[u8]) -> Result<macroquad::audio::Sound, macroquad::Error> {
    #[cfg(target_arch = "wasm32")]
    {
        let d = data.to_vec();
        present_while(async move { macroquad::audio::load_sound_from_bytes(&d).await }).await
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        macroquad::audio::load_sound_from_bytes(data).await
    }
}

/// `load_file`, but on the web keep presenting the loading screen every frame while waiting.
async fn fetch(path: &str) -> Result<Vec<u8>, macroquad::Error> {
    #[cfg(target_arch = "wasm32")]
    {
        let p = path.to_string();
        present_while(async move { load_file(&p).await }).await
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        load_file(path).await
    }
}

pub async fn bytes(base: &str, rel: &str) -> Vec<u8> {
    #[cfg(target_arch = "wasm32")]
    if let Some(b) = bundle::get(rel) {
        return b;
    }
    let path = format!("{base}/{rel}");
    fetch(&path).await.unwrap_or_else(|e| panic!("cannot load {path}: {e}"))
}

/// Web only: every asset except the loading-screen ones is shipped as one `assets.bin` that
/// `index.html` starts downloading immediately (one request, real byte progress), instead of
/// ~170 sequential requests. Layout: repeated `[u16 path len][path][u32 data len][data]`
/// (little endian), written by `web/build.sh`. Files not in it fall back to a normal fetch.
#[cfg(target_arch = "wasm32")]
pub mod bundle {
    use std::cell::RefCell;
    use std::collections::HashMap;

    extern "C" {
        fn dr_bundle_loaded() -> i32;
        fn dr_bundle_total() -> i32;
        /// 0 = downloading, 1 = done, -1 = failed
        fn dr_bundle_state() -> i32;
        fn dr_bundle_read(ptr: *mut u8) -> i32;
    }

    thread_local! {
        static BUNDLE: RefCell<Option<(Vec<u8>, HashMap<String, (usize, usize)>)>> = const { RefCell::new(None) };
    }

    /// (state, bytes loaded, bytes total or 0 if unknown)
    pub fn status() -> (i32, usize, usize) {
        unsafe { (dr_bundle_state(), dr_bundle_loaded().max(0) as usize, dr_bundle_total().max(0) as usize) }
    }

    /// Move the finished download into Rust memory and index it.
    pub fn take() {
        let n = unsafe { dr_bundle_loaded() }.max(0) as usize;
        let mut data = vec![0u8; n];
        let got = unsafe { dr_bundle_read(data.as_mut_ptr()) }.max(0) as usize;
        data.truncate(got);
        let mut index = HashMap::new();
        let mut i = 0usize;
        while i + 2 <= data.len() {
            let pl = u16::from_le_bytes([data[i], data[i + 1]]) as usize;
            i += 2;
            let Some(path) = data.get(i..i + pl).and_then(|b| std::str::from_utf8(b).ok()) else { break };
            let path = path.to_string();
            i += pl;
            let Some(l) = data.get(i..i + 4) else { break };
            let len = u32::from_le_bytes([l[0], l[1], l[2], l[3]]) as usize;
            i += 4;
            if i + len > data.len() {
                break;
            }
            index.insert(path, (i, len));
            i += len;
        }
        BUNDLE.with(|b| *b.borrow_mut() = Some((data, index)));
    }

    pub fn get(rel: &str) -> Option<Vec<u8>> {
        BUNDLE.with(|b| {
            let b = b.borrow();
            let (data, index) = b.as_ref()?;
            let &(off, len) = index.get(rel)?;
            Some(data[off..off + len].to_vec())
        })
    }
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

pub async fn load(base: &str, progress: &mut dyn FnMut(f32, &str, &Texture2D, &Option<Font>)) -> Assets {
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

    let font = match fetch(&format!("{base}/fonts/Roboto-Regular.ttf")).await {
        Ok(b) => load_ttf_font_from_bytes(&b).ok(),
        Err(_) => None,
    };
    let logo = texture(base, "logo.png", false, false).await;
    // web: wait for the single-file asset download (started by index.html), showing MB progress
    #[cfg(target_arch = "wasm32")]
    {
        loop {
            let (state, loaded, size) = bundle::status();
            if state != 0 {
                if state > 0 {
                    bundle::take();
                }
                break;
            }
            let mb = |b: usize| b as f32 / 1048576.0;
            let (frac, label) = if size > 0 {
                ((loaded as f32 / size as f32).min(1.0), format!("Downloading {:.1} / {:.1} MB", mb(loaded), mb(size)))
            } else {
                (0.0, format!("Downloading {:.1} MB", mb(loaded)))
            };
            progress(frac * 0.7, &label, &logo, &font);
            next_frame().await;
        }
    }
    // decoding: only present a frame every ~12 ms, so the many tiny steps don't cost a frame each
    let mut last_present = macroquad::miniquad::date::now();
    macro_rules! step {
        () => {{
            done += 1;
            let now = macroquad::miniquad::date::now();
            if now - last_present > 0.012 || done == total {
                #[cfg(target_arch = "wasm32")]
                let frac = 0.7 + 0.3 * done as f32 / total as f32;
                #[cfg(not(target_arch = "wasm32"))]
                let frac = done as f32 / total as f32;
                progress(frac, "", &logo, &font);
                next_frame().await;
                last_present = macroquad::miniquad::date::now();
            }
        }};
    }
    step!();
    let ui = texture(base, "ui.png", false, false).await;
    step!();
    let menu = texture(base, "menu.png", false, false).await;
    step!();
    let background = texture(base, "background.png", false, false).await;
    step!();
    let world = texture(base, "world.png", false, false).await;

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
        world,
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
