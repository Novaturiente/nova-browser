//! Windows, the tab list, and the native chrome (`chrome/`: tab bar with mode, keys and
//! progress; palette, which-key, tooltip/menu, password list, prompt and toast as GTK
//! overlays). Each tab's page is an off-screen CEF browser drawn by `page.rs`; pages live
//! in a GtkStack per window and are created lazily (first show).
//!
//! All state lives on the main thread (CEF's UI thread runs GTK). CEF calls that can
//! call back into our handlers (create, focus, close) run after the state borrow is dropped.
use crate::chrome::{self, motion, Chrome, Ev, ListHeight, Status, TabItem};
use crate::modes::{self, Mode};
use crate::page::Page;
use crate::theme::{self, TitleFields};
use crate::{config, devtools, history};
use cef::*;
use gtk::{glib, prelude::*};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

struct Tab {
    page: Page,
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
    /// Page lifecycle frozen (`tabs.freeze_after`): no JS, timers or network until shown.
    frozen: bool,
    /// The next committed URL was opened from the address bar (history ranks it higher).
    typed: bool,
    /// No page committed yet in a newly opened tab: its first URL is a visit even
    /// when it equals `url` (restored session tabs aren't fresh).
    fresh: bool,
    /// Favicon as a data: URL (client.rs FaviconDone), empty until the page reports one;
    /// `tex` is it decoded once, for the tab bar.
    icon: String,
    tex: Option<gtk::gdk::Texture>,
    /// Unique per tab (browser ids change on hibernation); `opener` is the key of the
    /// tab whose page opened this one (0 = none), so related tabs stack after it.
    key: u64,
    opener: u64,
}

thread_local! {
    static TAB_KEY: Cell<u64> = const { Cell::new(0) };
}

/// Where a tab opened from tab `active` goes: right after it and the tabs it already opened.
fn related_slot(tabs: &[Tab], active: usize) -> usize {
    let Some(a) = tabs.get(active) else { return tabs.len() };
    let mut at = active + 1;
    while tabs.get(at).is_some_and(|t| t.opener == a.key) {
        at += 1;
    }
    at.max(tabs.iter().take_while(|t| t.pinned).count())
}

/// Entries kept per direction in a saved session.
const HIST_MAX: usize = 50;

#[derive(Default)]
struct State {
    window: Option<gtk::Window>,
    /// tab bar + content, in `tabs.position` order
    outer: Option<gtk::Box>,
    /// the pages (one child per tab that has been shown)
    stack: Option<gtk::Stack>,
    /// stack + palette, which-key, prompt and toast
    content: Option<gtk::Overlay>,
    /// everything + tooltip, password list and tab menu (placed in window coordinates)
    root: Option<gtk::Overlay>,
    tabbar: Option<gtk::Widget>,
    chrome: Option<Chrome>,
    /// the page, live and blurred, under the palette: a Box holding a Picture (CSS `.nova-backdrop`)
    backdrop: Option<gtk::Box>,
    /// bumped on every keyhint change; a delayed show only applies if still current
    keyhint_gen: u64,
    /// Palette shown as the search pill (the page must stay visible).
    compact: bool,
    tabs: Vec<Tab>,
    active: usize,
    /// Previously active tab, for `tab-focus last`.
    last: usize,
    /// URLs of closed tabs, newest last, for `undo`.
    closed: Vec<String>,
    /// Message (level, text) and a serial so old timers don't clear newer ones.
    message: Option<(String, String)>,
    /// Open permission prompt (permissions.rs).
    prompt: Option<String>,
    message_id: u64,
    /// URL given on the command line (opened after the restored session).
    start_url: Option<String>,
    /// Message to show once the window exists (config errors at startup).
    startup: Option<(String, String)>,
    /// Chromium was started with force-dark on (switches are fixed at startup).
    dark_global: bool,
    /// Slot taken by a window (created or being created); slots are never reused.
    claimed: bool,
    /// Session part (one window's tabs) to open when this window is created.
    restore: Option<Value>,
    /// Tabs as last handed to the session autosave (urls, pins, active): titles and
    /// progress don't need a save.
    saved_key: String,
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

pub fn quitting() -> bool {
    QUITTING.get()
}

/// `S.with_borrow(..)`: the current window's state.
struct Cur;
static S: Cur = Cur;
impl Cur {
    fn with_borrow<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        ensure_slot();
        WINS.with_borrow(|w| f(&w[CUR.get()]))
    }
    fn with_borrow_mut<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        ensure_slot();
        WINS.with_borrow_mut(|w| f(&mut w[CUR.get()]))
    }
}

fn ensure_slot() {
    WINS.with_borrow_mut(|w| {
        if w.is_empty() {
            w.push(State::default());
        }
    });
}

fn bid(p: &Page) -> Option<i32> {
    p.browser().map(|b| b.identifier())
}

/// Slot of the window owning browser `id`.
fn owner(id: i32) -> Option<usize> {
    WINS.with_borrow(|w| w.iter().position(|s| s.tabs.iter().any(|t| bid(&t.page) == Some(id))))
}

fn chrome() -> Option<Chrome> {
    S.with_borrow(|s| s.chrome.clone())
}

/// Run `f` acting on `slot`'s window, then go back to the previous one.
pub fn in_slot<R>(slot: usize, f: impl FnOnce() -> R) -> R {
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

pub use nova_core::host::js_str;

pub fn data_url(html: &str) -> CefString {
    let b64 = CefString::from(&base64_encode(Some(html.as_bytes()))).to_string();
    CefString::from(format!("data:text/html;base64,{b64}").as_str())
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
        background_color: argb(&cfg.str("colors.webpage.bg")).unwrap_or(0xffff_ffff),
        // off-screen pages default to 30 fps
        // shortcut: fixed 60, not the monitor's rate; read it from the GdkMonitor for 120 Hz screens
        windowless_frame_rate: 60,
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

pub fn create(start_url: Option<String>) {
    create_window(start_url, None);
}

/// Open a new window (`open -w`, session restore); it opens `start_url` or `restore`'s tabs.
fn create_window(start_url: Option<String>, restore: Option<Value>) {
    let cfg = config::get();
    let dark = cfg.bool("colors.webpage.darkmode.enabled")
        || cfg.any_pattern_value("colors.webpage.darkmode.enabled", &Value::Bool(true));
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
        slot
    });
    in_slot(slot, || build_window(slot));
}

/// `open -w [url]`.
pub fn open_window(url: Option<String>) {
    create_window(Some(url.unwrap_or_else(|| config::get().str("url.default_page"))), None);
}

fn build_window(slot: usize) {
    let window = gtk::Window::builder()
        .default_width(1200)
        .default_height(800)
        .title("novabrowser")
        .icon_name("novabrowser")
        .build();
    let root = gtk::Overlay::new();
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let content = gtk::Overlay::new();
    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_transition_type(gtk::StackTransitionType::None);
    stack.add_css_class("nova-pages");
    content.set_child(Some(&stack));
    content.set_hexpand(true);
    content.set_vexpand(true);
    root.set_child(Some(&outer));
    window.set_child(Some(&root));
    for o in [&root, &content] {
        o.connect_get_child_position(fixed_overlay_position);
    }
    window.add_css_class(chrome::style::WINDOW_CLASS);
    chrome::style::install(&WidgetExt::display(&window));
    let sink: chrome::Sink = Rc::new(move |ev| in_slot(slot, || on_ev(ev)));
    let ch = Chrome::new(sink);
    let tabbar = ch.tabbar.widget();
    let backdrop = gtk::Picture::new();
    backdrop.add_css_class("nova-backdrop");
    backdrop.set_content_fit(gtk::ContentFit::Fill);
    backdrop.set_hexpand(true);
    // on black: the blur's soft edges would show the sharp page under them
    let backdrop_bg = gtk::Box::new(gtk::Orientation::Vertical, 0);
    backdrop_bg.add_css_class("nova-backdrop-bg");
    backdrop_bg.append(&backdrop);
    backdrop_bg.set_can_target(false);
    backdrop_bg.set_visible(false);
    content.add_overlay(&backdrop_bg); // added first: under the palette
    let palette = ch.palette.widget();
    palette.set_halign(gtk::Align::Fill);
    palette.set_valign(gtk::Align::Fill);
    let keyhint = ch.keyhint.widget();
    keyhint.set_halign(gtk::Align::End);
    keyhint.set_valign(gtk::Align::End);
    keyhint.set_margin_end(12);
    keyhint.set_margin_bottom(12);
    // keys answer through prompt mode, so the page keeps focus; clicks still reach the buttons
    let ask = ch.prompt.widget();
    ask.set_halign(gtk::Align::Center);
    ask.set_valign(gtk::Align::Start);
    ask.set_margin_top(12);
    let toast = ch.toast.widget();
    toast.set_halign(gtk::Align::Center);
    toast.set_valign(gtk::Align::End);
    toast.set_margin_bottom(18);
    // which-key, prompt and toast never take focus from the page
    for w in [&keyhint, &ask, &toast] {
        w.set_can_focus(false);
        content.add_overlay(w);
    }
    content.add_overlay(&palette);
    for w in [ch.tip.widget(), ch.suggest.widget(), ch.menu.widget()] {
        w.set_halign(gtk::Align::Start);
        w.set_valign(gtk::Align::Start);
        root.add_overlay(&w);
    }
    ch.tip.widget().set_can_focus(false);
    ch.suggest.widget().set_can_focus(false);
    // a press outside the open tab menu dismisses it (the page still gets the press)
    let outside = gtk::GestureClick::new();
    outside.set_button(0);
    outside.set_propagation_phase(gtk::PropagationPhase::Capture);
    let menu_w = ch.menu.widget();
    outside.connect_pressed(move |g, _, x, y| {
        let Some(root) = g.widget() else { return };
        let inside = root
            .pick(x, y, gtk::PickFlags::DEFAULT)
            .is_some_and(|w| w == menu_w || w.is_ancestor(&menu_w));
        if !inside && menu_w.is_visible() {
            in_slot(slot, close_popup);
        }
    });
    root.add_controller(outside);
    modes::attach_keys(&window, slot);
    window.connect_is_active_notify(move |w| {
        if w.is_active() {
            CUR.set(slot);
        } else if WINS.with_borrow(|v| {
            v.get(slot).and_then(|s| s.chrome.as_ref()).is_some_and(|c| c.menu.is_shown())
        }) {
            // switching away dismisses the tab menu
            in_slot(slot, close_popup);
        }
    });
    window.connect_default_width_notify(move |_| {
        glib::idle_add_local_once(move || in_slot(slot, apply_layout));
    });
    window.connect_close_request(move |_| {
        in_slot(slot, window_closing);
        glib::Propagation::Proceed
    });
    let (start, startup) = S.with_borrow_mut(|s| {
        s.window = Some(window.clone());
        s.outer = Some(outer);
        s.stack = Some(stack);
        s.content = Some(content);
        s.root = Some(root);
        s.tabbar = Some(tabbar);
        s.chrome = Some(ch);
        s.backdrop = Some(backdrop_bg);
        (s.start_url.clone(), s.startup.take())
    });
    apply_layout();
    window.present();
    let session = if STARTED.replace(true) { None } else { crate::session::startup() };
    match (session, S.with_borrow_mut(|s| s.restore.take()), start) {
        (Some(v), _, url) => {
            restore(&v, false);
            if let Some(u) = url {
                open(&u, true)
            }
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

/// The window is closing: save the session if it is the last one, close its browsers.
/// The process quits when the last browser is gone (client.rs on_before_close).
fn window_closing() {
    // The last window's tabs are the session to restore; closing one of several drops it.
    let last = WINS.with_borrow(|w| w.iter().filter(|s| s.window.is_some()).count() <= 1);
    if last && !QUITTING.get() {
        crate::session::save_now();
    }
    let hosts = S.with_borrow_mut(|s| {
        let hosts: Vec<BrowserHost> =
            s.tabs.iter().filter_map(|t| t.page.browser()?.host()).collect();
        *s = State { claimed: true, ..State::default() };
        hosts
    });
    for h in hosts {
        h.close_browser(1);
    }
    // act on some other open window from now on
    match WINS.with_borrow(|w| w.iter().position(|s| s.window.is_some())) {
        Some(i) => {
            CUR.set(i);
            if !QUITTING.get() {
                crate::session::save_now();
            }
        }
        // none had a browser to close (else the last one's on_before_close quits)
        None if crate::client::browsers_open() == 0 => quit_message_loop(),
        None => {}
    }
}

/// No window is open (the process quits once their browsers are gone too).
pub fn none_open() -> bool {
    WINS.with_borrow(|w| w.iter().all(|s| s.window.is_none()))
}

thread_local! {
    /// Set around an address-bar `open` that makes a new tab: that tab starts `typed`.
    static TYPED_NEXT: Cell<bool> = const { Cell::new(false) };
}

/// Run `f` (an `open` from the address bar) so the page it loads counts as typed:
/// new tabs made inside `f` start typed; `current` marks the active tab instead.
pub fn typed_open(current: bool, f: impl FnOnce()) {
    if current {
        S.with_borrow_mut(|s| {
            let i = s.active;
            if let Some(t) = s.tabs.get_mut(i) {
                t.typed = true;
            }
        });
    }
    TYPED_NEXT.set(!current);
    f();
    TYPED_NEXT.set(false);
}

/// `opener`: the index of the tab whose page opened it, so it goes next to that tab,
/// not at the end.
fn add_tab(page: Page, url: String, title: String, foreground: bool, opener: Option<usize>) {
    TAB_KEY.set(TAB_KEY.get() + 1);
    let n = S.with_borrow_mut(|s| {
        let (at, opener) = match opener {
            Some(o) => (related_slot(&s.tabs, o), s.tabs.get(o).map_or(0, |t| t.key)),
            None => (s.tabs.len(), 0),
        };
        let tab = Tab {
            page,
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
            frozen: false,
            icon: String::new(),
            tex: None,
            typed: TYPED_NEXT.get(),
            fresh: true,
            key: TAB_KEY.get(),
            opener,
        };
        s.tabs.insert(at, tab);
        // indexes at or after the new tab moved right
        if at <= s.active && s.tabs.len() > 1 {
            s.active += 1;
        }
        if at <= s.last && s.tabs.len() > 1 {
            s.last += 1;
        }
        at
    });
    if foreground || n == 0 {
        show(n);
    } else {
        refresh();
    }
    apply_tabs_show();
}

/// A background tab loads only when first shown (`Page::ensure`), which is also what
/// makes session restore lazy.
pub fn open(url: &str, foreground: bool) {
    open_titled(url, String::new(), foreground, false);
}

/// A link of the active tab's page opened in a new tab: placed next to it.
pub fn open_related(url: &str, foreground: bool) {
    open_titled(url, String::new(), foreground, true);
}

fn open_titled(url: &str, title: String, foreground: bool, related: bool) {
    // a browser created with no URL never gets a page and ignores every key (a saved
    // popup that never loaded comes back as "")
    let url = if url.is_empty() { "about:blank" } else { url };
    let opener = related.then(|| S.with_borrow(|s| s.active));
    add_tab(Page::new(url), url.to_string(), title, foreground, opener);
}

/// A popup (target=_blank, window.open) or DevTools has its browser now (page.rs): a tab
/// next to the tab that opened it.
pub fn adopt_popup(page: Page, foreground: bool, opener: &Browser) {
    let Some(b) = page.browser() else { return };
    let url = b.main_frame().map(|f| CefString::from(&f.url()).to_string()).unwrap_or_default();
    let id = opener.identifier();
    let at = S.with_borrow(|s| s.tabs.iter().position(|t| bid(&t.page) == Some(id)).unwrap_or(s.active));
    if !foreground {
        // not on screen until shown: no frames meanwhile
        if let Some(h) = b.host() {
            h.was_hidden(1);
        }
    }
    add_tab(page, url, String::new(), foreground, Some(at));
}

/// `devtools`: open or close this tab's DevTools. CEF 154 always makes them a native
/// Chrome window (a windowless request gets no frames), so they can't be a tab or pane.
pub fn toggle_devtools(browser: &Browser) {
    let Some(h) = browser.host() else { return };
    if h.has_dev_tools() != 0 {
        h.close_dev_tools();
    } else {
        h.show_dev_tools(Some(&WindowInfo::default()), None, None, None);
    }
}

/// Every open window, in session.rs's shape.
pub fn session_json() -> Option<Value> {
    let slots: Vec<usize> =
        WINS.with_borrow(|w| (0..w.len()).filter(|i| w[*i].window.is_some()).collect());
    let wins: Vec<Value> = slots.iter().filter_map(|i| in_slot(*i, window_json)).collect();
    if wins.is_empty() {
        return None;
    }
    let closed = S.with_borrow(|s| s.closed[s.closed.len().saturating_sub(100)..].to_vec());
    Some(crate::session::shape(wins, &closed))
}

fn window_json() -> Option<Value> {
    S.with_borrow(|s| {
        s.window.as_ref()?;
        // data: tabs are our snapshot pages (downloads, bookmarks): stale after a restart;
        // DevTools belong to a page of this run
        let tabs = s
            .tabs
            .iter()
            .filter(|t| !t.url.starts_with("data:") && !t.url.starts_with("devtools:"))
            .map(tab_json)
            .collect();
        Some(crate::session::window(tabs, s.active))
    })
}

/// One tab in session.rs's shape: URL, title, back/forward URLs, scroll, pinned.
fn tab_json(t: &Tab) -> Value {
    let (real, cur) = t.page.browser().map(|b| nav_entries(&b)).unwrap_or_default();
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

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|u| u.as_str().map(str::to_string))
        .collect()
}

/// Open a saved session: its first window's tabs go into this window (`clear`
/// closes the tabs that were there), every other window opens as a new window.
pub fn restore(v: &Value, clear: bool) {
    // sessions before multi-window: {tabs, active, closed} at the top level
    let wins = v["windows"].as_array().cloned().unwrap_or_else(|| vec![v.clone()]);
    let closed = strs(&v["closed"]);
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
fn restore_tabs(v: &Value, clear: bool) {
    let before = S.with_borrow(|s| s.tabs.len());
    let tabs = v["tabs"].as_array().cloned().unwrap_or_default();
    for t in &tabs {
        let url = t["url"].as_str().unwrap_or("about:blank");
        open_titled(url, t["title"].as_str().unwrap_or("").to_string(), false, false);
        S.with_borrow_mut(|s| {
            let tab = s.tabs.last_mut().unwrap();
            tab.fresh = false;
            tab.back = strs(&t["back"]);
            tab.fwd = strs(&t["fwd"]).into_iter().rev().collect();
            tab.scroll_to = t["scroll"].as_i64().filter(|y| *y > 0);
            tab.pinned = t["pinned"].as_bool().unwrap_or(false);
            tab.keep = t["keep"].as_bool().unwrap_or(false);
            tab.icon = t["icon"].as_str().unwrap_or("").to_string();
            tab.tex = chrome::texture_from_data_url(&tab.icon);
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

/// Bring tab n to the front (its page loads on first show); the old one is marked
/// hidden so Chromium throttles it.
pub fn show(n: usize) {
    let Some((stack, old, new, thaw)) = S.with_borrow_mut(|s| {
        let new = s.tabs.get(n)?.page.clone();
        let old = (s.active != n).then(|| s.tabs.get(s.active).map(|t| t.page.clone())).flatten();
        if old.is_some() {
            s.last = s.active;
            s.tabs[s.active].seen = std::time::Instant::now();
            crate::passwords::hide(); // the list belongs to the field in the tab we leave
        }
        s.active = n;
        s.tabs[n].asleep = false;
        let thaw = std::mem::take(&mut s.tabs[n].frozen);
        Some((s.stack.clone()?, old, new, thaw))
    }) else {
        return;
    };
    let w = new.widget();
    if w.parent().is_none() {
        stack.add_child(&w);
    }
    stack.set_visible_child(&w);
    new.ensure(&page_settings());
    // a split mobile view's side page hides and shows with its tab
    let hosts = |p: &Page| [Some(p.clone()), p.twin()].into_iter().flatten().filter_map(|p| p.browser()?.host());
    for h in old.iter().flat_map(hosts) {
        h.was_hidden(1);
    }
    // browser() is None until CEF has created it
    if let Some(b) = new.browser().filter(|_| thaw) {
        devtools::send(&b, "Page.setWebLifecycleState", serde_json::json!({"state": "active"}));
    }
    for h in hosts(&new) {
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
/// the prompt); the tab goes away in `did_close` once CEF reaches DoClose.
/// Never-shown and crashed tabs have nothing to ask and close at once.
pub fn close(n: usize) {
    let host = S.with_borrow(|s| {
        let t = s.tabs.get(n)?;
        // DevTools ignore a polite close when windowless (CEF: "not supported for this
        // DevTools window"); they have no beforeunload to ask anyway
        (t.crashed.is_none() && !t.url.starts_with("devtools:")).then(|| t.page.browser()?.host()).flatten()
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

/// CEF reached DoClose for `browser` (client.rs posts this): drop its tab if it is
/// still there (a page that closed itself, `close` after beforeunload, DevTools closed).
pub fn did_close(browser: &Browser) {
    in_owner(browser, || {
        let id = browser.identifier();
        if let Some(n) = S.with_borrow(|s| s.tabs.iter().position(|t| bid(&t.page) == Some(id))) {
            CLOSED.set(true);
            remove(n);
            CLOSED.set(false);
        }
    });
}

/// Take tab `n` out now and force-close its browser (no beforeunload).
pub fn remove(n: usize) {
    let Some((tab, was_active, left, window)) = S.with_borrow_mut(|s| {
        if n >= s.tabs.len() {
            return None;
        }
        let tab = s.tabs.remove(n);
        s.closed.push(tab.crashed.clone().unwrap_or_else(|| tab.url.clone()));
        s.last = s.last.min(s.tabs.len().saturating_sub(1));
        let was_active = n == s.active;
        // closing the active tab focuses the one before it (left), like qutebrowser `prev`
        if n < s.active || (was_active && s.active > 0) {
            s.active -= 1;
        }
        Some((tab, was_active, s.tabs.len(), s.window.clone()))
    }) else {
        return;
    };
    if left == 0 {
        // qutebrowser tabs.last_close; the tab we removed goes back as a fresh one
        let cfg = config::get();
        let reopen = match cfg.str("tabs.last_close").as_str() {
            "close" => None,
            "blank" => Some("about:blank".to_string()),
            "startpage" => Some(start_page()),
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
            // the window's other browsers close with it; quit follows in on_before_close
            release(tab.page);
            if let Some(w) = window {
                w.close();
            }
            return;
        };
        unparent(&tab.page);
        open(&url, true);
        release(tab.page);
        return;
    }
    unparent(&tab.page);
    if was_active {
        let active = S.with_borrow(|s| s.active);
        show(active);
    } else {
        refresh();
    }
    apply_tabs_show();
    release(tab.page);
}

fn unparent(p: &Page) {
    let w = p.widget();
    if let Some(stack) = w.parent().and_downcast::<gtk::Stack>() {
        stack.remove(&w);
    }
}

/// Close a removed tab's browser (CEF destroys it after DoClose). A tab that was never
/// shown has no browser and nothing to close.
fn release(page: Page) {
    if let Some(h) = page.unstage().and_then(|s| s.browser()?.host()) {
        h.close_browser(1);
    }
    if CLOSED.get() {
        return; // DoClose already ran
    }
    if let Some(h) = page.browser().and_then(|b| b.host()) {
        h.close_browser(1);
    }
}

/// `mobile-toggle [device] [device]`: show the active tab's page as the first device;
/// with a second, the same page as that device too, on its right. Off if on.
pub fn mobile_toggle(devices: &[String]) -> Result<(), String> {
    let page = S.with_borrow(|s| s.tabs.get(s.active).map(|t| t.page.clone())).ok_or("no active tab")?;
    let b = page.browser().ok_or("no page")?;
    if page.device().is_some() {
        if let Some(h) = page.unstage().and_then(|s| s.browser()?.host()) {
            h.close_browser(1);
        }
        page.set_device(None);
        b.reload(); // the user agent applies from the next load
        return Ok(());
    }
    let names = match devices {
        [] => &["iphone-16".to_string(), "ipad-air".to_string()][..],
        d => d,
    };
    let found = names.iter().map(|n| crate::page::device(n)).collect::<Result<Vec<_>, _>>()?;
    let [first, rest @ ..] = &found[..] else { unreachable!() };
    if rest.len() > 1 {
        return Err("usage: mobile-toggle [device] [device]".into());
    }
    page.set_device(Some(first));
    let side = rest.first().map(|second| {
        let url = b.main_frame().map(|f| CefString::from(&f.url()).to_string()).unwrap_or_default();
        let side = Page::new(&url);
        side.set_device(Some(second)); // before its first load: the UA picks it up
        side
    });
    page.stage(side.clone());
    if let Some(side) = side {
        side.ensure(&page_settings());
    }
    b.reload();
    Ok(())
}

/// Apply `f` to the tab owning `browser`, then redraw the bars.
fn update(browser: &Browser, f: impl FnOnce(&mut Tab)) {
    let id = browser.identifier();
    in_owner(browser, || {
        let found = S.with_borrow_mut(|s| {
            s.tabs.iter_mut().find(|t| bid(&t.page) == Some(id)).map(f).is_some()
        });
        if found {
            refresh();
        }
    });
}

pub fn set_url(browser: &Browser, url: String) {
    update(browser, |t| {
        // a typed open counts even when the URL didn't change (new tab, same page again)
        let typed = std::mem::take(&mut t.typed);
        let fresh = std::mem::take(&mut t.fresh);
        if t.crashed.is_none() && (t.url != url || typed || fresh) {
            if !url.starts_with("devtools:") {
                history::visit(&url, typed);
            }
            t.url = url.clone();
            t.scroll = 0;
        }
    });
    apply_dark(browser);
    let Some(p) = crate::page::of(browser) else { return };
    // a new renderer (first page, cross-site) starts without the emulation
    if p.device().is_some() {
        p.apply_mobile();
    }
    // the two pages of a split mobile view follow each other (it stops once both match)
    if let Some(f) = p.twin().and_then(|t| t.browser()?.main_frame()) {
        if CefString::from(&f.url()).to_string() != url {
            f.load_url(Some(&CefString::from(url.as_str())));
        }
    }
}

/// Per-site dark mode (S5): `pattern()` values of colors.webpage.darkmode.enabled
/// override Chromium's global force-dark for this tab; the override survives navigation.
fn apply_dark(browser: &Browser) {
    let id = browser.identifier();
    let cfg = config::get();
    let send = in_owner(browser, || {
        S.with_borrow_mut(|s| {
            let global = s.dark_global;
            let t = s.tabs.iter_mut().find(|t| bid(&t.page) == Some(id))?;
            // our own data: pages follow the UI theme, never force-dark
            let want = !t.url.starts_with("data:")
                && cfg.get_for("colors.webpage.darkmode.enabled", &t.url).as_bool().unwrap_or(false);
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
        Some(None) => {
            devtools::send(browser, "Emulation.setAutoDarkModeOverride", serde_json::json!({}))
        }
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
    update(browser, |t| {
        t.tex = chrome::texture_from_data_url(&icon);
        t.icon = icon;
    });
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
            if let Some(t) = s.tabs.iter_mut().find(|t| bid(&t.page) == Some(id)) {
                t.scroll = y;
            }
        })
    });
    crate::session::dirty();
}

/// Real navigation entries and the current index (CEF visits synchronously).
fn nav_entries(b: &Browser) -> (Vec<String>, usize) {
    let out = Rc::new(RefCell::new((Vec::new(), 0)));
    if let Some(h) = b.host() {
        h.navigation_entries(Some(&mut NavCollect::new(out.clone())), 0);
    }
    out.take()
}

wrap_navigation_entry_visitor! {
    struct NavCollect { out: Rc<RefCell<(Vec<String>, usize)>> }
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
        let t = s.tabs.iter_mut().find(|t| bid(&t.page) == Some(id))?;
        let real = if n < 0 { t.can_back } else { t.can_fwd };
        if real {
            return Some(Err(()));
        }
        let cur = t.url.clone();
        let (from, to) = if n < 0 { (&mut t.back, &mut t.fwd) } else { (&mut t.fwd, &mut t.back) };
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
                if n < 0 { "At beginning of history." } else { "At end of history." },
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

/// Push the current state into the tab bar (tabs, mode, keys, progress) and the
/// window title.
pub fn refresh() {
    update_keyhint();
    let cfg = config::get();
    let tab_fmt = cfg.str("tabs.title.format");
    let pin_fmt = cfg.str("tabs.title.format_pinned");
    let Some((ch, window, tabs, status, title, key)) = S.with_borrow(|s| {
        let tabs: Vec<TabItem> = s
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                // the focused pinned tab expands to the normal title
                let pinned = t.pinned && i != s.active;
                TabItem {
                    key: t.key,
                    label: theme::format_title(if pinned { &pin_fmt } else { &tab_fmt }, &fields(i, t)),
                    icon: t.tex.clone(),
                    active: i == s.active,
                    loading: t.loading,
                    progress: t.progress,
                    crashed: t.crashed.is_some(),
                    pinned: t.pinned,
                    asleep: t.asleep,
                }
            })
            .collect();
        let key = s.tabs.iter().fold(s.active.to_string(), |k, t| {
            format!("{k} {} {} {} {}", t.url, t.pinned, t.keep, t.crashed.is_some())
        });
        let t = s.tabs.get(s.active)?;
        let (dl, dlc) = crate::downloads::summary();
        let status = Status {
            profile: crate::profile().unwrap_or("").to_string(),
            mode: modes::mode_label().to_string(),
            modename: modes::mode_name(),
            cmd: modes::mode() == Mode::Command,
            keys: if modes::mode() == Mode::Hint { crate::hints::typed() } else { modes::pending() },
            dl,
            dlc,
            loading: t.loading,
            progress: t.progress,
        };
        let mut title = theme::format_title(&cfg.str("window.title_format"), &fields(s.active, t));
        if let Some(p) = crate::profile() {
            title = format!("[{p}] {title}");
        }
        Some((s.chrome.clone()?, s.window.clone()?, tabs, status, title, key))
    }) else {
        return;
    };
    if S.with_borrow_mut(|s| std::mem::replace(&mut s.saved_key, key) != s.saved_key) {
        crate::session::dirty();
    }
    ch.tabbar.apply(&tabs, &status);
    window.set_title(Some(&title));
}

// ---- queries and actions for commands.rs

thread_local! {
    /// Tab a userscript's FIFO line runs in (see run_in_tab); None = the active tab.
    static TARGET: Cell<Option<i32>> = const { Cell::new(None) };
}

pub fn active_browser() -> Option<Browser> {
    S.with_borrow(|s| {
        let i = match TARGET.get() {
            Some(id) => s.tabs.iter().position(|t| bid(&t.page) == Some(id))?,
            None => s.active,
        };
        s.tabs.get(i)?.page.browser()
    })
}

/// Run `f` with commands aimed at the tab whose browser id is `id` instead of the
/// active one (userscripts act on the tab that started them). False if it's gone.
pub fn run_in_tab(id: i32, f: impl FnOnce()) -> bool {
    let Some(slot) = owner(id) else { return false };
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
            "menu": s.chrome.as_ref().is_some_and(|c| c.menu.is_shown()),
            // on-screen size of the tooltip/password popup while shown
            "popup": s.chrome.as_ref().and_then(|c| c.suggest.shown_size().or_else(|| c.tip.shown_size())).map(|(w, h)| [w, h]),
            "message": s.message.as_ref().map(|m| format!("{}: {}", m.0, m.1)),
            "prompt": s.prompt,
        })
        .to_string()
    })
}

pub fn active_url_title() -> (String, String) {
    S.with_borrow(|s| {
        let i = match TARGET.get() {
            Some(id) => s.tabs.iter().position(|t| bid(&t.page) == Some(id)).unwrap_or(usize::MAX),
            None => s.active,
        };
        s.tabs.get(i).map(|t| (t.url.clone(), t.title.clone())).unwrap_or_default()
    })
}

/// The page area's size in logical px; None before the window is laid out.
pub fn content_size_px() -> Option<(i32, i32)> {
    let stack = S.with_borrow(|s| s.stack.clone())?;
    let (w, h) = (stack.width(), stack.height());
    (w > 0 && h > 0).then_some((w, h))
}

pub fn content_size() -> (f64, f64) {
    content_size_px().map_or((0.0, 0.0), |(w, h)| (w as f64, h as f64))
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
        let active = s.tabs[s.active].key;
        let mut tab = s.tabs.remove(n);
        tab.pinned = !tab.pinned;
        let to = s.tabs.iter().take_while(|t| t.pinned).count();
        s.tabs.insert(to, tab);
        // keep the same tab active
        s.active = s.tabs.iter().position(|t| t.key == active).unwrap_or(to);
    });
    refresh();
}

/// True if the page plays media or has typed form text: never hibernate it.
/// shortcut: main frame only (an embedded player in an iframe isn't seen).
const BUSY_JS: &str = "[...document.querySelectorAll('video,audio')].some(m=>!m.paused&&!m.ended)||\
[...document.querySelectorAll('input:not([type=hidden]),textarea')].some(e=>e.value!==e.defaultValue)";

/// Freezing also spares tabs that may want to notify (chat, mail).
const FREEZE_BUSY_JS: &str = "(self.Notification&&Notification.permission==='granted')||";

/// Start the once-a-minute idle check (`tabs.hibernate_after` / `tabs.freeze_after`,
/// minutes; 0 = off).
pub fn start_hibernation() {
    glib::timeout_add_seconds_local(60, || {
        hibernate_idle();
        glib::ControlFlow::Continue
    });
}

fn hibernate_idle() {
    let cfg = config::get();
    // per site via pattern(): 0 there exempts the site, a number there overrides the global one
    let due = |t: &Tab, name: &str| {
        let mins = cfg.get_for(name, &t.url).as_i64().unwrap_or(0);
        mins > 0 && t.seen.elapsed() >= Duration::from_secs(mins as u64 * 60)
    };
    // (browser, frozen already, hibernate due); frozen pages can't answer JS, and
    // can't have changed since they passed the busy check when frozen
    let candidates: Vec<(Browser, bool, bool)> = WINS.with_borrow(|w| {
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
                })
            })
            .filter_map(|(_, t)| {
                let sleep = due(t, "tabs.hibernate_after");
                let freeze = !t.frozen && !t.pinned && due(t, "tabs.freeze_after");
                (sleep || freeze).then(|| Some((t.page.browser()?, t.frozen, sleep)))?
            })
            .collect()
    });
    for (b, frozen, sleep) in candidates {
        if frozen {
            if sleep {
                hibernate_browser(b.identifier(), false);
            }
            continue;
        }
        let b2 = b.clone();
        let js = if sleep { BUSY_JS.to_string() } else { format!("{FREEZE_BUSY_JS}{BUSY_JS}") };
        devtools::eval_isolated(&b, js, move |r| {
            // no answer (hung, closed) = leave it alone
            if r.is_some_and(|(_, v)| v == Value::Bool(false)) {
                in_owner(&b2, || {
                    if sleep {
                        hibernate_browser(b2.identifier(), false)
                    } else {
                        freeze_browser(&b2)
                    }
                });
            }
        });
    }
}

/// Page lifecycle "frozen" for a background tab (re-checked: it may be in front now).
fn freeze_browser(b: &Browser) {
    let id = b.identifier();
    let ok = S.with_borrow_mut(|s| {
        let n = s.tabs.iter().position(|t| bid(&t.page) == Some(id))?;
        let t = &mut s.tabs[n];
        (n != s.active && !t.keep && !t.pinned && !t.loading).then(|| t.frozen = true)
    });
    if ok.is_some() {
        devtools::send(b, "Page.setWebLifecycleState", serde_json::json!({"state": "frozen"}));
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
        bid(&t.page).ok_or("Tab is already unloaded.")
    });
    match id {
        Ok(id) => hibernate_browser(id, true),
        Err(e) => message("error", e),
    }
}

/// Replace the tab's page with a not-yet-loaded one of the same URL; history
/// and scroll come back like a restored session tab when it's shown again.
fn hibernate_browser(id: i32, manual: bool) {
    let find = |s: &State| s.tabs.iter().position(|t| bid(&t.page) == Some(id));
    let Some(url) = S.with_borrow(|s| {
        let n = find(s)?;
        let t = &s.tabs[n];
        // re-check: things may have changed while the page was asked
        (n != s.active && t.crashed.is_none() && (manual || !t.keep)).then(|| t.url.clone())
    }) else {
        return;
    };
    let old = S.with_borrow_mut(|s| {
        let n = find(s)?;
        let tj = tab_json(&s.tabs[n]);
        let t = &mut s.tabs[n];
        let old = std::mem::replace(&mut t.page, Page::new(&url));
        t.back = strs(&tj["back"]);
        t.fwd = strs(&tj["fwd"]).into_iter().rev().collect();
        t.scroll_to = tj["scroll"].as_i64().filter(|y| *y > 0);
        t.asleep = true;
        t.loading = false;
        t.progress = 0.0;
        t.can_back = false;
        t.can_fwd = false;
        t.dark = None;
        t.frozen = false;
        Some(old)
    });
    if let Some(old) = old {
        unparent(&old);
        release(old); // its on_before_close forgets devtools/passwords state
        refresh();
    }
}

/// `tab-keep-awake [index]`: toggle the tab's exemption from hibernation.
pub fn keep_awake(n: Option<usize>) {
    let on = S.with_borrow_mut(|s| {
        let n = n.unwrap_or(s.active);
        let t = s.tabs.get_mut(n)?;
        t.keep = !t.keep;
        let thaw = (t.keep && std::mem::take(&mut t.frozen)).then(|| t.page.browser()).flatten();
        Some((t.keep, thaw))
    });
    if let Some((_, Some(b))) = &on {
        devtools::send(b, "Page.setWebLifecycleState", serde_json::json!({"state": "active"}));
    }
    match on.map(|(k, _)| k) {
        Some(true) => message("info", "This tab won't be hibernated."),
        Some(false) => message("info", "This tab can be hibernated again."),
        None => message("error", "No such tab."),
    }
    refresh();
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

/// Close every unpinned tab but `keep`.
fn close_others_than(keep: usize) {
    let Some(key) = S.with_borrow(|s| s.tabs.get(keep).map(|t| t.key)) else {
        return;
    };
    while let Some(n) =
        S.with_borrow(|s| s.tabs.iter().position(|t| !t.pinned && t.key != key && s.tabs.len() > 1))
    {
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
    tabbar.set_visible(!on);
    if !config::get().bool("content.fullscreen.window") {
        if on {
            window.fullscreen()
        } else {
            window.unfullscreen()
        }
    }
    if !on {
        apply_tabs_show();
    }
}

/// `:fullscreen`: toggle the window's fullscreen state.
pub fn toggle_fullscreen() {
    if let Some(w) = S.with_borrow(|s| s.window.clone()) {
        if w.is_fullscreen() {
            w.unfullscreen()
        } else {
            w.fullscreen()
        }
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
    let wins: Vec<gtk::Window> = WINS.with_borrow(|w| w.iter().filter_map(|s| s.window.clone()).collect());
    for w in wins {
        w.close();
    }
}

/// `tab-next`/`tab-prev`, wrapping (qutebrowser `tabs.wrap`).
pub fn cycle(by: i64) {
    let n = S.with_borrow(|s| (s.active as i64 + by).rem_euclid(s.tabs.len().max(1) as i64) as usize);
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
        let pinned = s.tabs.iter().filter(|t| t.pinned).count();
        let to = move_target(to, len as usize, pinned, s.tabs[s.active].pinned);
        let tab = s.tabs.remove(s.active);
        s.tabs.insert(to, tab);
        s.active = to;
    });
    refresh();
}

/// Clamp a move so pinned tabs stay in the pinned group at the start, unpinned ones after it.
fn move_target(to: i64, len: usize, pinned: usize, is_pinned: bool) -> usize {
    let (lo, hi) = if is_pinned { (0, pinned - 1) } else { (pinned, len - 1) };
    to.clamp(lo as i64, hi as i64) as usize
}

#[test]
fn move_keeps_pinned_group() {
    // 5 tabs, first 2 pinned
    assert_eq!(move_target(4, 5, 2, true), 1);
    assert_eq!(move_target(-3, 5, 2, true), 0);
    assert_eq!(move_target(0, 5, 2, false), 2);
    assert_eq!(move_target(3, 5, 2, false), 3);
    assert_eq!(move_target(9, 5, 0, false), 4);
}

pub fn undo_close() {
    match S.with_borrow_mut(|s| s.closed.pop()) {
        Some(url) => open(&url, true),
        None => message("error", "Nothing to undo!"),
    }
}

// ---- the chrome

/// Show (or hide, with None) the open question and its [key, label] answers; keys
/// still answer it through prompt mode.
pub fn set_prompt(p: Option<(String, Vec<[&'static str; 2]>)>) {
    let Some(c) = S.with_borrow_mut(|s| {
        s.prompt = p.as_ref().map(|p| p.0.clone());
        s.chrome.clone()
    }) else {
        return;
    };
    refresh();
    match p {
        Some((q, opts)) => c.prompt.show(&q, &opts),
        None => c.prompt.hide(),
    }
}

/// Show a message for `messages.timeout` ms (`info`, `warning`, `error`, `find`) in the
/// toast; find positions go in the search pill instead.
pub fn message(level: &str, text: &str) {
    eprintln!("novabrowser: {level}: {text}");
    let id = S.with_borrow_mut(|s| {
        s.message = Some((level.to_string(), text.to_string()));
        s.message_id += 1;
        s.message_id
    });
    refresh();
    let Some(c) = chrome() else { return };
    // typing a search: the count goes in the search pill, which sits where the toast would
    if modes::mode() == Mode::Command && S.with_borrow(|s| s.compact) {
        let count = match level {
            "find" => text.split('\t').nth(1),
            "warning" if text.ends_with("not found on page!") => Some("no match"),
            _ => None,
        };
        if let Some(n) = count {
            return c.palette.count(n);
        }
    }
    c.toast.show(level, text);
    let ms = config::get().int("messages.timeout");
    if ms > 0 {
        let slot = CUR.get();
        glib::timeout_add_local_once(Duration::from_millis(ms as u64), move || {
            in_slot(slot, || {
                let cleared = S.with_borrow_mut(|s| s.message_id == id && s.message.take().is_some());
                if cleared {
                    refresh();
                    c.toast.hide();
                }
            })
        });
    }
}

/// Show `text` once the window is up (startup config errors).
pub fn startup_message(level: &str, text: &str) {
    S.with_borrow_mut(|s| s.startup = Some((level.to_string(), text.to_string())));
}

/// `cmd-set-text`: open the palette with `text`. Search (`/`, `?`) is a pill so the
/// matches stay visible; everything else floats over the page, blurred live by GTK.
pub fn open_cmdline(text: &str) {
    modes::enter(Mode::Command);
    let compact = text.starts_with('/') || text.starts_with('?');
    let Some((c, backdrop, page)) = S.with_borrow_mut(|s| {
        s.compact = compact;
        Some((s.chrome.clone()?, s.backdrop.clone()?, s.tabs.get(s.active).map(|t| t.page.widget())))
    }) else {
        return;
    };
    match page.filter(|_| !compact) {
        Some(page) => {
            set_backdrop(&backdrop, Some(&page));
            motion::fade(&backdrop, 1.0, || {});
        }
        None => motion::fade(&backdrop, 0.0, || {}),
    }
    c.palette.open(text, compact);
}

/// Point the palette's backdrop at `page` (None: let go of it).
fn set_backdrop(b: &gtk::Box, page: Option<&gtk::Widget>) {
    if let Some(p) = b.first_child().and_downcast::<gtk::Picture>() {
        p.set_paintable(page.map(|v| gtk::WidgetPaintable::new(Some(v))).as_ref());
    }
}

/// Leave command mode and give focus back to the page.
pub fn close_cmdline() {
    modes::enter(Mode::Normal);
    crate::completion::reset();
    if let Some((c, b)) = S.with_borrow(|s| Some((s.chrome.clone()?, s.backdrop.clone()?))) {
        c.palette.close();
        let b2 = b.clone();
        // let go of the page: a live paintable redraws with it
        motion::fade(&b, 0.0, move || {
            if !b2.is_visible() {
                set_backdrop(&b2, None)
            }
        });
    }
    focus_active();
    refresh();
}

/// Replace the command line text (completion, history) without leaving it.
pub fn set_cmdline_text(text: &str) {
    if let Some(c) = chrome() {
        c.palette.set_text(text);
    }
}

/// Show completion rows `json` under the input, or clear them.
pub fn show_completion(json: Option<&str>) {
    // closing (completion::reset): keep the list while the palette springs out, or the
    // card snaps to one line first; the next open clears it
    if json.is_none() && modes::mode() != Mode::Command {
        return;
    }
    let Some(c) = chrome() else { return };
    let v = json.and_then(|j| serde_json::from_str::<Value>(j).ok());
    c.palette.complete(v.as_ref());
    let Some(v) = v else { return };
    // site icons for rows with a web address: the open tabs' favicons, by host
    // shortcut: only sites open in a tab get an icon; add Chromium's favicon store if missed
    let hosts: Vec<(String, gtk::gdk::Texture)> = S.with_borrow(|s| {
        s.tabs.iter().filter_map(|t| Some((url_host(&t.url)?, t.tex.clone()?))).collect()
    });
    if hosts.is_empty() {
        return;
    }
    let icons: Vec<(String, gtk::gdk::Texture)> = v["items"]
        .as_array()
        .into_iter()
        .flatten()
        .take(40)
        .filter_map(|it| {
            let u = it.as_array()?.iter().filter_map(|x| x.as_str()).find(|x| url_host(x).is_some())?;
            let h = url_host(u)?;
            Some((u.to_string(), hosts.iter().find(|(x, _)| *x == h)?.1.clone()))
        })
        .collect();
    if !icons.is_empty() {
        c.palette.icons(&icons);
    }
}

/// "host[:port]" of an http(s) URL.
fn url_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then(|| host.to_string())
}

/// `completion.height` to the palette.
fn place_palette() {
    let Some(c) = chrome() else { return };
    c.palette.set_list_height(match config::get().get("completion.height") {
        Value::String(p) if p.ends_with('%') => {
            ListHeight::Percent(p.trim_end_matches('%').parse().unwrap_or(50.0))
        }
        v => ListHeight::Px(v.as_i64().unwrap_or(300) as i32),
    });
}

/// Overlay children with a size request get exactly that size, placed by their align +
/// margins; others use GTK's default placement.
fn fixed_overlay_position(o: &gtk::Overlay, child: &gtk::Widget) -> Option<gtk::gdk::Rectangle> {
    let (w, h) = child.size_request();
    if w <= 0 || h <= 0 {
        return None;
    }
    let bw = child.margin_start() + w + child.margin_end();
    let bh = child.margin_top() + h + child.margin_bottom();
    let at = |a: gtk::Align, outer: i32, inner: i32| match a {
        gtk::Align::End => outer - inner,
        gtk::Align::Center => (outer - inner) / 2,
        _ => 0,
    };
    Some(gtk::gdk::Rectangle::new(
        at(child.halign(), o.width(), bw),
        at(child.valign(), o.height(), bh),
        bw,
        bh,
    ))
}

/// Place a root-overlay popup (already filled) at window coords (x, y), kept inside.
fn popup_at(w: &gtk::Widget, x: i32, y: i32) {
    let (ww, wh) = S.with_borrow(|s| s.root.as_ref().map_or((0, 0), |r| (r.width(), r.height())));
    // measure without the last placement's margins (GTK adds a widget's own margins)
    w.set_margin_start(0);
    w.set_margin_top(0);
    let (_, nw, _, _) = w.measure(gtk::Orientation::Horizontal, -1);
    let (_, nh, _, _) = w.measure(gtk::Orientation::Vertical, nw);
    w.set_margin_start(x.min(ww - nw - 4).max(4));
    w.set_margin_top(y.min(wh - nh - 4).max(0));
}

/// A widget's origin in root (window content) coordinates.
fn origin_of(w: Option<impl IsA<gtk::Widget>>) -> (i32, i32) {
    let root = S.with_borrow(|s| s.root.clone());
    match (w, root) {
        (Some(w), Some(r)) => w
            .compute_point(&r, &gtk::graphene::Point::new(0.0, 0.0))
            .map_or((0, 0), |p| (p.x() as i32, p.y() as i32)),
        _ => (0, 0),
    }
}

/// Hover on tab i (left, bottom, right in tab bar coordinates): title + URL under (or
/// beside) it. None hides.
fn tab_tip(r: Option<(usize, i32, i32, i32)>) {
    let Some(c) = chrome() else { return };
    let Some((i, left, bottom, right)) = r else {
        return c.tip.hide();
    };
    let Some((title, url)) = S.with_borrow(|s| s.tabs.get(i).map(|t| (t.title.clone(), t.url.clone())))
    else {
        return;
    };
    let (ox, oy) = origin_of(S.with_borrow(|s| s.tabbar.clone()));
    let vertical = matches!(config::get().str("tabs.position").as_str(), "left" | "right");
    let (x, y) = if vertical { (ox + right + 6, oy + bottom - 24) } else { (ox + left, oy + bottom + 4) };
    c.tip.show(&title, &url);
    popup_at(&c.tip.widget(), x, y);
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

/// Right-click on tab i at (x, bottom) in tab bar coordinates: its menu at the mouse.
fn tab_menu(i: usize, x: i32, bottom: i32) {
    let Some(c) = chrome() else { return };
    c.tip.hide();
    let (ox, oy) = origin_of(S.with_borrow(|s| s.tabbar.clone()));
    c.menu.show(i, MENU);
    popup_at(&c.menu.widget(), ox + x, oy + bottom + 2);
}

/// A key while the tab menu is open (modes.rs asks first); true = the menu took it.
pub fn menu_key(name: &str) -> bool {
    chrome().is_some_and(|c| c.menu.key(name))
}

/// Password suggestions (passwords.rs) under a login field; `field` is in the page's
/// CSS px from the page's top-left.
pub fn suggest(field: Rect, data: &Value) {
    let Some((c, stack)) = S.with_borrow(|s| Some((s.chrome.clone()?, s.stack.clone()?))) else {
        return;
    };
    let (cx, cy) = origin_of(Some(stack));
    c.suggest.show(data);
    popup_at(&c.suggest.widget(), cx + field.x, cy + field.y + field.height + 4);
}

pub fn suggest_hide() {
    if let Some(c) = chrome() {
        c.suggest.hide();
    }
}

pub fn close_popup() {
    if let Some(c) = chrome() {
        c.menu.hide();
    }
    focus_active();
}

/// A tab menu pick: run `action` on tab i (not necessarily the active one).
fn tab_menu_action(i: usize, action: &str) {
    close_popup();
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
    // a tab never shown has no browser yet: show it first
    if S.with_borrow(|s| s.tabs.get(i).is_some_and(|t| t.page.browser().is_none())) {
        show(i);
        return message("info", "The tab is loading; try again.");
    }
    let Some(id) = S.with_borrow(|s| s.tabs.get(i).and_then(|t| bid(&t.page))) else {
        return;
    };
    run_in_tab(id, || crate::commands::run(cmd, None));
}

/// Which-key: after a pending prefix (e.g. `g`) waits 350 ms, list what can follow.
/// Hidden as soon as the chain ends.
fn update_keyhint() {
    let rows = modes::keyhint().1;
    let (generation, slot) = S.with_borrow_mut(|s| {
        s.keyhint_gen += 1;
        (s.keyhint_gen, CUR.get())
    });
    if rows.is_empty() {
        if let Some(c) = chrome() {
            c.keyhint.hide();
        }
    } else {
        glib::timeout_add_local_once(Duration::from_millis(350), move || {
            in_slot(slot, || show_keyhint(generation))
        });
    }
}

fn show_keyhint(generation: u64) {
    let Some(c) = S.with_borrow(|s| (s.keyhint_gen == generation).then(|| s.chrome.clone())?) else {
        return;
    };
    let (prefix, rows) = modes::keyhint();
    if !rows.is_empty() {
        c.keyhint.show(&prefix, &rows);
    }
}

/// Focus whichever tab is active once the current command finishes (the palette keeps
/// focus in command mode).
pub fn focus_active() {
    let slot = CUR.get();
    glib::idle_add_local_once(move || {
        in_slot(slot, || {
            let p = S.with_borrow(|s| s.tabs.get(s.active).map(|t| t.page.clone()));
            if let Some(p) = p.filter(|_| modes::mode() != Mode::Command) {
                p.focus();
            }
        })
    });
}

// ---- layout and theme from config

/// Put the tab bar where `tabs.position` says and size it; at window creation and when
/// the config changes. Widgets are re-parented, not recreated.
fn apply_layout() {
    let Some((window, outer, content, tabbar)) = S.with_borrow(|s| {
        Some((s.window.clone()?, s.outer.clone()?, s.content.clone()?, s.tabbar.clone()?))
    }) else {
        return;
    };
    let cfg = config::get();
    let pos = cfg.str("tabs.position");
    let vertical = pos == "left" || pos == "right";
    for w in [tabbar.upcast_ref::<gtk::Widget>(), content.upcast_ref()] {
        if w.parent().is_some() {
            outer.remove(w);
        }
    }
    outer.set_orientation(if vertical { gtk::Orientation::Horizontal } else { gtk::Orientation::Vertical });
    if pos == "bottom" || pos == "right" {
        outer.append(&content);
        outer.append(&tabbar);
    } else {
        outer.append(&tabbar);
        outer.append(&content);
    }
    if vertical {
        let ww = window.width().max(window.default_width());
        let w = match cfg.get("tabs.width") {
            Value::String(p) => p
                .trim_end_matches('%')
                .parse::<f64>()
                .map_or(200, |p| (ww as f64 * p / 100.0) as i32),
            v => v.as_i64().unwrap_or(200) as i32,
        };
        tabbar.set_size_request(w.max(1), -1);
        tabbar.set_vexpand(true);
        tabbar.set_hexpand(false);
    } else {
        // + 8: room for the tabs' 4 px margin above and below (chrome/tabbar.rs); never
        // thinner than Safari's 38 px
        let h = (theme::bar_height(&cfg, "fonts.tabs.selected", "tabs.padding") + 8).max(38);
        tabbar.set_size_request(-1, h);
        tabbar.set_hexpand(true);
        tabbar.set_vexpand(false);
    }
    apply_tabs_show();
    place_palette();
    push_theme();
    focus_active(); // re-parenting moves focus off the page
}

/// `tabs.show`: never / always / multiple.
fn apply_tabs_show() {
    let Some(tabbar) = S.with_borrow(|s| s.tabbar.clone()) else {
        return;
    };
    let show = match config::get().str("tabs.show").as_str() {
        "never" => false,
        // shortcut: "switching" (show briefly on tab switch) behaves like "multiple"
        "multiple" | "switching" => S.with_borrow(|s| s.tabs.len() > 1),
        _ => true,
    };
    tabbar.set_visible(show);
}

/// Theme variables to the chrome's CSS, tab bar orientation.
fn push_theme() {
    let cfg = config::get();
    chrome::style::apply(&theme::vars(&cfg));
    if let Some(c) = chrome() {
        c.tabbar.set_vertical(matches!(cfg.str("tabs.position").as_str(), "left" | "right"));
    }
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
                S.with_borrow(|s| s.tabs.iter().filter_map(|t| t.page.browser()).collect());
            for b in &browsers {
                apply_dark(b);
            }
            refresh();
        });
    }
}

/// A user action in the chrome of the current window.
fn on_ev(ev: Ev) {
    match ev {
        Ev::Tab(n) => show(n),
        Ev::Close(n) => close_user(n, false),
        Ev::Move(n) => move_active(n as i64 + 1, false),
        Ev::Menu(i, x, bottom) => tab_menu(i, x, bottom),
        Ev::Tip(r) => tab_tip(r),
        Ev::Downloads => crate::commands::run_from_bar("downloads"),
        Ev::Text(t) => crate::completion::on_text(&t),
        Ev::Run(line) => {
            crate::completion::record(&line);
            close_cmdline();
            crate::commands::run_from_bar(&line);
        }
        Ev::Complete(by) => crate::completion::select(by),
        Ev::Hist(by) => crate::completion::history_step(by),
        Ev::CmdLeave => close_cmdline(),
        Ev::MenuAct(i, action) => tab_menu_action(i, action),
        Ev::MenuClose => close_popup(),
        Ev::PwPick(i) => crate::passwords::on_click(i),
        Ev::Answer(key) => crate::permissions::on_key(&key),
    }
}
