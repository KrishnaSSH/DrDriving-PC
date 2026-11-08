//! Sound output.
//!
//! * Web / non-Linux: macroquad's built-in mixer (`cfg(native_audio)`).
//! * Linux: the system sound server, through `pw-play`/`paplay` (one-shots) and `ffplay` (loops).
//!   macroquad's own mixer is deliberately not used there: it opens the ALSA device directly,
//!   which takes the sound card away from PipeWire/PulseAudio and silences every other app.
//!
//! All external-player work happens on a worker thread so the game loop never blocks on it.

#[cfg(native_audio)]
use macroquad::audio::{load_sound_from_bytes, play_sound, set_sound_volume, stop_sound, PlaySoundParams, Sound};
#[cfg(not(target_arch = "wasm32"))]
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Snd {
    Start,
    Crash,
    Clear,
    Click,
    Submission,
    Clap,
    Horn,
    Brake,
    Blinker,
    Drift,
    Engine,
    Music,
}

const ALL: [(Snd, &str); 12] = [
    (Snd::Start, "start2.ogg"),
    (Snd::Crash, "Crash2.ogg"),
    (Snd::Clear, "toggle2.ogg"),
    (Snd::Click, "click1.ogg"),
    (Snd::Submission, "submission.ogg"),
    (Snd::Clap, "clap.ogg"),
    (Snd::Horn, "horn.ogg"),
    (Snd::Brake, "brake.ogg"),
    (Snd::Blinker, "blinker.ogg"),
    (Snd::Drift, "driftlow.ogg"),
    (Snd::Engine, "engine6.ogg"),
    (Snd::Music, "newlife.ogg"),
];

enum Backend {
    #[cfg(native_audio)]
    Native(Vec<Sound>),
    #[cfg(not(target_arch = "wasm32"))]
    Players { tx: std::sync::mpsc::Sender<players::Cmd>, names: String },
}

pub struct Audio {
    backend: Backend,
    #[cfg(not(target_arch = "wasm32"))]
    paths: Vec<PathBuf>,
    flags: Vec<bool>,
    pub enabled: bool,
}

#[cfg(not(target_arch = "wasm32"))]
mod players {
    use super::*;

    /// Spawn `cmd` so the child dies with us (a looping `ffplay` must never outlive the game).
    pub fn spawn_tied(cmd: &mut Command) -> std::io::Result<Child> {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: prctl is async-signal-safe and only touches this (forked) process.
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                    Ok(())
                });
            }
        }
        cmd.stdout(Stdio::null()).stderr(Stdio::null()).spawn()
    }

    pub fn have(cmd: &str) -> bool {
        Command::new(cmd).arg("--help").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
    }

    pub enum Cmd {
        Play { path: PathBuf, vol: f32 },
        LoopStart { i: usize, path: PathBuf, vol: f32 },
        LoopStop { i: usize },
    }

    pub fn worker(rx: std::sync::mpsc::Receiver<Cmd>, one_shot: Option<&'static str>, looper: Option<&'static str>, n: usize) {
        let mut looping: Vec<Option<Child>> = (0..n).map(|_| None).collect();
        let mut oneshots: Vec<Child> = Vec::new();
        while let Ok(cmd) = rx.recv() {
            oneshots.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
            match cmd {
                Cmd::Play { path, vol } => {
                    let child = match (one_shot, looper) {
                        (Some("pw-play"), _) => spawn_tied(Command::new("pw-play").arg(format!("--volume={vol}")).arg(&path)),
                        (Some(p), _) => spawn_tied(Command::new(p).arg(format!("--volume={}", (vol * 65536.0) as i32)).arg(&path)),
                        (None, Some(f)) => spawn_tied(
                            Command::new(f).args(["-nodisp", "-autoexit", "-loglevel", "quiet", "-volume", &format!("{}", (vol * 100.0) as i32)]).arg(&path),
                        ),
                        _ => continue,
                    };
                    if let Ok(c) = child {
                        oneshots.push(c);
                    }
                }
                Cmd::LoopStart { i, path, vol } => {
                    if let Some(f) = looper {
                        if let Some(mut old) = looping[i].take() {
                            let _ = old.kill();
                            let _ = old.wait();
                        }
                        looping[i] = spawn_tied(
                            Command::new(f)
                                .args(["-nodisp", "-loop", "0", "-autoexit", "-loglevel", "quiet", "-volume", &format!("{}", (vol * 100.0) as i32)])
                                .arg(&path),
                        )
                        .ok();
                    }
                }
                Cmd::LoopStop { i } => {
                    if let Some(mut c) = looping[i].take() {
                        let _ = c.kill();
                        let _ = c.wait();
                    }
                }
            }
        }
        for c in looping.iter_mut().flatten() {
            let _ = c.kill();
        }
    }
}

/// Kept for the call in `conf()`: nothing to do any more (the built-in mixer is not started on
/// Linux, so its thread can no longer fail).
pub fn install_panic_hook() {}

fn idx(s: Snd) -> usize {
    ALL.iter().position(|(k, _)| *k == s).unwrap()
}

impl Audio {
    pub async fn load(base: &str) -> Audio {
        let n = ALL.len();
        #[cfg(not(native_audio))]
        {
            let paths: Vec<PathBuf> = ALL.iter().map(|(_, f)| PathBuf::from(format!("{base}/sounds/{f}"))).collect();
            let one_shot = ["pw-play", "paplay"].into_iter().find(|c| players::have(c));
            let looper = if players::have("ffplay") { Some("ffplay") } else { None };
            if one_shot.is_none() && looper.is_none() {
                eprintln!("audio: no player found (install pipewire-tools/pulseaudio-utils or ffmpeg) — running silent");
            }
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || players::worker(rx, one_shot, looper, n));
            return Audio {
                backend: Backend::Players {
                    tx,
                    names: format!("sound server players (one-shot: {}, loops: {})", one_shot.unwrap_or("none"), looper.unwrap_or("none")),
                },
                paths,
                flags: vec![false; n],
                enabled: true,
            };
        }
        #[cfg(native_audio)]
        {
            let mut sounds = Vec::new();
            for (_, f) in ALL.iter() {
                let b = crate::assets::bytes(base, &format!("sounds/{f}")).await;
                sounds.push(crate::assets::decode_sound(&b).await.unwrap_or_else(|e| panic!("bad sound {f}: {e:?}")));
            }
            Audio {
                backend: Backend::Native(sounds),
                #[cfg(not(target_arch = "wasm32"))]
                paths: Vec::new(),
                flags: vec![false; n],
                enabled: true,
            }
        }
    }

    pub fn backend_name(&self) -> String {
        match &self.backend {
            #[cfg(native_audio)]
            Backend::Native(_) => "built-in mixer".into(),
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Players { names, .. } => names.clone(),
        }
    }

    pub fn play(&mut self, s: Snd, vol: f32) {
        if !self.enabled {
            return;
        }
        let i = idx(s);
        match &self.backend {
            #[cfg(native_audio)]
            Backend::Native(v) => play_sound(&v[i], PlaySoundParams { looped: false, volume: vol }),
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Players { tx, .. } => {
                let _ = tx.send(players::Cmd::Play { path: self.paths[i].clone(), vol });
            }
        }
    }

    /// Start or stop a loop. Starting an already-running loop is a no-op.
    pub fn set_loop(&mut self, s: Snd, want: bool, vol: f32) {
        let i = idx(s);
        let want = want && self.enabled;
        if want == self.flags[i] {
            return;
        }
        self.flags[i] = want;
        match &self.backend {
            #[cfg(native_audio)]
            Backend::Native(v) => {
                if want {
                    play_sound(&v[i], PlaySoundParams { looped: true, volume: vol });
                } else {
                    stop_sound(&v[i]);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Players { tx, .. } => {
                let _ = if want {
                    tx.send(players::Cmd::LoopStart { i, path: self.paths[i].clone(), vol })
                } else {
                    tx.send(players::Cmd::LoopStop { i })
                };
            }
        }
    }

    /// Change the volume of a running loop (only the built-in mixer supports this live).
    #[allow(unused_variables)]
    pub fn loop_volume(&mut self, s: Snd, vol: f32) {
        #[cfg(native_audio)]
        if let Backend::Native(v) = &self.backend {
            if self.flags[idx(s)] {
                set_sound_volume(&v[idx(s)], vol);
            }
        }
    }
}
