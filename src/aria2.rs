//! aria2 download backend (`downloads.backend` = auto | aria2 | chromium).
//! novabrowser starts its own `aria2c` with JSON-RPC on 127.0.0.1 (random port,
//! random secret), hands it every GET download (with the page's cookies, user
//! agent and referer) and magnet: links, and polls progress on a thread for the
//! downloads pill/page. Unfinished downloads live in `<data>/aria2.session` and
//! resume on the next start; quitting shuts aria2 down. Torrents stop seeding
//! when done (`seed-time=0`). POST, blob: and data: downloads stay with Chromium.
use crate::{config, xdg_dir};
use cef::*;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

struct Rpc {
    port: u16,
    secret: String,
}

enum Job {
    /// id = our request number (for a fallback to Chromium if aria2 refuses it)
    Add {
        id: u64,
        uris: Vec<String>,
        opts: Value,
    },
    Remove(String),
}

static RPC: OnceLock<Rpc> = OnceLock::new();
static TX: OnceLock<mpsc::Sender<Job>> = OnceLock::new();

const KEYS: &[&str] = &[
    "gid",
    "status",
    "totalLength",
    "completedLength",
    "files",
    "bittorrent",
    "followedBy",
    "errorMessage",
    "dir",
];

fn find_aria2c() -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|d| PathBuf::from(d).join("aria2c"))
        .find(|p| p.is_file())
}

/// Start aria2c (browser process, after CEF init) unless the backend is
/// chromium or aria2c isn't installed.
pub fn start() {
    let backend = config::get().str("downloads.backend");
    if backend == "chromium" {
        return;
    }
    let Some(bin) = find_aria2c() else {
        if backend == "aria2" {
            eprintln!(
                "novabrowser: downloads.backend = aria2 but aria2c isn't installed; using Chromium"
            );
        }
        return;
    };
    let port = match std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()) {
        Ok(a) => a.port(),
        Err(e) => return eprintln!("novabrowser: aria2: no free port: {e}"),
    };
    let secret = random_hex();
    // holds cookies of unfinished downloads; private via the 0700 data dir (main.rs)
    let session = xdg_dir("XDG_DATA_HOME", ".local/share").join("aria2.session");
    if !session.exists() {
        let _ = std::fs::write(&session, "");
    }
    let s = session.to_string_lossy();
    let child = std::process::Command::new(bin)
        .args([
            "--enable-rpc=true".into(),
            format!("--rpc-listen-port={port}"),
            format!("--rpc-secret={secret}"),
            format!("--input-file={s}"),
            format!("--save-session={s}"),
            "--save-session-interval=10".into(),
            // aria2 quits by itself if we crash
            format!("--stop-with-process={}", std::process::id()),
            // not --continue: it treats any same-named file as a partial copy of this one;
            // session resume works through aria2's .aria2 control files anyway
            // 8 connections per file: fast, but fewer servers throttle us than at 16
            "--max-connection-per-server=8".into(),
            "--split=8".into(),
            "--min-split-size=1M".into(),
            "--content-disposition-default-utf8=true".into(),
            "--auto-file-renaming=true".into(),
            "--file-allocation=falloc".into(),
            "--seed-time=0".into(),
            "--quiet=true".into(),
            format!(
                "--log={}",
                xdg_dir("XDG_STATE_HOME", ".local/state")
                    .join("aria2.log")
                    .display()
            ),
            "--log-level=warn".into(),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Err(e) = child {
        return eprintln!("novabrowser: aria2: can't start aria2c: {e}");
    }
    RPC.set(Rpc { port, secret }).ok();
    let (tx, rx) = mpsc::channel();
    TX.set(tx).ok();
    std::thread::Builder::new()
        .name("aria2".into())
        .spawn(move || run(rx))
        .expect("spawn aria2 thread");
}

pub fn enabled() -> bool {
    RPC.get().is_some()
}

fn random_hex() -> String {
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .expect("/dev/urandom");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// One JSON-RPC call (HTTP/1.1, Connection: close).
fn call(method: &str, params: Vec<Value>) -> Result<Value, String> {
    let rpc = RPC.get().ok_or("aria2 not running")?;
    let mut p = vec![json!(format!("token:{}", rpc.secret))];
    p.extend(params);
    let body = json!({ "jsonrpc": "2.0", "id": "n", "method": method, "params": p }).to_string();
    let mut s = TcpStream::connect(("127.0.0.1", rpc.port)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    write!(
        s,
        "POST /jsonrpc HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .map_err(|e| e.to_string())?;
    let mut resp = String::new();
    s.read_to_string(&mut resp).map_err(|e| e.to_string())?;
    let json = resp.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    let v: Value = serde_json::from_str(json).map_err(|e| format!("bad reply: {e}"))?;
    match v.get("error") {
        Some(e) => Err(e["message"].as_str().unwrap_or("error").to_string()),
        None => Ok(v["result"].clone()),
    }
}

wrap_task! {
    struct Update { items: Vec<Value> }
    impl Task {
        fn execute(&self) { crate::downloads::aria2_update(self.items.clone()); }
    }
}

wrap_task! {
    struct Added { id: u64, result: Result<String, String> }
    impl Task {
        fn execute(&self) { crate::downloads::aria2_added(self.id, self.result.clone()); }
    }
}

/// Worker: runs jobs, polls every second while something is going on, else
/// sleeps until a new job (or 30 s, for downloads resumed from the session).
fn run(rx: mpsc::Receiver<Job>) {
    // wait until aria2 listens
    for _ in 0..50 {
        if call("aria2.getVersion", vec![]).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut busy = true;
    loop {
        let wait = if busy {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(30)
        };
        match rx.recv_timeout(wait) {
            Ok(Job::Add { id, uris, opts }) => {
                let r = call("aria2.addUri", vec![json!(uris), opts])
                    .map(|g| g.as_str().unwrap_or("").to_string());
                post_task(ThreadId::UI, Some(&mut Added::new(id, r)));
            }
            Ok(Job::Remove(gid)) => {
                let _ = call("aria2.remove", vec![json!(gid)]);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        busy = poll();
    }
}

/// Report every download aria2 knows; finished ones are reported once, then purged.
fn poll() -> bool {
    let keys = json!(KEYS);
    let mut items = Vec::new();
    for (m, p) in [
        ("aria2.tellActive", vec![keys.clone()]),
        (
            "aria2.tellWaiting",
            vec![json!(0), json!(1000), keys.clone()],
        ),
        ("aria2.tellStopped", vec![json!(0), json!(1000), keys]),
    ] {
        if let Ok(Value::Array(a)) = call(m, p) {
            items.extend(a);
        }
    }
    let busy = items
        .iter()
        .any(|i| matches!(i["status"].as_str(), Some("active" | "waiting")));
    let stopped: Vec<String> = items
        .iter()
        .filter(|i| matches!(i["status"].as_str(), Some("complete" | "error" | "removed")))
        .filter_map(|i| i["gid"].as_str().map(str::to_string))
        .collect();
    if !items.is_empty() {
        post_task(ThreadId::UI, Some(&mut Update::new(items)));
    }
    for gid in stopped {
        let _ = call("aria2.removeDownloadResult", vec![json!(gid)]);
    }
    busy
}

fn send(job: Job) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(job);
    }
}

/// Hand `url` to aria2 with the cookies, user agent and referer the tab would
/// send. Async: the result arrives as `downloads::aria2_added(id, …)`.
pub fn add(browser: &Browser, url: String, id: u64) {
    let referer = browser
        .main_frame()
        .and_then(|f| crate::net::referrer(&CefString::from(&f.url()).to_string(), &url))
        .unwrap_or_default();
    let b = browser.clone();
    crate::devtools::call(
        browser,
        "Network.getCookies",
        json!({ "urls": [url] }),
        move |r| {
            let cookies: Vec<String> = r
                .as_ref()
                .and_then(|r| r["cookies"].as_array())
                .into_iter()
                .flatten()
                .filter_map(|c| Some(format!("{}={}", c["name"].as_str()?, c["value"].as_str()?)))
                .collect();
            let expr = json!({ "expression": "navigator.userAgent", "returnByValue": true });
            crate::devtools::call(&b, "Runtime.evaluate", expr, move |ua| {
                let ua = ua
                    .as_ref()
                    .and_then(|u| u["result"]["value"].as_str())
                    .unwrap_or("")
                    .to_string();
                let mut header = Vec::new();
                if !cookies.is_empty() {
                    header.push(format!("Cookie: {}", cookies.join("; ")));
                }
                if !referer.is_empty() {
                    header.push(format!("Referer: {referer}"));
                }
                let mut opts =
                    json!({ "dir": crate::downloads::dir().to_string_lossy(), "header": header });
                if !ua.is_empty() {
                    opts["user-agent"] = json!(ua);
                }
                send(Job::Add {
                    id,
                    uris: vec![url],
                    opts,
                });
            });
        },
    );
}

/// A magnet: link (any thread).
pub fn add_magnet(url: String, id: u64) {
    let opts = json!({ "dir": crate::downloads::dir().to_string_lossy() });
    send(Job::Add {
        id,
        uris: vec![url],
        opts,
    });
}

pub fn remove(gid: &str) {
    send(Job::Remove(gid.to_string()));
}

/// Quit: let aria2 save its session (unfinished downloads resume next start).
pub fn stop() {
    if enabled() {
        let _ = call("aria2.shutdown", vec![]);
    }
}
