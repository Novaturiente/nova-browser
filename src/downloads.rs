//! Downloads (PLAN §5.4): straight into `downloads.location.directory` (default
//! ~/Downloads), no prompt, unique names. The tab bar shows a small pill (running
//! count + progress, then finished-but-unseen count); `:downloads` (or a click on
//! the pill) opens the history page; finished entries persist in
//! `<data>/downloads.json`. `download-open [N]` opens entry N of that page (newest
//! = 1) with `downloads.open_dispatcher` or xdg-open.
//! shortcut: `downloads.location.prompt` is ignored (always saves).
use crate::{config, window, xdg_dir};
use cef::*;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
enum St {
    Running,
    Done,
    Failed,
}

struct Dl {
    /// CEF download id; None for entries loaded from disk.
    id: Option<u32>,
    /// aria2 download (aria2.rs); None for Chromium ones.
    gid: Option<String>,
    url: String,
    path: PathBuf,
    st: St,
    received: i64,
    total: i64,
    start: u64,
    end: u64,
    cb: Option<DownloadItemCallback>,
}

const KEEP: usize = 500;

thread_local! {
    /// Oldest first; None until loaded from disk.
    static LIST: RefCell<Option<Vec<Dl>>> = const { RefCell::new(None) };
    /// Downloads finished since the history page was last opened.
    static UNSEEN: Cell<(u32, u32)> = const { Cell::new((0, 0)) };
}

fn file() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share").join("downloads.json")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn with<R>(f: impl FnOnce(&mut Vec<Dl>) -> R) -> R {
    LIST.with_borrow_mut(|l| {
        let l = l.get_or_insert_with(|| {
            let v: Value = std::fs::read_to_string(file())
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();
            v.as_array()
                .into_iter()
                .flatten()
                .map(|e| Dl {
                    id: None,
                    gid: None,
                    url: e["url"].as_str().unwrap_or("").into(),
                    path: e["path"].as_str().unwrap_or("").into(),
                    st: if e["ok"].as_bool().unwrap_or(false) {
                        St::Done
                    } else {
                        St::Failed
                    },
                    received: e["size"].as_i64().unwrap_or(0),
                    total: e["size"].as_i64().unwrap_or(0),
                    start: e["start"].as_u64().unwrap_or(0),
                    end: e["end"].as_u64().unwrap_or(0),
                    cb: None,
                })
                .collect()
        });
        f(l)
    })
}

fn save() {
    let v: Vec<Value> = with(|l| {
        let fin: Vec<&Dl> = l.iter().filter(|d| d.st != St::Running).collect();
        fin[fin.len().saturating_sub(KEEP)..]
            .iter()
            .map(|d| json!({ "url": d.url, "path": d.path, "ok": d.st == St::Done, "size": d.received, "start": d.start, "end": d.end }))
            .collect()
    });
    let f = file();
    if let Err(e) = crate::write_atomic(&f, serde_json::to_string(&v).unwrap().as_bytes()) {
        eprintln!("novabrowser: {}: {e}", f.display());
    }
}

pub fn dir() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    match config::get().str("downloads.location.directory") {
        d if d.is_empty() => home.join("Downloads"),
        d => match d.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(d),
        },
    }
}

/// `name` in `dir`, or `name (1).ext`, `name (2).ext`, … if taken (also by a running download).
fn unique(dir: &Path, name: &str, taken: &[PathBuf]) -> PathBuf {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    let free = |p: &PathBuf| !p.exists() && !taken.contains(p);
    let mut p = dir.join(name);
    let mut i = 1;
    while !free(&p) {
        p = dir.join(format!("{stem} ({i}){ext}"));
        i += 1;
    }
    p
}

/// Untrusted server-suggested name → a plain file name.
fn sanitize(name: &str) -> String {
    let n: String = name
        .chars()
        .map(|c| if c == '/' || c == '\0' { '_' } else { c })
        .collect();
    let n = n.trim().trim_start_matches('.').to_string();
    if n.is_empty() {
        "download".into()
    } else {
        n
    }
}

pub fn before(item: &DownloadItem, suggested: &str, cb: &BeforeDownloadCallback) {
    let d = dir();
    if let Err(e) = std::fs::create_dir_all(&d) {
        return window::message("error", &format!("download: {}: {e}", d.display()));
    }
    let taken: Vec<PathBuf> = with(|l| {
        l.iter()
            .filter(|x| x.st == St::Running)
            .map(|x| x.path.clone())
            .collect()
    });
    let path = unique(&d, &sanitize(suggested), &taken);
    with(|l| {
        l.push(Dl {
            id: Some(item.id()),
            gid: None,
            url: CefString::from(&item.url()).to_string(),
            path: path.clone(),
            st: St::Running,
            received: 0,
            total: item.total_bytes(),
            start: now(),
            end: 0,
            cb: None,
        })
    });
    cb.cont(
        Some(&CefString::from(path.to_str().unwrap_or("download"))),
        0,
    );
    window::refresh();
}

pub fn updated(item: &DownloadItem, cb: Option<&DownloadItemCallback>) {
    let id = item.id();
    let (complete, failed) = (
        item.is_complete() != 0,
        item.is_canceled() != 0 || item.is_interrupted() != 0,
    );
    let report = with(|l| {
        let d = l.iter_mut().find(|d| d.id == Some(id))?;
        if d.st != St::Running {
            return None;
        }
        d.received = item.received_bytes();
        d.total = item.total_bytes();
        d.cb = cb.cloned();
        if complete || failed {
            d.st = if failed { St::Failed } else { St::Done };
            d.end = now();
            d.cb = None;
            return Some(d.path.clone());
        }
        None
    });
    match report {
        Some(p) => finished(&p, failed, None),
        None => window::refresh(),
    }
}

fn finished(p: &Path, failed: bool, why: Option<&str>) {
    let (ok, bad) = UNSEEN.get();
    UNSEEN.set(if failed { (ok, bad + 1) } else { (ok + 1, bad) });
    save();
    let msg = match (failed, why) {
        (true, Some(w)) => format!("Download failed: {} ({w})", p.display()),
        (true, None) => format!("Download failed: {}", p.display()),
        (false, _) => format!("Downloaded: {}", p.display()),
    };
    window::message(if failed { "error" } else { "info" }, &msg);
}

thread_local! {
    /// aria2 requests waiting for their gid: id → URL (to fall back to Chromium).
    static ARIA_PENDING: RefCell<Vec<(u64, String)>> = const { RefCell::new(Vec::new()) };
    static ARIA_NEXT: Cell<u64> = const { Cell::new(1) };
    /// URLs aria2 refused: let Chromium download them (can_download sees them again).
    static FORCE_CHROMIUM: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// `DownloadHandler::can_download`: true = Chromium downloads it, false = we
/// handed it to aria2 (GET http(s)/ftp only; POST/blob:/data: stay in Chromium).
pub fn chromium_should_download(browser: &Browser, url: &str, method: &str) -> bool {
    if FORCE_CHROMIUM
        .with_borrow_mut(|f| f.iter().position(|u| u == url).map(|i| f.remove(i)))
        .is_some()
    {
        return true;
    }
    let handoff = crate::aria2::enabled()
        && method.eq_ignore_ascii_case("GET")
        && ["http://", "https://", "ftp://"]
            .iter()
            .any(|s| url.starts_with(s));
    if !handoff {
        return true;
    }
    let id = ARIA_NEXT.replace(ARIA_NEXT.get() + 1);
    ARIA_PENDING.with_borrow_mut(|p| p.push((id, url.to_string())));
    crate::aria2::add(browser, url.to_string(), id);
    false
}

/// A magnet: link (IO thread): aria2 only.
pub fn magnet(url: String) -> bool {
    if !crate::aria2::enabled() {
        return false;
    }
    crate::aria2::add_magnet(url, 0);
    true
}

/// aria2 took (gid) or refused a download we handed it.
pub fn aria2_added(id: u64, result: Result<String, String>) {
    let url = ARIA_PENDING.with_borrow_mut(|p| {
        let i = p.iter().position(|(i, _)| *i == id)?;
        Some(p.remove(i).1)
    });
    match result {
        Ok(gid) => {
            let known = with(|l| l.iter().any(|d| d.gid.as_deref() == Some(&gid)));
            if !known {
                with(|l| l.push(aria_entry(gid, url.clone().unwrap_or_default())));
            }
            window::message("info", "Download started (aria2).");
        }
        Err(e) => match url.filter(|_| id != 0) {
            // fall back to Chromium for this URL
            Some(u) => {
                window::message(
                    "warning",
                    &format!("aria2 refused the download ({e}); using Chromium."),
                );
                FORCE_CHROMIUM.with_borrow_mut(|f| f.push(u.clone()));
                if let Some(h) = window::active_browser().and_then(|b| b.host()) {
                    h.start_download(Some(&CefString::from(u.as_str())));
                }
            }
            None => window::message("error", &format!("aria2: {e}")),
        },
    }
}

fn aria_entry(gid: String, url: String) -> Dl {
    Dl {
        id: None,
        gid: Some(gid),
        path: PathBuf::from(&url),
        url,
        st: St::Running,
        received: 0,
        total: 0,
        start: now(),
        end: 0,
        cb: None,
    }
}

/// aria2 status poll (aria2.rs): update, add resumed/unknown ones, finish.
pub fn aria2_update(items: Vec<Value>) {
    let mut done = Vec::new();
    with(|l| {
        // magnet / .torrent / metalink: the real download continues under a new gid;
        // move our entry to it first, so the follow-up never shows up as a second one
        for it in &items {
            if let (Some(gid), Some(next)) = (it["gid"].as_str(), it["followedBy"][0].as_str()) {
                if let Some(d) = l.iter_mut().find(|d| d.gid.as_deref() == Some(gid)) {
                    d.gid = Some(next.to_string());
                }
            }
        }
        for it in &items {
            let Some(gid) = it["gid"].as_str() else {
                continue;
            };
            let status = it["status"].as_str().unwrap_or("");
            if it["followedBy"][0].is_string() {
                continue; // handled above
            }
            let i = match l.iter().position(|d| d.gid.as_deref() == Some(gid)) {
                Some(i) => i,
                // resumed from aria2's session (also when it fails at once), or added by a magnet
                None => {
                    let url = it["files"][0]["uris"][0]["uri"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    l.push(aria_entry(gid.to_string(), url));
                    l.len() - 1
                }
            };
            let d = &mut l[i];
            if d.st != St::Running {
                continue;
            }
            d.received = it["completedLength"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            d.total = it["totalLength"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let dir = it["dir"].as_str().unwrap_or("");
            // a torrent's name (folder or file), else the first file's path
            if let Some(name) = it["bittorrent"]["info"]["name"].as_str() {
                d.path = Path::new(dir).join(name);
            } else if let Some(p) = it["files"][0]["path"].as_str().filter(|p| !p.is_empty()) {
                d.path = PathBuf::from(p);
            }
            match status {
                "complete" => {
                    d.st = St::Done;
                    d.end = now();
                    done.push((d.path.clone(), false, None));
                }
                "removed" | "error" => {
                    // cancelled (we only remove on download-cancel): drop the partial file, like Chromium
                    // (a torrent folder is left alone: it may hold files the user already had)
                    if status == "removed" {
                        if d.path.is_file() {
                            let _ = std::fs::remove_file(&d.path);
                        }
                        let _ = std::fs::remove_file(format!("{}.aria2", d.path.display()));
                    }
                    d.st = St::Failed;
                    d.end = now();
                    let why = it["errorMessage"]
                        .as_str()
                        .filter(|m| !m.is_empty())
                        .map(str::to_string);
                    done.push((d.path.clone(), true, why));
                }
                _ => {}
            }
        }
    });
    for (p, failed, why) in &done {
        finished(p, *failed, why.as_deref());
    }
    if done.is_empty() {
        window::refresh();
    }
}

/// Tab bar pill: "↓ 2 · 45%" while running, "↓ 3 ✓" / "↓ 1 ✗" for unseen
/// finished ones, "" otherwise. Second value = CSS class.
pub fn summary() -> (String, &'static str) {
    let (n, got, total) = with(|l| {
        l.iter()
            .filter(|d| d.st == St::Running)
            .fold((0, 0i64, 0i64), |(n, g, t), d| {
                (
                    n + 1,
                    g + d.received,
                    if d.total > 0 && t >= 0 {
                        t + d.total
                    } else {
                        -1
                    },
                )
            })
    });
    if n > 0 {
        let pct = if total > 0 {
            format!(" · {}%", got * 100 / total)
        } else {
            String::new()
        };
        return (format!("↓ {n}{pct}"), "run");
    }
    match UNSEEN.get() {
        (0, 0) => (String::new(), ""),
        (ok, 0) => (format!("↓ {ok} ✓"), "done"),
        (ok, bad) => (format!("↓ {} ✗", ok + bad), "fail"),
    }
}

/// Newest first, as numbered on the history page.
fn nth(n: usize) -> Option<(PathBuf, St)> {
    with(|l| {
        l.iter()
            .rev()
            .nth(n.checked_sub(1)?)
            .map(|d| (d.path.clone(), d.st))
    })
}

/// `download-open [N] [cmd…]`: entry N of the history page (default: newest finished),
/// with `cmd` ({} = path, else appended) or the dispatcher / xdg-open.
pub fn open(args: &[String]) -> Result<(), String> {
    let (path, cmdline) = match args.first().and_then(|a| a.parse::<usize>().ok()) {
        Some(n) => match nth(n) {
            Some((p, St::Done)) => (p, &args[1..]),
            Some(_) => return Err(format!("Download {n} isn't finished")),
            None => return Err(format!("No download {n}")),
        },
        None => (
            with(|l| {
                l.iter()
                    .rev()
                    .find(|d| d.st == St::Done)
                    .map(|d| d.path.clone())
            })
            .ok_or("No finished download")?,
            args,
        ),
    };
    if !path.exists() {
        return Err(format!("{} no longer exists", path.display()));
    }
    let p = path.to_string_lossy().to_string();
    let argv: Vec<String> = if !cmdline.is_empty() {
        cmdline
            .iter()
            .map(|a| a.replace("{}", &p))
            .chain((!cmdline.iter().any(|a| a.contains("{}"))).then(|| p.clone()))
            .collect()
    } else {
        match config::get().str("downloads.open_dispatcher") {
            d if d.is_empty() => vec!["xdg-open".into(), p],
            d => vec![d, p],
        }
    };
    crate::commands::spawn(&argv)
}

/// `download-cancel [N]`: entry N, or the newest running download.
pub fn cancel(n: Option<usize>) -> Result<(), String> {
    let (cb, gid) = with(|l| {
        let d = match n {
            Some(n) => l.iter().rev().nth(n.checked_sub(1)?),
            None => l.iter().rev().find(|d| d.st == St::Running),
        }?;
        Some((d.cb.clone(), d.gid.clone()))
    })
    .unwrap_or_default();
    match (cb, gid) {
        (Some(cb), _) => cb.cancel(),
        (None, Some(gid)) => crate::aria2::remove(&gid),
        _ => return Err("No such running download".into()),
    }
    Ok(())
}

/// `download-clear`: forget finished entries (files stay on disk).
pub fn clear() {
    with(|l| l.retain(|d| d.st == St::Running));
    UNSEEN.set((0, 0));
    save();
    window::refresh();
}

/// `:downloads` / pill click: the history page (a snapshot; reopen to refresh).
pub fn page() -> String {
    UNSEEN.set((0, 0));
    let rows: Vec<Value> = with(|l| {
        l.iter()
            .rev()
            .map(|d| {
                json!({
                    "name": d.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                    "dir": d.path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
                    "url": d.url,
                    "st": match d.st { St::Running => "running", St::Done => if d.path.exists() { "done" } else { "deleted" }, St::Failed => "failed" },
                    "got": d.received, "total": d.total, "start": d.start, "end": d.end,
                })
            })
            .collect()
    });
    let data = serde_json::to_string(&rows)
        .unwrap()
        .replace('<', "\\u003c");
    crate::theme::themed_page(PAGE.replace("/*DATA*/[]", &data))
}

const PAGE: &str = r#"<!doctype html><meta charset=utf-8><title>Downloads</title>
<style>
body{font:14px system-ui,sans-serif;max-width:64em;margin:2em auto;padding:0 1em;color-scheme:light dark}
h1{font-size:1.3em;font-weight:600}.hint{opacity:.6;font-size:.85em}
.d{display:grid;grid-template-columns:2.5em 1fr auto;gap:.1em 1em;padding:.7em .8em;border-radius:10px;margin:.4em 0;background:color-mix(in srgb,currentColor 6%,transparent)}
.n{opacity:.5;grid-row:span 3;font-variant-numeric:tabular-nums}.name{font-weight:600;word-break:break-all}
.st{font-size:.85em;padding:0 .6em;border-radius:999px;align-self:start}
.done{background:#2a7a3a33}.failed,.deleted{background:#a0303033}.running{background:#3060a033}
.meta,.src{grid-column:2/4;font-size:.85em;opacity:.7;word-break:break-all}
</style>
<h1>Downloads</h1>
<p class=hint><code>:download-open N</code> opens entry N · <code>:download-cancel N</code> · <code>:download-clear</code> forgets finished entries · reopen <code>:downloads</code> to refresh</p>
<div id=list></div>
<script>
const rows=/*DATA*/[];
const size=b=>b<1024?b+" B":b<1048576?(b/1024).toFixed(1)+" KB":b<1073741824?(b/1048576).toFixed(1)+" MB":(b/1073741824).toFixed(2)+" GB";
const time=s=>s?new Date(s*1000).toLocaleString():"";
const list=document.getElementById("list");
if(!rows.length)list.textContent="No downloads yet.";
rows.forEach((r,i)=>{
  const d=document.createElement("div");d.className="d";
  const add=(cls,text)=>{const e=document.createElement("div");e.className=cls;e.textContent=text;d.append(e);};
  add("n",i+1);add("name",r.name);
  add("st "+r.st,r.st==="running"&&r.total>0?Math.floor(r.got*100/r.total)+"%":r.st);
  add("meta",[r.st==="running"?size(r.got)+(r.total>0?" of "+size(r.total):""):size(r.got),"in "+r.dir,r.end?"finished "+time(r.end):"started "+time(r.start)].join(" · "));
  add("src",r.url);
  list.append(d);
});
</script>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_names_and_sanitizing() {
        let d = std::env::temp_dir().join(format!("nova-dl-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.pdf"), "").unwrap();
        assert_eq!(unique(&d, "a.pdf", &[]), d.join("a (1).pdf"));
        assert_eq!(
            unique(&d, "a.pdf", &[d.join("a (1).pdf")]),
            d.join("a (2).pdf")
        );
        assert_eq!(unique(&d, "b", &[]), d.join("b"));
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(sanitize("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize(".bashrc"), "bashrc");
        assert_eq!(sanitize(""), "download");
    }
}
