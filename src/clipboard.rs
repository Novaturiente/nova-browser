//! System clipboard / primary selection through `wl-copy` / `wl-paste`, off the UI thread.
// shortcut: Wayland only (wl-clipboard); add xclip when X11 matters
use cef::*;
use std::io::Write;
use std::process::{Command, Stdio};

pub fn write(text: &str, primary: bool) {
    let text = text.to_string();
    std::thread::spawn(move || {
        let mut cmd = Command::new("wl-copy");
        if primary {
            cmd.arg("--primary");
        }
        let child = cmd.stdin(Stdio::piped()).stdout(Stdio::null()).spawn();
        match child {
            Ok(mut c) => {
                if let Some(mut stdin) = c.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = c.wait();
            }
            Err(e) => eprintln!("novabrowser: wl-copy: {e}"),
        }
    });
}

fn read(primary: bool) -> String {
    let mut cmd = Command::new("wl-paste");
    cmd.arg("--no-newline");
    if primary {
        cmd.arg("--primary");
    }
    cmd.stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
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
