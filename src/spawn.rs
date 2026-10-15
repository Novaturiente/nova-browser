//! External tools (PLAN §5.7): qutebrowser-compatible userscripts (env vars +
//! `QUTE_FIFO`), and `fake-key` / `insert-text` as trusted DevTools input.
use crate::keys::{self, Mods};
use crate::{commands, config, devtools, window};
use cef::*;
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// `spawn [-u] [-d] [-o] <cmd> [args]` (flags already parsed off).
pub fn spawn(
    argv: &[String],
    userscript: bool,
    output: bool,
    count: Option<u32>,
) -> Result<(), String> {
    let (prog, args) = argv
        .split_first()
        .ok_or("usage: spawn [-u] [-o] <command> [args…]")?;
    if !userscript {
        let mut cmd = Command::new(expand_home(prog));
        cmd.args(args).stdin(Stdio::null());
        return run_child(cmd, prog.clone(), output, None);
    }
    let path = find_userscript(prog)?;
    let browser = window::active_browser();
    let tab = browser.as_ref().map(|b| b.identifier()).unwrap_or(-1);
    let (url, title) = window::active_url_title();
    let (prog, args) = (prog.clone(), args.to_vec());
    let start = move |page: Option<(String, String, String)>| {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(e) => return window::message("error", &format!("spawn: {e}")),
        };
        let fifo = dir.join("fifo");
        let (sel, html, text) = page.unwrap_or_default();
        let _ = std::fs::write(dir.join("html"), html);
        let _ = std::fs::write(dir.join("text"), text);
        let cfg = config::get();
        let mut cmd = Command::new(&path);
        cmd.args(&args)
            .stdin(Stdio::null())
            .env("QUTE_USERSCRIPT", "1")
            .env("QUTE_MODE", "command")
            .env("QUTE_URL", &url)
            .env("QUTE_TITLE", &title)
            .env("QUTE_SELECTED_TEXT", sel)
            .env("QUTE_HTML", dir.join("html"))
            .env("QUTE_TEXT", dir.join("text"))
            .env("QUTE_FIFO", &fifo)
            .env(
                "QUTE_CONFIG_DIR",
                config::path().parent().unwrap_or(Path::new(".")),
            )
            .env(
                "QUTE_DATA_DIR",
                crate::xdg_dir("XDG_DATA_HOME", ".local/share"),
            )
            .env(
                "QUTE_DOWNLOAD_DIR",
                expand_home(&cfg.str("downloads.location.directory")),
            )
            .env("QUTE_COMMANDLINE_TEXT", "")
            .env(
                "QUTE_VERSION",
                concat!("novabrowser ", env!("CARGO_PKG_VERSION")),
            );
        if let Some(n) = count {
            cmd.env("QUTE_COUNT", n.to_string());
        }
        if let Err(e) = mkfifo(&fifo) {
            return window::message("error", &format!("spawn: {e}"));
        }
        read_fifo(fifo.clone(), tab);
        if let Err(e) = run_child(cmd, prog.clone(), output, Some(dir)) {
            window::message("error", &format!("spawn: {e}"));
        }
    };
    match browser {
        // Selection, HTML and text are read before the script starts (qutebrowser does too).
        Some(b) => devtools::eval_isolated(
            &b,
            "[String(getSelection()), document.documentElement.outerHTML, document.body ? document.body.innerText : '']".into(),
            move |r| {
                let page = r.map(|(_, v)| {
                    let s = |i: usize| v[i].as_str().unwrap_or("").to_string();
                    (s(0), s(1), s(2))
                });
                start(page)
            },
        ),
        None => start(None),
    }
    Ok(())
}

/// `name` as given if it has a `/`, else from `<config>/userscripts`, then `<data>/userscripts`.
fn find_userscript(name: &str) -> Result<PathBuf, String> {
    if name.contains('/') {
        return Ok(PathBuf::from(expand_home(name)));
    }
    let dirs = [
        config::path().parent().map(|p| p.join("userscripts")),
        Some(crate::xdg_dir("XDG_DATA_HOME", ".local/share").join("userscripts")),
    ];
    dirs.into_iter()
        .flatten()
        .map(|d| d.join(name))
        .find(|p| p.is_file())
        .ok_or_else(|| format!("userscript {name} not found in the config or data userscripts dir"))
}

/// Start `cmd`; a thread waits for it, reports a failure (last stderr line) or
/// the output (`-o`), and cleans up the userscript dir.
fn run_child(
    mut cmd: Command,
    name: String,
    output: bool,
    dir: Option<PathBuf>,
) -> Result<(), String> {
    cmd.stderr(Stdio::piped());
    if output {
        cmd.stdout(Stdio::piped());
    }
    let child = cmd.spawn().map_err(|e| format!("{name}: {e}"))?;
    std::thread::spawn(move || {
        // reads stdout and stderr together: one at a time deadlocks once the other pipe fills
        let (status, out, err) = match child.wait_with_output() {
            Ok(o) => (
                Ok(o.status),
                String::from_utf8_lossy(&o.stdout).into_owned(),
                String::from_utf8_lossy(&o.stderr).into_owned(),
            ),
            Err(e) => (Err(e), String::new(), String::new()),
        };
        if let Some(dir) = &dir {
            // Wake the FIFO reader so it stops, then drop the files.
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .write(true)
                .open(dir.join("fifo"))
            {
                let _ = f.write_all(b"\0\n");
            }
        }
        let msg = match status {
            Ok(s) if !s.success() => {
                let last = err
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("");
                Some((
                    "error",
                    format!("{name} exited with {}: {last}", s.code().unwrap_or(-1)),
                ))
            }
            Err(e) => Some(("error", format!("{name}: {e}"))),
            _ if output => {
                // qutebrowser shows the output in a new tab as plain text
                post_task(ThreadId::UI, Some(&mut ShowOutput::new(out)));
                None
            }
            _ => None,
        };
        if let Some((level, text)) = msg {
            post_task(
                ThreadId::UI,
                Some(&mut Message::new(level.to_string(), text)),
            );
        }
    });
    Ok(())
}

/// Each line written to the FIFO runs as a command on the UI thread. Opened
/// read+write so writers closing it don't end the loop; `\0` from the waiter does.
fn read_fifo(fifo: PathBuf, tab: i32) {
    std::thread::spawn(move || {
        let Ok(f) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&fifo)
        else {
            return;
        };
        for line in BufReader::new(f).lines() {
            let Ok(line) = line else { break };
            if line == "\0" {
                break;
            }
            if !line.trim().is_empty() {
                post_task(ThreadId::UI, Some(&mut RunLine::new(line, tab)));
            }
        }
        if let Some(dir) = fifo.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    });
}

wrap_task! {
    struct RunLine { line: String, tab: i32 }
    impl Task {
        fn execute(&self) {
            // In the tab that started the script, never whatever is in front now.
            if !window::run_in_tab(self.tab, || commands::run(&self.line, None)) {
                window::message("warning", &format!("userscript tab closed, dropped: {}", self.line));
            }
        }
    }
}

wrap_task! {
    struct ShowOutput { text: String }
    impl Task {
        fn execute(&self) {
            let b64 = CefString::from(&base64_encode(Some(self.text.as_bytes()))).to_string();
            window::open(&format!("data:text/plain;charset=utf-8;base64,{b64}"), true);
        }
    }
}

wrap_task! {
    struct Message { level: String, text: String }
    impl Task {
        fn execute(&self) { window::message(&self.level, &self.text); }
    }
}

fn tempdir() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    for i in 0..100u32 {
        let d = base.join(format!("novabrowser-userscript-{}-{i}", std::process::id()));
        if std::fs::create_dir(&d).is_ok() {
            return Ok(d);
        }
    }
    Err("can't create a userscript dir".into())
}

fn mkfifo(path: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        fn mkfifo(path: *const std::ffi::c_char, mode: u32) -> i32;
    }
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    // SAFETY: c is a valid NUL-terminated path for the duration of the call.
    if unsafe { mkfifo(c.as_ptr(), 0o600) } == 0 {
        Ok(())
    } else {
        Err(format!(
            "mkfifo {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ))
    }
}

fn expand_home(p: &str) -> String {
    match p.strip_prefix('~') {
        Some(rest) => format!("{}{rest}", std::env::var("HOME").unwrap_or_default()),
        None => p.to_string(),
    }
}

/// `fake-key <keystring>`: send keys to the page as trusted input. qutebrowser
/// notation: `abc` types three chars, `<Tab>`, `<Ctrl-a>`.
pub fn fake_key(keystring: &str) -> Result<(), String> {
    let browser = window::active_browser().ok_or("no active tab")?;
    for key in keys::parse_keystring(keystring)? {
        for ev in key_events(&key.name, key.mods) {
            devtools::send(&browser, "Input.dispatchKeyEvent", ev);
        }
    }
    Ok(())
}

/// CDP key events for one key: printable chars carry `text`, named keys a vk code.
fn key_events(name: &str, mods: Mods) -> Vec<serde_json::Value> {
    let m = (mods.contains(Mods::ALT) as u8)
        | (mods.contains(Mods::CTRL) as u8) << 1
        | (mods.contains(Mods::META) as u8) << 2
        | (mods.contains(Mods::SHIFT) as u8) << 3;
    let named: &[(&str, &str, i32)] = &[
        ("Tab", "Tab", 9),
        ("Return", "Enter", 13),
        ("Escape", "Escape", 27),
        ("Backspace", "Backspace", 8),
        ("Delete", "Delete", 46),
        ("Space", " ", 32),
        ("Left", "ArrowLeft", 37),
        ("Up", "ArrowUp", 38),
        ("Right", "ArrowRight", 39),
        ("Down", "ArrowDown", 40),
        ("Home", "Home", 36),
        ("End", "End", 35),
        ("PgUp", "PageUp", 33),
        ("PgDown", "PageDown", 34),
        ("Insert", "Insert", 45),
    ];
    let mut chars = name.chars();
    let single = chars.next().filter(|_| chars.next().is_none());
    let (key, vk, text) = match (named.iter().find(|n| n.0 == name), single) {
        (Some(&(_, key, vk)), _) => (
            key.to_string(),
            vk,
            match vk {
                13 => "\r",
                9 => "\t",
                32 => " ",
                _ => "",
            }
            .to_string(),
        ),
        (None, Some(c)) => {
            let vk = if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase() as i32
            } else {
                0
            };
            (c.to_string(), vk, c.to_string())
        }
        // shortcut: F-keys and other named keys go out without a vk code; add when a script needs them
        _ => (name.to_string(), 0, String::new()),
    };
    // With Ctrl/Alt/Meta a char is a shortcut, not text.
    let text = if m & 0b111 != 0 { String::new() } else { text };
    let down = if text.is_empty() {
        "rawKeyDown"
    } else {
        "keyDown"
    };
    vec![
        json!({"type": down, "key": key, "text": text, "unmodifiedText": text, "windowsVirtualKeyCode": vk, "modifiers": m}),
        json!({"type": "keyUp", "key": key, "windowsVirtualKeyCode": vk, "modifiers": m}),
    ]
}

/// `insert-text <text>`: insert at the focused element (no key events, S6).
pub fn insert_text(text: &str) -> Result<(), String> {
    let browser = window::active_browser().ok_or("no active tab")?;
    devtools::send(&browser, "Input.insertText", json!({ "text": text }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_events_text_named_and_ctrl() {
        let e = key_events("a", Mods::NONE);
        assert_eq!(e[0]["type"], "keyDown");
        assert_eq!(e[0]["text"], "a");
        assert_eq!(e[0]["windowsVirtualKeyCode"], 65);
        let e = key_events("Tab", Mods::NONE);
        assert_eq!(
            (e[0]["key"].as_str(), e[0]["windowsVirtualKeyCode"].as_i64()),
            (Some("Tab"), Some(9))
        );
        let e = key_events("a", Mods::CTRL);
        assert_eq!(
            (
                e[0]["type"].as_str(),
                e[0]["text"].as_str(),
                e[0]["modifiers"].as_i64()
            ),
            (Some("rawKeyDown"), Some(""), Some(2))
        );
        assert_eq!(key_events("@", Mods::NONE)[0]["text"], "@");
    }

    #[test]
    fn fifo_and_finding_scripts() {
        let d = tempdir().unwrap();
        mkfifo(&d.join("fifo")).unwrap();
        use std::os::unix::fs::FileTypeExt;
        assert!(std::fs::metadata(d.join("fifo"))
            .unwrap()
            .file_type()
            .is_fifo());
        std::fs::remove_dir_all(&d).unwrap();
        assert!(find_userscript("/bin/sh").is_ok());
        assert!(find_userscript("no-such-script-xyz").is_err());
    }
}
