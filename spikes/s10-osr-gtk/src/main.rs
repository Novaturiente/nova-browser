//! S10 spike: CEF off-screen rendering (OSR) inside a GTK4 window, against today's
//! windowed CEF. MODE (env):
//! - `gpu`: OnAcceleratedPaint dmabuf -> GdkDmabufTexture, copied into a GTK-owned GPU
//!   texture inside the callback (CEF hands the buffer back to its pool afterwards)
//! - `gpu-nocopy`: the same dmabuf texture shown lazily (no copy; may tear)
//! - `cpu`: OnPaint BGRA buffer -> GdkMemoryTexture
//! - `windowed`: CEF Views window, no GTK (what novabrowser does today; the baseline)
//! CARD=1 puts a translucent, blurred, rounded GTK card over the page.
//! URL (env) is the start page; CDP_PORT (default 9310) the DevTools port.
//! Prints `s10 paints=N frames=M` once a second (OSR paints, GTK frames drawn).
use cef::*;
use gtk::{gdk, glib, graphene, gsk, prelude::*};
use std::cell::{Cell, RefCell};
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Windowed,
    Gpu,
    GpuNoCopy,
    Cpu,
}

fn mode() -> Mode {
    match std::env::var("MODE").as_deref() {
        Ok("windowed") => Mode::Windowed,
        Ok("gpu-nocopy") => Mode::GpuNoCopy,
        Ok("cpu") => Mode::Cpu,
        _ => Mode::Gpu,
    }
}

struct Ui {
    window: gtk::Window,
    page: gtk::Picture,
    /// the card's blurred copy of the page (CARD=1)
    backdrop: Option<gtk::Picture>,
    fixed: gtk::Fixed,
    popup: gtk::Picture,
    /// input method (compose, CJK, emoji): preedit and commits go to CEF's IME calls
    im: gtk::IMMulticontext,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    static HOST: RefCell<Option<BrowserHost>> = const { RefCell::new(None) };
    static MAIN_LOOP: RefCell<Option<glib::MainLoop>> = const { RefCell::new(None) };
    static PUMP: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
    static PAINTS: Cell<u32> = const { Cell::new(0) };
    static FRAMES: Cell<u32> = const { Cell::new(0) };
    static SIZE: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
    static POINTER: Cell<(f64, f64)> = const { Cell::new((0.0, 0.0)) };
    static LAST_DMABUF: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
    /// inside filter_keypress of a key press: a one-char commit is plain typing
    static IN_KEY: Cell<bool> = const { Cell::new(false) };
    static COMPOSING: Cell<bool> = const { Cell::new(false) };
    static PUMPS: Cell<(u32, u32)> = const { Cell::new((0, 0)) }; // (asks, of which delay 0)
}

const CARD: (i32, i32, i32, i32) = (60, 60, 420, 140);

fn s(v: &str) -> CefString {
    CefString::from(v)
}

fn host() -> Option<BrowserHost> {
    HOST.with_borrow(|h| h.clone())
}

fn page_size() -> (i32, i32) {
    let (w, h) = SIZE.get();
    if w > 0 && h > 0 {
        (w, h)
    } else {
        (1280, 800)
    }
}

/// The surface's scale, fractional too (1.5 at 150 %); CEF renders at size x scale.
fn scale() -> f64 {
    UI.with_borrow(|u| u.as_ref().and_then(|u| u.window.surface()).map_or(1.0, |s| s.scale()))
}

/// CefRange::InvalidRange: "no replacement"
const NO_RANGE: Range = Range { from: u32::MAX, to: u32::MAX };

// ------------------------------------------------------------------ frames

/// Wrap CEF's dmabuf planes in a GdkDmabufTexture. The fds are dup'ed: CEF closes its own.
fn dmabuf_texture(info: &AcceleratedPaintInfo) -> Result<gdk::Texture, String> {
    // DRM fourcc: BGRA bytes = ARGB8888 ("AR24"), RGBA bytes = ABGR8888 ("AB24")
    let fourcc = if info.format == ColorType::BGRA_8888 { 0x3432_5241 } else { 0x3432_4241 };
    let (w, h) = (info.extra.coded_size.width, info.extra.coded_size.height);
    let n = info.plane_count.clamp(0, 4) as usize;
    let planes = &info.planes[..n];
    if LAST_DMABUF.replace((w, h)) != (w, h) {
        eprintln!(
            "s10 dmabuf: {w}x{h} visible={:?} planes={n} modifier={:#x} format={:?} stride={}",
            (info.extra.visible_rect.width, info.extra.visible_rect.height),
            info.modifier,
            info.format,
            planes.first().map_or(0, |p| p.stride)
        );
    }
    let fds: Vec<OwnedFd> = planes
        .iter()
        .map(|p| unsafe { BorrowedFd::borrow_raw(p.fd) }.try_clone_to_owned())
        .collect::<Result<_, _>>()
        .map_err(|e| format!("dup: {e}"))?;
    let display = gdk::Display::default().ok_or("no display")?;
    let mut b = gdk::DmabufTextureBuilder::new()
        .set_display(&display)
        .set_width(w as u32)
        .set_height(h as u32)
        .set_fourcc(fourcc)
        .set_modifier(info.modifier)
        .set_premultiplied(true)
        .set_n_planes(n as u32);
    for (i, (p, fd)) in planes.iter().zip(&fds).enumerate() {
        b = unsafe { b.set_fd(i as u32, fd.as_raw_fd()) }
            .set_stride(i as u32, p.stride)
            .set_offset(i as u32, p.offset as u32);
    }
    unsafe { b.build_with_release_func(move || drop(fds)) }.map_err(|e| e.to_string())
}

/// Copy into a texture the GTK renderer owns, on the GPU (the dmabuf goes back to CEF's
/// pool when the callback returns).
fn gpu_copy(t: &gdk::Texture) -> Option<gdk::Texture> {
    let renderer = UI.with_borrow(|u| u.as_ref().and_then(|u| u.window.renderer()))?;
    let r = graphene::Rect::new(0.0, 0.0, t.width() as f32, t.height() as f32);
    Some(renderer.render_texture(gsk::TextureNode::new(t, &r), None))
}

fn show(kind: PaintElementType, t: &gdk::Texture) {
    PAINTS.set(PAINTS.get() + 1);
    UI.with_borrow(|u| {
        let Some(u) = u else { return };
        if kind == PaintElementType::POPUP {
            u.popup.set_paintable(Some(t));
        } else {
            u.page.set_paintable(Some(t));
            if let Some(b) = &u.backdrop {
                b.set_paintable(Some(t));
            }
        }
    });
}

wrap_render_handler! {
    struct Osr;
    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            let (w, h) = page_size();
            if let Some(r) = rect {
                r.x = 0;
                r.y = 0;
                r.width = w;
                r.height = h;
            }
        }

        fn screen_info(&self, _browser: Option<&mut Browser>, info: Option<&mut ScreenInfo>) -> ::std::os::raw::c_int {
            let Some(info) = info else { return 0 };
            let (w, h) = page_size();
            info.device_scale_factor = std::env::var("SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(scale() as f32);
            info.depth = 24;
            info.depth_per_component = 8;
            info.rect = Rect { x: 0, y: 0, width: w, height: h };
            info.available_rect = info.rect.clone();
            1
        }

        fn on_ime_composition_range_changed(&self, _browser: Option<&mut Browser>, _range: Option<&Range>, bounds: Option<&[Rect]>) {
            // the candidate window goes next to the text being composed
            let Some(r) = bounds.and_then(|b| b.first()) else { return };
            UI.with_borrow(|u| if let Some(u) = u {
                u.im.set_cursor_location(&gdk::Rectangle::new(r.x, r.y, r.width, r.height));
            });
        }

        fn on_popup_show(&self, _browser: Option<&mut Browser>, show: ::std::os::raw::c_int) {
            UI.with_borrow(|u| if let Some(u) = u {
                u.popup.set_visible(show != 0);
                if show == 0 { u.popup.set_paintable(None::<&gdk::Paintable>); }
            });
        }

        fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&Rect>) {
            let Some(r) = rect else { return };
            UI.with_borrow(|u| if let Some(u) = u {
                u.fixed.move_(&u.popup, r.x as f64, r.y as f64);
                u.popup.set_size_request(r.width, r.height);
            });
        }

        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            let len = (width * height * 4) as usize;
            let bytes = glib::Bytes::from(unsafe { std::slice::from_raw_parts(buffer, len) });
            let t = gdk::MemoryTexture::new(width, height, gdk::MemoryFormat::B8g8r8a8Premultiplied, &bytes, (width * 4) as usize);
            show(type_, t.upcast_ref());
        }

        fn on_accelerated_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            info: Option<&AcceleratedPaintInfo>,
        ) {
            let Some(info) = info else { return };
            match dmabuf_texture(info) {
                Ok(t) if mode() == Mode::GpuNoCopy => show(type_, &t),
                Ok(t) => match gpu_copy(&t) {
                    Some(c) => show(type_, &c),
                    None => eprintln!("s10 no renderer yet"),
                },
                Err(e) => eprintln!("s10 dmabuf import failed: {e}"),
            }
        }
    }
}

// ------------------------------------------------------------------ client

wrap_client! {
    struct NovaClient { osr: bool }
    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            self.osr.then(Osr::new)
        }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(NovaLifeSpan::new())
        }
        fn display_handler(&self) -> Option<DisplayHandler> {
            self.osr.then(NovaDisplay::new)
        }
    }
}

wrap_display_handler! {
    struct NovaDisplay;
    impl DisplayHandler {
        fn on_cursor_change(
            &self,
            _browser: Option<&mut Browser>,
            _cursor: ::std::os::raw::c_ulong,
            type_: CursorType,
            _custom_cursor_info: Option<&CursorInfo>,
        ) -> ::std::os::raw::c_int {
            // shortcut: common shapes only; a port maps all of CursorType (and custom images)
            let name = match type_ {
                CursorType::HAND => "pointer",
                CursorType::IBEAM => "text",
                CursorType::WAIT => "wait",
                CursorType::PROGRESS => "progress",
                CursorType::CROSS => "crosshair",
                CursorType::NOTALLOWED => "not-allowed",
                _ => "default",
            };
            UI.with_borrow(|u| if let Some(u) = u { u.page.set_cursor_from_name(Some(name)) });
            1
        }
    }
}

wrap_life_span_handler! {
    struct NovaLifeSpan;
    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let host = browser.and_then(|b| b.host());
            if let Some(h) = &host {
                h.set_focus(1);
                h.was_resized(); // the window may have its size before the browser existed
            }
            HOST.with_borrow_mut(|x| *x = host);
        }
        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            HOST.with_borrow_mut(|x| *x = None);
            match MAIN_LOOP.with_borrow(|l| l.clone()) {
                Some(l) => l.quit(),
                None => quit_message_loop(),
            }
        }
    }
}

fn browser_settings() -> BrowserSettings {
    BrowserSettings {
        windowless_frame_rate: 60,
        background_color: 0xffff_ffff,
        ..Default::default()
    }
}

/// CEF renders a frame when GTK's frame clock (the display's vsync) ticks.
fn vsync() -> bool {
    std::env::var("VSYNC").as_deref() != Ok("0")
}

fn start_url() -> CefString {
    s(&std::env::var("URL").unwrap_or_else(|_| "https://example.com/".into()))
}

fn create_osr_browser() {
    let info = WindowInfo {
        windowless_rendering_enabled: 1,
        shared_texture_enabled: (mode() != Mode::Cpu) as i32,
        // VSYNC=0: CEF's own 60 Hz timer, not in step with the display (judder)
        external_begin_frame_enabled: vsync() as i32,
        runtime_style: RuntimeStyle::ALLOY,
        ..Default::default()
    };
    let ok = browser_host_create_browser(
        Some(&info),
        Some(&mut NovaClient::new(true)),
        Some(&start_url()),
        Some(&browser_settings()),
        None,
        None,
    );
    assert_eq!(ok, 1, "browser_host_create_browser");
}

// --------------------------------------------------------- windowed baseline

wrap_browser_view_delegate! {
    struct NovaBrowserViewDelegate {}
    impl ViewDelegate {}
    impl BrowserViewDelegate {
        fn browser_runtime_style(&self) -> RuntimeStyle { RuntimeStyle::ALLOY }
    }
}

wrap_window_delegate! {
    struct NovaWindowDelegate {}
    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size { Size { width: 1280, height: 800 } }
    }
    impl PanelDelegate {}
    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let window = window.unwrap();
            let view = browser_view_create(
                Some(&mut NovaClient::new(false)),
                Some(&start_url()),
                Some(&browser_settings()),
                None,
                None,
                Some(&mut NovaBrowserViewDelegate::new()),
            )
            .expect("browser_view_create");
            window.add_child_view(Some(&mut View::from(&view)));
            window.show();
            view.request_focus();
        }
        fn window_runtime_style(&self) -> RuntimeStyle { RuntimeStyle::ALLOY }
    }
}

// ------------------------------------------------------------------ process

/// External message pump: CEF asks for work from any thread; run it on GTK's main loop.
fn schedule_pump(delay_ms: i64) {
    let (n, z) = PUMPS.get();
    PUMPS.set((n + 1, z + (delay_ms <= 0) as u32));
    if let Some(id) = PUMP.with_borrow_mut(Option::take) {
        id.remove();
    }
    let id = glib::timeout_add_local_once(Duration::from_millis(delay_ms.max(0) as u64), || {
        PUMP.with_borrow_mut(|p| *p = None);
        do_message_loop_work();
    });
    PUMP.with_borrow_mut(|p| *p = Some(id));
}

wrap_browser_process_handler! {
    struct NovaBrowserProcess;
    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            if mode() == Mode::Windowed {
                window_create_top_level(Some(&mut NovaWindowDelegate::new()));
            } else {
                create_osr_browser();
            }
        }
        fn on_schedule_message_pump_work(&self, delay_ms: i64) {
            glib::MainContext::default().invoke(move || schedule_pump(delay_ms));
        }
    }
}

wrap_app! {
    struct NovaApp;
    impl App {
        fn on_before_command_line_processing(&self, process_type: Option<&CefString>, command_line: Option<&mut CommandLine>) {
            let (Some(cmd), true) = (command_line, process_type.is_none_or(|t| t.to_string().is_empty())) else { return };
            let p = if std::env::var_os("WAYLAND_DISPLAY").is_some() { "wayland" } else { "x11" };
            if cmd.has_switch(Some(&s("ozone-platform"))) == 0 {
                cmd.append_switch_with_value(Some(&s("ozone-platform")), Some(&s(p)));
            }
            // Off-screen, the page's scale is ours (screen_info); with this feature on,
            // Chromium's Wayland code overrides it with 1 (spike: devicePixelRatio stayed 1)
            if mode() != Mode::Windowed {
                cmd.append_switch_with_value(Some(&s("disable-features")), Some(&s("WaylandFractionalScaleV1")));
            }
            // GTK4 is already in this process: Chromium must not load GTK3 next to it
            cmd.append_switch_with_value(Some(&s("gtk-version")), Some(&s("4")));
            let port = std::env::var("CDP_PORT").unwrap_or_else(|_| "9310".into());
            cmd.append_switch_with_value(Some(&s("remote-debugging-port")), Some(&s(&port)));
        }
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(NovaBrowserProcess::new())
        }
    }
}

// ---------------------------------------------------------------------- GTK

fn modifiers(state: gdk::ModifierType) -> u32 {
    let mut m = 0;
    if state.contains(gdk::ModifierType::SHIFT_MASK) { m |= 1 << 1; }
    if state.contains(gdk::ModifierType::CONTROL_MASK) { m |= 1 << 2; }
    if state.contains(gdk::ModifierType::ALT_MASK) { m |= 1 << 3; }
    if state.contains(gdk::ModifierType::BUTTON1_MASK) { m |= 1 << 4; }
    if state.contains(gdk::ModifierType::BUTTON2_MASK) { m |= 1 << 5; }
    if state.contains(gdk::ModifierType::BUTTON3_MASK) { m |= 1 << 6; }
    m
}

/// Windows virtual key code (what Chromium keys on); shortcut: common keys only, a real
/// port needs the full table (ui/events/keycodes/keyboard_code_conversion_xkb.cc).
fn vk(key: gdk::Key) -> i32 {
    use gdk::Key as K;
    match key {
        K::Return | K::KP_Enter => 0x0D,
        K::BackSpace => 0x08,
        K::Tab => 0x09,
        K::Escape => 0x1B,
        K::space => 0x20,
        K::Page_Up => 0x21,
        K::Page_Down => 0x22,
        K::End => 0x23,
        K::Home => 0x24,
        K::Left => 0x25,
        K::Up => 0x26,
        K::Right => 0x27,
        K::Down => 0x28,
        K::Delete => 0x2E,
        _ => key
            .to_unicode()
            .map(|c| c.to_ascii_uppercase())
            .filter(char::is_ascii_alphanumeric)
            .map_or(0, |c| c as i32),
    }
}

fn send_key(key: gdk::Key, code: u32, state: gdk::ModifierType, down: bool) {
    let Some(h) = host() else { return };
    let mods = modifiers(state);
    let ch = key.to_unicode().map_or(0, |c| c as u32 as u16);
    let ev = KeyEvent {
        type_: if down { KeyEventType::RAWKEYDOWN } else { KeyEventType::KEYUP },
        modifiers: mods,
        windows_key_code: vk(key),
        native_key_code: code as i32,
        character: ch,
        unmodified_character: ch,
        ..Default::default()
    };
    h.send_key_event(Some(&ev));
}

/// Typed text: a CHAR event (what the page sees as input after its keydown).
fn send_char(c: char, mods: u32) {
    let Some(h) = host() else { return };
    let mut buf = [0u16; 2];
    for &u in c.encode_utf16(&mut buf).iter() {
        h.send_key_event(Some(&KeyEvent {
            type_: KeyEventType::CHAR,
            modifiers: mods,
            windows_key_code: u as i32,
            character: u,
            unmodified_character: u,
            ..Default::default()
        }));
    }
}

fn im_wire(im: &gtk::IMMulticontext) {
    im.connect_commit(|_, text| {
        let mut it = text.chars();
        match (IN_KEY.get() && !COMPOSING.get(), it.next(), it.next()) {
            // plain typing: the key's own keydown went out already
            (true, Some(c), None) => send_char(c, 0),
            _ => {
                if let Some(h) = host() {
                    h.ime_commit_text(Some(&s(text)), Some(&NO_RANGE), 0);
                }
            }
        }
        COMPOSING.set(false);
    });
    im.connect_preedit_changed(|im| {
        let Some(h) = host() else { return };
        let (text, _, cursor) = im.preedit_string();
        if text.is_empty() {
            if COMPOSING.replace(false) {
                h.ime_cancel_composition();
            }
            return;
        }
        COMPOSING.set(true);
        let len = text.encode_utf16().count() as u32;
        let underline = CompositionUnderline {
            range: Range { from: 0, to: len },
            color: 0xff00_0000,
            style: CompositionUnderlineStyle::SOLID,
            ..Default::default()
        };
        let caret = text.chars().take(cursor.max(0) as usize).map(char::len_utf16).sum::<usize>() as u32;
        h.ime_set_composition(Some(&s(&text)), Some(&[underline]), Some(&NO_RANGE), Some(&Range { from: caret, to: caret }));
    });
}

fn mouse(x: f64, y: f64, state: gdk::ModifierType) -> MouseEvent {
    MouseEvent { x: x as i32, y: y as i32, modifiers: modifiers(state) }
}

fn build_ui() {
    let css = gtk::CssProvider::new();
    css.load_from_string(
        ".card { border-radius: 18px; box-shadow: 0 12px 40px alpha(black, .35); }
         .backdrop { filter: blur(24px) saturate(1.5); }
         .tint { background: alpha(#1c1c1e, .45); }
         .card label { color: white; font: 15px sans-serif; }",
    );
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("display"),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let window = gtk::Window::builder().title("s10 osr").default_width(1280).default_height(800).build();
    let overlay = gtk::Overlay::new();
    let page = gtk::Picture::new();
    page.set_content_fit(gtk::ContentFit::Fill);
    page.set_can_shrink(true);
    page.set_hexpand(true);
    page.set_vexpand(true);
    overlay.set_child(Some(&page));

    // page popups (<select> lists) at the rect CEF gives
    let fixed = gtk::Fixed::new();
    fixed.set_can_target(false);
    let popup = gtk::Picture::new();
    popup.set_content_fit(gtk::ContentFit::Fill);
    popup.set_can_shrink(true);
    popup.set_visible(false);
    fixed.put(&popup, 0.0, 0.0);
    overlay.add_overlay(&fixed);

    let backdrop = std::env::var_os("CARD").map(|_| {
        let (x, y, w, h) = CARD;
        // the card: a rounded clip (a scrolled window with no scrollbars, scrolled to the
        // card's spot) over a blurred copy of the whole page, a tint, then its text
        let clip = gtk::ScrolledWindow::new();
        clip.set_policy(gtk::PolicyType::External, gtk::PolicyType::External);
        clip.add_css_class("card");
        clip.set_overflow(gtk::Overflow::Hidden);
        let bd = gtk::Picture::new();
        bd.add_css_class("backdrop");
        bd.set_content_fit(gtk::ContentFit::Fill);
        bd.set_can_shrink(true);
        clip.set_child(Some(&bd));
        clip.hadjustment().connect_changed(move |a| a.set_value(x as f64));
        clip.vadjustment().connect_changed(move |a| a.set_value(y as f64));
        let card = gtk::Overlay::new();
        card.set_child(Some(&clip));
        let tint = gtk::Box::new(gtk::Orientation::Vertical, 0);
        tint.add_css_class("tint");
        tint.add_css_class("card");
        card.add_overlay(&tint);
        let label = gtk::Label::new(Some("Translucent GTK card over a CEF off-screen page"));
        label.set_halign(gtk::Align::Start);
        label.set_valign(gtk::Align::Start);
        label.set_margin_start(22);
        label.set_margin_top(22);
        card.add_overlay(&label);
        card.set_halign(gtk::Align::Start);
        card.set_valign(gtk::Align::Start);
        card.set_margin_start(x);
        card.set_margin_top(y);
        card.set_size_request(w, h);
        card.set_can_target(false);
        overlay.add_overlay(&card);
        bd
    });

    // input: everything to CEF
    let motion = gtk::EventControllerMotion::new();
    motion.connect_motion(|c, x, y| {
        POINTER.set((x, y));
        if let Some(h) = host() {
            h.send_mouse_move_event(Some(&mouse(x, y, c.current_event_state())), 0);
        }
    });
    motion.connect_leave(|c| {
        let (x, y) = POINTER.get();
        if let Some(h) = host() {
            h.send_mouse_move_event(Some(&mouse(x, y, c.current_event_state())), 1);
        }
    });
    overlay.add_controller(motion);
    let click = gtk::GestureClick::new();
    click.set_button(0);
    let button = |g: &gtk::GestureClick| match g.current_button() {
        2 => MouseButtonType::MIDDLE,
        3 => MouseButtonType::RIGHT,
        _ => MouseButtonType::LEFT,
    };
    click.connect_pressed(move |g, n, x, y| {
        if let Some(h) = host() {
            h.set_focus(1);
            h.send_mouse_click_event(Some(&mouse(x, y, g.current_event_state())), button(g), 0, n);
        }
    });
    click.connect_released(move |g, n, x, y| {
        if let Some(h) = host() {
            h.send_mouse_click_event(Some(&mouse(x, y, g.current_event_state())), button(g), 1, n);
        }
    });
    overlay.add_controller(click);
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    scroll.connect_scroll(|c, dx, dy| {
        // a wheel notch is 1.0 here; Chromium wants pixels (~100 a notch, like Chrome)
        let k = if c.unit() == gdk::ScrollUnit::Wheel { -100.0 } else { -1.0 };
        let (x, y) = POINTER.get();
        if let Some(h) = host() {
            h.send_mouse_wheel_event(Some(&mouse(x, y, c.current_event_state())), (dx * k) as i32, (dy * k) as i32);
        }
        glib::Propagation::Stop
    });
    overlay.add_controller(scroll);
    // keys: keydown to CEF first, then through the input method, which commits the text
    // (one char: a CHAR event) or composes it (preedit: CEF's IME calls)
    let im = gtk::IMMulticontext::new();
    im.set_client_widget(Some(&overlay));
    im_wire(&im);
    let keys = gtk::EventControllerKey::new();
    let im2 = im.clone();
    keys.connect_key_pressed(move |c, key, code, state| {
        send_key(key, code, state, true);
        IN_KEY.set(true);
        let filtered = c.current_event().is_some_and(|e| im2.filter_keypress(&e));
        IN_KEY.set(false);
        let mods = modifiers(state);
        if let (false, Some(ch), 0) = (filtered, key.to_unicode(), mods & (1 << 2 | 1 << 3)) {
            send_char(ch, mods);
        }
        glib::Propagation::Stop
    });
    let im2 = im.clone();
    keys.connect_key_released(move |c, key, code, state| {
        if let Some(e) = c.current_event() {
            im2.filter_keypress(&e);
        }
        send_key(key, code, state, false)
    });
    window.add_controller(keys);
    let im2 = im.clone();
    window.connect_is_active_notify(move |w| {
        if let Some(h) = host() {
            h.set_focus(w.is_active() as i32);
        }
        if w.is_active() { im2.focus_in() } else { im2.focus_out() }
    });
    window.connect_close_request(|_| {
        match host() {
            Some(h) => h.close_browser(1),
            None => MAIN_LOOP.with_borrow(|l| l.as_ref().map(|l| l.quit())).unwrap_or(()),
        }
        glib::Propagation::Stop
    });
    window.set_child(Some(&overlay));
    if vsync() {
        // shortcut: ticks every frame, also when the page is still; a port pauses the
        // clock while nothing on the page moves
        page.add_tick_callback(|_, _| {
            if let Some(h) = host() {
                h.send_external_begin_frame();
            }
            glib::ControlFlow::Continue
        });
    }
    window.connect_realize(|w| {
        // fractional scale (wp-fractional-scale): CEF re-renders at the new density
        w.surface().expect("surface").connect_scale_notify(|s| {
            eprintln!("s10 scale {}", s.scale());
            if let Some(h) = host() {
                h.notify_screen_info_changed();
                h.was_resized();
            }
        });
        // count frames GTK draws; a size change tells CEF to re-layout
        w.frame_clock().expect("frame clock").connect_after_paint(|_| {
            FRAMES.set(FRAMES.get() + 1);
            let size = UI.with_borrow(|u| u.as_ref().map(|u| (u.page.width(), u.page.height())));
            if let Some(size) = size.filter(|&s| s != SIZE.get() && s.0 > 0) {
                SIZE.set(size);
                UI.with_borrow(|u| {
                    if let (Some(u), Some(bd)) = (u, u.as_ref().and_then(|u| u.backdrop.as_ref())) {
                        let _ = u;
                        bd.set_size_request(size.0, size.1);
                    }
                });
                eprintln!("s10 view {}x{} scale {}", size.0, size.1, scale());
                if let Some(h) = host() {
                    h.was_resized();
                }
            }
        });
    });
    window.present();
    UI.with_borrow_mut(|u| *u = Some(Ui { window, page, backdrop, fixed, popup, im }));
}

fn main() {
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = args::Args::new();
    let mut app = NovaApp::new();
    let code = execute_process(Some(args.as_main_args()), Some(&mut app), std::ptr::null_mut());
    if code >= 0 {
        std::process::exit(code);
    }
    let m = mode();
    eprintln!("s10 mode {m:?}");
    let osr = m != Mode::Windowed;
    if osr {
        gtk::init().expect("gtk init");
        build_ui();
    }
    // PUMP=external: drive CEF from GTK's loop (do_message_loop_work). Default: CEF's own
    // loop, which on Linux is a glib loop on the default context, so GTK runs inside it.
    let external = osr && std::env::var("PUMP").as_deref() == Ok("external");
    let settings = Settings {
        root_cache_path: s("/tmp/novabrowser-s10"),
        windowless_rendering_enabled: osr as i32,
        external_message_pump: external as i32,
        ..Default::default()
    };
    assert_eq!(initialize(Some(args.as_main_args()), Some(&settings), Some(&mut app), std::ptr::null_mut()), 1);
    if std::env::var_os("IMETEST").is_some() {
        // CEF's IME side without an input method: compose "nihon", then commit 日本
        glib::timeout_add_local_once(Duration::from_secs(5), || {
            let Some(h) = host() else { return };
            let ul = CompositionUnderline { range: Range { from: 0, to: 5 }, color: 0xff00_0000, style: CompositionUnderlineStyle::SOLID, ..Default::default() };
            h.ime_set_composition(Some(&s("nihon")), Some(&[ul]), Some(&NO_RANGE), Some(&Range { from: 5, to: 5 }));
            eprintln!("s10 ime composing");
            glib::timeout_add_local_once(Duration::from_secs(2), || {
                if let Some(h) = host() {
                    h.ime_commit_text(Some(&s("日本")), Some(&NO_RANGE), 0);
                    eprintln!("s10 ime committed");
                }
            });
        });
    }
    if osr {
        glib::timeout_add_local(Duration::from_secs(1), || {
            eprintln!("s10 paints={} frames={} pumps={:?}", PAINTS.replace(0), FRAMES.replace(0), PUMPS.replace((0, 0)));
            glib::ControlFlow::Continue
        });
    }
    if external {
        let main_loop = glib::MainLoop::new(None, false);
        MAIN_LOOP.with_borrow_mut(|l| *l = Some(main_loop.clone()));
        // CEF's own examples also pump at least every 1/30 s
        glib::timeout_add_local(Duration::from_millis(33), || {
            do_message_loop_work();
            glib::ControlFlow::Continue
        });
        main_loop.run();
    } else {
        run_message_loop();
    }
    shutdown();
}
