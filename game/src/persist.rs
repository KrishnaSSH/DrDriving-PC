//! Save-file storage. Native: a JSON file (the same JSON the Android app keeps in its
//! SharedPreferences `game` key). Web: the browser's `localStorage`.

use drdriving_core::save::SaveGame;
use std::sync::atomic::{AtomicBool, Ordering};

/// When set (`--ephemeral`), nothing is ever written.
pub static READ_ONLY: AtomicBool = AtomicBool::new(false);

#[cfg(not(target_arch = "wasm32"))]
pub fn save_path() -> std::path::PathBuf {
    use std::path::PathBuf;
    if let Ok(p) = std::env::var("DRDRIVING_SAVE") {
        return PathBuf::from(p);
    }
    // per-OS data directory: %APPDATA% on Windows, ~/Library/Application Support on macOS,
    // $XDG_DATA_HOME (or ~/.local/share) elsewhere
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        home.clone().map(|h| h.join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| home.clone().map(|h| h.join(".local/share")))
    }
    .unwrap_or_else(|| PathBuf::from("."));
    base.join("drdriving").join("save.json")
}

#[cfg(target_arch = "wasm32")]
pub fn save_path() -> String {
    "localStorage[drdriving.save]".to_string()
}

/// Country code from `$LANG` (`en_US.UTF-8` -> `us`) for the right-/left-hand-drive default.
fn locale_country() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var("LC_ALL")
            .or_else(|_| std::env::var("LANG"))
            .ok()
            .and_then(|l| l.split('.').next().map(str::to_string))
            .and_then(|l| l.split('_').nth(1).map(str::to_lowercase))
            .unwrap_or_default()
    }
    #[cfg(target_arch = "wasm32")]
    {
        String::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read() -> Option<String> {
    std::fs::read_to_string(save_path()).ok()
}
#[cfg(target_arch = "wasm32")]
extern "C" {
    fn dr_storage_set(ptr: *const u8, len: u32);
    fn dr_storage_get(ptr: *mut u8, cap: u32) -> i32;
}

#[cfg(target_arch = "wasm32")]
fn read() -> Option<String> {
    let mut cap = 64 * 1024usize;
    loop {
        let mut buf = vec![0u8; cap];
        let n = unsafe { dr_storage_get(buf.as_mut_ptr(), cap as u32) };
        if n == 0 {
            return None;
        }
        if n < 0 {
            cap = (-n) as usize + 16;
            continue;
        }
        buf.truncate(n as usize);
        return String::from_utf8(buf).ok();
    }
}

pub fn load() -> SaveGame {
    match read() {
        Some(text) if !text.trim().is_empty() => {
            let mut s = SaveGame::from_json_lenient(&text);
            s.not_first = true;
            s
        }
        _ => SaveGame::zero_for_country(&locale_country()),
    }
}

pub fn store(save: &SaveGame) {
    if READ_ONLY.load(Ordering::Relaxed) {
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = save_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // write-then-rename so a crash never leaves a half-written save
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, save.to_json()).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let json = save.to_json();
        unsafe { dr_storage_set(json.as_ptr(), json.len() as u32) };
    }
}
