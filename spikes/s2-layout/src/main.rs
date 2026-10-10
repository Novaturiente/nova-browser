//! S2 spike: tabbar + content panel + statusbar + overlay; tab swap keeps state.
//! Control from DevTools: set `document.title` in any view to
//! `nova:tab:<n>` (show tab n) or `nova:overlay` (toggle overlay).
use cef::*;
use std::cell::{Cell, RefCell};

const BAR_H: i32 = 24;

#[derive(Default)]
struct Ui {
    window: Option<Window>,
    content: Option<Panel>,
    tabs: Vec<BrowserView>,
    active: usize,
    overlay: Option<OverlayController>,
}

thread_local! {
    static UI: RefCell<Ui> = RefCell::new(Ui::default());
    static OPEN: Cell<i32> = const { Cell::new(0) };
}

fn data_url(html: &str) -> CefString {
    let b64 = CefString::from(&base64_encode(Some(html.as_bytes()))).to_string();
    CefString::from(format!("data:text/html;base64,{b64}").as_str())
}

fn bar_html(text: &str, bg: &str) -> String {
    format!(
        "<body style='margin:0;padding:3px 6px;background:{bg};color:#ddd;font:13px monospace;overflow:hidden'>{text}</body>"
    )
}

fn browser_view(url: &CefString, width: i32, height: i32) -> BrowserView {
    let mut client = NovaClient::new();
    let mut delegate = NovaBrowserViewDelegate::new(width, height);
    browser_view_create(
        Some(&mut client),
        Some(url),
        Some(&BrowserSettings::default()),
        None,
        None,
        Some(&mut delegate),
    )
    .expect("browser_view_create")
}

fn switch_tab(n: usize) {
    UI.with_borrow_mut(|ui| {
        if n >= ui.tabs.len() || n == ui.active {
            return;
        }
        let content = ui.content.as_ref().unwrap();
        let old = &ui.tabs[ui.active];
        if let Some(host) = old.browser().and_then(|b| b.host()) {
            host.was_hidden(1);
        }
        content.remove_child_view(Some(&mut View::from(old)));
        let new = &ui.tabs[n];
        content.add_child_view(Some(&mut View::from(new)));
        content.layout();
        // browser() is None until the view first joins a window.
        if let Some(host) = new.browser().and_then(|b| b.host()) {
            host.was_hidden(0);
        }
        new.request_focus();
        ui.active = n;
        eprintln!("s2: switched to tab {n}");
    });
}

fn dump() {
    UI.with_borrow(|ui| {
        let w = ui.window.as_ref().unwrap();
        eprintln!(
            "s2: window {:?} children {}",
            w.bounds(),
            w.child_view_count()
        );
        for i in 0..w.child_view_count() as i32 {
            // count includes the overlay host, which child_view_at doesn't return.
            let Some(v) = w.child_view_at(i) else { continue };
            eprintln!(
                "s2:  child {i} {:?} pref {:?} visible {} {}",
                v.bounds(),
                v.preferred_size(),
                v.is_visible(),
                CefString::from(&v.to_string(0))
            );
        }
        if let Some(o) = &ui.overlay {
            eprintln!(
                "s2: overlay {:?} visible {} view {:?}",
                o.bounds(),
                o.is_visible(),
                o.contents_view().map(|v| (v.bounds(), v.is_visible(), v.is_drawn()))
            );
        }
        eprintln!(
            "s2: content child {:?}",
            ui.content
                .as_ref()
                .unwrap()
                .child_view_at(0)
                .map(|v| v.bounds())
        );
    });
}

fn toggle_overlay() {
    UI.with_borrow(|ui| {
        if let Some(o) = &ui.overlay {
            o.set_visible((o.is_visible() == 0) as i32);
        }
    });
}

wrap_client! {
    struct NovaClient;
    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> { Some(NovaDisplay::new()) }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(NovaLifeSpan::new()) }
    }
}

wrap_display_handler! {
    struct NovaDisplay;
    impl DisplayHandler {
        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            let title = title.map(|t| t.to_string()).unwrap_or_default();
            if let Some(n) = title.strip_prefix("nova:tab:") {
                if let Ok(n) = n.parse() { switch_tab(n); }
            } else if title == "nova:overlay" {
                toggle_overlay();
            } else if title == "nova:dump" {
                dump();
            }
        }
    }
}

wrap_life_span_handler! {
    struct NovaLifeSpan;
    impl LifeSpanHandler {
        fn on_after_created(&self, _browser: Option<&mut Browser>) {
            OPEN.set(OPEN.get() + 1);
        }
        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            OPEN.set(OPEN.get() - 1);
            if OPEN.get() == 0 { quit_message_loop(); }
        }
    }
}

wrap_browser_view_delegate! {
    struct NovaBrowserViewDelegate { width: i32, height: i32 }
    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            // A 0 width/height makes the size "empty" and CEF falls back to the default (whole window).
            // Docked overlays also take their size from here, not from set_size().
            Size { width: self.width, height: self.height }
        }
    }
    impl BrowserViewDelegate {
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
                .unwrap();

            let tabbar = browser_view(&data_url(&bar_html("[0] wikipedia &nbsp; [1] example", "#333")), 1, BAR_H);
            let content = panel_create(None).unwrap();
            content.set_to_fill_layout();
            let status = browser_view(&data_url(&bar_html("-- NORMAL -- s2 spike", "#000")), 1, BAR_H);

            let tabs = vec![
                browser_view(&CefString::from("https://en.wikipedia.org/wiki/Rust_(programming_language)"), 1, 1),
                browser_view(&CefString::from("https://example.com/"), 1, 1),
            ];
            content.add_child_view(Some(&mut View::from(&tabs[0])));

            window.add_child_view(Some(&mut View::from(&tabbar)));
            window.add_child_view(Some(&mut View::from(&content)));
            window.add_child_view(Some(&mut View::from(&status)));
            layout.set_flex_for_view(Some(&mut View::from(&content)), 1);

            let completion = browser_view(
                &data_url(&bar_html("completion overlay<br>:open<br>:tab-close<br>:quit", "#1d3557")),
                600,
                120,
            );
            let overlay = window
                .add_overlay_view(Some(&mut View::from(&completion)), DockingMode::BOTTOM_LEFT, 0)
                .unwrap();
            overlay.set_insets(Some(&Insets { bottom: BAR_H, ..Default::default() }));
            overlay.set_visible(1);

            UI.with_borrow_mut(|ui| {
                ui.window = Some(window.clone());
                ui.content = Some(content.clone());
                ui.tabs = tabs;
                ui.overlay = Some(overlay);
            });
            window.show();
            for (name, v) in [("tabbar", View::from(&tabbar)), ("content", View::from(&content)), ("status", View::from(&status))] {
                eprintln!("s2: {name} bounds {:?}", v.bounds());
            }
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            // Detached tabs are not destroyed with the window; close them by hand.
            UI.with_borrow_mut(|ui| {
                for (i, t) in ui.tabs.iter().enumerate() {
                    if i != ui.active {
                        if let Some(h) = t.browser().and_then(|b| b.host()) { h.close_browser(1); }
                    }
                }
                *ui = Ui::default();
            });
        }

        fn window_runtime_style(&self) -> RuntimeStyle { RuntimeStyle::ALLOY }
    }
}

wrap_browser_process_handler! {
    struct NovaBrowserProcess;
    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            window_create_top_level(Some(&mut NovaWindowDelegate::new()));
        }
    }
}

wrap_app! {
    struct NovaApp;
    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(NovaBrowserProcess::new())
        }
    }
}

fn main() {
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = args::Args::new();
    let code = execute_process(Some(args.as_main_args()), None, std::ptr::null_mut());
    if code >= 0 {
        std::process::exit(code);
    }
    let settings = Settings {
        root_cache_path: CefString::from("/tmp/novabrowser-s2"),
        ..Default::default()
    };
    let mut app = NovaApp::new();
    assert_eq!(
        initialize(
            Some(args.as_main_args()),
            Some(&settings),
            Some(&mut app),
            std::ptr::null_mut()
        ),
        1
    );
    run_message_loop();
    shutdown();
}
