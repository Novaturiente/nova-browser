//! Tab bar: pill tabs in a strip that lays them out itself from springs (open, close,
//! reorder, live drag), then the profile pill, status pills (keys, downloads, mode) and
//! the load progress line. Same look and classes as the old `ui/tabbar.html`.
use super::motion;
use super::{Ev, Sink, Status, TabItem};
use crate::config;
use adw::prelude::*;
use gtk::glib;
use gtk::graphene::{Point, Rect};
use gtk::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

/// Gap between pills and around the strip (tabbar.html: `gap: 4px; padding: 4px`).
const GAP: f64 = 4.0;
/// Pointer travel before a press on a tab becomes a drag.
const DRAG_START: f64 = 4.0;

pub const CSS: &str = "
window.nova .nova-tabbar { background: var(--colors-tabs-bar-bg, #555); }
window.nova .nova-tabbar .t {
  border-radius: 999px; padding: var(--tabs-padding, 0 5px); padding-left: 10px; padding-right: 6px;
  font: var(--fonts-tabs-unselected, 10pt monospace);
  color: var(--colors-tabs-odd-fg);
  background: color-mix(in srgb, var(--colors-tabs-selected-odd-bg) 35%, var(--colors-tabs-odd-bg));
  transition: background 140ms ease, box-shadow 180ms ease;
}
window.nova .nova-tabbar .t.even { color: var(--colors-tabs-even-fg); }
window.nova .nova-tabbar .t:hover {
  background: color-mix(in srgb, var(--colors-tabs-selected-odd-bg) 70%, var(--colors-tabs-odd-bg));
}
window.nova .nova-tabbar .t.a {
  font: var(--fonts-tabs-selected, 10pt monospace);
  color: var(--colors-tabs-selected-odd-fg); background: var(--colors-tabs-selected-odd-bg);
}
window.nova .nova-tabbar .t.a.even {
  color: var(--colors-tabs-selected-even-fg); background: var(--colors-tabs-selected-even-bg);
}
window.nova .nova-tabbar .t.e { box-shadow: inset 0 0 0 2px var(--colors-tabs-indicator-error); }
window.nova .nova-tabbar .t.z { opacity: .5; }
window.nova .nova-tabbar .t.drag { box-shadow: 0 6px 18px rgba(0,0,0,.45); }
window.nova .nova-tabbar .t.circle { padding: 0; }
window.nova .nova-tabbar .t image { margin-right: 5px; }
window.nova .nova-tabbar .t.circle image { margin: 0; }
window.nova .nova-tabbar .t .sp { color: var(--colors-tabs-indicator-start); margin-right: 6px; }
window.nova .nova-tabbar .t.circle .sp { margin: 0; }
window.nova .nova-tabbar .t button.x {
  min-width: 16px; min-height: 16px; padding: 0; margin-left: 4px; border-radius: 999px;
  border: none; box-shadow: none; background: none; color: inherit; opacity: 0;
  transition: opacity 140ms ease, background 140ms ease;
}
window.nova .nova-tabbar .t:hover button.x { opacity: .65; }
window.nova .nova-tabbar .t button.x:hover { opacity: 1; background: alpha(currentColor, .16); }
window.nova .nova-tabbar .profile {
  margin: 0 6px; padding: 1px 10px; border-radius: 999px;
  font: var(--fonts-tabs-selected, 10pt monospace); font-weight: bold;
  background: var(--colors-tabs-indicator-start, #48f); color: var(--colors-tabs-bar-bg, #000);
}
window.nova .nova-tabbar .info { padding: 0 8px; }
window.nova .nova-tabbar .info label {
  padding: 1px 9px; border-radius: 999px; font: var(--fonts-statusbar, 10pt monospace);
  color: var(--colors-statusbar-normal-fg); background: var(--colors-statusbar-normal-bg);
  transition: background 160ms ease, color 160ms ease;
}
window.nova .nova-tabbar .info label.mode.insert { color: var(--colors-statusbar-insert-fg); background: var(--colors-statusbar-insert-bg); }
window.nova .nova-tabbar .info label.mode.passthrough { color: var(--colors-statusbar-passthrough-fg); background: var(--colors-statusbar-passthrough-bg); }
window.nova .nova-tabbar .info label.mode.command { color: var(--colors-statusbar-command-fg); background: var(--colors-statusbar-command-bg); }
window.nova .nova-tabbar .info label.dl { font-feature-settings: \"tnum\"; }
window.nova .nova-tabbar .info label.dl.run { color: var(--colors-downloads-start-fg, inherit); background: var(--colors-downloads-start-bg, inherit); }
window.nova .nova-tabbar .info label.dl.done { color: var(--colors-downloads-stop-fg, inherit); background: var(--colors-downloads-stop-bg, inherit); }
window.nova .nova-tabbar .info label.dl.fail { color: var(--colors-downloads-error-fg, inherit); background: var(--colors-downloads-error-bg, inherit); }
window.nova .nova-tabbar .progress { background: var(--colors-statusbar-progress-bg); min-height: 2px; }
";

/// Main-axis (start, length) of each tab, in order: horizontal pills share the width
/// equally within [min, max]; `circle` tabs (pinned, not active) are `circle` long.
fn slots(circles: &[bool], avail: f64, circle: f64, min: f64, max: f64) -> Vec<(f64, f64)> {
    let n = circles.len() as f64;
    let nc = circles.iter().filter(|c| **c).count() as f64;
    let free = (avail - GAP * (n + 1.0) - circle * nc).max(0.0);
    let w = if n > nc { (free / (n - nc)).min(max).max(min) } else { 0.0 };
    let mut x = GAP;
    circles
        .iter()
        .map(|c| {
            let s = if *c { circle } else { w };
            let r = (x, s);
            x += s + GAP;
            r
        })
        .collect()
}

/// Where a dragged tab lands: its index among the tabs of its own group (pinned or not)
/// by the dragged tab's centre. `centres`: the other tabs' centres in order.
fn drop_index(centres: &[(f64, bool)], centre: f64, pinned: bool) -> usize {
    let first = centres.iter().position(|c| c.1 == pinned);
    let before = centres.iter().filter(|c| c.1 == pinned && c.0 < centre).count();
    match first {
        Some(f) => f + before,
        // only tab of its group: pinned go first, others after the pinned
        None if pinned => 0,
        None => centres.len(),
    }
}

/// One tab's widgets and its animated geometry (main-axis start, length; opacity).
struct TabW {
    root: gtk::Box,
    icon: gtk::Image,
    spin: adw::Spinner,
    label: gtk::Label,
    close: gtk::Button,
    item: RefCell<TabItem>,
    classes: RefCell<Vec<&'static str>>,
    icon_src: RefCell<Option<gtk::gdk::Texture>>,
    cur: Cell<(f64, f64, f64)>,
    from: Cell<(f64, f64, f64)>,
    to: Cell<(f64, f64, f64)>,
    anim: RefCell<Option<adw::SpringAnimation>>,
}

struct Drag {
    key: u64,
    from: usize,
    /// main-axis start of the tab when the drag began
    start: f64,
    active: bool,
    target: usize,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Strip {
        pub(super) order: RefCell<Vec<u64>>,
        pub(super) tabs: RefCell<HashMap<u64, Rc<TabW>>>,
        pub(super) vertical: Cell<bool>,
        pub(super) size: Cell<(i32, i32)>,
        pub(super) layout_key: RefCell<String>,
        pub(super) offset: Cell<f64>,
        pub(super) offset_anim: RefCell<Option<adw::SpringAnimation>>,
        pub(super) drag: RefCell<Option<Drag>>,
        pub(super) hover: Cell<Option<u64>>,
        pub(super) tip_timer: RefCell<Option<glib::SourceId>>,
        pub(super) tip_shown: Cell<bool>,
        pub(super) sink: RefCell<Option<Sink>>,
        /// the tab bar's root, for event coordinates
        pub(super) bar: RefCell<Option<gtk::Widget>>,
        pub(super) applied: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Strip {
        const NAME: &'static str = "NovaTabStrip";
        type Type = super::Strip;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Strip {
        fn dispose(&self) {
            for t in self.tabs.borrow().values() {
                t.root.unparent();
            }
        }
    }

    impl WidgetImpl for Strip {
        fn measure(&self, o: gtk::Orientation, _for: i32) -> (i32, i32, i32, i32) {
            let row = self.obj().row_size();
            match (self.vertical.get(), o) {
                (false, gtk::Orientation::Vertical) => (0, (row + 2.0 * GAP) as i32, -1, -1),
                (true, gtk::Orientation::Vertical) => {
                    let n = self.order.borrow().len() as f64;
                    (0, (n * (row + GAP) + GAP) as i32, -1, -1)
                }
                _ => (0, 0, -1, -1),
            }
        }

        fn size_allocate(&self, w: i32, h: i32, _baseline: i32) {
            let obj = self.obj();
            if self.size.replace((w, h)) != (w, h) {
                // a resize snaps; only tab changes animate
                obj.relayout(false);
            }
            let off = self.offset.get();
            for t in self.tabs.borrow().values() {
                let (pos, len, alpha) = t.cur.get();
                t.root.set_opacity(alpha.clamp(0.0, 1.0));
                let tr;
                let (aw, ah);
                if self.vertical.get() {
                    (aw, ah) = ((w as f64 - 12.0).max(1.0) as i32, len.max(1.0) as i32);
                    tr = Point::new(6.0, (pos - off) as f32);
                } else {
                    // never below the tab's own minimum; the snapshot clips to `len`
                    let min = t.root.measure(gtk::Orientation::Horizontal, -1).0;
                    (aw, ah) = ((len as i32).max(min), (h as f64 - 2.0 * GAP).max(1.0) as i32);
                    tr = Point::new((pos - off) as f32, GAP as f32);
                }
                t.root.allocate(aw, ah, -1, Some(gtk::gsk::Transform::new().translate(&tr)));
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let (w, h) = self.size.get();
            let off = self.offset.get();
            let dragged = self.drag.borrow().as_ref().filter(|d| d.active).map(|d| d.key);
            let tabs = self.tabs.borrow();
            let mut list: Vec<(&u64, &Rc<TabW>)> = tabs.iter().collect();
            // the dragged tab draws on top
            list.sort_by_key(|(k, _)| Some(**k) == dragged);
            for (_, t) in list {
                let (pos, len, _) = t.cur.get();
                if len < 1.0 {
                    continue;
                }
                let r = if self.vertical.get() {
                    Rect::new(0.0, (pos - off) as f32, w as f32, len as f32)
                } else {
                    // room for the lifted tab's shadow above and below
                    Rect::new((pos - off) as f32, 0.0, len as f32, h as f32)
                };
                snapshot.push_clip(&r);
                obj.snapshot_child(&t.root, snapshot);
                snapshot.pop();
            }
        }
    }
}

glib::wrapper! {
    pub struct Strip(ObjectSubclass<imp::Strip>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Strip {
    fn new(sink: Sink) -> Self {
        let s: Self = glib::Object::new();
        s.set_overflow(gtk::Overflow::Hidden);
        s.imp().sink.replace(Some(sink));
        s.attach_input();
        s
    }

    fn send(&self, ev: Ev) {
        let sink = self.imp().sink.borrow().clone();
        if let Some(s) = sink {
            s(ev);
        }
    }

    /// Natural cross size of a tab row (height of a pill).
    fn row_size(&self) -> f64 {
        let tabs = self.imp().tabs.borrow();
        let h = tabs.values().map(|t| t.root.measure(gtk::Orientation::Vertical, -1).1).max();
        h.unwrap_or(20) as f64
    }

    fn tab(&self, key: u64) -> Option<Rc<TabW>> {
        self.imp().tabs.borrow().get(&key).cloned()
    }

    fn index_of(&self, key: u64) -> Option<usize> {
        self.imp().order.borrow().iter().position(|k| *k == key)
    }

    /// Animate tab `t` to (pos, len, alpha); `done` after it settles.
    fn move_tab(&self, t: &Rc<TabW>, to: (f64, f64, f64), animate: bool, done: Option<Box<dyn Fn()>>) {
        if let Some(a) = t.anim.take() {
            a.pause();
        }
        if !animate || t.cur.get() == to {
            t.cur.set(to);
            t.to.set(to);
            self.queue_allocate();
            if let Some(d) = done {
                d();
            }
            return;
        }
        t.from.set(t.cur.get());
        t.to.set(to);
        let (me, tw) = (self.downgrade(), Rc::downgrade(t));
        let params = if t.from.get().1 < 1.0 { motion::snappy() } else { motion::layout() };
        let a = motion::spring_then(self, 0.0, 1.0, params, move |v| {
            let (Some(s), Some(t)) = (me.upgrade(), tw.upgrade()) else { return };
            let (f, g) = (t.from.get(), t.to.get());
            let l = |a: f64, b: f64| a + (b - a) * v;
            t.cur.set((l(f.0, g.0), l(f.1, g.1).max(0.0), l(f.2, g.2)));
            s.queue_allocate();
        }, move || {
            if let Some(d) = &done {
                d();
            }
        });
        t.anim.replace(Some(a));
    }

    /// Order to lay out: the live order, with a dragged tab moved to its drop index.
    fn layout_order(&self) -> Vec<u64> {
        let imp = self.imp();
        let mut order = imp.order.borrow().clone();
        if let Some(d) = imp.drag.borrow().as_ref().filter(|d| d.active) {
            if let Some(i) = order.iter().position(|k| *k == d.key) {
                order.remove(i);
                order.insert(d.target.min(order.len()), d.key);
            }
        }
        order
    }

    /// Targets for every live tab (and the scroll offset), from the current size.
    fn relayout(&self, animate: bool) {
        let imp = self.imp();
        let (w, h) = imp.size.get();
        if w <= 0 {
            return;
        }
        let order = self.layout_order();
        let vertical = imp.vertical.get();
        let items: Vec<(u64, Rc<TabW>)> = order.iter().filter_map(|k| Some((*k, self.tab(*k)?))).collect();
        let place: Vec<(f64, f64)> = if vertical {
            let row = self.row_size();
            (0..items.len()).map(|i| (GAP / 2.0 + i as f64 * (row + GAP), row)).collect()
        } else {
            let cfg = config::get();
            let px = |k: &str, none: f64| match cfg.int(k) {
                n if n < 0 => none,
                n => n as f64,
            };
            let circles: Vec<bool> = items.iter().map(|(_, t)| is_circle(&t.item.borrow(), false)).collect();
            let circle = (h as f64 - 2.0 * GAP).max(1.0);
            slots(&circles, w as f64, circle, px("tabs.min_width", 0.0).min(w as f64), px("tabs.max_width", f64::INFINITY))
        };
        let dragged = imp.drag.borrow().as_ref().filter(|d| d.active).map(|d| d.key);
        for ((k, t), s) in items.iter().zip(&place) {
            if Some(*k) != dragged {
                self.move_tab(t, (s.0, s.1, 1.0), animate, None);
            }
        }
        // keep the active tab in view (tabbar.html: scrollIntoView nearest)
        let avail = if vertical { h } else { w } as f64;
        let end = place.last().map_or(0.0, |s| s.0 + s.1 + GAP);
        let active = items.iter().position(|(_, t)| t.item.borrow().active);
        let mut off = imp.offset.get();
        if end <= avail {
            off = 0.0;
        } else if let Some(s) = active.and_then(|i| place.get(i)) {
            off = off.min(s.0 - GAP).max(s.0 + s.1 + GAP - avail);
        }
        off = off.clamp(0.0, (end - avail).max(0.0));
        self.scroll_to(off, animate);
    }

    fn scroll_to(&self, off: f64, animate: bool) {
        let imp = self.imp();
        if let Some(a) = imp.offset_anim.take() {
            a.pause();
        }
        let from = imp.offset.get();
        if !animate || (from - off).abs() < 0.5 {
            imp.offset.set(off);
            return self.queue_allocate();
        }
        let me = self.downgrade();
        let a = motion::spring(self, from, off, motion::layout(), move |v| {
            if let Some(s) = me.upgrade() {
                s.imp().offset.set(v);
                s.queue_allocate();
            }
        });
        imp.offset_anim.replace(Some(a));
    }

    fn set_vertical(&self, v: bool) {
        if self.imp().vertical.replace(v) != v {
            self.imp().layout_key.replace(String::new());
            self.queue_resize();
            self.relayout(false);
        }
    }

    fn apply(&self, items: &[TabItem]) {
        let imp = self.imp();
        let first = !imp.applied.replace(true);
        let vertical = imp.vertical.get();
        let mut added = Vec::new();
        let xalign = match config::get().str("tabs.title.alignment").as_str() {
            "center" => 0.5,
            "right" => 1.0,
            _ => 0.0,
        };
        for (i, it) in items.iter().enumerate() {
            let t = match self.tab(it.key) {
                Some(t) => t,
                None => {
                    let t = self.new_tab(it.key);
                    imp.tabs.borrow_mut().insert(it.key, t.clone());
                    added.push(it.key);
                    t
                }
            };
            update_tab(&t, it, i, vertical, xalign);
        }
        // gone: shrink out where they stand, then drop
        let live: Vec<u64> = items.iter().map(|t| t.key).collect();
        let gone: Vec<u64> = imp
            .order
            .borrow()
            .iter()
            .filter(|k| !live.contains(k))
            .copied()
            .collect();
        imp.order.replace(live.clone());
        for k in gone {
            let Some(t) = self.tab(k) else { continue };
            // a shrinking tab's × must not catch clicks meant for the neighbour sliding in
            t.root.set_can_target(false);
            let (pos, len, _) = t.cur.get();
            let me = self.downgrade();
            let to = if vertical { (pos, len, 0.0) } else { (pos + len / 2.0, 0.0, 0.0) };
            self.move_tab(&t, to, !first, Some(Box::new(move || {
                if let Some(s) = me.upgrade() {
                    let t = s.imp().tabs.borrow_mut().remove(&k);
                    if let Some(t) = t {
                        t.root.unparent();
                    }
                }
            })));
        }
        let key = items
            .iter()
            // the active tab too: switching scrolls it into view
            .map(|t| format!("{}:{}:{}", t.key, is_circle(t, vertical), t.active))
            .collect::<Vec<_>>()
            .join(",");
        let changed = imp.layout_key.replace(key.clone()) != key;
        if changed {
            self.relayout(!first);
            // new tabs start collapsed at their slot and grow in
            for t in added.iter().filter_map(|k| self.tab(*k)).filter(|_| !first) {
                let to = t.to.get();
                t.cur.set(if vertical { (to.0, to.1, 0.0) } else { (to.0 + to.1 / 2.0, 0.0, 0.0) });
                self.move_tab(&t, to, true, None);
            }
        }
    }

    fn new_tab(&self, key: u64) -> Rc<TabW> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("t");
        let icon = gtk::Image::new();
        icon.set_pixel_size(16);
        let spin = adw::Spinner::new();
        spin.add_css_class("sp");
        spin.set_size_request(12, 12);
        spin.set_valign(gtk::Align::Center);
        let label = gtk::Label::new(None);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_hexpand(true);
        label.set_width_chars(1);
        let close = gtk::Button::with_label("×");
        close.add_css_class("x");
        close.set_valign(gtk::Align::Center);
        close.set_focusable(false);
        let me = self.downgrade();
        close.connect_clicked(move |_| {
            if let Some(s) = me.upgrade() {
                if let Some(i) = s.index_of(key) {
                    s.send(Ev::Close(i));
                }
            }
        });
        for w in [icon.upcast_ref::<gtk::Widget>(), spin.upcast_ref(), label.upcast_ref(), close.upcast_ref()] {
            root.append(w);
        }
        root.set_parent(self);
        Rc::new(TabW {
            root,
            icon,
            spin,
            label,
            close,
            item: RefCell::new(TabItem { key, ..Default::default() }),
            classes: RefCell::new(Vec::new()),
            icon_src: RefCell::new(None),
            cur: Cell::new((0.0, 0.0, 1.0)),
            from: Cell::new((0.0, 0.0, 1.0)),
            to: Cell::new((0.0, 0.0, 1.0)),
            anim: RefCell::new(None),
        })
    }

    /// Live tab under main-axis point `p` (strip coordinates): (index, key).
    fn tab_at(&self, x: f64, y: f64) -> Option<(usize, u64)> {
        let p = if self.imp().vertical.get() { y } else { x } + self.imp().offset.get();
        let order = self.imp().order.borrow().clone();
        order.iter().enumerate().find_map(|(i, k)| {
            let (pos, len, _) = self.tab(*k)?.cur.get();
            (p >= pos && p < pos + len).then_some((i, *k))
        })
    }

    /// Is (x, y) on a tab's close button?
    fn on_close_button(&self, x: f64, y: f64) -> bool {
        let mut w = self.pick(x, y, gtk::PickFlags::DEFAULT);
        while let Some(x) = w {
            if x.is::<gtk::Button>() {
                return true;
            }
            if &x == self.upcast_ref::<gtk::Widget>() {
                break;
            }
            w = x.parent();
        }
        false
    }

    /// Tab `key`'s (left, bottom, right) in the tab bar root's coordinates.
    fn rect_in_bar(&self, key: u64) -> Option<(i32, i32, i32)> {
        let (pos, len, _) = self.tab(key)?.cur.get();
        let off = self.imp().offset.get();
        let (w, h) = self.imp().size.get();
        let (l, t, r, b) = if self.imp().vertical.get() {
            (0.0, pos - off, w as f64, pos - off + len)
        } else {
            (pos - off, 0.0, pos - off + len, h as f64 - GAP)
        };
        let bar = self.imp().bar.borrow().clone()?;
        let p1 = self.compute_point(&bar, &Point::new(l as f32, t as f32))?;
        let p2 = self.compute_point(&bar, &Point::new(r as f32, b as f32))?;
        Some((p1.x() as i32, p2.y() as i32, p2.x() as i32))
    }

    fn hide_tip(&self) {
        let imp = self.imp();
        if let Some(id) = imp.tip_timer.take() {
            id.remove();
        }
        if imp.tip_shown.replace(false) {
            self.send(Ev::Tip(None));
        }
    }

    fn attach_input(&self) {
        let click = gtk::GestureClick::new();
        click.set_button(0);
        let me = self.downgrade();
        click.connect_pressed(move |g, _, x, y| {
            let Some(s) = me.upgrade() else { return };
            s.hide_tip();
            if s.on_close_button(x, y) {
                return;
            }
            let Some((i, key)) = s.tab_at(x, y) else { return };
            match g.current_button() {
                1 => s.send(Ev::Tab(i)),
                2 => s.send(Ev::Close(i)),
                3 => {
                    let Some(bar) = s.imp().bar.borrow().clone() else { return };
                    let px = s.compute_point(&bar, &Point::new(x as f32, y as f32)).map_or(0, |p| p.x() as i32);
                    if let Some((_, bottom, _)) = s.rect_in_bar(key) {
                        s.send(Ev::Menu(i, px, bottom));
                    }
                }
                _ => {}
            }
        });
        self.add_controller(click);

        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        let me = self.downgrade();
        drag.connect_drag_begin(move |g, x, y| {
            let Some(s) = me.upgrade() else { return };
            let hit = (!s.on_close_button(x, y)).then(|| s.tab_at(x, y)).flatten();
            let Some((i, key)) = hit else {
                g.set_state(gtk::EventSequenceState::Denied);
                return;
            };
            let start = s.tab(key).map_or(0.0, |t| t.cur.get().0);
            s.imp().drag.replace(Some(Drag { key, from: i, start, active: false, target: i }));
        });
        let me = self.downgrade();
        drag.connect_drag_update(move |_, dx, dy| {
            let Some(s) = me.upgrade() else { return };
            s.drag_to(if s.imp().vertical.get() { dy } else { dx });
        });
        let me = self.downgrade();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(s) = me.upgrade() {
                s.drop_drag(true);
            }
        });
        let me = self.downgrade();
        drag.connect_cancel(move |_, _| {
            if let Some(s) = me.upgrade() {
                s.drop_drag(false);
            }
        });
        self.add_controller(drag);

        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let me = self.downgrade();
        wheel.connect_scroll(move |c, dx, dy| {
            let Some(s) = me.upgrade() else { return glib::Propagation::Proceed };
            let step = if c.unit() == gtk::gdk::ScrollUnit::Wheel { 40.0 } else { 1.0 };
            let imp = s.imp();
            let (w, h) = imp.size.get();
            let avail = if imp.vertical.get() { h } else { w } as f64;
            let end = imp.tabs.borrow().values().map(|t| t.to.get().0 + t.to.get().1 + GAP).fold(0.0, f64::max);
            let off = (imp.offset.get() + (dx + dy) * step).clamp(0.0, (end - avail).max(0.0));
            s.scroll_to(off, false);
            glib::Propagation::Stop
        });
        self.add_controller(wheel);

        let motion_c = gtk::EventControllerMotion::new();
        let me = self.downgrade();
        motion_c.connect_motion(move |_, x, y| {
            let Some(s) = me.upgrade() else { return };
            let imp = s.imp();
            if imp.drag.borrow().as_ref().is_some_and(|d| d.active) {
                return;
            }
            let key = s.tab_at(x, y).map(|t| t.1);
            if imp.hover.replace(key) == key {
                return;
            }
            s.hide_tip();
            let Some(key) = key else { return };
            let me = s.downgrade();
            let id = glib::timeout_add_local_once(Duration::from_millis(450), move || {
                let Some(s) = me.upgrade() else { return };
                s.imp().tip_timer.take();
                if let (Some(i), Some((l, b, r))) = (s.index_of(key), s.rect_in_bar(key)) {
                    s.imp().tip_shown.set(true);
                    s.send(Ev::Tip(Some((i, l, b, r))));
                }
            });
            imp.tip_timer.replace(Some(id));
        });
        let me = self.downgrade();
        motion_c.connect_leave(move |_| {
            if let Some(s) = me.upgrade() {
                s.imp().hover.set(None);
                s.hide_tip();
            }
        });
        self.add_controller(motion_c);
    }

    /// The pointer moved `d` along the strip since the press on a tab.
    fn drag_to(&self, d: f64) {
        let imp = self.imp();
        let (key, start, was_active) = match imp.drag.borrow().as_ref() {
            Some(g) => (g.key, g.start, g.active),
            None => return,
        };
        let Some(t) = self.tab(key) else { return };
        if !was_active {
            if d.abs() < DRAG_START {
                return;
            }
            if let Some(g) = imp.drag.borrow_mut().as_mut() {
                g.active = true;
            }
            self.hide_tip();
            t.root.add_css_class("drag");
            if let Some(a) = t.anim.take() {
                a.pause();
            }
        }
        let (_, len, _) = t.cur.get();
        let (w, h) = imp.size.get();
        let avail = if imp.vertical.get() { h } else { w } as f64;
        let end = self.layout_order().iter().filter_map(|k| self.tab(*k)).map(|t| t.to.get().0 + t.to.get().1).fold(0.0, f64::max);
        let pos = (start + d).clamp(0.0, (end.max(avail) - len).max(0.0));
        t.cur.set((pos, len, 1.0));
        // drop index among the others, by centre, within its group
        let pinned = t.item.borrow().pinned;
        let centres: Vec<(f64, bool)> = imp
            .order
            .borrow()
            .iter()
            .filter(|k| **k != key)
            .filter_map(|k| {
                let o = self.tab(*k)?;
                let (p, l, _) = o.to.get();
                let pinned = o.item.borrow().pinned;
                Some((p + l / 2.0, pinned))
            })
            .collect();
        let target = drop_index(&centres, pos + len / 2.0, pinned);
        let moved = imp.drag.borrow_mut().as_mut().is_some_and(|g| std::mem::replace(&mut g.target, target) != target);
        if moved {
            self.relayout(true);
        }
        self.queue_allocate();
    }

    /// Release (`commit`) or cancel a drag: the tab springs into its slot.
    fn drop_drag(&self, commit: bool) {
        let imp = self.imp();
        let Some(d) = imp.drag.take() else { return };
        if !d.active {
            return;
        }
        if let Some(t) = self.tab(d.key) {
            t.root.remove_css_class("drag");
        }
        let target = d.target.min(imp.order.borrow().len().saturating_sub(1));
        if commit && target != d.from {
            // reorder here first so the window's refresh finds nothing left to move
            {
                // by key: a tab may have closed during the drag
                let mut o = imp.order.borrow_mut();
                if let Some(from) = o.iter().position(|k| *k == d.key) {
                    o.remove(from);
                    let at = target.min(o.len());
                    o.insert(at, d.key);
                }
            }
            self.relayout(true);
            self.send(Ev::Move(target));
        } else {
            self.relayout(true);
        }
    }
}

/// A pinned tab that isn't the active one shows as an icon-only circle (horizontal bar).
fn is_circle(t: &TabItem, vertical: bool) -> bool {
    t.pinned && !t.active && !vertical
}

fn update_tab(t: &TabW, it: &TabItem, index: usize, vertical: bool, xalign: f32) {
    let circle = is_circle(it, vertical);
    let mut classes = vec!["t"];
    for (on, c) in [
        (it.active, "a"),
        (index % 2 == 1, "even"),
        (it.crashed, "e"),
        (it.asleep, "z"),
        (it.pinned, "n"),
        (circle, "circle"),
    ] {
        if on {
            classes.push(c);
        }
    }
    if t.root.has_css_class("drag") {
        classes.push("drag");
    }
    if *t.classes.borrow() != classes {
        t.root.set_css_classes(&classes);
        t.classes.replace(classes);
    }
    if *t.icon_src.borrow() != it.icon {
        t.icon.set_paintable(it.icon.as_ref());
        t.icon_src.replace(it.icon.clone());
    }
    let mut old = t.item.borrow_mut();
    if old.label != it.label {
        t.label.set_label(&it.label);
    }
    let has_icon = it.icon.is_some();
    t.spin.set_visible(it.loading && !it.crashed);
    t.icon.set_visible(has_icon && !(it.loading && !it.crashed));
    // circles: icon (or spinner) only, centred; nothing to show: the clipped label (HTML)
    t.label.set_visible(!circle || !(has_icon || it.loading));
    t.close.set_visible(!it.pinned);
    t.icon.set_hexpand(circle);
    t.spin.set_hexpand(circle);
    t.label.set_xalign(xalign);
    *old = it.clone();
}

/// The tab bar: profile pill, tab strip, status pills, progress line.
#[derive(Clone)]
pub struct TabBar(Rc<Inner>);

struct Inner {
    root: gtk::Overlay,
    row: gtk::Box,
    strip: Strip,
    profile: gtk::Label,
    info: gtk::Box,
    keys: gtk::Label,
    dl: gtk::Label,
    mode: gtk::Label,
    progress: gtk::Box,
    progress_now: Cell<f64>,
    progress_anim: RefCell<Option<adw::SpringAnimation>>,
    status: RefCell<Status>,
}

impl TabBar {
    pub fn new(sink: Sink) -> Self {
        let root = gtk::Overlay::new();
        root.add_css_class("nova-tabbar");
        root.set_focusable(true);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.set_child(Some(&row));
        let profile = gtk::Label::new(None);
        profile.add_css_class("profile");
        profile.set_valign(gtk::Align::Center);
        profile.set_visible(false);
        let strip = Strip::new(sink.clone());
        strip.imp().bar.replace(Some(root.clone().upcast()));
        strip.set_hexpand(true);
        let info = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        info.add_css_class("info");
        info.set_valign(gtk::Align::Center);
        let pill = |c: &str| {
            let l = gtk::Label::new(None);
            l.add_css_class(c);
            l.set_visible(false);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            info.append(&l);
            l
        };
        let keys = pill("keys");
        let dl = pill("dl");
        let mode = pill("mode");
        let click = gtk::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| sink(Ev::Downloads));
        dl.add_controller(click);
        dl.set_cursor_from_name(Some("pointer"));
        row.append(&profile);
        row.append(&strip);
        row.append(&info);
        let progress = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        progress.add_css_class("progress");
        progress.set_halign(gtk::Align::Start);
        progress.set_valign(gtk::Align::End);
        progress.set_can_target(false);
        progress.set_visible(false);
        root.add_overlay(&progress);
        TabBar(Rc::new(Inner {
            root,
            row,
            strip,
            profile,
            info,
            keys,
            dl,
            mode,
            progress,
            progress_now: Cell::new(0.0),
            progress_anim: RefCell::new(None),
            status: RefCell::new(Status::default()),
        }))
    }

    pub fn widget(&self) -> gtk::Widget {
        self.0.root.clone().upcast()
    }

    pub fn set_vertical(&self, vertical: bool) {
        let i = &self.0;
        let o = if vertical { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal };
        i.row.set_orientation(o);
        i.strip.set_hexpand(!vertical);
        i.strip.set_vexpand(vertical);
        i.profile.set_halign(if vertical { gtk::Align::Fill } else { gtk::Align::Start });
        i.profile.set_margin_top(if vertical { 6 } else { 0 });
        i.info.set_halign(if vertical { gtk::Align::Start } else { gtk::Align::End });
        i.info.set_margin_top(if vertical { 6 } else { 0 });
        i.info.set_margin_bottom(if vertical { 6 } else { 0 });
        i.progress.set_valign(if vertical { gtk::Align::Start } else { gtk::Align::End });
        i.strip.set_vertical(vertical);
    }

    pub fn apply(&self, tabs: &[TabItem], status: &Status) {
        let i = &self.0;
        i.strip.apply(tabs);
        if *i.status.borrow() == *status {
            return;
        }
        let old = i.status.replace(status.clone());
        let set = |l: &gtk::Label, text: &str| {
            if l.label() != text {
                l.set_label(text);
            }
            l.set_visible(!text.is_empty());
        };
        set(&i.profile, &status.profile);
        set(&i.keys, &status.keys);
        set(&i.dl, &status.dl);
        let mode = if status.cmd { "" } else { status.mode.trim_start_matches("-- ").trim_end_matches(" --") };
        set(&i.mode, mode);
        if old.modename != status.modename {
            i.mode.set_css_classes(&["mode", status.modename]);
        }
        if old.dlc != status.dlc {
            let c: Vec<&str> = ["dl", status.dlc].into_iter().filter(|c| !c.is_empty()).collect();
            i.dl.set_css_classes(&c);
        }
        if (old.loading, old.progress) != (status.loading, status.progress) {
            self.progress(if status.loading { status.progress.max(0.03) } else { 0.0 });
        }
    }

    /// Spring the progress line to `frac` of the bar; 0 hides it once there.
    fn progress(&self, frac: f64) {
        let i = &self.0;
        if let Some(a) = i.progress_anim.take() {
            a.pause();
        }
        if frac > 0.0 {
            i.progress.set_visible(true);
        }
        let me = Rc::downgrade(&self.0);
        let a = motion::spring(&i.root, i.progress_now.get(), frac, motion::smooth(), move |v| {
            let Some(i) = me.upgrade() else { return };
            i.progress_now.set(v);
            let len = (v.max(0.0) * i.root.width() as f64) as i32;
            i.progress.set_size_request(len.max(1), 2);
        });
        let me = Rc::downgrade(&self.0);
        a.connect_done(move |_| {
            if let Some(i) = me.upgrade() {
                if i.progress_now.get() <= 0.001 {
                    i.progress.set_visible(false);
                }
            }
        });
        i.progress_anim.replace(Some(a));
    }
}

#[cfg(test)]
mod tests {
    use super::{drop_index, slots};

    #[test]
    fn tabs_share_the_width() {
        // 3 tabs in 400 px: 4 px gaps around and between, (400 - 16) / 3 each
        let s = slots(&[false, false, false], 400.0, 20.0, 0.0, f64::INFINITY);
        assert_eq!(s[0], (4.0, 128.0));
        assert_eq!(s[2], (268.0, 128.0));
        // max width caps; a pinned circle keeps its size
        let s = slots(&[true, false], 1000.0, 20.0, 0.0, 200.0);
        assert_eq!(s, vec![(4.0, 20.0), (28.0, 200.0)]);
        // min width overflows instead of shrinking
        let s = slots(&[false, false], 100.0, 20.0, 80.0, f64::INFINITY);
        assert_eq!(s[1], (88.0, 80.0));
    }

    #[test]
    fn drops_stay_in_their_group() {
        // others: pinned at 10, unpinned at 50, 90
        let c = [(10.0, true), (50.0, false), (90.0, false)];
        assert_eq!(drop_index(&c, 0.0, false), 1, "unpinned can't pass the pinned");
        assert_eq!(drop_index(&c, 70.0, false), 2);
        assert_eq!(drop_index(&c, 200.0, false), 3);
        assert_eq!(drop_index(&c, 200.0, true), 1, "pinned can't leave the pinned group");
        assert_eq!(drop_index(&[(50.0, false)], 0.0, true), 0);
    }
}