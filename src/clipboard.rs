//! Clipboard. Copy goes through Chromium's own clipboard (works on Wayland and
//! X11, no extra tools); the primary selection and reading (for `{clipboard}`)
//! use `wl-copy`/`wl-paste`, else `xclip`, off the UI thread.
use cef::*;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

pub fn write(text: &str, primary: bool) {
    let Some(ui) = (!primary).then(crate::window::tabbar_browser).flatten() else {
        return write_external(text.to_string(), primary);
    };
    // a copy event in our own tab-bar page; userGesture lets execCommand reach the clipboard
    let js = format!(
        "document.addEventListener('copy',e=>{{e.clipboardData.setData('text/plain',{});e.preventDefault()}},{{once:true}});document.execCommand('copy')",
        serde_json::Value::from(text)
    );
    let text = text.to_string();
    let params = serde_json::json!({"expression": js, "userGesture": true, "returnByValue": true});
    crate::devtools::call(&ui, "Runtime.evaluate", params, move |r| {
        if r.and_then(|r| r["result"]["value"].as_bool()) != Some(true) {
            write_external(text, false);
        }
    });
}

/// (program, args) tried in order: Wayland tool first, then X11.
fn tools(primary: bool, copy: bool) -> [(&'static str, Vec<&'static str>); 2] {
    let sel = if primary { "primary" } else { "clipboard" };
    match (copy, primary) {
        (true, true) => [
            ("wl-copy", vec!["--primary"]),
            ("xclip", vec!["-selection", sel]),
        ],
        (true, false) => [("wl-copy", vec![]), ("xclip", vec!["-selection", sel])],
        (false, true) => [
            ("wl-paste", vec!["--no-newline", "--primary"]),
            ("xclip", vec!["-o", "-selection", sel]),
        ],
        (false, false) => [
            ("wl-paste", vec!["--no-newline"]),
            ("xclip", vec!["-o", "-selection", sel]),
        ],
    }
}

fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn write_external(text: String, primary: bool) {
    std::thread::spawn(move || {
        for (prog, args) in tools(primary, true)
            .into_iter()
            .skip(usize::from(!wayland()))
        {
            let Ok(mut c) = Command::new(prog)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .spawn()
            else {
                continue;
            };
            if let Some(mut stdin) = c.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = c.wait();
            return;
        }
        eprintln!("novabrowser: copy needs wl-clipboard or xclip");
    });
}

fn read(primary: bool) -> String {
    for (prog, args) in tools(primary, false)
        .into_iter()
        .skip(usize::from(!wayland()))
    {
        let Ok(mut c) = Command::new(prog)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        // a page can put megabytes there, and wl-paste can hang on a stuck owner
        const CAP: u64 = 1 << 20;
        let mut out = c.stdout.take().expect("piped stdout");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = (&mut out).take(CAP).read_to_end(&mut b);
            let _ = tx.send(b);
        });
        let b = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap_or_default();
        let _ = c.kill(); // past the cap or timed out; no-op if it already exited
        let _ = c.wait();
        return String::from_utf8_lossy(&b).into_owned();
    }
    String::new()
}

wrap_task! {
    struct RunLine { line: String, count: Option<u32>, clip: String, primary: String }
    impl Task {
        fn execute(&self) { crate::commands::run_with_clipboard(&self.line, self.count, self.clip.clone(), self.primary.clone()); }
    }
}

/// Read the clipboards `line` needs on a thread, then run it on the UI thread.
pub fn expand_then_run(line: &str, count: Option<u32>) {
    let line = line.to_string();
    std::thread::spawn(move || {
        let clip = if line.contains("{clipboard}") {
            read(false)
        } else {
            String::new()
        };
        let primary = if line.contains("{primary}") {
            read(true)
        } else {
            String::new()
        };
        post_task(
            ThreadId::UI,
            Some(&mut RunLine::new(line, count, clip, primary)),
        );
    });
}
