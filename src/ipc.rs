//! Control socket (PLAN §5.7): one instance per profile. `novabrowser --cmd ':open -t x'`,
//! `novabrowser --query state`, and `novabrowser <url>` while one runs (opens a tab).
//!
//! Protocol: one request line, one reply line. `cmd <line>` → `ok`; `query state` → JSON.
//! The socket lives in the profile dir, so test profiles don't talk to the real one.
use cef::*;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;

pub fn socket_path() -> PathBuf {
    crate::xdg_dir("XDG_DATA_HOME", ".local/share").join("ipc")
}

/// Send one request to a running instance; None if none is listening.
pub fn request(line: &str) -> Option<String> {
    let mut s = UnixStream::connect(socket_path()).ok()?;
    writeln!(s, "{line}").ok()?;
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply).ok()?;
    Some(reply.trim_end().to_string())
}

/// Listen on a thread; each request runs on the UI thread and its reply comes back over a channel.
pub fn start() {
    let path = socket_path();
    let _ = std::fs::remove_file(&path); // stale: request() already failed to connect
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => return eprintln!("novabrowser: ipc {}: {e}", path.display()),
    };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || serve(stream));
        }
    });
}

fn serve(stream: UnixStream) {
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    post_task(
        ThreadId::UI,
        Some(&mut Handle::new(line.trim_end().to_string(), Reply(tx))),
    );
    let reply = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap_or_else(|_| "error: timeout".into());
    let _ = writeln!(&stream, "{reply}");
}

/// Task fields must be Clone (wrap_task!); mpsc::Sender is.
#[derive(Clone)]
struct Reply(mpsc::Sender<String>);

wrap_task! {
    struct Handle { line: String, reply: Reply }
    impl Task {
        fn execute(&self) {
            let out = match self.line.split_once(' ') {
                Some(("cmd", c)) => { crate::commands::run(c, None); "ok".to_string() }
                Some(("query", "state")) => crate::window::state_json(),
                _ => format!("error: unknown request {:?}", self.line),
            };
            let _ = self.reply.0.send(out);
        }
    }
}
