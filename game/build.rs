//! Chooses the audio backend at compile time.
//!
//! macroquad's built-in mixer opens the ALSA device *directly*. On a PipeWire/PulseAudio desktop
//! that steals the sound card from the sound server (every app then falls back to "Dummy
//! Output"), so on Linux the game does not use it and plays through the sound server instead.
//! On the web and on other platforms the built-in mixer is used (`cfg(native_audio)`).
fn main() {
    println!("cargo:rustc-check-cfg=cfg(native_audio)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if arch == "wasm32" || os != "linux" {
        println!("cargo:rustc-cfg=native_audio");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
