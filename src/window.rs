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
const KEYHINT_HTML: &str = include_str!("../ui/keyhint.html");
const POPUP_HTML: &str = include_str!("../ui/popup.html");

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
    /// History from a restored session that CEF can't hold (it can't restore
    /// navigation entries): `back` oldest first, `fwd` nearest last. Used once
    /// the real history runs out, via `location.replace` (see `go`).
    back: Vec<String>,
    fwd: Vec<String>,
    /// Last reported scroll position (renderer.rs) and one to restore after load.
    scroll: i64,
    scroll_to: Option<i64>,
    /// Pinned tabs sit first, use `tabs.title.format_pinned` and need `--force` to close.
    pinned: bool,
    /// Hibernation: when the tab was last in front, whether the user exempted it
    /// (`tab-keep-awake`), and whether its page is unloaded right now.
    seen: std::time::Instant,
    keep: bool,
    asleep: bool,
    /// Favicon as a data: URL (client.rs FaviconDone), empty until the page reports one.
    icon: String,
}

/// Entries kept per direction in a saved session.
const HIST_MAX: usize = 50;

#[derive(Default)]
struct State {
    window: Option<Window>,
    /// Holds content (+ the tab bar when it is on the left/right).
    mid: Option<Panel>,
    content: Option<Panel>,
    tabbar: Option<BrowserView>,
    /// Command line + completion, floating over the content (hidden outside command mode).
    palette: Option<(BrowserView, OverlayController)>,
    keyhint: Option<(BrowserView, OverlayController)>,
    /// tab tooltip (can't take focus) and tab right-click menu (can)
    tip: Option<(BrowserView, OverlayController)>,
    menu: Option<(BrowserView, OverlayController)>,
    /// bumped on every keyhint change; a delayed show only applies if still current
    keyhint_gen: u64,
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
    /// Slot taken by a window (created or being created); slots are never reused.
    claimed: bool,
    /// Preferred size the tab bar view reports (S2: must be non-zero).
    tabbar_size: (i32, i32),
    /// Session part (one window's tabs) to open when this window is created.
    restore: Option<serde_json::Value>,
}

thread_local! {
    /// One slot per window ever opened (index = slot id, fixed for its lifetime).
    static WINS: RefCell<Vec<State>> = const { RefCell::new(Vec::new()) };
    /// Window the code below acts on: the focused one, or the one owning the
    /// browser an event came from (see in_owner).
    static CUR: Cell<usize> = const { Cell::new(0) };
    /// The first window was created (it restores the saved session).
    static STARTED: Cell<bool> = const { Cell::new(false) };
    /// `quit` is closing every window: don't autosave the shrinking session.
    static QUITTING: Cell<bool> = const { Cell::new(false) };
}

/// `S.with_borrow(..)`: the current window's state.
struct Cur;
static S: Cur = Cur;
impl Cur {
    fn with_borrow<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        WINS.with_borrow_mut(|w| {
            if w.is_empty() {
                w.push(State::default());
            }
        });
        WINS.with_borrow(|w| f(&w[CUR.get()]))
    }
    fn with_borrow_mut<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        WINS.with_borrow_mut(|w| {
            if w.is_empty() {
                w.push(State::default());
            }
            f(&mut w[CUR.get()])
        })
    }
}

/// Slot of the window owning browser `id` (a tab, a closing tab or one of its UI pages).
fn owner(id: i32) -> Option<usize> {
    let has = |v: &Option<BrowserView>| v.as_ref().and_then(browser_id) == Some(id);
    let has2 = |v: &Option<(BrowserView, OverlayController)>| {
        v.as_ref().and_then(|p| browser_id(&p.0)) == Some(id)
    };
    WINS.with_borrow(|w| {
        w.iter().position(|s| {
            s.tabs.iter().any(|t| browser_id(&t.view) == Some(id))
                || s.closing.iter().any(|v| browser_id(v) == Some(id))
                || has(&s.tabbar)
                || has2(&s.palette)
                || has2(&s.keyhint)
                || has2(&s.tip)
                || has2(&s.menu)
        })
    })
}

/// Run `f` acting on `slot`'s window, then go back to the previous one.
fn in_slot<R>(slot: usize, f: impl FnOnce() -> R) -> R {
    let prev = CUR.replace(slot);
    let r = f();
    if WINS.with_borrow(|w| w.get(prev).is_some_and(|s| s.window.is_some())) {
        CUR.set(prev);
    }
    r
}

/// Run `f` acting on the window that owns `browser` (events from background windows).
pub fn in_owner<R>(browser: &Browser, f: impl FnOnce() -> R) -> R {
    match owner(browser.identifier()) {
        Some(slot) => in_slot(slot, f),
        None => f(),
    }
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
    create_window(start_url, None);
}

/// Open a new window (`open -w`, session restore); it opens `start_url` or `restore`'s tabs.
fn create_window(start_url: Option<String>, restore: Option<serde_json::Value>) {
    let cfg = config::get();
    let dark = cfg.bool("colors.webpage.darkmode.enabled")
        || cfg.any_pattern_value(
            "colors.webpage.darkmode.enabled",
            &serde_json::Value::Bool(true),
        );
    // the first window takes the slot startup messages were queued in
    let slot = WINS.with_borrow_mut(|w| {
        let slot = match w.iter().position(|s| !s.claimed) {
            Some(i) => i,
            None => {
                w.push(State::default());
                w.len() - 1
            }
        };
        let s = &mut w[slot];
        s.claimed = true;
        s.start_url = start_url;
        s.restore = restore;
        s.dark_global = dark;
        s.tabbar_size = (1, 24);
        slot
    });
    window_create_top_level(Some(&mut NovaWindowDelegate::new(slot)));
}

/// `open -w [url]`.
pub fn open_window(url: Option<String>) {
    create_window(
        Some(url.unwrap_or_else(|| config::get().str("url.default_page"))),
        None,
    );
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
            back: Vec::new(),
            fwd: Vec::new(),
            scroll: 0,
            scroll_to: None,
            pinned: false,
            seen: std::time::Instant::now(),
            keep: false,
            asleep: false,
            icon: String::new(),
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
/// Our tab-bar page's browser (used for clipboard writes).
pub fn tabbar_browser() -> Option<Browser> {
    S.with_borrow(|s| s.tabbar.as_ref()?.browser())
}

/// Every open window, in session.rs's shape.
pub fn session_json() -> Option<serde_json::Value> {
    let slots: Vec<usize> =
        WINS.with_borrow(|w| (0..w.len()).filter(|i| w[*i].window.is_some()).collect());
    let wins: Vec<serde_json::Value> = slots
        .iter()
        .filter_map(|i| in_slot(*i, window_json))
        .collect();
    if wins.is_empty() {
        return None;
    }
    let closed = S.with_borrow(|s| s.closed[s.closed.len().saturating_sub(100)..].to_vec());
    Some(crate::session::shape(wins, &closed))
}

fn window_json() -> Option<serde_json::Value> {
    S.with_borrow(|s| {
        s.window.as_ref()?;
        // data: tabs are our snapshot pages (downloads, bookmarks): stale after a restart
        let tabs = s
            .tabs
            .iter()
            .filter(|t| !t.url.starts_with("data:"))
            .map(tab_json)
            .collect();
        Some(crate::session::window(tabs, s.active))
    })
}

/// One tab in session.rs's shape: URL, title, back/forward URLs, scroll, pinned.
fn tab_json(t: &Tab) -> serde_json::Value {
    let (real, cur) = t
        .view
        .browser()
        .map(|b| nav_entries(&b))
        .unwrap_or_default();
    let mut back = t.back.clone();
    let mut fwd = Vec::new();
    if cur < real.len() {
        back.extend_from_slice(&real[..cur]);
        fwd.extend_from_slice(&real[cur + 1..]);
    }
    fwd.extend(t.fwd.iter().rev().cloned());
    back.retain(|u| !u.starts_with("data:"));
    fwd.retain(|u| !u.starts_with("data:"));
    let skip = back.len().saturating_sub(HIST_MAX);
    fwd.truncate(HIST_MAX);
    serde_json::json!({
        "url": t.crashed.clone().unwrap_or_else(|| t.url.clone()),
        "title": t.title,
        "back": back[skip..],
        "fwd": fwd,
        "scroll": t.scroll_to.unwrap_or(t.scroll),
        "pinned": t.pinned,
        "keep": t.keep,
        "icon": t.icon,
    })
}

fn strs(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|u| u.as_str().map(str::to_string))
        .collect()
}

/// Open a saved session: its first window's tabs go into this window (`clear`
/// closes the tabs that were there), every other window opens as a new window.
pub fn restore(v: &serde_json::Value, clear: bool) {
    // sessions before multi-window: {tabs, active, closed} at the top level
    let wins = v["windows"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| vec![v.clone()]);
    let closed: Vec<String> = v["closed"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|u| u.as_str().map(str::to_string))
        .collect();
    S.with_borrow_mut(|s| {
        let mut c = closed;
        c.append(&mut s.closed);
        s.closed = c;
    });
    for (i, w) in wins.iter().enumerate() {
        if i == 0 {
            restore_tabs(w, clear);
        } else {
            create_window(None, Some(w.clone()));
        }
    }
}

/// One window's tabs (lazily), focusing its active one.
fn restore_tabs(v: &serde_json::Value, clear: bool) {
    let before = S.with_borrow(|s| s.tabs.len());
    let tabs = v["tabs"].as_array().cloned().unwrap_or_default();
    for t in &tabs {
        let url = t["url"].as_str().unwrap_or("about:blank");
        open_titled(url, t["title"].as_str().unwrap_or("").to_string(), false);
        S.with_borrow_mut(|s| {
            let tab = s.tabs.last_mut().unwrap();
            tab.back = strs(&t["back"]);
            tab.fwd = strs(&t["fwd"]).into_iter().rev().collect();
            tab.scroll_to = t["scroll"].as_i64().filter(|y| *y > 0);
            tab.pinned = t["pinned"].as_bool().unwrap_or(false);
            tab.keep = t["keep"].as_bool().unwrap_or(false);
            tab.icon = t["icon"].as_str().unwrap_or("").to_string();
        });
    }
    if !tabs.is_empty() {
        show(before + (v["active"].as_u64().unwrap_or(0) as usize).min(tabs.len() - 1));
    }
    if clear {
        for _ in 0..before {
            remove(0);
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
            s.tabs[s.active].seen = std::time::Instant::now();
        }
        s.active = n;
        s.tabs[n].asleep = false;
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

/// `tab-close` / `d` / middle click: ask the page first (beforeunload, answered in
/// the tab bar prompt); the tab goes away in `did_close` once CEF reaches DoClose.
/// Never-shown and crashed tabs have nothing to ask and close at once.
pub fn close(n: usize) {
    let host = S.with_borrow(|s| {
        let t = s.tabs.get(n)?;
        t.crashed
            .is_none()
            .then(|| t.view.browser()?.host())
            .flatten()
    });
    match host {
        Some(h) => h.close_browser(0),
        None => remove(n),
    }
}

thread_local! {
    /// Set while `did_close` removes a tab whose browser already passed DoClose.
    static CLOSED: Cell<bool> = const { Cell::new(false) };
}

/// CEF reached DoClose for `browser` (client.rs posts this): drop a tab that asked
/// via `close`, or release a view `remove` already took out.
pub fn did_close(browser: &Browser) {
    in_owner(browser, || did_close_here(browser));
}

fn did_close_here(browser: &Browser) {
    let id = browser.identifier();
    let n = S.with_borrow(|s| s.tabs.iter().position(|t| browser_id(&t.view) == Some(id)));
    match n {
        Some(n) => {
            CLOSED.set(true);
            remove(n);
            CLOSED.set(false);
        }
        None => release_closing(browser),
    }
}

/// Take tab `n` out now and force-close its browser (no beforeunload).
pub fn remove(n: usize) {
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
    if CLOSED.get() {
        return; // DoClose already ran: dropping the view destroys the browser
    }
    let Some(h) = view.browser().and_then(|b| b.host()) else {
        return;
    };
    S.with_borrow_mut(|s| s.closing.push(view));
    h.close_browser(1);
}

/// CEF reached DoClose for `browser`: drop its view so CEF destroys it.
fn release_closing(browser: &Browser) {
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
    in_owner(browser, || {
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
    });
}

pub fn set_url(browser: &Browser, url: String) {
    update(browser, |t| {
        if t.crashed.is_none() && t.url != url {
            history::visit(&url);
            t.url = url;
            t.scroll = 0;
        }
    });
    apply_dark(browser);
}

/// Per-site dark mode (S5): `pattern()` values of colors.webpage.darkmode.enabled
/// override Chromium's global force-dark for this tab; the override survives navigation.
fn apply_dark(browser: &Browser) {
    let id = browser.identifier();
    let cfg = config::get();
    let send = in_owner(browser, || {
        S.with_borrow_mut(|s| {
            let global = s.dark_global;
            let t = s
                .tabs
                .iter_mut()
                .find(|t| browser_id(&t.view) == Some(id))?;
            // our own data: pages follow the UI theme, never force-dark
            let want = !t.url.starts_with("data:")
                && cfg
                    .get_for("colors.webpage.darkmode.enabled", &t.url)
                    .as_bool()
                    .unwrap_or(false);
            let over = (want != global).then_some(want);
            // Resend overrides every time: one sent before the renderer exists is lost
            // (first page of a new tab). Clear only once.
            let prev = t.dark.replace(over);
            (over.is_some() || matches!(prev, Some(Some(_)))).then_some(over)
        })
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
pub fn set_favicon(browser: &Browser, icon: String) {
    update(browser, |t| t.icon = icon);
}
pub fn set_progress(browser: &Browser, p: f64) {
    update(browser, |t| t.progress = p);
}
pub fn set_loading(browser: &Browser, loading: bool, back: bool, fwd: bool) {
    let mut scroll = None;
    update(browser, |t| {
        t.loading = loading;
        t.can_back = back;
        t.can_fwd = fwd;
        // a new page on top of the restored one: its saved forward entries are gone
        if back {
            t.fwd.clear();
        }
        if !loading {
            scroll = t.scroll_to.take();
        }
    });
    if let (Some(y), Some(f)) = (scroll, browser.main_frame()) {
        // again after 1 s for pages that grow after load
        let js = format!("scrollTo(0,{y});setTimeout(()=>scrollY||scrollTo(0,{y}),1000)");
        f.execute_java_script(Some(&CefString::from(js.as_str())), None, 0);
    }
    apply_dark(browser);
}

pub fn set_scroll(browser: &Browser, y: i64) {
    let id = browser.identifier();
    in_owner(browser, || {
        S.with_borrow_mut(|s| {
            if let Some(t) = s.tabs.iter_mut().find(|t| browser_id(&t.view) == Some(id)) {
                t.scroll = y;
            }
        })
    });
    crate::session::dirty();
}

/// Real navigation entries and the current index (CEF visits synchronously).
fn nav_entries(b: &Browser) -> (Vec<String>, usize) {
    let out = std::rc::Rc::new(RefCell::new((Vec::new(), 0)));
    if let Some(h) = b.host() {
        h.navigation_entries(Some(&mut NavCollect::new(out.clone())), 0);
    }
    out.take()
}

wrap_navigation_entry_visitor! {
    struct NavCollect { out: std::rc::Rc<RefCell<(Vec<String>, usize)>> }
    impl NavigationEntryVisitor {
        fn visit(&self, entry: Option<&mut NavigationEntry>, current: ::std::os::raw::c_int, index: ::std::os::raw::c_int, _total: ::std::os::raw::c_int) -> ::std::os::raw::c_int {
            let url = entry.map(|e| CefString::from(&e.url()).to_string()).unwrap_or_default();
            let mut o = self.out.borrow_mut();
            o.0.push(url);
            if current != 0 { o.1 = index as usize; }
            1
        }
    }
}

/// `back`/`forward` (n < 0 = back): the page's own history first, then the
/// entries a restored session saved, swapped in with `location.replace` so
/// the real history doesn't grow.
pub fn go(browser: &Browser, n: i32) {
    let id = browser.identifier();
    let target = S.with_borrow_mut(|s| {
        let t = s
            .tabs
            .iter_mut()
            .find(|t| browser_id(&t.view) == Some(id))?;
        let real = if n < 0 { t.can_back } else { t.can_fwd };
        if real {
            return Some(Err(()));
        }
        let cur = t.url.clone();
        let (from, to) = if n < 0 {
            (&mut t.back, &mut t.fwd)
        } else {
            (&mut t.fwd, &mut t.back)
        };
        let k = (n.unsigned_abs() as usize).min(from.len());
        if k == 0 {
            return None;
        }
        to.push(cur);
        let mut moved = from.split_off(from.len() - k);
        let target = moved.remove(0);
        to.extend(moved.into_iter().rev());
        Some(Ok(target))
    });
    let js = match target {
        Some(Err(())) => format!("history.go({n})"),
        Some(Ok(url)) => format!("location.replace({})", js_str(&url)),
        None => {
            return message(
                "warning",
                if n < 0 {
                    "At beginning of history."
                } else {
                    "At end of history."
                },
            )
        }
    };
    if let Some(f) = browser.main_frame() {
        f.execute_java_script(Some(&CefString::from(js.as_str())), None, 0);
    }
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
    update_keyhint();
    let cfg = config::get();
    let tab_fmt = cfg.str("tabs.title.format");
    let pin_fmt = cfg.str("tabs.title.format_pinned");
    let Some((tabbar, window, bar_json, title)) = S.with_borrow(|s| {
        let tabs: Vec<String> = s
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let label = theme::format_title(if t.pinned { &pin_fmt } else { &tab_fmt }, &fields(i, t));
                format!(
                    // shortcut: icons (~1-2 KB each) are resent on every refresh; cache by id in the tabbar if it ever shows up in profiles
                    "{{\"t\":{},\"i\":{},\"a\":{},\"l\":{},\"p\":{},\"e\":{},\"n\":{},\"z\":{}}}",
                    js_str(&label),
                    js_str(&t.icon),
                    i == s.active,
                    t.loading,
                    t.progress,
                    t.crashed.is_some(),
                    t.pinned,
                    t.asleep
                )
            })
            .collect();
        let t = s.tabs.get(s.active)?;
        let (level, msg) = match &s.prompt {
            Some(p) => ("prompt".to_string(), p.clone()),
            None => s.message.clone().unwrap_or_default(),
        };
        let status = format!(
            "{{\"profile\":{},\"url\":{},\"p\":{},\"loading\":{},\"crashed\":{},\"mode\":{},\"cmd\":{},\"keys\":{},\"level\":{},\"msg\":{},\"tab\":{},\"hist\":{},\"modename\":{}}}",
            js_str(crate::profile().unwrap_or("")),
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
        let (dl, dl_class) = crate::downloads::summary();
        let status = format!("{},\"dl\":{},\"dlc\":{}}}", &status[..status.len() - 1], js_str(&dl), js_str(dl_class));
        let mut title = theme::format_title(&cfg.str("window.title_format"), &fields(s.active, t));
        if let Some(p) = crate::profile() {
            title = format!("[{p}] {title}");
        }
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
    let Some(slot) = owner(id) else { return false };
    if !in_slot(slot, || {
        S.with_borrow(|s| s.tabs.iter().any(|t| browser_id(&t.view) == Some(id)))
    }) {
        return false;
    }
    in_slot(slot, || {
        TARGET.set(Some(id));
        f();
        TARGET.set(None);
    });
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

pub fn close_active(force: bool) {
    let n = S.with_borrow(|s| s.active);
    close_user(n, force);
}

/// `d`, middle click, tab menu: pinned tabs only with `force` (qutebrowser).
pub fn close_user(n: usize, force: bool) {
    if !force && S.with_borrow(|s| s.tabs.get(n).is_some_and(|t| t.pinned)) {
        return message("error", "Tab is pinned! Unpin it or use tab-close --force.");
    }
    close(n);
}

/// `tab-pin [index]`: toggle; pinned tabs move to the end of the pinned group,
/// unpinned ones to just after it.
pub fn pin(n: Option<usize>) {
    S.with_borrow_mut(|s| {
        let n = n.unwrap_or(s.active);
        if n >= s.tabs.len() {
            return;
        }
        let active = browser_id_or_index(s, s.active);
        let mut tab = s.tabs.remove(n);
        tab.pinned = !tab.pinned;
        let to = s.tabs.iter().take_while(|t| t.pinned).count();
        s.tabs.insert(to, tab);
        // keep the same tab active
        s.active = match active {
            Ok(id) => s
                .tabs
                .iter()
                .position(|t| browser_id(&t.view) == Some(id))
                .unwrap_or(to),
            Err(i) if i == n => to,
            Err(i) => {
                let i = i - (i > n) as usize;
                i + (i >= to) as usize
            }
        };
    });
    refresh();
}

/// True if the page plays media or has typed form text: never hibernate it.
/// shortcut: main frame only (an embedded player in an iframe isn't seen).
const BUSY_JS: &str = "[...document.querySelectorAll('video,audio')].some(m=>!m.paused&&!m.ended)||\
[...document.querySelectorAll('input:not([type=hidden]),textarea')].some(e=>e.value!==e.defaultValue)";

wrap_task! {
    struct HibernateTick {}
    impl Task {
        fn execute(&self) {
            hibernate_idle();
            post_delayed_task(ThreadId::UI, Some(&mut HibernateTick::new()), 60_000);
        }
    }
}

/// Start the once-a-minute idle check (`tabs.hibernate_after`, minutes; 0 = off).
pub fn start_hibernation() {
    post_delayed_task(ThreadId::UI, Some(&mut HibernateTick::new()), 60_000);
}

fn hibernate_idle() {
    let cfg = config::get();
    // per site via pattern(): 0 there exempts the site, a number there overrides the global one
    let due = |t: &Tab| {
        let mins = cfg
            .get_for("tabs.hibernate_after", &t.url)
            .as_i64()
            .unwrap_or(0);
        mins > 0 && t.seen.elapsed() >= std::time::Duration::from_secs(mins as u64 * 60)
    };
    let candidates: Vec<Browser> = WINS.with_borrow(|w| {
        w.iter()
            .filter(|s| s.window.is_some())
            .flat_map(|s| {
                s.tabs.iter().enumerate().filter(move |(i, t)| {
                    *i != s.active
                        && !t.keep
                        && !t.asleep
                        && !t.loading
                        && t.crashed.is_none()
                        && !t.url.starts_with("data:")
                        && due(t)
                })
            })
            .filter_map(|(_, t)| t.view.browser())
            .collect()
    });
    for b in candidates {
        let b2 = b.clone();
        devtools::eval_isolated(&b, BUSY_JS.into(), move |r| {
            // no answer (hung, closed) = leave it alone
            if r.is_some_and(|(_, v)| v == serde_json::Value::Bool(false)) {
                in_owner(&b2, || hibernate_browser(b2.identifier(), false));
            }
        });
    }
}

/// `tab-hibernate [index]`: unload a background tab now (not the active one).
pub fn hibernate(n: Option<usize>) {
    let id = S.with_borrow(|s| {
        let n = n.unwrap_or(s.active);
        if n == s.active {
            return Err("Can't hibernate the tab you're looking at.");
        }
        let t = s.tabs.get(n).ok_or("No such tab.")?;
        browser_id(&t.view).ok_or("Tab is already unloaded.")
    });
    match id {
        Ok(id) => hibernate_browser(id, true),
        Err(e) => message("error", e),
    }
}

/// Replace the tab's page with a not-yet-loaded view of the same URL; history
/// and scroll come back like a restored session tab when it's shown again.
fn hibernate_browser(id: i32, manual: bool) {
    let find = |s: &State| s.tabs.iter().position(|t| browser_id(&t.view) == Some(id));
    let Some(url) = S.with_borrow(|s| {
        let n = find(s)?;
        let t = &s.tabs[n];
        // re-check: things may have changed while the page was asked
        (n != s.active && t.crashed.is_none() && (manual || !t.keep)).then(|| t.url.clone())
    }) else {
        return;
    };
    let view = browser_view(&mut TabClient::new(), &url, KIND_FIXED, &page_settings());
    let old = S.with_borrow_mut(|s| {
        let n = find(s)?;
        let tj = tab_json(&s.tabs[n]);
        let t = &mut s.tabs[n];
        let old = std::mem::replace(&mut t.view, view);
        t.back = strs(&tj["back"]);
        t.fwd = strs(&tj["fwd"]).into_iter().rev().collect();
        t.scroll_to = tj["scroll"].as_i64().filter(|y| *y > 0);
        t.asleep = true;
        t.loading = false;
        t.progress = 0.0;
        t.can_back = false;
        t.can_fwd = false;
        t.dark = None;
        Some(old)
    });
    if let Some(old) = old {
        release_view(old); // its on_before_close forgets devtools/passwords state
        refresh();
    }
}

/// `tab-keep-awake [index]`: toggle the tab's exemption from hibernation.
pub fn keep_awake(n: Option<usize>) {
    let on = S.with_borrow_mut(|s| {
        let n = n.unwrap_or(s.active);
        let t = s.tabs.get_mut(n)?;
        t.keep = !t.keep;
        Some(t.keep)
    });
    match on {
        Some(true) => message("info", "This tab won't be hibernated."),
        Some(false) => message("info", "This tab can be hibernated again."),
        None => message("error", "No such tab."),
    }
    refresh();
}

/// The tab's browser id, or its index for a tab that has no browser yet.
fn browser_id_or_index(s: &State, i: usize) -> Result<i32, usize> {
    s.tabs.get(i).and_then(|t| browser_id(&t.view)).ok_or(i)
}

/// `tab-give` / `gD`: move tab `n` (URL, back/forward, scroll) into a new window.
/// The page reloads there: CEF can't move a live browser between windows.
pub fn give(n: Option<usize>) {
    let Some((n, mut tj)) = S.with_borrow(|s| {
        let n = n.unwrap_or(s.active);
        Some((n, tab_json(s.tabs.get(n)?)))
    }) else {
        return;
    };
    tj["pinned"] = false.into();
    create_window(None, Some(serde_json::json!({ "tabs": [tj], "active": 0 })));
    remove(n);
}

pub fn close_others() {
    close_others_than(S.with_borrow(|s| s.active));
}

/// Close every tab but `keep`.
fn close_others_than(keep: usize) {
    let Some(id) = S.with_borrow(|s| s.tabs.get(keep).and_then(|t| browser_id(&t.view))) else {
        return;
    };
    loop {
        let n = S.with_borrow(|s| {
            s.tabs
                .iter()
                .position(|t| !t.pinned && browser_id(&t.view) != Some(id) && s.tabs.len() > 1)
        });
        let Some(n) = n else { break };
        remove(n);
    }
}

/// First of `url.start_pages`.
pub fn start_page() -> String {
    config::get()
        .list("url.start_pages")
        .into_iter()
        .next()
        .unwrap_or_else(|| "about:blank".into())
}

/// A page entered/left fullscreen (video). `content.fullscreen.window`: fill our
/// window only (hide the tab bar); otherwise the window goes fullscreen too.
pub fn page_fullscreen(on: bool) {
    let Some((window, tabbar)) = S.with_borrow(|s| Some((s.window.clone()?, s.tabbar.clone()?)))
    else {
        return;
    };
    if config::get().bool("content.fullscreen.window") {
        if on {
            tabbar.set_visible(0);
            window.layout();
        } else {
            apply_layout();
        }
    } else {
        window.set_fullscreen(on as i32);
    }
}

/// `:fullscreen`: toggle the window's fullscreen state.
pub fn toggle_fullscreen() {
    if let Some(w) = S.with_borrow(|s| s.window.clone()) {
        w.set_fullscreen((w.is_fullscreen() == 0) as i32);
    }
}

pub fn close_window() {
    if let Some(w) = S.with_borrow(|s| s.window.clone()) {
        w.close();
    }
}

/// `quit`: save the session once, then close every window.
pub fn quit() {
    crate::session::save_now();
    QUITTING.set(true);
    let wins: Vec<Window> =
        WINS.with_borrow(|w| w.iter().filter_map(|s| s.window.clone()).collect());
    for w in wins {
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

/// Show `overlay` at `r` (window coords, kept inside the window) and run `js` in it.
fn popup_at(view: &BrowserView, overlay: &OverlayController, mut r: Rect, js: &str) {
    if let Some(w) = S.with_borrow(|s| s.window.clone()) {
        let b = w.bounds();
        r.x = r.x.min(b.width - r.width - 4).max(4);
        r.y = r.y.min(b.height - r.height - 4).max(0);
    }
    overlay.set_bounds(Some(&r));
    overlay.set_visible(1);
    if let Some(f) = view.browser().and_then(|b| b.main_frame()) {
        f.execute_java_script(Some(&CefString::from(js)), None, 0);
    }
}

/// Tab bar's origin in window coordinates (it may sit inside `mid` when vertical).
fn tabbar_origin() -> (i32, i32) {
    S.with_borrow(|s| {
        let tb = s.tabbar.as_ref()?;
        let mut p = Point { x: 0, y: 0 };
        View::from(tb).convert_point_to_window(Some(&mut p));
        Some((p.x, p.y))
    })
    .unwrap_or((0, 0))
}

/// Parse "i left bottom right" (tab-bar CSS px) from the tab bar page.
fn tab_rect(arg: &str) -> Option<(usize, i32, i32, i32)> {
    let v: Vec<i64> = arg.split(' ').filter_map(|x| x.parse().ok()).collect();
    match v[..] {
        [i, l, b, r] if i >= 0 => Some((i as usize, l as i32, b as i32, r as i32)),
        _ => None,
    }
}

/// "tip i left bottom right" from the tab bar: show title + URL under (or beside) tab i.
pub fn tab_tip(arg: &str) {
    let Some((view, overlay)) = S.with_borrow(|s| s.tip.clone()) else {
        return;
    };
    let Some((i, left, bottom, right)) = tab_rect(arg) else {
        return overlay.set_visible(0);
    };
    let Some((title, url)) =
        S.with_borrow(|s| s.tabs.get(i).map(|t| (t.title.clone(), t.url.clone())))
    else {
        return;
    };
    let (ox, oy) = tabbar_origin();
    let vertical = matches!(
        config::get().str("tabs.position").as_str(),
        "left" | "right"
    );
    let (w, h) = (480, 30 + 20 * (url.len() as i32 / 60 + 1).min(4));
    let (x, y) = if vertical {
        (ox + right + 6, oy + bottom - 24)
    } else {
        (ox + left, oy + bottom + 4)
    };
    popup_at(
        &view,
        &overlay,
        Rect {
            x,
            y,
            width: w,
            height: h,
        },
        &format!("nova.tip({}, {})", js_str(&title), js_str(&url)),
    );
}

const MENU: &[(&str, &str, &str)] = &[
    ("reload", "Reload", "r"),
    ("clone", "Duplicate", "gC"),
    ("yank", "Copy URL", "yy"),
    ("mute", "Mute / unmute", "Alt-m"),
    ("bookmark", "Bookmark", "M"),
    ("pin", "Pin / unpin", "Ctrl-p"),
    ("give", "Move to new window", "gD"),
    ("sleep", "Hibernate now", ""),
    ("keep", "Keep awake on / off", ""),
    ("close", "Close", "d"),
    ("others", "Close other tabs", "co"),
];

/// "menu i x bottom right": the tab's right-click menu at the mouse.
pub fn tab_menu(arg: &str) {
    let Some((view, overlay)) = S.with_borrow(|s| s.menu.clone()) else {
        return;
    };
    let Some((i, x, bottom, _)) = tab_rect(arg) else {
        return;
    };
    if let Some((_, o)) = S.with_borrow(|s| s.tip.clone()) {
        o.set_visible(0);
    }
    let (ox, oy) = tabbar_origin();
    let items: Vec<_> = MENU.iter().map(|(a, l, k)| [a, l, k]).collect();
    let r = Rect {
        x: ox + x,
        y: oy + bottom + 2,
        width: 230,
        height: MENU.len() as i32 * 28 + 14,
    };
    popup_at(
        &view,
        &overlay,
        r,
        &format!("nova.menu({i}, {})", serde_json::json!(items)),
    );
    // a just-shown overlay ignores request_focus (same as the palette)
    post_task(ThreadId::UI, Some(&mut FocusMenu::new()));
}

wrap_task! {
    struct FocusMenu {}
    impl Task {
        fn execute(&self) {
            if let Some((v, o)) = S.with_borrow(|s| s.menu.clone()) {
                if o.is_visible() != 0 {
                    v.request_focus();
                }
            }
        }
    }
}

/// Password suggestions (passwords.rs) under a login field; `field` is in the page's
/// CSS px from the content's top-left. Uses the tooltip overlay (never takes focus,
/// so typing stays in the field).
pub fn suggest(field: Rect, data: &serde_json::Value) {
    let Some((view, overlay, mid, content)) = S.with_borrow(|s| {
        let (v, o) = s.tip.clone()?;
        Some((v, o, s.mid.clone()?, s.content.clone()?))
    }) else {
        return;
    };
    let (m, c) = (mid.bounds(), content.bounds());
    let n = data["rows"].as_array().map_or(0, |a| a.len()) as i32;
    let r = Rect {
        x: m.x + c.x + field.x,
        y: m.y + c.y + field.y + field.height + 4,
        width: 360,
        height: 34 + n.min(8) * 28,
    };
    popup_at(&view, &overlay, r, &format!("nova.suggest({data})"));
}

pub fn suggest_hide() {
    if let Some((v, o)) = S.with_borrow(|s| s.tip.clone()) {
        o.set_visible(0);
        if let Some(f) = v.browser().and_then(|b| b.main_frame()) {
            f.execute_java_script(Some(&CefString::from("nova.suggest(null)")), None, 0);
        }
    }
}

pub fn close_popup() {
    if let Some((_, o)) = S.with_borrow(|s| s.menu.clone()) {
        o.set_visible(0);
    }
    focus_active();
}

/// "i action" from the menu: run it on tab i (not necessarily the active one).
pub fn tab_menu_action(arg: &str) {
    close_popup();
    let Some((i, action)) = arg
        .split_once(' ')
        .and_then(|(i, a)| Some((i.parse::<usize>().ok()?, a)))
    else {
        return;
    };
    let Some(id) = S.with_borrow(|s| s.tabs.get(i).and_then(|t| browser_id(&t.view))) else {
        return;
    };
    let cmd = match action {
        "close" => return close_user(i, false),
        "others" => return close_others_than(i),
        "pin" => return pin(Some(i)),
        "give" => return give(Some(i)),
        "sleep" => return hibernate(Some(i)),
        "keep" => return keep_awake(Some(i)),
        "reload" => "reload",
        "clone" => "tab-clone -b",
        "yank" => "yank",
        "mute" => "tab-mute",
        "bookmark" => "bookmark-add",
        _ => return,
    };
    run_in_tab(id, || crate::commands::run(cmd, None));
}

wrap_task! {
    struct ShowKeyhint { generation: u64 }
    impl Task {
        fn execute(&self) { show_keyhint(self.generation); }
    }
}

/// Which-key: after a pending prefix (e.g. `g`) waits briefly, then lists what can
/// follow, bottom-right of the page. Hidden as soon as the chain ends.
fn update_keyhint() {
    let rows = modes::keyhint().1;
    let generation = S.with_borrow_mut(|s| {
        s.keyhint_gen += 1;
        s.keyhint_gen
    });
    if rows.is_empty() {
        if let Some((_, o)) = S.with_borrow(|s| s.keyhint.clone()) {
            o.set_visible(0);
        }
    } else {
        post_delayed_task(ThreadId::UI, Some(&mut ShowKeyhint::new(generation)), 350);
    }
}

fn show_keyhint(generation: u64) {
    let Some((view, overlay, mid, content)) = S.with_borrow(|s| {
        if s.keyhint_gen != generation {
            return None;
        }
        let (v, o) = s.keyhint.clone()?;
        Some((v, o, s.mid.clone()?, s.content.clone()?))
    }) else {
        return;
    };
    let (prefix, rows) = modes::keyhint();
    if rows.is_empty() {
        return;
    }
    let cfg = config::get();
    let row_h = (theme::setting_px(&cfg, "fonts.keyhint") * 1.6).ceil() as i32;
    let (m, c) = (mid.bounds(), content.bounds());
    let h = (rows.len() as i32 * row_h + 18).min(c.height - 24);
    let w = 380.min(c.width - 24);
    overlay.set_bounds(Some(&Rect {
        x: m.x + c.x + c.width - w - 12,
        y: m.y + c.y + c.height - h - 12,
        width: w,
        height: h,
    }));
    overlay.set_visible(1);
    if let Some(f) = view.browser().and_then(|b| b.main_frame()) {
        let js = format!(
            "nova.keyhint({}, {})",
            js_str(&prefix),
            serde_json::json!(rows)
        );
        f.execute_java_script(Some(&CefString::from(js.as_str())), None, 0);
    }
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
    struct Relayout { slot: usize }
    impl Task {
        fn execute(&self) { in_slot(self.slot, apply_layout); }
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
    S.with_borrow_mut(|s| s.tabbar_size = size);
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
    let views = S.with_borrow(|s| {
        [
            s.tabbar.clone(),
            s.palette.as_ref().map(|c| c.0.clone()),
            s.keyhint.as_ref().map(|c| c.0.clone()),
            s.tip.as_ref().map(|c| c.0.clone()),
            s.menu.as_ref().map(|c| c.0.clone()),
        ]
    });
    for v in views.into_iter().flatten() {
        let Some(b) = v.browser() else { continue };
        // force-dark (colors.webpage.darkmode) is for web pages; it would invert a light theme
        devtools::send(
            &b,
            "Emulation.setAutoDarkModeOverride",
            serde_json::json!({ "enabled": false }),
        );
        if let Some(f) = b.main_frame() {
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
    crate::apply_spellcheck();
    // every window, not just the one that ran :set
    let slots: Vec<usize> =
        WINS.with_borrow(|w| (0..w.len()).filter(|&i| w[i].window.is_some()).collect());
    for i in slots {
        in_slot(i, || {
            apply_layout();
            let browsers: Vec<Browser> =
                S.with_borrow(|s| s.tabs.iter().filter_map(|t| t.view.browser()).collect());
            for b in &browsers {
                apply_dark(b);
            }
            refresh();
        });
    }
}

wrap_browser_view_delegate! {
    struct NovaBrowserViewDelegate { kind: u8 }
    impl ViewDelegate {
        fn preferred_size(&self, view: Option<&mut View>) -> Size {
            // Width and height must be non-zero, else CEF uses the whole window (S2).
            let tabbar = || {
                let w = view?.window()?;
                // try_borrow: CEF may ask during layout we started with state borrowed
                WINS.with(|ws| ws.try_borrow().ok()?.iter().find(|s| s.window.as_ref().is_some_and(|x| View::from(x).is_same(Some(&mut View::from(&w))) == 1)).map(|s| s.tabbar_size))
            };
            let (width, height) = match self.kind {
                KIND_TABBAR => tabbar().unwrap_or((1, 24)),
                _ => (1, 1),
            };
            Size { width, height }
        }
    }
    impl BrowserViewDelegate {
        fn delegate_for_popup_browser_view(&self, _view: Option<&mut BrowserView>, _settings: Option<&BrowserSettings>, _client: Option<&mut Client>, _is_devtools: ::std::os::raw::c_int) -> Option<BrowserViewDelegate> {
            Some(NovaBrowserViewDelegate::new(KIND_FIXED))
        }
        fn on_popup_browser_view_created(&self, view: Option<&mut BrowserView>, popup: Option<&mut BrowserView>, is_devtools: ::std::os::raw::c_int) -> ::std::os::raw::c_int {
            // DevTools get their own window (0 = CEF creates one); everything else becomes a tab
            // in the opener's window.
            let Some(popup) = popup.filter(|_| is_devtools == 0) else { return 0 };
            let slot = view.and_then(|v| browser_id(v)).and_then(owner);
            match slot {
                Some(slot) => in_slot(slot, || adopt_popup(popup.clone())),
                None => adopt_popup(popup.clone()),
            }
            1
        }
        fn browser_runtime_style(&self) -> RuntimeStyle { RuntimeStyle::ALLOY }
    }
}

wrap_window_delegate! {
    struct NovaWindowDelegate { slot: usize }
    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size { Size { width: 1200, height: 800 } }
        fn on_layout_changed(&self, _view: Option<&mut View>, bounds: Option<&Rect>) {
            in_slot(self.slot, place_palette);
            // vertical tab bar width is a percentage of the window: re-layout on resize
            let w = bounds.map_or(0, |b| b.width);
            let (_, size) = bar_sizes(w);
            let cur = WINS.with(|ws| ws.try_borrow().ok().and_then(|ws| ws.get(self.slot).map(|s| s.tabbar_size)));
            if size.1 == 1 && Some(size) != cur {
                post_task(ThreadId::UI, Some(&mut Relayout::new(self.slot)));
            }
        }
    }
    impl PanelDelegate {}
    impl WindowDelegate {
        fn on_window_activation_changed(&self, _window: Option<&mut Window>, active: ::std::os::raw::c_int) {
            if active != 0 {
                CUR.set(self.slot);
            }
        }

        fn on_window_created(&self, window: Option<&mut Window>) {
            CUR.set(self.slot);
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
            let keyhint = browser_view(&mut UiClient::new(), &data_url(KEYHINT_HTML).to_string(), KIND_FIXED, &ui);
            let kh_overlay = window
                .add_overlay_view(Some(&mut View::from(&keyhint)), DockingMode::CUSTOM, 0)
                .expect("keyhint overlay");
            let tip = browser_view(&mut UiClient::new(), &data_url(POPUP_HTML).to_string(), KIND_FIXED, &ui);
            let tip_overlay = window.add_overlay_view(Some(&mut View::from(&tip)), DockingMode::CUSTOM, 0).expect("tip overlay");
            let menu = browser_view(&mut UiClient::new(), &data_url(POPUP_HTML).to_string(), KIND_FIXED, &ui);
            let menu_overlay = window.add_overlay_view(Some(&mut View::from(&menu)), DockingMode::CUSTOM, 1).expect("menu overlay");
            // Added last so it stays on top (cef_window.h); positioned by place_palette.
            let palette = browser_view(&mut UiClient::new(), &data_url(PALETTE_HTML).to_string(), KIND_FIXED, &ui);
            let overlay = window
                .add_overlay_view(Some(&mut View::from(&palette)), DockingMode::CUSTOM, 1) // can_activate: takes keys
                .expect("palette overlay");
            let (start, startup) = S.with_borrow_mut(|s| {
                s.palette = Some((palette, overlay));
                s.keyhint = Some((keyhint, kh_overlay));
                s.tip = Some((tip, tip_overlay));
                s.menu = Some((menu, menu_overlay));
                s.window = Some(window.clone());
                s.mid = Some(mid);
                s.content = Some(content);
                s.tabbar = Some(tabbar);
                (s.start_url.clone(), s.startup.take())
            });
            apply_layout();
            window.show();
            let session = if STARTED.replace(true) { None } else { crate::session::startup() };
            match (session, S.with_borrow_mut(|s| s.restore.take()), start) {
                (Some(v), _, url) => {
                    restore(&v, false);
                    if let Some(u) = url { open(&u, true) }
                }
                (None, Some(w), _) => restore_tabs(&w, false),
                (None, None, url) => open(&url.unwrap_or_else(start_page), true),
            }
            if crate::welcome::first_run() {
                open(&data_url(&crate::welcome::page()).to_string(), true);
            }
            apply_layout(); // tabs.show = multiple depends on the tab count
            if let Some((level, text)) = startup {
                message(&level, &text);
            }
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            CUR.set(self.slot);
            // The last window's tabs are the session to restore; closing one of several drops it.
            let last = WINS.with_borrow(|w| w.iter().filter(|s| s.window.is_some()).count() <= 1);
            if last && !QUITTING.get() {
                crate::session::save_now();
            }
            // Detached tabs aren't destroyed with the window; close them by hand (S2).
            // Their views stay in `closing` until DoClose (see remove()).
            let hosts = S.with_borrow_mut(|s| {
                let active = s.active;
                let mut closing = std::mem::take(&mut s.closing);
                // Never-shown tabs have no browser and get no DoClose: drop them now, here,
                // not at thread exit after CEF shutdown (CHECK observers_.empty()).
                closing.extend(s.tabs.drain(..).enumerate().filter(|(i, t)| *i != active && t.view.browser().is_some()).map(|(_, t)| t.view));
                let hosts: Vec<_> = closing.iter().filter_map(|v| v.browser().and_then(|b| b.host())).collect();
                *s = State { closing, claimed: true, ..State::default() };
                hosts
            });
            for h in hosts { h.close_browser(1); }
            // act on some other open window from now on
            if let Some(i) = WINS.with_borrow(|w| w.iter().position(|s| s.window.is_some())) {
                CUR.set(i);
                if !QUITTING.get() {
                    crate::session::save_now();
                }
            }
        }

        // cef-rs defaults every int return to 0; CEF's C++ defaults for these are true.
        // shortcut: closing the window skips beforeunload (tab-close asks); add confirm_quit if missed.
        fn can_close(&self, _window: Option<&mut Window>) -> ::std::os::raw::c_int { 1 }

        /// Wayland app_id / X11 WM_CLASS = the .desktop file name, so docks, task
        /// switchers and Alt-Tab find novabrowser.desktop and its icon.
        fn linux_window_properties(&self, _window: Option<&mut Window>, properties: Option<&mut LinuxWindowProperties>) -> ::std::os::raw::c_int {
            let Some(p) = properties else { return 0 };
            // cef-rs only writes back "borrowed" raw strings (an owned CefString becomes
            // empty, try_set refuses struct fields): allocate a raw one with CEF's
            // allocator; CEF frees it when it clears the struct.
            let raw = || {
                let mut r: sys::_cef_string_utf16_t = unsafe { std::mem::zeroed() };
                let id = "novabrowser";
                unsafe { sys::cef_string_utf8_to_utf16(id.as_ptr().cast(), id.len(), &mut r) };
                CefString::from(r)
            };
            p.wayland_app_id = raw();
            p.wm_class_class = raw();
            p.wm_class_name = raw();
            1
        }
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
