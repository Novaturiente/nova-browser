//! Window layout (S2), the tab list and the HTML tab/status bars.
//!
//! All state lives on the CEF UI thread. CEF calls that can call back into our
//! handlers (attach, focus, close) run after the state borrow is dropped.
use crate::client::{TabClient, UiClient};
use cef::*;
use std::cell::RefCell;

const BAR_H: i32 = 24;
const TABBAR_HTML: &str = include_str!("../ui/tabbar.html");
const STATUS_HTML: &str = include_str!("../ui/statusbar.html");

struct Tab {
    view: BrowserView,
    url: String,
    title: String,
    progress: f64,
    loading: bool,
    /// Set while the crashed page shows; holds the URL `r` reloads.
    crashed: Option<String>,
}

#[derive(Default)]
struct State {
    window: Option<Window>,
    content: Option<Panel>,
    tabbar: Option<BrowserView>,
    status: Option<BrowserView>,
    tabs: Vec<Tab>,
    active: usize,
    start_url: String,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State::default());
}

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

fn browser_view(client: &mut Client, url: &str, height: i32) -> BrowserView {
    browser_view_create(
        Some(client),
        Some(&CefString::from(url)),
        Some(&BrowserSettings::default()),
        None,
        None,
        Some(&mut NovaBrowserViewDelegate::new(height)),
    )
    .expect("browser_view_create")
}

fn browser_id(view: &BrowserView) -> Option<i32> {
    view.browser().map(|b| b.identifier())
}

pub fn create(start_url: String) {
    S.with_borrow_mut(|s| s.start_url = start_url);
    window_create_top_level(Some(&mut NovaWindowDelegate::new()));
}

fn add_tab(view: BrowserView, url: String, foreground: bool) {
    let n = S.with_borrow_mut(|s| {
        s.tabs.push(Tab {
            view,
            url,
            title: String::new(),
            progress: 0.0,
            loading: false,
            crashed: None,
        });
        s.tabs.len() - 1
    });
    if foreground || n == 0 {
        show(n);
    } else {
        refresh();
    }
}

/// shortcut: a background tab created here loads only when first shown (CEF
/// creates the browser on attach); fine for mouse use, revisit for session restore.
pub fn open(url: &str, foreground: bool) {
    let view = browser_view(&mut TabClient::new(), url, 1);
    add_tab(view, url.to_string(), foreground);
}

/// Swap the active tab into the content panel (S2: state survives, old tab
/// is marked hidden so Chromium throttles it).
pub fn show(n: usize) {
    let Some((content, old, new)) = S.with_borrow_mut(|s| {
        let new = s.tabs.get(n)?.view.clone();
        let old = (s.active != n)
            .then(|| s.tabs.get(s.active).map(|t| t.view.clone()))
            .flatten();
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
    new.request_focus();
    refresh();
}

pub fn close(n: usize) {
    let Some((tab, was_active, left, content, window)) = S.with_borrow_mut(|s| {
        if n >= s.tabs.len() {
            return None;
        }
        let tab = s.tabs.remove(n);
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
        // Closing the window closes the attached browsers; quit follows in on_before_close.
        if let Some(w) = window {
            w.close();
        }
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
    if let Some(h) = tab.view.browser().and_then(|b| b.host()) {
        h.close_browser(1);
    }
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
        if t.crashed.is_none() {
            t.url = url
        }
    });
}
pub fn set_title(browser: &Browser, title: String) {
    update(browser, |t| {
        if t.crashed.is_none() {
            t.title = title
        }
    });
}
pub fn set_progress(browser: &Browser, p: f64) {
    update(browser, |t| t.progress = p);
}
pub fn set_loading(browser: &Browser, loading: bool) {
    update(browser, |t| t.loading = loading);
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

/// Push the current state into the tab bar, status bar and window title.
pub fn refresh() {
    let Some((tabbar, status, window, bar_json, status_json, title)) = S.with_borrow(|s| {
        let tabs: Vec<String> = s
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let label = if t.title.is_empty() { &t.url } else { &t.title };
                format!(
                    "{{\"t\":{},\"a\":{},\"l\":{}}}",
                    js_str(label),
                    i == s.active,
                    t.loading
                )
            })
            .collect();
        let t = s.tabs.get(s.active)?;
        let status = format!(
            "{{\"url\":{},\"p\":{},\"loading\":{},\"crashed\":{}}}",
            js_str(&t.url),
            t.progress,
            t.loading,
            t.crashed.is_some()
        );
        let title = if t.title.is_empty() {
            t.url.clone()
        } else {
            t.title.clone()
        };
        Some((
            s.tabbar.clone()?,
            s.status.clone()?,
            s.window.clone()?,
            format!("{{\"tabs\":[{}]}}", tabs.join(",")),
            status,
            title,
        ))
    }) else {
        return;
    };
    for (view, json) in [(tabbar, bar_json), (status, status_json)] {
        if let Some(f) = view.browser().and_then(|b| b.main_frame()) {
            let js = format!("window.nova && nova.apply && nova.apply({json})");
            f.execute_java_script(Some(&CefString::from(js.as_str())), None, 0);
        }
    }
    window.set_title(Some(&CefString::from(
        format!("{title} - novabrowser").as_str(),
    )));
}

/// A popup (target=_blank, window.open) arrives as a ready BrowserView: make it a tab.
fn adopt_popup(view: BrowserView) {
    let url = view
        .browser()
        .and_then(|b| b.main_frame())
        .map(|f| CefString::from(&f.url()).to_string())
        .unwrap_or_default();
    let foreground = !crate::client::take_background_popup();
    add_tab(view, url, foreground);
}

wrap_browser_view_delegate! {
    struct NovaBrowserViewDelegate { height: i32 }
    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            // Width and height must be non-zero, else CEF uses the whole window (S2).
            Size { width: 1, height: self.height }
        }
    }
    impl BrowserViewDelegate {
        fn delegate_for_popup_browser_view(&self, _view: Option<&mut BrowserView>, _settings: Option<&BrowserSettings>, _client: Option<&mut Client>, _is_devtools: ::std::os::raw::c_int) -> Option<BrowserViewDelegate> {
            Some(NovaBrowserViewDelegate::new(1))
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
            let tabbar = browser_view(&mut UiClient::new(), &data_url(TABBAR_HTML).to_string(), BAR_H);
            let content = panel_create(None).expect("panel");
            content.set_to_fill_layout();
            let status = browser_view(&mut UiClient::new(), &data_url(STATUS_HTML).to_string(), BAR_H);
            window.add_child_view(Some(&mut View::from(&tabbar)));
            window.add_child_view(Some(&mut View::from(&content)));
            window.add_child_view(Some(&mut View::from(&status)));
            layout.set_flex_for_view(Some(&mut View::from(&content)), 1);
            let start = S.with_borrow_mut(|s| {
                s.window = Some(window.clone());
                s.content = Some(content);
                s.tabbar = Some(tabbar);
                s.status = Some(status);
                s.start_url.clone()
            });
            window.show();
            open(&start, true);
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            // Detached tabs aren't destroyed with the window; close them by hand (S2).
            let tabs = S.with_borrow_mut(|s| {
                let active = s.active;
                let tabs: Vec<BrowserView> = s.tabs.drain(..).enumerate().filter(|(i, _)| *i != active).map(|(_, t)| t.view).collect();
                *s = State::default();
                tabs
            });
            for t in tabs {
                if let Some(h) = t.browser().and_then(|b| b.host()) { h.close_browser(1); }
            }
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
