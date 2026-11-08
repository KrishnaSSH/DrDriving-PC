//! Discord Rich Presence over Discord's local IPC socket (no extra crates, no network traffic
//! of our own: the Discord desktop client does the talking).
//!
//! All socket work happens on a background thread so a missing / slow / restarting Discord can
//! never stall a frame. If Discord is not running the thread just retries every 15 s.
//!
//! The Discord application id is hard-coded (`APP_ID`); `DRDRIVING_DISCORD_APP_ID` overrides it and
//! `DRDRIVING_NO_RPC=1` turns the whole thing off.
//! The web build has no local sockets, so there it is an inert stub.

/// What to show on the Discord profile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Presence {
    /// Changing this restarts the "elapsed" timer (a new race, the menus again, ...).
    pub scene: u64,
    pub details: String,
    pub state: String,
    pub large_text: String,
    pub small_image: String,
    pub small_text: String,
    /// "(1 of 2)" next to the state line, used for online races.
    pub party: Option<(u32, u32)>,
}

#[cfg(target_arch = "wasm32")]
pub struct Rpc;

#[cfg(target_arch = "wasm32")]
impl Rpc {
    pub fn new() -> Rpc {
        Rpc
    }
    pub fn update(&mut self, _p: Presence) {}
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::Rpc;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::Presence;
    use serde_json::{json, Value};
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, RecvTimeoutError, Sender};
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    /// The Discord application id. `DRDRIVING_DISCORD_APP_ID` at run time overrides it.
    const APP_ID: &str = "1569008512938541056";
    const IMAGE: &str = "https://krishnassh.github.io/DrDriving-PC/icon.png";
    const WEB_URL: &str = "https://krishnassh.github.io/DrDriving-PC/";
    const REPO_URL: &str = "https://github.com/KrishnaSSH/drdriving-pc";
    /// Discord accepts 5 activity updates per 20 s; stay well under it.
    const MIN_INTERVAL: Duration = Duration::from_secs(5);
    const RECONNECT_EVERY: Duration = Duration::from_secs(15);

    #[cfg(unix)]
    type Stream = std::os::unix::net::UnixStream;
    #[cfg(windows)]
    type Stream = std::fs::File;

    enum Msg {
        Set(Option<Value>),
    }

    pub struct Rpc {
        tx: Option<Sender<Msg>>,
        last: Option<Presence>,
        scene_start: i64,
        last_sent: Option<Instant>,
        /// A changed presence waiting for the rate limit.
        pending: bool,
    }

    fn now_secs() -> i64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
    }

    impl Rpc {
        pub fn new() -> Rpc {
            let id = std::env::var("DRDRIVING_DISCORD_APP_ID").unwrap_or_else(|_| APP_ID.to_string());
            let off = std::env::var_os("DRDRIVING_NO_RPC").is_some_and(|v| v != "0");
            let tx = if id.trim().is_empty() || off {
                None
            } else {
                let (tx, rx) = mpsc::channel();
                let id = id.trim().to_string();
                std::thread::Builder::new().name("discord-rpc".into()).spawn(move || worker(id, rx)).ok().map(|_| tx)
            };
            Rpc { tx, last: None, scene_start: now_secs(), last_sent: None, pending: false }
        }

        /// Call as often as you like; only real changes are sent, at most one every few seconds.
        pub fn update(&mut self, p: Presence) {
            let Some(tx) = &self.tx else { return };
            if self.last.as_ref() != Some(&p) {
                if self.last.as_ref().map_or(true, |l| l.scene != p.scene) {
                    self.scene_start = now_secs();
                }
                self.last = Some(p);
                self.pending = true;
            }
            if !self.pending || self.last_sent.is_some_and(|t| t.elapsed() < MIN_INTERVAL) {
                return;
            }
            if let Some(p) = &self.last {
                let _ = tx.send(Msg::Set(Some(activity(p, self.scene_start))));
                self.last_sent = Some(Instant::now());
                self.pending = false;
            }
        }
    }

    impl Drop for Rpc {
        fn drop(&mut self) {
            if let Some(tx) = &self.tx {
                let _ = tx.send(Msg::Set(None));
                // give the worker a moment to flush the "clear" before the process exits
                std::thread::sleep(Duration::from_millis(150));
            }
        }
    }

    /// Discord wants 2..=128 bytes per text field.
    fn clip(s: &str) -> String {
        let mut s: String = s.trim().to_string();
        while s.len() > 128 {
            s.pop();
        }
        if s.chars().count() < 2 {
            s.push_str("  ");
        }
        s
    }

    fn activity(p: &Presence, start: i64) -> Value {
        let mut assets = json!({ "large_image": IMAGE, "large_text": clip(&p.large_text) });
        if !p.small_image.is_empty() {
            assets["small_image"] = json!(p.small_image);
            assets["small_text"] = json!(clip(&p.small_text));
        }
        let mut a = json!({
            "details": clip(&p.details),
            "state": clip(&p.state),
            "timestamps": { "start": start },
            "assets": assets,
            "buttons": [ { "label": "Play in browser", "url": WEB_URL }, { "label": "Source on GitHub", "url": REPO_URL } ],
        });
        if let Some((cur, max)) = p.party {
            a["party"] = json!({ "id": "drdriving-online", "size": [cur, max] });
        }
        a
    }

    // ---------------------------------------------------------------- IPC

    fn frame(op: u32, v: &Value) -> Vec<u8> {
        let body = v.to_string().into_bytes();
        let mut out = Vec::with_capacity(8 + body.len());
        out.extend_from_slice(&op.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    fn read_frame(s: &mut impl Read) -> std::io::Result<(u32, Vec<u8>)> {
        let mut head = [0u8; 8];
        s.read_exact(&mut head)?;
        let op = u32::from_le_bytes(head[0..4].try_into().unwrap());
        let len = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
        if len > 1 << 20 {
            return Err(std::io::Error::other("oversized frame"));
        }
        let mut body = vec![0u8; len];
        s.read_exact(&mut body)?;
        Ok((op, body))
    }

    #[cfg(unix)]
    fn open_socket() -> Option<Stream> {
        let mut dirs: Vec<String> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"].iter().filter_map(|k| std::env::var(k).ok()).collect();
        dirs.push("/tmp".into());
        let mut paths = Vec::new();
        for d in &dirs {
            for sub in ["", "app/com.discordapp.Discord/", "app/com.discordapp.DiscordCanary/", "snap.discord/", "snap.discord-canary/"] {
                for n in 0..10 {
                    paths.push(format!("{}/{}discord-ipc-{}", d.trim_end_matches('/'), sub, n));
                }
            }
        }
        paths.into_iter().find_map(|p| Stream::connect(p).ok())
    }

    #[cfg(windows)]
    fn open_socket() -> Option<Stream> {
        (0..10).find_map(|n| std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\?\pipe\discord-ipc-{n}")).ok())
    }

    /// Connects, performs the handshake, and spawns a thread that drains Discord's replies
    /// (they must be read or the socket buffer eventually fills up).
    fn connect(app_id: &str, dead: &Arc<AtomicBool>) -> Option<Stream> {
        let mut s = open_socket()?;
        s.write_all(&frame(0, &json!({ "v": 1, "client_id": app_id }))).ok()?;
        let (op, _) = read_frame(&mut s).ok()?; // READY (op 1) or an error frame (op 2)
        if op != 1 {
            return None;
        }
        let mut reader = s.try_clone().ok()?;
        dead.store(false, Ordering::SeqCst);
        let dead = dead.clone();
        std::thread::spawn(move || {
            while let Ok((op, _)) = read_frame(&mut reader) {
                if op == 2 {
                    break; // CLOSE
                }
            }
            dead.store(true, Ordering::SeqCst);
        });
        Some(s)
    }

    fn set_activity(s: &mut Stream, activity: &Option<Value>, nonce: u64) -> std::io::Result<()> {
        let pid = std::process::id();
        let v = json!({ "cmd": "SET_ACTIVITY", "args": { "pid": pid, "activity": activity }, "nonce": nonce.to_string() });
        s.write_all(&frame(1, &v))
    }

    fn worker(app_id: String, rx: mpsc::Receiver<Msg>) {
        let dead = Arc::new(AtomicBool::new(false));
        let mut conn: Option<Stream> = None;
        let mut latest: Option<Option<Value>> = None;
        let mut sent = true;
        let mut next_try = Instant::now();
        let mut nonce = 0u64;
        loop {
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(Msg::Set(a)) => {
                    latest = Some(a);
                    sent = false;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if dead.load(Ordering::SeqCst) {
                conn = None;
                dead.store(false, Ordering::SeqCst);
                sent = false; // resend once Discord is back
            }
            if conn.is_none() && latest.is_some() && Instant::now() >= next_try {
                conn = connect(&app_id, &dead);
                if conn.is_none() {
                    next_try = Instant::now() + RECONNECT_EVERY;
                }
            }
            if let (Some(s), Some(a), false) = (conn.as_mut(), latest.as_ref(), sent) {
                nonce += 1;
                if set_activity(s, a, nonce).is_ok() {
                    sent = true;
                } else {
                    conn = None;
                }
            }
        }
    }
}
