//! Tab tooltip (title + URL), tab right-click menu and the password suggestion list:
//! small cards in the window's root overlay, placed by `window.rs` (`popup_at`) right
//! after `show()`, so each one's natural size is final as soon as `show()` returns.
use super::motion::{self, Origin, Pop};
use super::{Ev, Sink};
use adw::prelude::*;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

pub const CSS: &str = "
window.nova .nova-pop {
  padding: 6px;
  border: 1px solid var(--colors-completion-category-border-top);
  border-radius: 14px;
  background: color-mix(in srgb, var(--colors-tooltip-bg) 85%, transparent);
  color: var(--colors-tooltip-fg);
  font: var(--fonts-tooltip, var(--fonts-statusbar));
}
window.nova .nova-pop.menu {
  background: color-mix(in srgb, var(--colors-contextmenu-menu-bg) 85%, transparent);
  color: var(--colors-contextmenu-menu-fg);
  font: var(--fonts-contextmenu, var(--fonts-statusbar));
}
window.nova .nova-pop .t { font-weight: bold; padding: 3px 6px; }
window.nova .nova-pop .u { opacity: .65; padding: 2px 6px; }
window.nova .nova-pop-row {
  min-height: 28px;
  padding: 0 10px;
  border-radius: 9px;
  transition: background-color 140ms ease-out, color 140ms ease-out;
}
window.nova .nova-pop-row:hover:not(.sel) {
  background: color-mix(in srgb, var(--colors-contextmenu-selected-bg) 50%, transparent);
}
window.nova .nova-pop-row.sel { color: var(--colors-contextmenu-selected-fg); }
window.nova .nova-pop-row .a { font-weight: bold; }
window.nova .nova-pop-row .k { opacity: .5; }
window.nova .nova-pop-hl {
  min-height: 28px;
  border-radius: 9px;
  background: color-mix(in srgb, var(--colors-tabs-indicator-start) 22%, var(--colors-contextmenu-selected-bg));
}
window.nova .nova-pop-h { padding: 2px 6px 6px; font-size: .9em; opacity: .7; }
window.nova .nova-pop-badge {
  min-width: 20px;
  min-height: 20px;
  border-radius: 50%;
  font-size: .75em;
  color: var(--colors-tabs-indicator-start);
  background: color-mix(in srgb, var(--colors-tabs-indicator-start) 20%, transparent);
}
";

const ROW: i32 = 28;
/// Password list: card width and the most rows shown before it scrolls.
const SUGGEST_W: i32 = 360;
const SUGGEST_ROWS: usize = 8;
/// Tooltip card width cap.
const TIP_W: i32 = 480;

/// Natural size of a card (not of its `Pop`: measuring a widget includes its own margins,
/// which `window.rs` uses for placement).
fn natural(w: &impl IsA<gtk::Widget>) -> (i32, i32) {
    let (_, nw, _, _) = w.measure(gtk::Orientation::Horizontal, -1);
    let (_, nh, _, _) = w.measure(gtk::Orientation::Vertical, nw);
    (nw, nh)
}

fn card(class: Option<&str>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("nova-card");
    b.add_css_class("nova-pop");
    if let Some(c) = class {
        b.add_css_class(c);
    }
    b
}

/// A label that ellipsizes and takes the space it is given instead of asking for its
/// whole text, so a fixed-width card stays fixed.
fn fill_label() -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_xalign(0.0);
    l.set_hexpand(true);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(1);
    l
}

#[derive(Clone)]
pub struct Tip {
    pop: Pop,
    card: gtk::Box,
    title: gtk::Label,
    url: gtk::Label,
}

impl Tip {
    pub fn new() -> Self {
        let title = fill_label();
        title.add_css_class("t");
        title.set_single_line_mode(true);
        let url = gtk::Label::new(None);
        url.add_css_class("u");
        url.set_xalign(0.0);
        url.set_wrap(true);
        url.set_wrap_mode(gtk::pango::WrapMode::Char);
        let card = card(None);
        card.append(&title);
        card.append(&url);
        let pop = Pop::new(&card, Origin::Top, false);
        pop.set_can_focus(false);
        Tip { pop, card, title, url }
    }
    pub fn widget(&self) -> gtk::Widget {
        self.pop.clone().upcast()
    }
    pub fn show(&self, title: &str, url: &str) {
        self.title.set_text(title);
        // at most ~4 wrapped lines (a label's `lines` + ellipsize reserves all 4 in its height)
        match url.char_indices().nth(240) {
            Some((i, _)) => self.url.set_text(&format!("{}…", &url[..i])),
            None => self.url.set_text(url),
        }
        // a label has no max width in pixels: past TIP_W, pin the card to TIP_W and let
        // the labels ask for nothing so they ellipsize / wrap inside it
        let labels = [&self.title, &self.url];
        labels.iter().for_each(|l| l.set_max_width_chars(-1));
        self.card.set_size_request(-1, -1);
        if self.card.measure(gtk::Orientation::Horizontal, -1).1 > TIP_W {
            labels.iter().for_each(|l| l.set_max_width_chars(1));
            self.card.set_size_request(TIP_W, -1);
        }
        self.pop.reveal();
    }
    pub fn hide(&self) {
        self.pop.conceal(|| {});
    }
    pub fn shown_size(&self) -> Option<(i32, i32)> {
        self.pop.is_shown().then(|| natural(&self.card))
    }
}

struct Row {
    root: gtk::Box,
    a: gtk::Label,
    b: gtk::Label,
}

/// Rows over a highlight that springs from row to row (menu and password list).
struct List {
    root: gtk::Overlay,
    rows_box: gtk::Box,
    hl: gtk::Box,
    rows: RefCell<Vec<Row>>,
    sel: Cell<Option<usize>>,
    y: Cell<f64>,
    anim: RefCell<Option<adw::SpringAnimation>>,
    hover_selects: bool,
    pick: RefCell<Box<dyn Fn(usize)>>,
}

impl List {
    fn new(hover_selects: bool) -> Rc<Self> {
        let hl = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        hl.add_css_class("nova-pop-hl");
        hl.set_valign(gtk::Align::Start);
        hl.set_visible(false);
        let under = gtk::Box::new(gtk::Orientation::Vertical, 0);
        under.set_can_target(false);
        under.append(&hl);
        let rows_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let root = gtk::Overlay::new();
        root.set_child(Some(&under));
        root.add_overlay(&rows_box);
        root.set_measure_overlay(&rows_box, true);
        Rc::new(List {
            root,
            rows_box,
            hl,
            rows: RefCell::new(Vec::new()),
            sel: Cell::new(None),
            y: Cell::new(0.0),
            anim: RefCell::new(None),
            hover_selects,
            pick: RefCell::new(Box::new(|_| {})),
        })
    }

    fn row(self: &Rc<Self>, i: usize, a_class: &str) -> Row {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        root.add_css_class("nova-pop-row");
        let a = fill_label();
        a.add_css_class(a_class);
        let b = gtk::Label::new(None);
        b.add_css_class("k");
        b.set_xalign(1.0);
        b.set_ellipsize(gtk::pango::EllipsizeMode::End);
        // about 45% of a password row, like the HTML's max-width
        b.set_max_width_chars(22);
        root.append(&a);
        root.append(&b);
        let click = gtk::GestureClick::new();
        let me = Rc::downgrade(self);
        click.connect_pressed(move |g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(l) = me.upgrade() {
                (l.pick.borrow())(i);
            }
        });
        root.add_controller(click);
        if self.hover_selects {
            let motion = gtk::EventControllerMotion::new();
            let me = Rc::downgrade(self);
            motion.connect_enter(move |_, _, _| {
                if let Some(l) = me.upgrade() {
                    l.select(Some(i), true);
                }
            });
            root.add_controller(motion);
        }
        self.rows_box.append(&root);
        Row { root, a, b }
    }

    /// Fill with (label, hint) pairs, reusing the existing rows.
    fn set_rows<'a>(self: &Rc<Self>, items: impl Iterator<Item = (&'a str, &'a str)>, a_class: &str) {
        let mut n = 0;
        for (i, (a, b)) in items.enumerate() {
            if i == self.rows.borrow().len() {
                let r = self.row(i, a_class);
                self.rows.borrow_mut().push(r);
            }
            let rows = self.rows.borrow();
            rows[i].a.set_text(a);
            rows[i].b.set_text(b);
            n = i + 1;
        }
        let mut rows = self.rows.borrow_mut();
        while rows.len() > n {
            let r = rows.pop().unwrap();
            self.rows_box.remove(&r.root);
        }
        if self.sel.get().is_some_and(|s| s >= n) {
            self.sel.set(None);
        }
    }

    fn len(&self) -> usize {
        self.rows.borrow().len()
    }

    fn set_y(&self, y: f64) {
        self.y.set(y);
        let max = (self.len().max(1) - 1) as f64 * ROW as f64;
        self.hl.set_margin_top(y.clamp(0.0, max).round() as i32);
    }

    /// Highlight row i (None: no highlight); `animate` slides it from the current row.
    fn select(self: &Rc<Self>, i: Option<usize>, animate: bool) {
        let old = self.sel.replace(i);
        let rows = self.rows.borrow();
        if old != i {
            if let Some(r) = old.and_then(|o| rows.get(o)) {
                r.root.remove_css_class("sel");
            }
        }
        let Some(r) = i.and_then(|i| rows.get(i)) else {
            self.hl.set_visible(false);
            return;
        };
        r.root.add_css_class("sel");
        drop(rows);
        if let Some(a) = self.anim.take() {
            a.pause();
        }
        let to = (i.unwrap() as i32 * ROW) as f64;
        if animate && old.is_some() && self.hl.is_visible() {
            let me = Rc::downgrade(self);
            let a = motion::spring(&self.hl, self.y.get(), to, motion::layout(), move |v| {
                if let Some(l) = me.upgrade() {
                    l.set_y(v);
                }
            });
            self.anim.replace(Some(a));
        } else {
            self.set_y(to);
        }
        self.hl.set_visible(true);
    }
}

#[derive(Clone)]
pub struct Menu(Rc<MenuInner>);

struct MenuInner {
    pop: Pop,
    list: Rc<List>,
    items: Cell<&'static [(&'static str, &'static str, &'static str)]>,
    tab: Cell<usize>,
    sink: Sink,
}

impl MenuInner {
    fn pick(&self, i: usize) {
        if let Some((a, _, _)) = self.items.get().get(i) {
            (self.sink)(Ev::MenuAct(self.tab.get(), a));
        }
    }
}

impl Menu {
    pub fn new(sink: Sink) -> Self {
        let list = List::new(true);
        let c = card(Some("menu"));
        c.set_size_request(230, -1);
        c.append(&list.root);
        let pop = Pop::new(&c, Origin::Top, true);
        let inner = Rc::new(MenuInner { pop, list, items: Cell::new(&[]), tab: Cell::new(0), sink });
        let me: Weak<MenuInner> = Rc::downgrade(&inner);
        *inner.list.pick.borrow_mut() = Box::new(move |i| {
            if let Some(m) = me.upgrade() {
                m.pick(i);
            }
        });
        Menu(inner)
    }
    pub fn widget(&self) -> gtk::Widget {
        self.0.pop.clone().upcast()
    }
    /// items: (action, label, key hint)
    pub fn show(&self, tab: usize, items: &'static [(&'static str, &'static str, &'static str)]) {
        let m = &self.0;
        m.tab.set(tab);
        if !std::ptr::eq(m.items.get(), items) {
            m.items.set(items);
            m.list.set_rows(items.iter().map(|(_, l, k)| (*l, *k)), "l");
        }
        m.list.select(Some(0), m.pop.is_shown());
        m.pop.reveal();
    }
    pub fn hide(&self) {
        self.0.pop.conceal(|| {});
    }
    pub fn is_shown(&self) -> bool {
        self.0.pop.is_shown()
    }
    /// A key while the menu is open (j/k/Up/Down/Tab/Return/Escape); true = handled.
    pub fn key(&self, name: &str) -> bool {
        let m = &self.0;
        if !self.is_shown() {
            return false;
        }
        let n = m.list.len();
        let s = m.list.sel.get().unwrap_or(0);
        match name {
            "Escape" => (m.sink)(Ev::MenuClose),
            "j" | "Down" | "Tab" if n > 0 => m.list.select(Some((s + 1) % n), true),
            "k" | "Up" | "ISO_Left_Tab" if n > 0 => m.list.select(Some((s + n - 1) % n), true),
            "Return" | "Enter" | "KP_Enter" => m.pick(s),
            _ => {}
        }
        true
    }
}

#[derive(Clone)]
pub struct Suggest(Rc<SuggestInner>);

struct SuggestInner {
    pop: Pop,
    card: gtk::Box,
    title: gtk::Label,
    scroll: gtk::ScrolledWindow,
    list: Rc<List>,
}

impl SuggestInner {
    /// Scroll so the selected row is in view (also once the list is laid out).
    fn keep_visible(&self) {
        let Some(i) = self.list.sel.get() else { return };
        let adj = self.scroll.vadjustment();
        if adj.page_size() <= 0.0 {
            return;
        }
        let (top, page) = ((i as i32 * ROW) as f64, adj.page_size());
        if top < adj.value() {
            adj.set_value(top);
        } else if top + ROW as f64 > adj.value() + page {
            adj.set_value(top + ROW as f64 - page);
        }
    }
}

impl Suggest {
    pub fn new(sink: Sink) -> Self {
        let list = List::new(false);
        *list.pick.borrow_mut() = Box::new(move |i| sink(Ev::PwPick(i)));
        let badge = gtk::Label::new(Some("🔑"));
        badge.add_css_class("nova-pop-badge");
        let title = fill_label();
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.add_css_class("nova-pop-h");
        head.append(&badge);
        head.append(&title);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_vexpand(true);
        scroll.set_child(Some(&list.root));
        let c = card(Some("menu"));
        c.append(&head);
        c.append(&scroll);
        let pop = Pop::new(&c, Origin::Top, true);
        pop.set_can_focus(false);
        let inner = Rc::new(SuggestInner { pop, card: c, title, scroll, list });
        let me = Rc::downgrade(&inner);
        inner.scroll.vadjustment().connect_changed(move |_| {
            if let Some(s) = me.upgrade() {
                s.keep_visible();
            }
        });
        Suggest(inner)
    }
    pub fn widget(&self) -> gtk::Widget {
        self.0.pop.clone().upcast()
    }
    /// data: {title, rows: [{a, b, sel}]}
    pub fn show(&self, data: &Value) {
        let s = &self.0;
        let rows = data["rows"].as_array().map_or(&[][..], |r| &r[..]);
        s.title.set_text(data["title"].as_str().unwrap_or(""));
        let text = |r: &'_ Value, k: &str| r[k].as_str().unwrap_or("").to_owned();
        let items: Vec<(String, String)> = rows.iter().map(|r| (text(r, "a"), text(r, "b"))).collect();
        s.list.set_rows(items.iter().map(|(a, b)| (a.as_str(), b.as_str())), "a");
        let shown = rows.len().min(SUGGEST_ROWS) as i32;
        s.card.set_size_request(SUGGEST_W, 44 + shown * ROW);
        let sel = rows.iter().position(|r| r["sel"].as_bool() == Some(true));
        s.list.select(sel, s.pop.is_shown());
        s.keep_visible();
        s.pop.reveal();
    }
    pub fn hide(&self) {
        self.0.pop.conceal(|| {});
    }
    pub fn shown_size(&self) -> Option<(i32, i32)> {
        self.0.pop.is_shown().then(|| natural(&self.0.card))
    }
}
