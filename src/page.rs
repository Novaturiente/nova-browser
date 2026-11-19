//! A tab's page: a windowless (off-screen) CEF browser drawn into a GTK widget.
//! CEF paints into a GPU buffer (dmabuf); we copy it into a texture GTK owns and show
//! it. Mouse, wheel and keys that modes.rs passes on go to the browser; text goes
//! through a GTK input method (compose, CJK) to CEF's IME calls. Spike: S10.
//!
//! Each page has an id (`pid`) that its client, render and life-span handlers carry,
//! so CEF callbacks find their page before the browser exists.
use cef::*;
use gtk::{gdk, glib, graphene, gsk, prelude::*};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::rc::{Rc, Weak};

pub struct Inner {
    pid: u64,
    /// the page area: the picture, with `<select>` popups on a layer above
    widget: gtk::Overlay,
    picture: gtk::Picture,
    layer: gtk::Fixed,
    popup: gtk::Picture,
    im: gtk::IMMulticontext,
    browser: RefCell<Option<Browser>>,
    /// URL to load when the browser is created (first show); None once created
    pending: RefCell<Option<String>>,
    /// last size told to CEF, in logical px
    size: Cell<(i32, i32)>,
    pointer: Cell<(f64, f64)>,
    /// inside the input method's filter for a key press: a one-char commit is typing
    in_key: Cell<bool>,
    composing: Cell<bool>,
}

#[derive(Clone)]
pub struct Page(Rc<Inner>);

thread_local! {
    static NEXT: Cell<u64> = const { Cell::new(0) };
    static PAGES: RefCell<HashMap<u64, Weak<Inner>>> = RefCell::new(HashMap::new());
    /// popups and DevTools CEF is creating: pid -> (opener, foreground); they become tabs
    static ADOPT: RefCell<HashMap<u64, (Browser, bool)>> = RefCell::new(HashMap::new());
}

/// CefRange::InvalidRange: "no replacement"
const NO_RANGE: Range = Range { from: u32::MAX, to: u32::MAX };

fn page(pid: u64) -> Option<Page> {
    PAGES.with_borrow(|p| p.get(&pid)?.upgrade().map(Page))
}

/// The page showing `browser`.
pub fn of(browser: &Browser) -> Option<Page> {
    let id = browser.identifier();
    PAGES.with_borrow(|p| {
        p.values()
            .filter_map(Weak::upgrade)
            .find(|i| i.browser.borrow().as_ref().is_some_and(|b| b.identifier() == id))
            .map(Page)
    })
}

impl Page {
    /// A page that loads `url` when first shown (`ensure`).
    pub fn new(url: &str) -> Page {
        let p = Page::blank();
        p.0.pending.replace(Some(url.to_string()));
        p
    }

    fn blank() -> Page {
        let pid = NEXT.get() + 1;
        NEXT.set(pid);
        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Fill);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        let widget = gtk::Overlay::new();
        widget.set_child(Some(&picture));
        widget.set_focusable(true);
        widget.set_can_focus(true);
        let layer = gtk::Fixed::new();
        layer.set_can_target(false);
        let popup = gtk::Picture::new();
        popup.set_content_fit(gtk::ContentFit::Fill);
        popup.set_can_shrink(true);
        popup.set_visible(false);
        layer.put(&popup, 0.0, 0.0);
        widget.add_overlay(&layer);
        let im = gtk::IMMulticontext::new();
        im.set_client_widget(Some(&widget));
        let inner = Rc::new(Inner {
            pid,
            widget,
            picture,
            layer,
            popup,
            im,
            browser: RefCell::new(None),
            pending: RefCell::new(None),
            size: Cell::new((0, 0)),
            pointer: Cell::new((0.0, 0.0)),
            in_key: Cell::new(false),
            composing: Cell::new(false),
        });
        PAGES.with_borrow_mut(|p| {
            p.retain(|_, w| w.strong_count() > 0);
            p.insert(pid, Rc::downgrade(&inner));
        });
        let page = Page(inner);
        page.wire();
        page
    }

    pub fn widget(&self) -> gtk::Widget {
        self.0.widget.clone().upcast()
    }

    pub fn browser(&self) -> Option<Browser> {
        self.0.browser.borrow().clone()
    }

    fn host(&self) -> Option<BrowserHost> {
        self.browser()?.host()
    }

    /// Create the browser if this page has none yet (tabs load lazily: first show).
    pub fn ensure(&self, settings: &BrowserSettings) {
        let Some(url) = self.0.pending.take() else { return };
        let info = window_info();
        let ok = browser_host_create_browser(
            Some(&info),
            Some(&mut crate::client::TabClient::new(self.0.pid)),
            Some(&CefString::from(url.as_str())),
            Some(settings),
            None,
            None,
        );
        if ok != 1 {
            eprintln!("novabrowser: could not create a browser for {url}");
        }
    }

    pub fn focus(&self) {
        self.0.widget.grab_focus();
    }

    // ---- input

    fn wire(&self) {
        let w = &self.0.widget;
        let weak = Rc::downgrade(&self.0);
        let me = move || weak.upgrade().map(Page);
        let m = me.clone();
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(move |c, x, y| {
            let Some(p) = m() else { return };
            p.0.pointer.set((x, y));
            if let Some(h) = p.host() {
                h.send_mouse_move_event(Some(&mouse(x, y, c.current_event_state())), 0);
            }
        });
        let m = me.clone();
        motion.connect_leave(move |c| {
            let Some(p) = m() else { return };
            let (x, y) = p.0.pointer.get();
            if let Some(h) = p.host() {
                h.send_mouse_move_event(Some(&mouse(x, y, c.current_event_state())), 1);
            }
        });
        w.add_controller(motion);

        let click = gtk::GestureClick::new();
        click.set_button(0);
        let button = |g: &gtk::GestureClick| match g.current_button() {
            2 => Some(MouseButtonType::MIDDLE),
            3 => Some(MouseButtonType::RIGHT),
            1 => Some(MouseButtonType::LEFT),
            _ => None, // side buttons: the renderer turns them into back/forward
        };
        let m = me.clone();
        click.connect_pressed(move |g, n, x, y| {
            let (Some(p), Some(b)) = (m(), button(g)) else { return };
            p.focus();
            if let Some(h) = p.host() {
                h.send_mouse_click_event(Some(&mouse(x, y, g.current_event_state())), b, 0, n);
            }
        });
        let m = me.clone();
        click.connect_released(move |g, n, x, y| {
            let (Some(p), Some(b)) = (m(), button(g)) else { return };
            if let Some(h) = p.host() {
                h.send_mouse_click_event(Some(&mouse(x, y, g.current_event_state())), b, 1, n);
            }
        });
        w.add_controller(click);

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let m = me.clone();
        scroll.connect_scroll(move |c, dx, dy| {
            let Some(p) = m() else { return glib::Propagation::Proceed };
            // a wheel notch is 1.0 here; Chromium wants pixels (Chrome: 100 a notch)
            let k = if c.unit() == gdk::ScrollUnit::Wheel { -100.0 } else { -1.0 };
            let (x, y) = p.0.pointer.get();
            if let Some(h) = p.host() {
                h.send_mouse_wheel_event(Some(&mouse(x, y, c.current_event_state())), (dx * k) as i32, (dy * k) as i32);
            }
            glib::Propagation::Stop
        });
        w.add_controller(scroll);

        // keys modes.rs passed on: keydown to CEF, then the input method, which commits
        // text (one char: a CHAR event) or composes it (CEF's IME calls)
        let keys = gtk::EventControllerKey::new();
        let m = me.clone();
        keys.connect_key_pressed(move |c, key, code, state| {
            let Some(p) = m() else { return glib::Propagation::Proceed };
            p.send_key(key, code, state, true);
            p.0.in_key.set(true);
            let filtered = c.current_event().is_some_and(|e| p.0.im.filter_keypress(&e));
            p.0.in_key.set(false);
            let mods = modifiers(state);
            if let (false, Some(ch), 0) = (filtered, key.to_unicode(), mods & (CTRL | ALT)) {
                p.send_char(ch, mods);
            }
            glib::Propagation::Stop
        });
        let m = me.clone();
        keys.connect_key_released(move |c, key, code, state| {
            let Some(p) = m() else { return };
            if let Some(e) = c.current_event() {
                p.0.im.filter_keypress(&e);
            }
            p.send_key(key, code, state, false);
        });
        w.add_controller(keys);

        let m = me.clone();
        self.0.im.connect_commit(move |_, text| {
            let Some(p) = m() else { return };
            let mut it = text.chars();
            match (p.0.in_key.get() && !p.0.composing.get(), it.next(), it.next()) {
                (true, Some(c), None) => p.send_char(c, 0),
                _ => {
                    if let Some(h) = p.host() {
                        h.ime_commit_text(Some(&CefString::from(text)), Some(&NO_RANGE), 0);
                    }
                }
            }
            p.0.composing.set(false);
        });
        let m = me.clone();
        self.0.im.connect_preedit_changed(move |im| {
            let Some(p) = m() else { return };
            let Some(h) = p.host() else { return };
            let (text, _, cursor) = im.preedit_string();
            if text.is_empty() {
                if p.0.composing.replace(false) {
                    h.ime_cancel_composition();
                }
                return;
            }
            p.0.composing.set(true);
            let len = text.encode_utf16().count() as u32;
            let underline = CompositionUnderline {
                range: Range { from: 0, to: len },
                color: 0xff00_0000,
                style: CompositionUnderlineStyle::SOLID,
                ..Default::default()
            };
            let caret = text.chars().take(cursor.max(0) as usize).map(char::len_utf16).sum::<usize>() as u32;
            h.ime_set_composition(
                Some(&CefString::from(text.as_str())),
                Some(&[underline]),
                Some(&NO_RANGE),
                Some(&Range { from: caret, to: caret }),
            );
        });

        let focus = gtk::EventControllerFocus::new();
        let m = me.clone();
        focus.connect_enter(move |_| {
            let Some(p) = m() else { return };
            p.0.im.focus_in();
            if let Some(h) = p.host() {
                h.set_focus(1);
            }
        });
        let m = me.clone();
        focus.connect_leave(move |_| {
            let Some(p) = m() else { return };
            p.0.im.focus_out();
            if let Some(h) = p.host() {
                h.set_focus(0);
            }
        });
        w.add_controller(focus);

        // a new size (window resize, tab bar moved, fractional scale): CEF re-renders
        let m = me;
        w.connect_realize(move |w| {
            let m = m.clone();
            if let Some(clock) = w.frame_clock() {
                clock.connect_after_paint(move |_| {
                    if let Some(p) = m() {
                        p.check_size();
                    }
                });
            }
        });
    }

    fn check_size(&self) {
        let w = &self.0.widget;
        let size = (w.width(), w.height());
        if size.0 <= 0 || size == self.0.size.get() {
            return;
        }
        self.0.size.set(size);
        if let Some(h) = self.host() {
            h.notify_screen_info_changed();
            h.was_resized();
        }
    }

    fn send_key(&self, key: gdk::Key, code: u32, state: gdk::ModifierType, down: bool) {
        let Some(h) = self.host() else { return };
        let ch = key.to_unicode().map_or(0, |c| c as u32 as u16);
        h.send_key_event(Some(&KeyEvent {
            type_: if down { KeyEventType::RAWKEYDOWN } else { KeyEventType::KEYUP },
            modifiers: modifiers(state),
            windows_key_code: crate::keys_vk::vk(key),
            native_key_code: code as i32,
            character: ch,
            unmodified_character: ch,
            ..Default::default()
        }));
    }

    /// Typed text: a CHAR event (the page's input after its keydown).
    fn send_char(&self, c: char, mods: u32) {
        let Some(h) = self.host() else { return };
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

    /// Logical size for CEF; before the first allocation, the window's page area.
    fn view_size(&self) -> (i32, i32) {
        let (w, h) = self.0.size.get();
        if w > 0 && h > 0 {
            return (w, h);
        }
        crate::window::content_size_px().unwrap_or((1200, 760))
    }

    /// The surface's scale, fractional too (1.5 at 150 %).
    fn scale(&self) -> f64 {
        self.0.widget.native().and_then(|n| n.surface()).map_or(1.0, |s| s.scale())
    }

    fn show_frame(&self, kind: PaintElementType, t: &gdk::Texture) {
        if kind == PaintElementType::POPUP {
            self.0.popup.set_paintable(Some(t));
        } else {
            self.0.picture.set_paintable(Some(t));
        }
    }

    /// Copy CEF's buffer into a texture GTK owns (CEF reuses the buffer after the callback).
    fn gpu_copy(&self, t: &gdk::Texture) -> gdk::Texture {
        let renderer = self.0.widget.native().and_then(|n| n.renderer());
        match renderer {
            Some(r) => {
                let rect = graphene::Rect::new(0.0, 0.0, t.width() as f32, t.height() as f32);
                r.render_texture(gsk::TextureNode::new(t, &rect), None)
            }
            // not on screen yet: GTK can't copy it, so take the pixels now
            None => gdk::MemoryTexture::new(
                t.width(),
                t.height(),
                gdk::MemoryFormat::B8g8r8a8Premultiplied,
                &gdk::TextureDownloader::new(t).download_bytes().0,
                t.width() as usize * 4,
            )
            .upcast(),
        }
    }
}

fn window_info() -> WindowInfo {
    WindowInfo {
        windowless_rendering_enabled: 1,
        shared_texture_enabled: 1,
        runtime_style: RuntimeStyle::ALLOY,
        ..Default::default()
    }
}

// ---- popups and DevTools: CEF creates the browser, it becomes a tab

/// `on_before_popup`: make the popup windowless with a page of its own; it becomes a
/// tab of `opener`'s window in `created`.
pub fn popup(opener: &Browser, foreground: bool, info: &mut WindowInfo, client: &mut Option<Client>) {
    let p = Page::blank();
    *info = WindowInfo { size: info.size, ..window_info() };
    *client = Some(crate::client::TabClient::new(p.0.pid));
    ADOPT.with_borrow_mut(|a| a.insert(p.0.pid, (opener.clone(), foreground)));
    // the page lives in PAGES (weak) until adopted: keep it alive meanwhile
    KEEP.with_borrow_mut(|k| k.push(p));
}

/// DevTools for `browser`, in a new tab next to it.
pub fn devtools(browser: &Browser) {
    let Some(h) = browser.host() else { return };
    let p = Page::blank();
    ADOPT.with_borrow_mut(|a| a.insert(p.0.pid, (browser.clone(), true)));
    let pid = p.0.pid;
    KEEP.with_borrow_mut(|k| k.push(p));
    h.show_dev_tools(
        Some(&window_info()),
        Some(&mut crate::client::TabClient::new(pid)),
        Some(&BrowserSettings { windowless_frame_rate: 60, ..Default::default() }),
        None,
    );
}

thread_local! {
    static KEEP: RefCell<Vec<Page>> = const { RefCell::new(Vec::new()) };
}

/// `on_after_created` for the page `pid`: it has its browser now.
pub fn created(pid: u64, browser: &Browser) {
    let Some(p) = page(pid) else { return };
    p.0.browser.replace(Some(browser.clone()));
    if let Some(h) = browser.host() {
        h.was_resized();
        if p.0.widget.has_focus() {
            h.set_focus(1);
        }
    }
    let adopt = ADOPT.with_borrow_mut(|a| a.remove(&pid));
    KEEP.with_borrow_mut(|k| k.retain(|x| x.0.pid != pid));
    if let Some((opener, foreground)) = adopt {
        crate::window::in_owner(&opener, || crate::window::adopt_popup(p, foreground));
    }
}

/// The page's right-click menu (CEF's own needs a native window, which off-screen pages
/// don't have): `model`'s items (ours from client.rs TabMenu, then Chromium's) as a GTK
/// popover at the mouse. `cb` gets the pick, or cancel when it closes without one.
/// shortcut: submenus (spelling suggestions' "Language settings") are left out.
pub fn context_menu(browser: &Browser, x: i32, y: i32, model: &MenuModel, cb: &RunContextMenuCallback) -> bool {
    let Some(p) = of(browser) else { return false };
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let pop = gtk::Popover::new();
    pop.set_has_arrow(false);
    pop.set_position(gtk::PositionType::Bottom);
    pop.add_css_class("menu");
    let cb = Rc::new(RefCell::new(Some(cb.clone())));
    let mut sep = false;
    for i in 0..model.count() {
        if model.is_visible_at(i) == 0 || model.type_at(i) == MenuItemType::SUBMENU {
            continue;
        }
        if model.type_at(i) == MenuItemType::SEPARATOR {
            sep = list.first_child().is_some();
            continue;
        }
        if std::mem::take(&mut sep) {
            list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        }
        // "&Back": a mnemonic marker; "&&" is a literal &
        let label = CefString::from(&model.label_at(i)).to_string().replace("&&", "\0").replace('&', "").replace('\0', "&");
        let b = gtk::Button::with_label(&label);
        b.add_css_class("flat");
        if let Some(l) = b.child().and_downcast::<gtk::Label>() {
            l.set_xalign(0.0);
        }
        b.set_sensitive(model.is_enabled_at(i) != 0);
        let (id, cb, pop) = (model.command_id_at(i), cb.clone(), pop.clone());
        b.connect_clicked(move |_| {
            if let Some(c) = cb.take() {
                c.cont(id, EventFlags::default());
            }
            pop.popdown();
        });
        list.append(&b);
    }
    if list.first_child().is_none() {
        return false;
    }
    pop.set_child(Some(&list));
    pop.set_parent(&p.0.widget);
    pop.set_pointing_to(Some(&gdk::Rectangle::new(x, y, 1, 1)));
    pop.connect_closed(move |pop| {
        if let Some(c) = cb.take() {
            c.cancel();
        }
        let pop = pop.clone();
        glib::idle_add_local_once(move || pop.unparent());
    });
    pop.popup();
    true
}

/// Mouse cursor over `browser`'s page.
pub fn set_cursor(browser: &Browser, kind: CursorType) {
    let Some(p) = of(browser) else { return };
    let name = match kind {
        CursorType::HAND => "pointer",
        CursorType::IBEAM | CursorType::VERTICALTEXT => "text",
        CursorType::WAIT => "wait",
        CursorType::PROGRESS => "progress",
        CursorType::CROSS => "crosshair",
        CursorType::HELP => "help",
        CursorType::MOVE => "move",
        CursorType::NOTALLOWED | CursorType::NODROP => "not-allowed",
        CursorType::COPY => "copy",
        CursorType::ALIAS => "alias",
        CursorType::CELL => "cell",
        CursorType::CONTEXTMENU => "context-menu",
        CursorType::GRAB => "grab",
        CursorType::GRABBING => "grabbing",
        CursorType::ZOOMIN => "zoom-in",
        CursorType::ZOOMOUT => "zoom-out",
        CursorType::NONE => "none",
        CursorType::COLUMNRESIZE | CursorType::EASTWESTRESIZE => "ew-resize",
        CursorType::ROWRESIZE | CursorType::NORTHSOUTHRESIZE => "ns-resize",
        CursorType::EASTRESIZE => "e-resize",
        CursorType::WESTRESIZE => "w-resize",
        CursorType::NORTHRESIZE => "n-resize",
        CursorType::SOUTHRESIZE => "s-resize",
        CursorType::NORTHEASTRESIZE => "ne-resize",
        CursorType::NORTHWESTRESIZE => "nw-resize",
        CursorType::SOUTHEASTRESIZE => "se-resize",
        CursorType::SOUTHWESTRESIZE => "sw-resize",
        CursorType::NORTHEASTSOUTHWESTRESIZE => "nesw-resize",
        CursorType::NORTHWESTSOUTHEASTRESIZE => "nwse-resize",
        CursorType::MIDDLEPANNING => "all-scroll",
        _ => "default",
    };
    p.0.picture.set_cursor_from_name(Some(name));
}

// ---- rendering

/// Wrap CEF's dmabuf planes in a GdkDmabufTexture (fds dup'ed: CEF closes its own).
fn dmabuf_texture(info: &AcceleratedPaintInfo) -> Result<gdk::Texture, String> {
    // DRM fourcc: BGRA bytes = ARGB8888 ("AR24"), RGBA bytes = ABGR8888 ("AB24")
    let fourcc = if info.format == ColorType::BGRA_8888 { 0x3432_5241 } else { 0x3432_4241 };
    let (w, h) = (info.extra.coded_size.width, info.extra.coded_size.height);
    let planes = &info.planes[..info.plane_count.clamp(0, 4) as usize];
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
        .set_n_planes(planes.len() as u32);
    for (i, (p, fd)) in planes.iter().zip(&fds).enumerate() {
        b = unsafe { b.set_fd(i as u32, fd.as_raw_fd()) }
            .set_stride(i as u32, p.stride)
            .set_offset(i as u32, p.offset as u32);
    }
    unsafe { b.build_with_release_func(move || drop(fds)) }.map_err(|e| e.to_string())
}

wrap_render_handler! {
    pub struct PageRender { pid: u64 }
    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            let (w, h) = page(self.pid).map_or((1200, 760), |p| p.view_size());
            if let Some(r) = rect {
                r.x = 0;
                r.y = 0;
                r.width = w.max(1);
                r.height = h.max(1);
            }
        }

        fn screen_info(&self, _browser: Option<&mut Browser>, info: Option<&mut ScreenInfo>) -> ::std::os::raw::c_int {
            let (Some(info), Some(p)) = (info, page(self.pid)) else { return 0 };
            let (w, h) = p.view_size();
            info.device_scale_factor = p.scale() as f32;
            info.depth = 24;
            info.depth_per_component = 8;
            info.rect = Rect { x: 0, y: 0, width: w.max(1), height: h.max(1) };
            info.available_rect = info.rect.clone();
            1
        }

        fn on_ime_composition_range_changed(&self, _browser: Option<&mut Browser>, _range: Option<&Range>, bounds: Option<&[Rect]>) {
            // the input method's candidate window goes next to the text being composed
            let (Some(r), Some(p)) = (bounds.and_then(|b| b.first()), page(self.pid)) else { return };
            p.0.im.set_cursor_location(&gdk::Rectangle::new(r.x, r.y, r.width, r.height));
        }

        fn on_popup_show(&self, _browser: Option<&mut Browser>, show: ::std::os::raw::c_int) {
            let Some(p) = page(self.pid) else { return };
            p.0.popup.set_visible(show != 0);
            if show == 0 {
                p.0.popup.set_paintable(None::<&gdk::Paintable>);
            }
        }

        fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&Rect>) {
            let (Some(r), Some(p)) = (rect, page(self.pid)) else { return };
            p.0.layer.move_(&p.0.popup, r.x as f64, r.y as f64);
            p.0.popup.set_size_request(r.width, r.height);
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
            // only without GPU buffers (shared textures failed): BGRA in memory
            let Some(p) = page(self.pid) else { return };
            let len = (width * height * 4) as usize;
            let bytes = glib::Bytes::from(unsafe { std::slice::from_raw_parts(buffer, len) });
            let t = gdk::MemoryTexture::new(width, height, gdk::MemoryFormat::B8g8r8a8Premultiplied, &bytes, (width * 4) as usize);
            p.show_frame(type_, t.upcast_ref());
        }

        fn on_accelerated_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            info: Option<&AcceleratedPaintInfo>,
        ) {
            let (Some(info), Some(p)) = (info, page(self.pid)) else { return };
            match dmabuf_texture(info) {
                Ok(t) => {
                    let t = p.gpu_copy(&t);
                    p.show_frame(type_, &t);
                }
                Err(e) => eprintln!("novabrowser: page frame import failed: {e}"),
            }
        }
    }
}

// ---- key and mouse event helpers

const SHIFT: u32 = 1 << 1;
const CTRL: u32 = 1 << 2;
const ALT: u32 = 1 << 3;

/// CEF event flags from GTK's modifier state.
fn modifiers(state: gdk::ModifierType) -> u32 {
    use gdk::ModifierType as M;
    [
        (M::SHIFT_MASK, SHIFT),
        (M::CONTROL_MASK, CTRL),
        (M::ALT_MASK, ALT),
        (M::BUTTON1_MASK, 1 << 4),
        (M::BUTTON2_MASK, 1 << 5),
        (M::BUTTON3_MASK, 1 << 6),
        (M::SUPER_MASK, 1 << 7),
        (M::META_MASK, 1 << 7),
    ]
    .into_iter()
    .filter(|(m, _)| state.contains(*m))
    .fold(0, |a, (_, f)| a | f)
}

fn mouse(x: f64, y: f64, state: gdk::ModifierType) -> MouseEvent {
    MouseEvent { x: x as i32, y: y as i32, modifiers: modifiers(state) }
}
