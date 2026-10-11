//! Window layout (S2), the tab list, the HTML tab bar (also shows mode, keys,
//! messages, progress) and the floating command palette overlay.
//!
//! All state lives on the CEF UI thread. CEF calls that can call back into our
//! handlers (attach, focus, close) run after the state borrow is dropped.
use crate::client::{TabClient, UiClient};
use crate::modes::{self, Mode};
use crate::theme::{self, TitleFields};
use crate::{config, devtools, history};
use cef::*;
use std::cell::{Cell, RefCell};
const TABBAR_HTML: &str = include_str!("../ui/tabbar.html");
const PALETTE_HTML: &str = include_str!("../ui/palette.html");

struct Tab {
    view: BrowserView,
    url: String,
    title: String,
    progress: f64,
    loading: bool,
    /// Set while the crashed page shows; holds the URL `r` reloads.
    crashed: Option<String>,
    can_back: bool,
    can_fwd: bool,
    muted: bool,
    /// Dark-mode override sent to this tab: None = none sent, Some(None) = cleared.
    dark: Option<Option<bool>>,
}

#[derive(Default)]
struct State {
    window: Option<Window>,
    /// Holds content (+ the tab bar when it is on the left/right).
    mid: Option<Panel>,
    content: Option<Panel>,
    tabbar: Option<BrowserView>,
    /// Command line + completion, floating over the content (hidden outside command mode).
    palette: Option<(BrowserView, OverlayController)>,
    /// Palette shown as a thin strip (search: the page must stay visible).
    compact: bool,
    tabs: Vec<Tab>,
    active: usize,
    /// Previously active tab, for `tab-focus last`.
    last: usize,
    /// URLs of closed tabs, newest last, for `undo`.
    closed: Vec<String>,
    /// Status bar message (level, text) and a serial so old timers don't clear newer ones.
    message: Option<(String, String)>,
    /// Open permission prompt (permissions.rs); shown instead of the message until answered.
    prompt: Option<String>,
    message_id: u64,
    /// URL given on the command line (opened after the restored session).
    start_url: Option<String>,
    /// Closed tabs kept alive until CEF's DoClose: dropping the view earlier leaves
    /// a page with unload handlers running forever (no OnBeforeClose).
    closing: Vec<BrowserView>,
    /// Message to show once the window exists (config errors at startup).
    startup: Option<(String, String)>,
    /// Chromium was started with force-dark on (switches are fixed at startup).
    dark_global: bool,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State::default());
    /// Preferred size the tab bar view reports (S2: must be non-zero).
    static TABBAR_SIZE: Cell<(i32, i32)> = const { Cell::new((1, 24)) };
}

const KIND_FIXED: u8 = 0;
const KIND_TABBAR: u8 = 1;

/// JS string literal; everything but ASCII alphanumerics and space as \uXXXX,
/// so page titles/URLs can't break out of the script we inject them into.
pub fn js_str(v: &str) -> String {
    let mut out = String::with_capacity(v.len() + 2);
    out.push('"');
    for u in v.encode_utf16() {
        match u {
            0x20 | 0x30..=0x39 | 0x41..=0x5a | 0x61..=0x7a => out.push(u as u8 as char),
            _ => out.push_str(&format!("\\u{u:04x}")),
        }
    }
    out.push('"');
    out
}

pub fn data_url(html: &str) -> CefString {
    let b64 = CefString::from(&base64_encode(Some(html.as_bytes()))).to_string();
    CefString::from(format!("data:text/html;base64,{b64}").as_str())
}

fn browser_view(
    client: &mut Client,
    url: &str,
    kind: u8,
    settings: &BrowserSettings,
) -> BrowserView {
    browser_view_create(
        Some(client),
        Some(&CefString::from(url)),
        Some(settings),
        None,
        None,
        Some(&mut NovaBrowserViewDelegate::new(kind)),
    )
    .expect("browser_view_create")
}

/// Web page settings from `fonts.web.*` and `colors.webpage.bg` (fixed per browser at creation).
fn page_settings() -> BrowserSettings {
    let cfg = config::get();
    let s = |n: &str| CefString::from(cfg.str(n).as_str());
    let px = |n: &str| cfg.int(n) as i32;
    BrowserSettings {
        standard_font_family: s("fonts.web.family.standard"),
        fixed_font_family: s("fonts.web.family.fixed"),
        serif_font_family: s("fonts.web.family.serif"),
        sans_serif_font_family: s("fonts.web.family.sans_serif"),
        cursive_font_family: s("fonts.web.family.cursive"),
        fantasy_font_family: s("fonts.web.family.fantasy"),
        default_font_size: px("fonts.web.size.default"),
        default_fixed_font_size: px("fonts.web.size.default_fixed"),
        minimum_font_size: px("fonts.web.size.minimum"),
        minimum_logical_font_size: px("fonts.web.size.minimum_logical"),
        background_color: argb(&cfg.str("colors.webpage.bg")).unwrap_or(0),
        ..Default::default()
    }
}

/// `#rgb`/`#rrggbb`/`#aarrggbb` → CEF ARGB (opaque unless given). Named colours → None.
fn argb(c: &str) -> Option<u32> {
    let h = c.trim().strip_prefix('#')?;
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(match h.len() {
        3 => {
            0xff00_0000
                | ((v >> 8 & 0xf) * 0x11) << 16
                | ((v >> 4 & 0xf) * 0x11) << 8
                | ((v & 0xf) * 0x11)
        }
        6 => 0xff00_0000 | v,
        8 => v,
        _ => return None,
    })
}

fn browser_id(view: &BrowserView) -> Option<i32> {
    view.browser().map(|b| b.identifier())
}

pub fn create(start_url: Option<String>) {
    let cfg = config::get();
    let dark = cfg.bool("colors.webpage.darkmode.enabled")
        || cfg.any_pattern_value(
            "colors.webpage.darkmode.enabled",
            &serde_json::Value::Bool(true),
        );
    S.with_borrow_mut(|s| {
        s.start_url = start_url;
        s.dark_global = dark;
    });
    window_create_top_level(Some(&mut NovaWindowDelegate::new()));
}

fn add_tab(view: BrowserView, url: String, title: String, foreground: bool) {
    let n = S.with_borrow_mut(|s| {
        s.tabs.push(Tab {
            view,
            url,
            title,
            progress: 0.0,
            loading: false,
            crashed: None,
            can_back: false,
            can_fwd: false,
            muted: false,
            dark: None,
        });
        s.tabs.len() - 1
    });
    if foreground || n == 0 {
        show(n);
    } else {
        refresh();
    }
}

/// A background tab loads only when first shown (CEF creates the browser on
/// attach), which is also what makes session restore lazy.
pub fn open(url: &str, foreground: bool) {
    open_titled(url, String::new(), foreground);
}

fn open_titled(url: &str, title: String, foreground: bool) {
    let view = browser_view(&mut TabClient::new(), url, KIND_FIXED, &page_settings());
    add_tab(view, url.to_string(), title, foreground);
}

/// Current tabs in session.rs's shape; None before the window exists.
pub fn session_json() -> Option<serde_json::Value> {
    S.with_borrow(|s| {
        s.window.as_ref()?;
        let tabs = s.tabs.iter().map(|t| (t.crashed.clone().unwrap_or_else(|| t.url.clone()), t.title.clone())).collect();
        // shortcut: last 100 closed tabs survive a restart, enough for undo
        let closed = &s.closed[s.closed.len().saturating_sub(100)..];
        Some(crate::session::shape(tabs, s.active, closed))
    })
}

/// Open a saved session's tabs (lazily) and focus its active one; `clear` closes
/// the tabs that were open before.
pub fn restore(v: &serde_json::Value, clear: bool) {
    let before = S.with_borrow(|s| s.tabs.len());
    let tabs = v["tabs"].as_array().cloned().unwrap_or_default();
    for t in &tabs {
        let url = t["url"].as_str().unwrap_or("about:blank");
        open_titled(url, t["title"].as_str().unwrap_or("").to_string(), false);
    }
    let closed: Vec<String> = v["closed"].as_array().into_iter().flatten().filter_map(|u| u.as_str().map(str::to_string)).collect();
    S.with_borrow_mut(|s| {
        let mut c = closed;
        c.append(&mut s.closed);
        s.closed = c;
    });
    if !tabs.is_empty() {
        show(before + (v["active"].as_u64().unwrap_or(0) as usize).min(tabs.len() - 1));
    }
    if clear {
        for _ in 0..before {
            close(0);
        }
    }
}

/// Swap the active tab into the content panel (S2: state survives, old tab
/// is marked hidden so Chromium throttles it).
pub fn show(n: usize) {
    let Some((content, old, new)) = S.with_borrow_mut(|s| {
        let new = s.tabs.get(n)?.view.clone();
        let old = (s.active != n)
            .then(|| s.tabs.get(s.active).map(|t| t.view.clone()))
            .flatten();
        if old.is_some() {
            s.last = s.active;
        }
        s.active = n;
        Some((s.content.clone()?, old, new))
    }) else {
        return;
    };
    if let Some(old) = old {
        if let Some(h) = old.browser().and_then(|b| b.host()) {
            h.was_hidden(1);
        }
        content.remove_child_view(Some(&mut View::from(&old)));
    }
    if content.child_view_count() == 0 {
        content.add_child_view(Some(&mut View::from(&new)));
        content.layout();
    }
    // browser() is None until the view first joins a window.
    if let Some(h) = new.browser().and_then(|b| b.host()) {
        h.was_hidden(0);
    }
    // qutebrowser leaves insert/passthrough when the tab changes
    if modes::mode() != Mode::Command {
        modes::enter(Mode::Normal);
        focus_active();
    }
    refresh();
}

pub fn close(n: usize) {
    let Some((tab, was_active, left, content, window)) = S.with_borrow_mut(|s| {
        if n >= s.tabs.len() {
            return None;
        }
        let tab = s.tabs.remove(n);
        s.closed
            .push(tab.crashed.clone().unwrap_or_else(|| tab.url.clone()));
        s.last = s.last.min(s.tabs.len().saturating_sub(1));
        let was_active = n == s.active;
        if n < s.active || (was_active && s.active == s.tabs.len() && s.active > 0) {
            s.active -= 1;
        }
        Some((
            tab,
            was_active,
            s.tabs.len(),
            s.content.clone(),
            s.window.clone(),
        ))
    }) else {
        return;
    };
    if left == 0 {
        // qutebrowser tabs.last_close; the tab we removed goes back as a fresh one
        let cfg = config::get();
        let reopen = match cfg.str("tabs.last_close").as_str() {
            "close" => None,
            "blank" => Some("about:blank".to_string()),
            "startpage" => Some(
                cfg.list("url.start_pages")
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| "about:blank".into()),
            ),
            "default-page" => Some(cfg.str("url.default_page")),
            _ => {
                // "ignore": keep the tab
                S.with_borrow_mut(|s| {
                    s.closed.pop();
                    s.tabs.push(tab);
                    s.active = 0;
                });
                return;
            }
        };
        let Some(url) = reopen else {
            // Closing the window closes the attached browsers; quit follows in on_before_close.
            if let Some(w) = window {
                w.close();
            }
            return;
        };
        if let Some(c) = content {
            c.remove_child_view(Some(&mut View::from(&tab.view)));
        }
        open(&url, true);
        release_view(tab.view);
        return;
    }
    if was_active {
        if let Some(c) = content {
            c.remove_child_view(Some(&mut View::from(&tab.view)));
        }
        let active = S.with_borrow(|s| s.active);
        show(active);
    } else {
        refresh();
    }
    release_view(tab.view);
}

/// Close a removed tab's browser; its view lives in `closing` until DoClose. A tab
/// that was never shown has no browser and is dropped right away.
fn release_view(view: BrowserView) {
    let Some(h) = view.browser().and_then(|b| b.host()) else { return };
    S.with_borrow_mut(|s| s.closing.push(view));
    h.close_browser(1);
}

/// CEF reached DoClose for `browser`: drop its view so CEF destroys it.
pub fn release_closing(browser: &Browser) {
    let view = S.with_borrow_mut(|s| {
        let i = s.closing.iter().position(|v| {
            v.browser()
                .is_some_and(|b| b.is_same(Some(&mut browser.clone())) == 1)
        })?;
        Some(s.closing.remove(i))
    });
    drop(view);
}

/// Apply `f` to the tab owning `browser`, then redraw the bars.
fn update(browser: &Browser, f: impl FnOnce(&mut Tab)) {
    let id = browser.identifier();
    let found = S.with_borrow_mut(|s| {
        s.tabs
            .iter_mut()
            .find(|t| browser_id(&t.view) == Some(id))
            .map(f)
            .is_some()
    });
    if found {
        refresh();
    }
}

pub fn set_url(browser: &Browser, url: String) {
    update(browser, |t| {
        if t.crashed.is_none() && t.url != url {
            history::visit(&url);
            t.url = url
        }
    });
    apply_dark(browser);
}

/// Per-site dark mode (S5): `pattern()` values of colors.webpage.darkmode.enabled
/// override Chromium's global force-dark for this tab; the override survives navigation.
fn apply_dark(browser: &Browser) {
    let id = browser.identifier();
    let cfg = config::get();
    let send = S.with_borrow_mut(|s| {
        let global = s.dark_global;
        let t = s
            .tabs
            .iter_mut()
            .find(|t| browser_id(&t.view) == Some(id))?;
        let want = cfg
            .get_for("colors.webpage.darkmode.enabled", &t.url)
            .as_bool()
            .unwrap_or(false);
        let over = (want != global).then_some(want);
        // Resend overrides every time: one sent before the renderer exists is lost
        // (first page of a new tab). Clear only once.
        let prev = t.dark.replace(over);
        (over.is_some() || matches!(prev, Some(Some(_)))).then_some(over)
    });
    match send {
        Some(Some(on)) => devtools::send(
            browser,
            "Emulation.setAutoDarkModeOverride",
            serde_json::json!({ "enabled": on }),
        ),
        Some(None) => devtools::send(
            browser,
            "Emulation.setAutoDarkModeOverride",
            serde_json::json!({}),
        ),
        None => {}
    }
}
pub fn set_title(browser: &Browser, title: String) {
    update(browser, |t| {
        if t.crashed.is_none() {
            history::title(&t.url, &title);
            t.title = title
        }
    });
}
pub fn set_progress(browser: &Browser, p: f64) {
    update(browser, |t| t.progress = p);
}
pub fn set_loading(browser: &Browser, loading: bool, back: bool, fwd: bool) {
    update(browser, |t| {
        t.loading = loading;
        t.can_back = back;
        t.can_fwd = fwd;
    });
    apply_dark(browser);
}

pub fn set_muted(browser: &Browser, muted: bool) {
    update(browser, |t| t.muted = muted);
}

/// Renderer died: keep the tab, show a local page with the URL (PLAN.md §7).
pub fn crashed(browser: &Browser, why: &str) {
    let mut url = String::new();
    update(browser, |t| {
        url = t.url.clone();
        t.crashed = Some(t.url.clone());
        t.loading = false;
    });
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>crashed</title>\
         <body style='font:16px sans-serif;background:#222;color:#ddd;padding:2em'>\
         <h2>This tab crashed ({why})</h2><p id=u style='font-family:monospace'></p>\
         <p>Press <b>r</b> to reload.</p>\
         <script>document.getElementById('u').textContent = {};</script>",
        js_str(&url)
    );
    if let Some(f) = browser.main_frame() {
        f.load_url(Some(&data_url(&html)));
    }
}

/// `r` on a crashed tab: load its URL again. Returns whether it was crashed.
pub fn reload_crashed(browser: &Browser) -> bool {
    let mut url = None;
    update(browser, |t| url = t.crashed.take());
    let Some(url) = url else { return false };
    if let Some(f) = browser.main_frame() {
        f.load_url(Some(&CefString::from(url.as_str())));
    }
    true
}

fn fields(i: usize, t: &Tab) -> TitleFields<'_> {
    TitleFields {
        index: i + 1,
        title: &t.title,
        url: &t.url,
        progress: t.loading.then_some(t.progress),
        muted: t.muted,
    }
}

/// Push the current state into the tab bar, status bar and window title.
pub fn refresh() {
    let cfg = config::get();
    let tab_fmt = cfg.str("tabs.title.format");
    let Some((tabbar, window, bar_json, title)) = S.with_borrow(|s| {
        let tabs: Vec<String> = s
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let label = theme::format_title(&tab_fmt, &fields(i, t));
                format!(
                    "{{\"t\":{},\"a\":{},\"l\":{},\"p\":{},\"e\":{}}}",
                    js_str(&label),
                    i == s.active,
                    t.loading,
                    t.progress,
                    t.crashed.is_some()
                )
            })
            .collect();
        let t = s.tabs.get(s.active)?;
        let (level, msg) = match &s.prompt {
            Some(p) => ("prompt".to_string(), p.clone()),
            None => s.message.clone().unwrap_or_default(),
        };
        let status = format!(
            "{{\"url\":{},\"p\":{},\"loading\":{},\"crashed\":{},\"mode\":{},\"cmd\":{},\"keys\":{},\"level\":{},\"msg\":{},\"tab\":{},\"hist\":{},\"modename\":{}}}",
            js_str(&t.url),
            t.progress,
            t.loading,
            t.crashed.is_some(),
            js_str(modes::mode_label()),
            modes::mode() == Mode::Command,
            js_str(&if modes::mode() == Mode::Hint { crate::hints::typed() } else { modes::pending() }),
            js_str(&level),
            js_str(&msg),
            js_str(&format!("[{}/{}]", s.active + 1, s.tabs.len())),
            js_str(match (t.can_back, t.can_fwd) {
                (true, true) => "[<>]",
                (true, false) => "[<]",
                (false, true) => "[>]",
                _ => "",
            }),
            js_str(modes::mode_name()),
        );
        let title = theme::format_title(&cfg.str("window.title_format"), &fields(s.active, t));
        Some((
            s.tabbar.clone()?,
            s.window.clone()?,
            format!("{{\"tabs\":[{}],\"status\":{status}}}", tabs.join(",")),
            title,
        ))
    }) else {
        return;
    };
    crate::session::dirty();
    if let Some(f) = tabbar.browser().and_then(|b| b.main_frame()) {
        let js = format!("window.nova && nova.apply && nova.apply({bar_json})");
        f.execute_java_script(Some(&CefString::from(js.as_str())), None, 0);
    }
    window.set_title(Some(&CefString::from(title.as_str())));
}

// ---- queries and actions for commands.rs

thread_local! {
    /// Tab a userscript's FIFO line runs in (see run_in_tab); None = the active tab.
    static TARGET: Cell<Option<i32>> = const { Cell::new(None) };
}

pub fn active_browser() -> Option<Browser> {
    S.with_borrow(|s| {
        let i = match TARGET.get() {
            Some(id) => s
                .tabs
                .iter()
                .position(|t| browser_id(&t.view) == Some(id))?,
            None => s.active,
        };
        s.tabs.get(i)?.view.browser()
    })
}

/// Run `f` with commands aimed at the tab whose browser id is `id` instead of the
/// active one (userscripts act on the tab that started them). False if it's gone.
pub fn run_in_tab(id: i32, f: impl FnOnce()) -> bool {
    if !S.with_borrow(|s| s.tabs.iter().any(|t| browser_id(&t.view) == Some(id))) {
        return false;
    }
    TARGET.set(Some(id));
    f();
    TARGET.set(None);
    true
}

/// `--query state`: tabs, active index, mode (for scripts and the smoke test).
pub fn state_json() -> String {
    S.with_borrow(|s| {
        serde_json::json!({
            "tabs": s.tabs.iter().map(|t| serde_json::json!({"url": t.url, "title": t.title})).collect::<Vec<_>>(),
            "active": s.active,
            "url": s.tabs.get(s.active).map(|t| t.url.as_str()),
            "mode": modes::mode_name(),
        })
        .to_string()
    })
}

pub fn active_url_title() -> (String, String) {
    S.with_borrow(|s| {
        let i = match TARGET.get() {
            Some(id) => s
                .tabs
                .iter()
                .position(|t| browser_id(&t.view) == Some(id))
                .unwrap_or(usize::MAX),
            None => s.active,
        };
        s.tabs
            .get(i)
            .map(|t| (t.url.clone(), t.title.clone()))
            .unwrap_or_default()
    })
}

pub fn content_size() -> (f64, f64) {
    S.with_borrow(|s| {
        s.content
            .as_ref()
            .map(|c| c.bounds())
            .map(|r| (r.width as f64, r.height as f64))
            .unwrap_or((0.0, 0.0))
    })
}

pub fn close_active() {
    let n = S.with_borrow(|s| s.active);
    close(n);
}

pub fn close_others() {
    loop {
        let n = S.with_borrow(|s| (s.tabs.len() > 1).then_some(if s.active == 0 { 1 } else { 0 }));
        let Some(n) = n else { break };
        close(n);
    }
}

/// First of `url.start_pages`.
pub fn start_page() -> String {
    config::get().list("url.start_pages").into_iter().next().unwrap_or_else(|| "about:blank".into())
}

pub fn close_window() {
    if let Some(w) = S.with_borrow(|s| s.window.clone()) {
        w.close();
    }
}

/// `tab-next`/`tab-prev`, wrapping (qutebrowser `tabs.wrap`).
pub fn cycle(by: i64) {
    let n =
        S.with_borrow(|s| (s.active as i64 + by).rem_euclid(s.tabs.len().max(1) as i64) as usize);
    show(n);
}

pub fn focus_last() {
    let n = S.with_borrow(|s| s.last);
    show(n);
}

/// 1-based; negative counts from the end (qutebrowser `tab-focus`).
pub fn focus_index(i: i64) {
    let len = S.with_borrow(|s| s.tabs.len()) as i64;
    let n = if i < 0 { len + i } else { i - 1 };
    if (0..len).contains(&n) {
        show(n as usize)
    } else {
        message("error", &format!("There's no tab with index {i}!"))
    }
}

/// `tab-move`: relative (`+`/`-` with count) or to a 1-based index (negative from the end).
pub fn move_active(i: i64, relative: bool) {
    S.with_borrow_mut(|s| {
        let len = s.tabs.len() as i64;
        let to = if relative {
            s.active as i64 + i
        } else if i < 0 {
            len + i
        } else {
            i - 1
        };
        let to = to.clamp(0, len - 1) as usize;
        let tab = s.tabs.remove(s.active);
        s.tabs.insert(to, tab);
        s.active = to;
    });
    refresh();
}

pub fn undo_close() {
    match S.with_borrow_mut(|s| s.closed.pop()) {
        Some(url) => open(&url, true),
        None => message("error", "Nothing to undo!"),
    }
}

wrap_task! {
    struct ClearMessage { id: u64 }
    impl Task {
        fn execute(&self) {
            let cleared = S.with_borrow_mut(|s| s.message_id == self.id && s.message.take().is_some());
            if cleared { refresh(); }
        }
    }
}

/// Show a status bar message for a few seconds (`info`, `warning`, `error`).
pub fn set_prompt(text: Option<String>) {
    S.with_borrow_mut(|s| s.prompt = text);
    refresh();
}

pub fn message(level: &str, text: &str) {
    eprintln!("novabrowser: {level}: {text}");
    let id = S.with_borrow_mut(|s| {
        s.message = Some((level.to_string(), text.to_string()));
        s.message_id += 1;
        s.message_id
    });
    refresh();
    let ms = config::get().int("messages.timeout");
    if ms > 0 {
        post_delayed_task(ThreadId::UI, Some(&mut ClearMessage::new(id)), ms);
    }
}

/// Show `text` once the window is up (startup config errors).
pub fn startup_message(level: &str, text: &str) {
    S.with_borrow_mut(|s| s.startup = Some((level.to_string(), text.to_string())));
}

fn palette_js(js: &str) {
    let view = S.with_borrow(|s| s.palette.as_ref().map(|p| p.0.clone()));
    if let Some(f) = view.and_then(|v| v.browser()).and_then(|b| b.main_frame()) {
        f.execute_java_script(Some(&CefString::from(js)), None, 0);
    }
}

/// `cmd-set-text`: open the floating palette with `text`. Search (`/`, `?`) gets a
/// thin strip so the matches stay visible; everything else floats over a blurred
/// screenshot of the page (CEF views can't be transparent).
pub fn open_cmdline(text: &str) {
    modes::enter(Mode::Command);
    let compact = text.starts_with('/') || text.starts_with('?');
    let Some((view, overlay, page)) = S.with_borrow_mut(|s| {
        s.compact = compact;
        let (v, o) = s.palette.clone()?;
        Some((v, o, s.tabs.get(s.active).and_then(|t| t.view.browser())))
    }) else {
        return;
    };
    place_palette();
    overlay.set_visible(1);
    // A just-shown overlay ignores focus requests; focus on the next UI task.
    let _ = view;
    post_task(ThreadId::UI, Some(&mut FocusPalette::new()));
    palette_js(&format!(
        "nova.backdrop(null); nova.cmdline({}, {compact})",
        js_str(text)
    ));
    if let (false, Some(page)) = (compact, page) {
        let params =
            serde_json::json!({ "format": "jpeg", "quality": 70, "optimizeForSpeed": true });
        devtools::call(&page, "Page.captureScreenshot", params, |r| {
            let data = r.as_ref().and_then(|r| r["data"].as_str());
            if let (Some(data), Mode::Command) = (data, modes::mode()) {
                palette_js(&format!("nova.backdrop(\"data:image/jpeg;base64,{data}\")"));
            }
        });
    }
}

wrap_task! {
    struct FocusPalette {}
    impl Task {
        fn execute(&self) {
            let view = S.with_borrow(|s| s.palette.as_ref().map(|p| p.0.clone()));
            if let (Some(v), Mode::Command) = (view, modes::mode()) {
                v.request_focus();
            }
        }
    }
}

/// Leave command mode and give focus back to the page.
pub fn close_cmdline() {
    modes::enter(Mode::Normal);
    crate::completion::reset();
    if let Some((_, o)) = S.with_borrow(|s| s.palette.clone()) {
        o.set_visible(0);
    }
    palette_js("nova.backdrop(null)");
    focus_active();
    refresh();
}

/// Replace the command line text (completion, history) without leaving it.
pub fn set_cmdline_text(text: &str) {
    palette_js(&format!("nova.setText({})", js_str(text)));
}

/// Show completion rows `json` (see ui/palette.html) under the input, or clear them.
pub fn show_completion(json: Option<&str>) {
    palette_js(&format!("nova.complete({})", json.unwrap_or("null")));
}

/// Full content area, or a top strip one input tall when compact.
fn place_palette() {
    let Some((overlay, mid, content, compact)) = S.with_borrow(|s| {
        Some((
            s.palette.as_ref()?.1.clone(),
            s.mid.clone()?,
            s.content.clone()?,
            s.compact,
        ))
    }) else {
        return;
    };
    // Overlay bounds are window coordinates; content sits in mid, mid in the window.
    let (m, c) = (mid.bounds(), content.bounds());
    let r = Rect {
        x: m.x + c.x,
        y: m.y + c.y,
        ..c
    };
    let cfg = config::get();
    let h = if compact {
        theme::bar_height(&cfg, "fonts.statusbar", "statusbar.padding") + 16
    } else {
        r.height
    };
    overlay.set_bounds(Some(&Rect {
        height: h.clamp(1, r.height.max(1)),
        ..r
    }));
}

wrap_task! {
    struct Relayout {}
    impl Task {
        fn execute(&self) { apply_layout(); }
    }
}

wrap_task! {
    struct FocusActive {}
    impl Task {
        fn execute(&self) {
            // A page with an open JS dialog ignores input, so prompts take keys in the tab bar.
            let v = S.with_borrow(|s| if modes::mode() == Mode::Prompt { s.tabbar.clone() } else { s.tabs.get(s.active).map(|t| t.view.clone()) });
            // CEF's focus task derefs the view's widget unchecked: only focus attached views.
            if let Some(v) = v.filter(|v| v.window().is_some()) {
                if modes::mode() != Mode::Command { v.request_focus(); }
            }
        }
    }
}

/// Focus whichever tab is active once the current command finishes; focusing a
/// view that gets detached before CEF's async focus runs is a SIGSEGV in CEF.
pub fn focus_active() {
    post_task(ThreadId::UI, Some(&mut FocusActive::new()));
}

/// A popup (target=_blank, window.open) arrives as a ready BrowserView: make it a tab.
fn adopt_popup(view: BrowserView) {
    let url = view
        .browser()
        .and_then(|b| b.main_frame())
        .map(|f| CefString::from(&f.url()).to_string())
        .unwrap_or_default();
    let foreground = !crate::client::take_background_popup();
    add_tab(view, url, String::new(), foreground);
}

// ---- layout and theme from config

/// Tab bar side and size from `tabs.position` / `tabs.width` / fonts + paddings.
fn bar_sizes(window_width: i32) -> (String, (i32, i32)) {
    let cfg = config::get();
    let pos = cfg.str("tabs.position");
    let tab_h = theme::bar_height(&cfg, "fonts.tabs.selected", "tabs.padding");
    let size = if pos == "left" || pos == "right" {
        let w = match cfg.get("tabs.width") {
            serde_json::Value::String(p) => p
                .trim_end_matches('%')
                .parse::<f64>()
                .map_or(200, |p| (window_width as f64 * p / 100.0) as i32),
            v => v.as_i64().unwrap_or(200) as i32,
        };
        (w.max(1), 1)
    } else {
        (1, tab_h)
    };
    (pos, size)
}

/// Put the tab bar where `tabs.position` says and size the bars; called at window
/// creation and when the config changes. Views are re-parented, not recreated.
fn apply_layout() {
    let Some((window, mid, content, tabbar)) = S.with_borrow(|s| {
        Some((
            s.window.clone()?,
            s.mid.clone()?,
            s.content.clone()?,
            s.tabbar.clone()?,
        ))
    }) else {
        return;
    };
    let (pos, size) = bar_sizes(window.bounds().width);
    TABBAR_SIZE.set(size);
    let (tb, m) = (View::from(&tabbar), View::from(&mid));
    for v in [&tb, &m] {
        if let Some(p) = v.parent_view() {
            if let Some(p) = p.as_panel() {
                p.remove_child_view(Some(&mut v.clone()))
            }
        }
    }
    let order: Vec<&View> = match pos.as_str() {
        "bottom" => vec![&m, &tb],
        "left" | "right" => vec![&m],
        _ => vec![&tb, &m],
    };
    for v in order {
        window.add_child_view(Some(&mut v.clone()));
    }
    let wl = window.get_layout().and_then(|l| l.as_box_layout());
    if let Some(l) = wl {
        l.set_flex_for_view(Some(&mut m.clone()), 1);
    }
    // mid: [tabbar] content [tabbar], horizontal
    let cv = View::from(&content);
    if cv.parent_view().is_none() {
        mid.add_child_view(Some(&mut cv.clone()));
    }
    match pos.as_str() {
        "left" => mid.add_child_view_at(Some(&mut tb.clone()), 0),
        "right" => mid.add_child_view(Some(&mut tb.clone())),
        _ => {}
    }
    if let Some(l) = mid.get_layout().and_then(|l| l.as_box_layout()) {
        l.set_flex_for_view(Some(&mut cv.clone()), 1);
    }
    let show = match config::get().str("tabs.show").as_str() {
        "never" => false,
        // shortcut: "switching" (show briefly on tab switch) behaves like "multiple"
        "multiple" | "switching" => S.with_borrow(|s| s.tabs.len() > 1),
        _ => true,
    };
    tabbar.set_visible(show as i32);
    tb.invalidate_layout();
    window.layout();
    place_palette();
    push_theme();
    focus_active(); // re-parenting moves focus off the page
}

/// Send the CSS variables and layout facts to every UI page.
fn push_theme() {
    let cfg = config::get();
    let vertical = matches!(cfg.str("tabs.position").as_str(), "left" | "right");
    let json = serde_json::json!({
        "vars": theme::vars(&cfg),
        "vertical": vertical,
        "listHeight": match cfg.get("completion.height") {
            serde_json::Value::String(p) if p.ends_with('%') => format!("{}vh", p.trim_end_matches('%')),
            v => format!("{}px", v.as_i64().unwrap_or(300)),
        },
    })
    .to_string();
    let views = S.with_borrow(|s| [s.tabbar.clone(), s.palette.as_ref().map(|c| c.0.clone())]);
    for v in views.into_iter().flatten() {
        if let Some(f) = v.browser().and_then(|b| b.main_frame()) {
            let js = format!("window.nova && nova.theme && nova.theme({json})");
            f.execute_java_script(Some(&CefString::from(js.as_str())), None, 0);
        }
    }
}

/// A UI page finished loading: give it the theme and the current state.
pub fn ui_ready() {
    push_theme();
    refresh();
}

/// After `:config-source` / `:set`: rebuild keys, layout, theme, per-site dark mode.
pub fn config_changed() {
    modes::init();
    crate::net::apply_cookie_policy();
    apply_layout();
    let browsers: Vec<Browser> =
        S.with_borrow(|s| s.tabs.iter().filter_map(|t| t.view.browser()).collect());
    for b in &browsers {
        apply_dark(b);
    }
    refresh();
}

wrap_browser_view_delegate! {
    struct NovaBrowserViewDelegate { kind: u8 }
    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            // Width and height must be non-zero, else CEF uses the whole window (S2).
            let (width, height) = match self.kind {
                KIND_TABBAR => TABBAR_SIZE.get(),
                _ => (1, 1),
            };
            Size { width, height }
        }
    }
    impl BrowserViewDelegate {
        fn delegate_for_popup_browser_view(&self, _view: Option<&mut BrowserView>, _settings: Option<&BrowserSettings>, _client: Option<&mut Client>, _is_devtools: ::std::os::raw::c_int) -> Option<BrowserViewDelegate> {
            Some(NovaBrowserViewDelegate::new(KIND_FIXED))
        }
        fn on_popup_browser_view_created(&self, _view: Option<&mut BrowserView>, popup: Option<&mut BrowserView>, _is_devtools: ::std::os::raw::c_int) -> ::std::os::raw::c_int {
            let Some(popup) = popup else { return 0 };
            adopt_popup(popup.clone());
            1
        }
        fn browser_runtime_style(&self) -> RuntimeStyle { RuntimeStyle::ALLOY }
    }
}

wrap_window_delegate! {
    struct NovaWindowDelegate {}
    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size { Size { width: 1200, height: 800 } }
        fn on_layout_changed(&self, _view: Option<&mut View>, bounds: Option<&Rect>) {
            place_palette();
            // vertical tab bar width is a percentage of the window: re-layout on resize
            let w = bounds.map_or(0, |b| b.width);
            let (_, size) = bar_sizes(w);
            if size.1 == 1 && size != TABBAR_SIZE.get() {
                post_task(ThreadId::UI, Some(&mut Relayout::new()));
            }
        }
    }
    impl PanelDelegate {}
    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let window = window.unwrap().clone();
            let layout = window
                .set_to_box_layout(Some(&BoxLayoutSettings {
                    horizontal: 0,
                    cross_axis_alignment: AxisAlignment::STRETCH,
                    ..Default::default()
                }))
                .expect("box layout");
            let _ = layout;
            let ui = BrowserSettings::default();
            let tabbar = browser_view(&mut UiClient::new(), &data_url(TABBAR_HTML).to_string(), KIND_TABBAR, &ui);
            let mid = panel_create(None).expect("panel");
            mid.set_to_box_layout(Some(&BoxLayoutSettings {
                horizontal: 1,
                cross_axis_alignment: AxisAlignment::STRETCH,
                ..Default::default()
            }));
            let content = panel_create(None).expect("panel");
            content.set_to_fill_layout();
            // Added last so it stays on top (cef_window.h); positioned by place_palette.
            let palette = browser_view(&mut UiClient::new(), &data_url(PALETTE_HTML).to_string(), KIND_FIXED, &ui);
            let overlay = window
                .add_overlay_view(Some(&mut View::from(&palette)), DockingMode::CUSTOM, 1) // can_activate: takes keys
                .expect("palette overlay");
            let (start, startup) = S.with_borrow_mut(|s| {
                s.palette = Some((palette, overlay));
                s.window = Some(window.clone());
                s.mid = Some(mid);
                s.content = Some(content);
                s.tabbar = Some(tabbar);
                (s.start_url.clone(), s.startup.take())
            });
            apply_layout();
            window.show();
            match (crate::session::startup(), start) {
                (Some(v), url) => {
                    restore(&v, false);
                    if let Some(u) = url { open(&u, true) }
                }
                (None, url) => open(&url.unwrap_or_else(start_page), true),
            }
            apply_layout(); // tabs.show = multiple depends on the tab count
            if let Some((level, text)) = startup {
                message(&level, &text);
            }
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            crate::session::save_now();
            // Detached tabs aren't destroyed with the window; close them by hand (S2).
            // Their views stay in `closing` until DoClose (see close()).
            let hosts = S.with_borrow_mut(|s| {
                let active = s.active;
                let mut closing = std::mem::take(&mut s.closing);
                // Never-shown tabs have no browser and get no DoClose: drop them now, here,
                // not at thread exit after CEF shutdown (CHECK observers_.empty()).
                closing.extend(s.tabs.drain(..).enumerate().filter(|(i, t)| *i != active && t.view.browser().is_some()).map(|(_, t)| t.view));
                let hosts: Vec<_> = closing.iter().filter_map(|v| v.browser().and_then(|b| b.host())).collect();
                *s = State { closing, ..State::default() };
                hosts
            });
            for h in hosts { h.close_browser(1); }
        }

        // cef-rs defaults every int return to 0; CEF's C++ defaults for these are true.
        // shortcut: closing skips beforeunload prompts; add try_close_browser in Phase 8.
        fn can_close(&self, _window: Option<&mut Window>) -> ::std::os::raw::c_int { 1 }
        fn can_resize(&self, _window: Option<&mut Window>) -> ::std::os::raw::c_int { 1 }
        fn can_maximize(&self, _window: Option<&mut Window>) -> ::std::os::raw::c_int { 1 }
        fn can_minimize(&self, _window: Option<&mut Window>) -> ::std::os::raw::c_int { 1 }

        fn window_runtime_style(&self) -> RuntimeStyle { RuntimeStyle::ALLOY }
    }
}

#[cfg(test)]
mod tests {
    use super::js_str;

    #[test]
    fn js_str_escapes_everything_risky() {
        assert_eq!(js_str("ab 09"), "\"ab 09\"");
        assert_eq!(
            js_str("\"</script>"),
            "\"\\u0022\\u003c\\u002fscript\\u003e\""
        );
        assert_eq!(js_str("\\\n\u{2028}é"), "\"\\u005c\\u000a\\u2028\\u00e9\"");
        assert_eq!(js_str("😀"), "\"\\ud83d\\ude00\"");
    }
}
