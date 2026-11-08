//! HTTP for the multiplayer database ([`drdriving_core::rtdb::Backend`]).
//!
//! Native: a small pool of worker threads around `ureq` (blocking calls never touch the game
//! loop). Browser: the page's `fetch`, reached through the `dr_http_*` imports in
//! `web/index.html`. Both only ever do what the Realtime Database REST API needs.

use drdriving_core::rtdb::{Backend, Method, Request, Response, DEFAULT_DB};

/// Current time in unix milliseconds (works in the browser, where `SystemTime` does not).
pub fn now_ms() -> u64 {
    (macroquad::miniquad::date::now() * 1000.0) as u64
}

/// The database to play on: `DRDRIVING_DB` (desktop) or `?db=` in the page URL, else the
/// original game's.
pub fn db_url() -> String {
    let custom = custom_db().filter(|s| s.starts_with("http://") || s.starts_with("https://"));
    custom.unwrap_or_else(|| DEFAULT_DB.to_string()).trim_end_matches('/').to_string()
}

pub fn backend() -> Box<dyn Backend> {
    Box::new(imp::Http::new(db_url()))
}

fn method_name(m: Method) -> &'static str {
    match m {
        Method::Get => "GET",
        Method::Put => "PUT",
        Method::Patch => "PATCH",
        Method::Delete => "DELETE",
    }
}

/// `https://db/<path>.json` plus `?print=silent` for writes (no echo of what was written).
/// Firebase refuses `print=silent` together with `if-match` (HTTP 400), so conditional writes
/// go without it.
fn url_for(base: &str, r: &Request) -> String {
    let silent = if r.method == Method::Get || r.if_match.is_some() { "" } else { "?print=silent" };
    format!("{base}/{}.json{silent}", r.path)
}

// ------------------------------------------------------------------------------- native

#[cfg(not(target_arch = "wasm32"))]
fn custom_db() -> Option<String> {
    std::env::var("DRDRIVING_DB").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::*;
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    const WORKERS: usize = 4;

    pub struct Http {
        jobs: Sender<(u64, Request)>,
        done: Receiver<(u64, Response)>,
        next: u64,
    }

    impl Http {
        pub fn new(base: String) -> Http {
            let (jobs, jrx) = channel::<(u64, Request)>();
            let (dtx, done) = channel();
            let jrx = Arc::new(Mutex::new(jrx));
            let agent = ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(6))
                .timeout(Duration::from_secs(10))
                .user_agent("drdriving-pc")
                .build();
            for _ in 0..WORKERS {
                let (jrx, dtx, agent, base) = (jrx.clone(), dtx.clone(), agent.clone(), base.clone());
                std::thread::spawn(move || loop {
                    // the lock is only held while waiting for the next job
                    let job = jrx.lock().map(|r| r.recv());
                    match job {
                        Ok(Ok((id, req))) => {
                            if dtx.send((id, perform(&agent, &base, &req))).is_err() {
                                return; // nobody is listening any more; still finish queued writes
                            }
                        }
                        _ => return,
                    }
                });
            }
            Http { jobs, done, next: 1 }
        }
    }

    fn answer(resp: ureq::Response) -> Response {
        let status = resp.status();
        let etag = resp.header("etag").map(str::to_string);
        let body = resp.into_string().unwrap_or_default();
        Response { status, body, etag }
    }

    fn perform(agent: &ureq::Agent, base: &str, r: &Request) -> Response {
        // a pooled keep-alive connection the server has closed fails at once: try again on a fresh one
        for attempt in 0..3 {
            let mut rq = agent.request(method_name(r.method), &url_for(base, r));
            if r.want_etag {
                rq = rq.set("X-Firebase-ETag", "true");
            }
            if let Some(tag) = &r.if_match {
                rq = rq.set("if-match", tag);
            }
            let res = match &r.body {
                Some(b) => rq.set("Content-Type", "application/json").send_string(b),
                None => rq.call(),
            };
            match res {
                Ok(resp) => return answer(resp),
                Err(ureq::Error::Status(_, resp)) => return answer(resp),
                Err(e) => {
                    if std::env::var_os("DRDRIVING_TRACE").is_some() {
                        eprintln!("[net] {} {} failed (attempt {}): {e}", method_name(r.method), r.path, attempt + 1);
                    }
                }
            }
        }
        Response::default()
    }

    impl Backend for Http {
        fn start(&mut self, req: Request) -> u64 {
            let id = self.next;
            self.next += 1;
            let _ = self.jobs.send((id, req));
            id
        }

        fn poll(&mut self) -> Vec<(u64, Response)> {
            self.done.try_iter().collect()
        }
    }
}

// ------------------------------------------------------------------------------- browser

#[cfg(target_arch = "wasm32")]
extern "C" {
    fn dr_http_start(method: *const u8, method_len: u32, url: *const u8, url_len: u32, body: *const u8, body_len: u32, headers: *const u8, headers_len: u32) -> u32;
    /// 0 = still running, 1 = finished (status/body/etag valid).
    fn dr_http_state(id: u32) -> u32;
    fn dr_http_status(id: u32) -> u32;
    fn dr_http_body_len(id: u32) -> u32;
    fn dr_http_body(id: u32, dst: *mut u8);
    fn dr_http_etag_len(id: u32) -> u32;
    fn dr_http_etag(id: u32, dst: *mut u8);
    fn dr_http_free(id: u32);
    /// The `db` query parameter of the page URL (bytes written, 0 = none).
    fn dr_http_db(dst: *mut u8, cap: u32) -> i32;
}

#[cfg(target_arch = "wasm32")]
fn custom_db() -> Option<String> {
    let mut buf = vec![0u8; 512];
    let n = unsafe { dr_http_db(buf.as_mut_ptr(), buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    String::from_utf8(buf).ok().map(|s| s.trim().to_string())
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::*;

    pub struct Http {
        base: String,
        pending: Vec<u32>,
    }

    impl Http {
        pub fn new(base: String) -> Http {
            Http { base, pending: Vec::new() }
        }
    }

    impl Backend for Http {
        fn start(&mut self, r: Request) -> u64 {
            let (method, url) = (method_name(r.method), url_for(&self.base, &r));
            let mut headers = String::new();
            if r.want_etag {
                headers.push_str("X-Firebase-ETag: true\n");
            }
            if let Some(tag) = &r.if_match {
                headers.push_str(&format!("if-match: {tag}\n"));
            }
            if r.body.is_some() {
                headers.push_str("Content-Type: application/json\n");
            }
            let body = r.body.unwrap_or_default();
            let id = unsafe {
                dr_http_start(method.as_ptr(), method.len() as u32, url.as_ptr(), url.len() as u32, body.as_ptr(), body.len() as u32, headers.as_ptr(), headers.len() as u32)
            };
            self.pending.push(id);
            id as u64
        }

        fn poll(&mut self) -> Vec<(u64, Response)> {
            let mut out = Vec::new();
            self.pending.retain(|&id| {
                if unsafe { dr_http_state(id) } == 0 {
                    return true;
                }
                let status = unsafe { dr_http_status(id) } as u16;
                let mut body = vec![0u8; unsafe { dr_http_body_len(id) } as usize];
                unsafe { dr_http_body(id, body.as_mut_ptr()) };
                let elen = unsafe { dr_http_etag_len(id) } as usize;
                let etag = (elen > 0).then(|| {
                    let mut e = vec![0u8; elen];
                    unsafe { dr_http_etag(id, e.as_mut_ptr()) };
                    String::from_utf8_lossy(&e).into_owned()
                });
                unsafe { dr_http_free(id) };
                out.push((id as u64, Response { status, body: String::from_utf8_lossy(&body).into_owned(), etag }));
                false
            });
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rq(method: Method, if_match: Option<&str>) -> Request {
        Request { method, path: "Waiting/x/status".into(), body: None, want_etag: false, if_match: if_match.map(String::from) }
    }

    #[test]
    fn conditional_writes_never_use_print_silent() {
        assert_eq!(url_for("https://db", &rq(Method::Get, None)), "https://db/Waiting/x/status.json");
        assert_eq!(url_for("https://db", &rq(Method::Put, None)), "https://db/Waiting/x/status.json?print=silent");
        // Firebase answers HTTP 400 to if-match + print=silent
        assert_eq!(url_for("https://db", &rq(Method::Put, Some("tag"))), "https://db/Waiting/x/status.json");
    }
}
