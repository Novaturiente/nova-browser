//! Clipboard. Copy goes through GTK (our window owns the clipboard; works on Wayland
//! and X11, no extra tools); reading (for `{clipboard}`) uses `wl-paste`, else
//! `xclip`, off the UI thread.
use cef::*;
use gtk::prelude::*;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

pub fn write(text: &str, primary: bool) {
    let Some(d) = gtk::gdk::Display::default() else {
        return write_external(text.to_string(), primary);
    };
    let c = if primary { d.primary_clipboard() } else { d.clipboard() };
    c.set_text(text);
}

/// Paste the system clipboard's text into `browser`'s focused frame. Off-screen
/// Chromium has no Wayland focus, so it never sees the system clipboard: hand the page
/// a paste event with the text, and insert it when the page doesn't take it.
/// shortcut: text only (no images or HTML); read other types too if pages need them.
pub fn paste(browser: &Browser) {
    let Some(d) = gtk::gdk::Display::default() else { return };
    let b = browser.clone();
    d.clipboard().read_text_async(None::<&gtk::gio::Cancellable>, move |r| {
        let (Ok(Some(text)), Some(f)) = (r, b.focused_frame().or_else(|| b.main_frame())) else { return };
        let js = format!("{PASTE_JS}({})", serde_json::Value::from(text.as_str()));
        f.execute_java_script(Some(&CefString::from(js.as_str())), None, 0);
    });
}

const PASTE_JS: &str = "(t=>{const a=document.activeElement||document.body,d=new DataTransfer();d.setData('text/plain',t);\
if(a.dispatchEvent(new ClipboardEvent('paste',{clipboardData:d,bubbles:true,cancelable:true,composed:true})))\
document.execCommand('insertText',false,t)})";

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
