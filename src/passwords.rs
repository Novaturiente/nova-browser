//! Password managers (PLAN Phase 10): `rbw` (Bitwarden), `bw` (Bitwarden CLI) and
//! `pass` behind one small contract — list entries for a site, secret, TOTP, save,
//! update. We never keep secrets at rest; they pass through memory on user action.
//! The CLIs run on worker threads; results come back through `crate::on_ui`.

use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cef::*;
use serde_json::Value;

use crate::{config, window};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Backend {
    Rbw,
    Bw,
    Pass,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub user: String,
    pub uris: Vec<String>,
}

pub fn backend() -> Option<Backend> {
    match config::get().str("passwords.backend").as_str() {
        "rbw" => Some(Backend::Rbw),
        "bw" => Some(Backend::Bw),
        "pass" => Some(Backend::Pass),
        _ => None,
    }
}

fn label(b: Backend) -> &'static str {
    match b {
        Backend::Rbw | Backend::Bw => "Bitwarden",
        Backend::Pass => "pass",
    }
}

/// Registrable domain ("base domain", Bitwarden's default match) of a URL or a
/// bare host as stored in vault URIs ("github.com", "100.88.1.2:8080/x").
pub fn site(uri: &str) -> Option<String> {
    let uri = uri.trim();
    let full = if uri.contains("://") { uri.to_string() } else { format!("https://{uri}") };
    if !(full.starts_with("http://") || full.starts_with("https://")) {
        return None; // androidapp://, ssh://, …
    }
    adblock::url_parser::parse_url(&full).map(|u| u.domain().to_string()).filter(|d| !d.is_empty())
}

fn host(url: &str) -> String {
    crate::commands::host(url).to_string()
}

/// scheme://host[:port] of a URL (no userinfo, path or query).
fn origin(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("https", url));
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    format!("{scheme}://{}", auth.rsplit_once('@').map_or(auth, |(_, h)| h))
}

// ---- running the CLIs ----

fn run(prog: &str, args: &[&str], stdin: Option<&str>, env: &[(&str, &str)]) -> Result<String, String> {
    let mut cmd = Command::new(prog);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().map_err(|e| format!("{prog}: {e}"))?;
    if let Some(input) = stdin {
        let mut i = child.stdin.take().unwrap();
        i.write_all(input.as_bytes()).map_err(|e| format!("{prog}: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("{prog}: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim().lines().last().unwrap_or("failed");
        return Err(format!("{prog} {}: {err}", args.first().unwrap_or(&"")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| format!("unexpected output: {e}"))
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn pass_dir() -> PathBuf {
    std::env::var_os("PASSWORD_STORE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").expect("HOME")).join(".password-store"))
}

/// Metadata only (no secrets kept). Cached: `bw` takes about a second per call.
static CACHE: Mutex<Option<(Instant, Backend, Vec<Entry>)>> = Mutex::new(None);

fn list_all(b: Backend) -> Result<Vec<Entry>, String> {
    if let Some((t, cb, e)) = &*CACHE.lock().unwrap() {
        if *cb == b && t.elapsed() < Duration::from_secs(60) {
            return Ok(e.clone());
        }
    }
    let entries: Vec<Entry> = match b {
        Backend::Rbw => json(&run("rbw", &["list", "--raw"], None, &[])?)?
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["type"] == "Login")
            .map(|e| Entry {
                id: s(&e["id"]),
                name: s(&e["name"]),
                user: s(&e["user"]),
                uris: e["uris"].as_array().into_iter().flatten().map(s).collect(),
            })
            .collect(),
        // shortcut: `bw list items` also returns secrets; we drop them right here.
        Backend::Bw => json(&run("bw", &["list", "items"], None, &[])?)?
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["type"] == 1)
            .map(|e| Entry {
                id: s(&e["id"]),
                name: s(&e["name"]),
                user: s(&e["login"]["username"]),
                uris: e["login"]["uris"].as_array().into_iter().flatten().map(|u| s(&u["uri"])).collect(),
            })
            .collect(),
        Backend::Pass => pass_entries(),
    };
    *CACHE.lock().unwrap() = Some((Instant::now(), b, entries.clone()));
    Ok(entries)
}

/// pass: `<site>/<user>.gpg` or `<anything>/<site>.gpg`; path parts with a dot are sites.
fn pass_entries() -> Vec<Entry> {
    let root = pass_dir();
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if let Some(stem) = name.strip_suffix(".gpg") {
                let id = p.strip_prefix(&root).unwrap().with_extension("").to_string_lossy().into_owned();
                let parts: Vec<&str> = id.split('/').collect();
                let uris: Vec<String> = parts.iter().filter(|x| x.contains('.')).map(|x| x.to_string()).collect();
                let parent_is_site = parts.len() > 1 && parts[parts.len() - 2].contains('.');
                let user = if parent_is_site { stem.to_string() } else { String::new() };
                out.push(Entry { id: id.clone(), name: id, user, uris });
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Entries whose URI shares the registrable domain of `url`; same host first.
pub fn matching(b: Backend, url: &str) -> Result<Vec<Entry>, String> {
    let Some(want) = site(url) else { return Ok(Vec::new()) };
    let h = host(url);
    let mut v: Vec<Entry> = list_all(b)?
        .into_iter()
        .filter(|e| e.uris.iter().any(|u| site(u).as_deref() == Some(&want)))
        .collect();
    let exact = |e: &Entry| e.uris.iter().any(|u| site(u).is_some() && host(&format!("https://{}", u.split("://").last().unwrap_or(u))) == h);
    v.sort_by_key(|e| !exact(e));
    Ok(v)
}

/// (username, password)
pub fn secret(b: Backend, e: &Entry) -> Result<(String, String), String> {
    match b {
        Backend::Rbw => {
            let d = json(&run("rbw", &["get", "--raw", &e.id], None, &[])?)?;
            Ok((s(&d["data"]["username"]), s(&d["data"]["password"])))
        }
        Backend::Bw => {
            let d = json(&run("bw", &["get", "item", &e.id], None, &[])?)?;
            Ok((s(&d["login"]["username"]), s(&d["login"]["password"])))
        }
        Backend::Pass => {
            let text = run("pass", &["show", &e.id], None, &[])?;
            let mut lines = text.lines();
            let pw = lines.next().unwrap_or_default().to_string();
            let user = lines
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    matches!(k.trim().to_lowercase().as_str(), "login" | "user" | "username" | "email")
                        .then(|| v.trim().to_string())
                })
                .unwrap_or_else(|| e.user.clone());
            Ok((user, pw))
        }
    }
}

pub fn totp(b: Backend, e: &Entry) -> Result<String, String> {
    let out = match b {
        Backend::Rbw => run("rbw", &["code", &e.id], None, &[])?,
        Backend::Bw => run("bw", &["get", "totp", &e.id], None, &[])?,
        Backend::Pass => run("pass", &["otp", &e.id], None, &[])?,
    };
    Ok(out.trim().to_string())
}

/// 20 chars from /dev/urandom: letters, digits and a few symbols sites accept.
pub fn generate() -> String {
    const CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789-_.!@#%";
    let mut buf = [0u8; 64];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut buf))
        .expect("/dev/urandom");
    // 256 % 64 == 0: no modulo bias
    buf.iter().take(20).map(|b| CHARS[*b as usize % CHARS.len()] as char).collect()
}

/// `$VISUAL` for `rbw add`/`rbw edit`: novabrowser itself in editor mode, the secret
/// in its environment (readable by this user only; never argv).
fn rbw_editor(mode: &str, secret: &str) -> Result<Vec<(&'static str, String)>, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(vec![
        ("VISUAL", exe.to_string_lossy().into_owned()),
        ("EDITOR", exe.to_string_lossy().into_owned()),
        ("NOVA_PW_SECRET", secret.to_string()),
        ("NOVA_PW_MODE", mode.to_string()),
    ])
}

/// Editor mode (see rbw_editor): put the secret on the first line of the file.
/// "replace" swaps the old password line (edit), "insert" adds it on top (add).
pub fn editor_main(path: &str) -> i32 {
    let (Ok(secret), Ok(mode)) = (std::env::var("NOVA_PW_SECRET"), std::env::var("NOVA_PW_MODE")) else { return 2 };
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let rest = match mode.as_str() {
        "replace" => text.split_once('\n').map(|(_, r)| r).unwrap_or(""),
        _ => text.as_str(),
    };
    match std::fs::write(path, format!("{secret}\n{rest}")) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn with_env<'a>(v: &'a [(&'static str, String)]) -> Vec<(&'a str, &'a str)> {
    v.iter().map(|(k, x)| (*k, x.as_str())).collect()
}

pub fn save(b: Backend, url: &str, user: &str, pw: &str) -> Result<(), String> {
    let h = host(url);
    let origin = origin(url);
    match b {
        Backend::Rbw => {
            let env = rbw_editor("insert", pw)?;
            let mut args = vec!["add", "--uri", origin.as_str(), h.as_str()];
            if !user.is_empty() {
                args.push(user);
            }
            run("rbw", &args, None, &with_env(&env))?;
        }
        Backend::Bw => {
            let item = serde_json::json!({
                "type": 1, "name": h, "notes": null, "favorite": false,
                "login": { "username": user, "password": pw, "uris": [{ "uri": origin, "match": null }] },
            });
            let enc = run("bw", &["encode"], Some(&item.to_string()), &[])?;
            run("bw", &["create", "item"], Some(enc.trim()), &[])?;
        }
        Backend::Pass => {
            let name = if user.is_empty() { h.clone() } else { format!("{h}/{}", user.replace('/', "_")) };
            if pass_dir().join(format!("{name}.gpg")).exists() {
                return Err(format!("pass: {name} already exists"));
            }
            run("pass", &["insert", "-m", &name], Some(&format!("{pw}\nlogin: {user}\nurl: {origin}\n")), &[])?;
        }
    }
    *CACHE.lock().unwrap() = None;
    Ok(())
}

pub fn update(b: Backend, e: &Entry, pw: &str) -> Result<(), String> {
    match b {
        Backend::Rbw => {
            let env = rbw_editor("replace", pw)?;
            run("rbw", &["edit", &e.id], None, &with_env(&env))?;
        }
        Backend::Bw => {
            let mut item = json(&run("bw", &["get", "item", &e.id], None, &[])?)?;
            item["login"]["password"] = pw.into();
            let enc = run("bw", &["encode"], Some(&item.to_string()), &[])?;
            run("bw", &["edit", "item", &e.id], Some(enc.trim()), &[])?;
        }
        Backend::Pass => {
            let old = run("pass", &["show", &e.id], None, &[])?;
            let rest = old.split_once('\n').map(|(_, r)| r).unwrap_or("");
            run("pass", &["insert", "-m", "-f", &e.id], Some(&format!("{pw}\n{rest}")), &[])?;
        }
    }
    *CACHE.lock().unwrap() = None;
    Ok(())
}

// ---- UI: suggestions under the focused login field, fill, save prompts ----

/// What the focused field wants (from the renderer's classifier).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    User,
    Password,
    New,
    Otp,
}

impl Kind {
    fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "user" => Kind::User,
            "password" => Kind::Password,
            "new" => Kind::New,
            "otp" => Kind::Otp,
            _ => return None,
        })
    }
    fn fill_what(self) -> &'static str {
        match self {
            Kind::Otp => "otp",
            Kind::New => "new",
            _ => "login",
        }
    }
}

/// One suggestion row: an entry (fill login or its TOTP) or "generate".
#[derive(Clone)]
enum Item {
    Entry(Entry),
    Generate,
}

struct Field {
    seq: u64,
    browser: i32,
    frame: Frame,
    kind: Kind,
    url: String,
    rect: Rect,
    items: Vec<Item>,
    sel: Option<usize>,
}

thread_local! {
    static FIELD: RefCell<Option<Field>> = const { RefCell::new(None) };
    static GEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// (browser, site) → username from a split login's first page
    static LAST_USER: RefCell<Vec<(i32, String, String)>> = const { RefCell::new(Vec::new()) };
    /// last (site, user, password) offered for saving, to ignore repeats of one submit
    /// our own fill moves focus; don't pop the list up again right after
    static FILLED_AT: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
    static LAST_SUBMIT: RefCell<Option<(Instant, String, String, String)>> = const { RefCell::new(None) };
}

fn frame_url(f: &Frame) -> String {
    CefString::from(&f.url()).to_string()
}

/// Cross-site frames get nothing: the frame must share the tab's site.
fn same_site_as_top(browser: &Browser, frame: &Frame) -> bool {
    let top = browser.main_frame().map(|m| frame_url(&m)).unwrap_or_default();
    let (a, b) = (site(&top), site(&frame_url(frame)));
    a.is_some() && a == b
}

/// Renderer said: focus moved; `kind` is "" for a non-login field. `rect` = field
/// bounds in the frame's CSS px.
pub fn on_focus(browser: &Browser, frame: &Frame, kind: &str, rect: Rect) {
    let Some(b) = backend() else { return };
    let Some(kind) = Kind::parse(kind) else { return hide() };
    if FILLED_AT.get().is_some_and(|t| t.elapsed() < Duration::from_millis(800)) {
        return;
    }
    if !same_site_as_top(browser, frame) {
        return hide();
    }
    let seq = GEN.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    let url = frame_url(frame);
    FIELD.set(Some(Field { seq, browser: browser.identifier(), frame: frame.clone(), kind, url: url.clone(), rect, items: Vec::new(), sel: None }));
    if kind == Kind::New {
        return show(seq, Vec::new());
    }
    std::thread::spawn(move || {
        let r = matching(b, &url);
        crate::on_ui(move || match r {
            Ok(v) => show(seq, v),
            Err(e) => window::message("error", &e),
        });
    });
}

fn show(seq: u64, entries: Vec<Entry>) {
    let rows = FIELD.with_borrow_mut(|f| {
        let f = f.as_mut().filter(|f| f.seq == seq)?;
        let mut items: Vec<Item> = entries.into_iter().map(Item::Entry).collect();
        if f.kind == Kind::New {
            items.push(Item::Generate);
        }
        f.items = items;
        f.sel = None;
        Some(rows(f))
    });
    match rows {
        Some(r) if r.1["rows"].as_array().is_some_and(|a| !a.is_empty()) => window::suggest(r.0, &r.1),
        _ => hide(),
    }
}

/// (field rect, rows json) for the popup.
fn rows(f: &Field) -> (Rect, Value) {
    let b = backend().map(label).unwrap_or("");
    let rows: Vec<Value> = f
        .items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let (a, c) = match it {
                Item::Entry(e) if f.kind == Kind::Otp => (format!("{} code", e.name), e.user.clone()),
                Item::Entry(e) => (if e.user.is_empty() { e.name.clone() } else { e.user.clone() }, e.name.clone()),
                Item::Generate => ("Generate a strong password".into(), format!("saved to {b} after you submit")),
            };
            serde_json::json!({ "a": a, "b": c, "sel": f.sel == Some(i) })
        })
        .collect();
    (f.rect.clone(), serde_json::json!({ "title": b, "rows": rows }))
}

/// Safe from any CEF callback: the overlay is hidden in a posted task, because CEF
/// may call us inside window code that holds window state (load, close).
pub fn hide() {
    if FIELD.with_borrow(|f| f.as_ref().is_some_and(|f| !f.items.is_empty())) {
        crate::on_ui(window::suggest_hide);
    }
    FIELD.with_borrow_mut(|f| {
        if let Some(f) = f {
            f.items.clear();
        }
    });
}

pub fn visible() -> bool {
    FIELD.with_borrow(|f| f.as_ref().is_some_and(|f| !f.items.is_empty()))
}

/// Keys while the list is shown: Down/Up select, Enter fills, Escape hides.
/// Returns true when the key was ours.
pub fn on_key(name: &str) -> bool {
    if !visible() {
        return false;
    }
    let pick = FIELD.with_borrow_mut(|f| {
        let f = f.as_mut()?;
        let n = f.items.len();
        match name {
            "Down" => f.sel = Some(f.sel.map_or(0, |s| (s + 1) % n)),
            "Up" => f.sel = Some(f.sel.map_or(n - 1, |s| (s + n - 1) % n)),
            "Return" | "Enter" if f.sel.is_some() => return Some(Some(f.sel.unwrap())),
            "Escape" => return Some(None),
            _ => return None,
        }
        let (r, j) = rows(f);
        window::suggest(r, &j);
        Some(None)
    });
    match (name, pick) {
        (_, Some(Some(i))) => pick_item(i),
        ("Escape", _) => hide(),
        ("Down" | "Up", _) => {}
        _ => return false,
    }
    true
}

/// A click on a row in the popup.
pub fn on_click(i: usize) {
    pick_item(i);
}

fn pick_item(i: usize) {
    let Some(b) = backend() else { return };
    let Some((item, kind, seq)) = FIELD.with_borrow(|f| f.as_ref().and_then(|f| Some((f.items.get(i)?.clone(), f.kind, f.seq)))) else { return };
    hide();
    match item {
        Item::Generate => fill(seq, String::new(), generate(), "new"),
        Item::Entry(e) => {
            std::thread::spawn(move || {
                let r = if kind == Kind::Otp { totp(b, &e).map(|c| (String::new(), c)) } else { secret(b, &e) };
                crate::on_ui(move || match r {
                    Ok((u, p)) => fill(seq, u, p, kind.fill_what()),
                    Err(e) => window::message("error", &e),
                });
            });
        }
    }
}

/// Send the secret to the field's frame, if it is still the same page.
fn fill(seq: u64, user: String, pass: String, what: &str) {
    let Some((frame, url)) = FIELD.with_borrow(|f| f.as_ref().filter(|f| f.seq == seq).map(|f| (f.frame.clone(), f.url.clone()))) else { return };
    send_fill(&frame, &url, &user, &pass, what);
}

fn send_fill(frame: &Frame, url: &str, user: &str, pass: &str, what: &str) {
    // the frame may have navigated meanwhile: its site must still be the entry's
    if frame.is_valid() == 0 || site(&frame_url(frame)) != site(url) {
        return window::message("warning", "page changed, nothing filled");
    }
    FILLED_AT.set(Some(Instant::now()));
    let msg = process_message_create(Some(&CefString::from("nova.fill"))).expect("process_message_create");
    let args = msg.argument_list().expect("argument_list");
    args.set_string(0, Some(&CefString::from(user)));
    args.set_string(1, Some(&CefString::from(pass)));
    args.set_string(2, Some(&CefString::from(what)));
    frame.send_process_message(ProcessId::RENDERER, Some(&mut msg.clone()));
}

/// `:password-fill [--otp]` (`<Space>pp`): fill the best match into the focused
/// login form, or the page's first one. Several matches → the list to pick from.
pub fn fill_command(otp: bool) -> Result<(), String> {
    let b = backend().ok_or("passwords.backend is none")?;
    let browser = window::active_browser().ok_or("no tab")?;
    let id = browser.identifier();
    // the focused field's frame when it's in this tab, else the main frame
    let focused = FIELD.with_borrow(|f| f.as_ref().filter(|f| f.browser == id).map(|f| f.frame.clone()));
    let frame = focused.filter(|f| f.is_valid() != 0).or_else(|| browser.main_frame()).ok_or("no page")?;
    if !same_site_as_top(&browser, &frame) {
        return Err("login form is in a cross-site frame".into());
    }
    let kind = if otp { Kind::Otp } else { Kind::Password };
    let seq = GEN.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    let url = frame_url(&frame);
    let rect = FIELD.with_borrow(|f| f.as_ref().filter(|f| f.browser == id).map(|f| f.rect.clone())).unwrap_or(Rect { x: 24, y: 0, width: 300, height: 8 });
    FIELD.set(Some(Field { seq, browser: id, frame, kind, url: url.clone(), rect, items: Vec::new(), sel: None }));
    std::thread::spawn(move || {
        let r = matching(b, &url);
        crate::on_ui(move || match r {
            Ok(v) if v.is_empty() => window::message("info", &format!("no {} login for {}", label(b), host(&url))),
            Ok(v) if v.len() == 1 => {
                FIELD.with_borrow_mut(|f| {
                    if let Some(f) = f.as_mut().filter(|f| f.seq == seq) {
                        f.items = vec![Item::Entry(v[0].clone())];
                    }
                });
                pick_item(0);
            }
            Ok(v) => {
                show(seq, v);
                FIELD.with_borrow_mut(|f| {
                    if let Some(f) = f.as_mut() {
                        f.sel = Some(0);
                        let (r, j) = rows(f);
                        window::suggest(r, &j);
                    }
                });
            }
            Err(e) => window::message("error", &e),
        });
    });
    Ok(())
}

/// A form with a password was submitted (renderer): offer to save or update.
/// `json` = {"user": "...", "pass": "..."}; no "pass" = first page of a split login.
pub fn on_submit(browser: &Browser, frame: &Frame, json: &str) {
    let Some(b) = backend() else { return };
    if !same_site_as_top(browser, frame) {
        return;
    }
    let Ok(v) = serde_json::from_str::<Value>(json) else { return };
    let url = frame_url(frame);
    let Some(st) = site(&url) else { return };
    let id = browser.identifier();
    let mut user = s(&v["user"]);
    let pass = s(&v["pass"]);
    if pass.is_empty() {
        if !user.is_empty() {
            LAST_USER.with_borrow_mut(|l| {
                l.retain(|(b, s, _)| !(*b == id && *s == st));
                l.push((id, st, user));
            });
        }
        return;
    }
    if user.is_empty() {
        user = LAST_USER.with_borrow(|l| l.iter().rev().find(|(b, s, _)| *b == id && *s == st).map(|x| x.2.clone())).unwrap_or_default();
    }
    let repeat = LAST_SUBMIT.with_borrow(|l| {
        l.as_ref().is_some_and(|(t, s, u, p)| t.elapsed() < Duration::from_secs(30) && *s == st && *u == user && *p == pass)
    });
    if repeat {
        return;
    }
    LAST_SUBMIT.set(Some((Instant::now(), st, user.clone(), pass.clone())));
    let browser = browser.clone();
    std::thread::spawn(move || {
        // Known already? Compare against entries with this username (or all, if unknown).
        let r = matching(b, &url).and_then(|v| {
            let mut same_user = None;
            for e in v.iter().filter(|e| user.is_empty() || e.user == user) {
                let (_, p) = secret(b, e)?;
                if p == pass {
                    return Ok(None);
                }
                same_user.get_or_insert(e.clone());
            }
            Ok(Some(same_user))
        });
        crate::on_ui(move || match r {
            Ok(None) => {}
            Ok(Some(existing)) => ask(&browser, b, &url, user, pass, existing),
            Err(e) => window::message("error", &e),
        });
    });
}

fn ask(browser: &Browser, b: Backend, url: &str, user: String, pass: String, existing: Option<Entry>) {
    let h = host(url);
    let who = if user.is_empty() { String::new() } else { format!(" for {user}") };
    let text = match &existing {
        Some(_) => format!("Update the {} password{who} on {h}? [y]es [n]o", label(b)),
        None => format!("Save this login{who} on {h} to {}? [y]es [n]o", label(b)),
    };
    let url = url.to_string();
    crate::permissions::ask(browser, text, move |yes| {
        if !yes {
            return;
        }
        let (url, user, pass, existing) = (url.clone(), user.clone(), pass.clone(), existing.clone());
        std::thread::spawn(move || {
            let r = match &existing {
                Some(e) => update(b, e, &pass).map(|_| "updated"),
                None => save(b, &url, &user, &pass).map(|_| "saved"),
            };
            crate::on_ui(move || match r {
                Ok(what) => window::message("info", &format!("login {what} in {}", label(b))),
                Err(e) => window::message("error", &e),
            });
        });
    });
}

/// Tab closed: forget its field and split-login usernames.
pub fn forget(browser: &Browser) {
    let id = browser.identifier();
    if FIELD.with_borrow(|f| f.as_ref().is_some_and(|f| f.browser == id)) {
        hide();
        FIELD.set(None);
    }
    LAST_USER.with_borrow_mut(|l| l.retain(|(b, ..)| *b != id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_matches_base_domain() {
        assert_eq!(site("https://login.github.com/x").as_deref(), Some("github.com"));
        assert_eq!(site("github.com").as_deref(), Some("github.com"));
        assert_eq!(site("accounts.google.co.uk:443/a").as_deref(), Some("google.co.uk"));
        assert_eq!(site("http://129.159.225.36").as_deref(), Some("129.159.225.36"));
        assert_eq!(site("androidapp://com.x"), None);
        assert_ne!(site("evilgithub.com"), site("github.com"));
    }

    #[test]
    fn generated_passwords_differ() {
        let (a, b) = (generate(), generate());
        assert_eq!(a.len(), 20);
        assert_ne!(a, b);
    }

    #[test]
    fn editor_puts_secret_first() {
        let p = std::env::temp_dir().join(format!("nova-pw-ed-{}", std::process::id()));
        std::fs::write(&p, "old\nnotes line\n").unwrap();
        // SAFETY: test-only env; tests in this module don't read these concurrently
        unsafe {
            std::env::set_var("NOVA_PW_SECRET", "s3cret");
            std::env::set_var("NOVA_PW_MODE", "replace");
        }
        assert_eq!(editor_main(p.to_str().unwrap()), 0);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "s3cret\nnotes line\n");
        unsafe { std::env::set_var("NOVA_PW_MODE", "insert") };
        std::fs::write(&p, "# help\n").unwrap();
        editor_main(p.to_str().unwrap());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "s3cret\n# help\n");
        std::fs::remove_file(p).unwrap();
    }
}
