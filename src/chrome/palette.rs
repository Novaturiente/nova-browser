//! Command palette: a floating card near the top (input pill + one card per completion
//! category) over the blurred page, or for search (`/`, `?`) a pill at the bottom centre
//! with the match count. The `:open [-t|-b|-w] ` style prefix stays out of sight; the
//! core always gets and sends the full command line. Port of `ui/palette.html`.
use super::motion::{self, Origin, Pop};
use super::{Ev, ListHeight, Sink};
use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use serde_json::Value;
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

pub const CSS: &str = "
window.nova .nova-palette { font: var(--fonts-completion-entry); color: var(--colors-completion-fg); }
window.nova .nova-palette .p-bar {
  min-height: 52px; padding: 0 20px; border-radius: 999px;
  background: var(--colors-completion-even-bg);
  border: 1px solid var(--colors-completion-category-border-top);
  box-shadow: 0 14px 40px rgba(0,0,0,.5);
  transition: border-color 140ms ease-out;
}
window.nova .nova-palette .p-bar:focus-within {
  border-color: color-mix(in srgb, var(--colors-completion-category-fg) 55%, transparent);
}
window.nova .nova-palette.compact .p-bar { box-shadow: 0 2px 6px rgba(0,0,0,.35); }
window.nova .nova-palette .p-icon {
  min-width: 26px; min-height: 26px; border-radius: 999px; font-weight: bold;
  color: var(--colors-completion-category-fg);
  background: color-mix(in srgb, var(--colors-completion-category-fg) 18%, transparent);
}
window.nova .nova-palette text {
  padding: 0; background: transparent; color: var(--colors-statusbar-command-fg);
  font: var(--fonts-statusbar); font-size: 1.15em;
}
window.nova .nova-palette text > placeholder { color: inherit; opacity: .45; }
window.nova .nova-palette .p-count { opacity: .6; font-feature-settings: \"tnum\"; }
window.nova .nova-palette .p-grp {
  padding: 0 6px 6px; border-radius: 18px;
  background: var(--colors-completion-even-bg);
  border: 1px solid var(--colors-completion-category-border-top);
  box-shadow: 0 6px 16px rgba(0,0,0,.35);
}
window.nova .nova-palette .p-cat {
  padding: 10px 10px 4px; font: var(--fonts-completion-category);
  color: var(--colors-completion-category-fg);
}
window.nova .nova-palette .p-cat .p-n { font-weight: normal; opacity: .6; }
window.nova .nova-palette .p-row { padding: 6px 12px; border-radius: 10px; transition: color 120ms ease-out; }
window.nova .nova-palette .p-row.sel { color: var(--colors-completion-item-selected-fg); }
window.nova .nova-palette .p-hl {
  border-radius: 10px; background: var(--colors-completion-item-selected-bg);
  box-shadow: inset 3px 0 0 var(--colors-completion-category-fg);
}
window.nova .nova-palette .p-fav { border-radius: 4px; }
window.nova .nova-palette .p-fav.fi { background: color-mix(in srgb, currentColor 15%, transparent); }
window.nova .nova-palette .p-b { opacity: .65; }
";

/// Gap between the input pill and the cards, and between cards.
const GAP: i32 = 10;

#[derive(Clone)]
pub struct Palette(Rc<Inner>);

struct Inner {
    sink: Sink,
    /// the full-area layer (a bottom pill's size when compact)
    root: gtk::Box,
    pop: Pop,
    bar: gtk::Box,
    icon: gtk::Label,
    text: gtk::Text,
    count: gtk::Label,
    list: gtk::Box,
    groups: RefCell<Vec<Group>>,
    /// items + cats of the shown list: a call with the same ones only moves `sel`
    last: RefCell<Option<(Value, Value)>>,
    /// selected (group, row)
    sel: Cell<Option<(usize, usize)>>,
    icons: RefCell<HashMap<String, gdk::Texture>>,
    /// hidden command prefix (`:open -t `)
    pre: RefCell<String>,
    killed: RefCell<String>,
    quiet: Cell<bool>,
    compact: Cell<bool>,
    tall: Cell<bool>,
    height: Cell<ListHeight>,
}

/// One category card.
struct Group {
    root: gtk::Box,
    cat: gtk::Label,
    n: gtk::Label,
    scroll: gtk::ScrolledWindow,
    rows_box: gtk::Box,
    hl_w: gtk::Box,
    rows: Vec<Row>,
    /// global index of the first row
    start: usize,
    hl: Rc<Hl>,
}

struct Row {
    root: gtk::Box,
    icon: gtk::Image,
    a: gtk::Label,
    b: gtk::Label,
    url: String,
}

/// Selection highlight + scroll state of one card.
#[derive(Default)]
struct Hl {
    target: Cell<Option<usize>>,
    /// (y, height) the slide starts from, and the one drawn last
    from: Cell<(f64, f64)>,
    drawn: Cell<(f64, f64)>,
    /// slide progress 0..1 (a spring, may overshoot)
    p: Cell<f64>,
    anim: RefCell<Option<adw::SpringAnimation>>,
    /// row to bring into view once the card is laid out, and whether to glide there
    pending: Cell<Option<(usize, bool)>>,
    scroll_to: Cell<Option<f64>>,
    scroll_anim: RefCell<Option<adw::SpringAnimation>>,
}

impl Palette {
    pub fn new(sink: Sink) -> Self {
        let icon = gtk::Label::new(Some(":"));
        icon.add_css_class("p-icon");
        icon.set_valign(gtk::Align::Center);
        let text = gtk::Text::new();
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        let count = gtk::Label::new(None);
        count.add_css_class("p-count");
        count.set_visible(false);
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        bar.add_css_class("p-bar");
        bar.append(&icon);
        bar.append(&text);
        bar.append(&count);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.set_visible(false);
        let card = gtk::Box::new(gtk::Orientation::Vertical, GAP);
        card.append(&bar);
        card.append(&list);
        let pop = Pop::new(&card, Origin::Top, true);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("nova-palette");
        root.set_can_target(false);
        pop.set_parent(&root);
        let inner = Rc::new(Inner {
            sink,
            root: root.clone(),
            pop,
            bar,
            icon,
            text: text.clone(),
            count,
            list: list.clone(),
            groups: RefCell::new(Vec::new()),
            last: RefCell::new(None),
            sel: Cell::new(None),
            icons: RefCell::new(HashMap::new()),
            pre: RefCell::new(String::new()),
            killed: RefCell::new(String::new()),
            quiet: Cell::new(false),
            compact: Cell::new(false),
            tall: Cell::new(false),
            height: Cell::new(ListHeight::Percent(50.0)),
        });

        let w = Rc::downgrade(&inner);
        set_layout(&root, {
            let w = w.clone();
            move |_, o, _| match w.upgrade() {
                Some(i) => i.measure_root(o),
                None => (0, 0, -1, -1),
            }
        }, move |_, width, height| {
            if let Some(i) = w.upgrade() {
                i.allocate_root(width, height);
            }
        });
        set_layout(&list, list_measure, list_allocate);

        // Deleting the leading ":" leaves command mode (qutebrowser); an emptied address stays open.
        let w = Rc::downgrade(&inner);
        text.connect_changed(move |t| {
            let Some(i) = w.upgrade() else { return };
            if i.quiet.get() {
                return;
            }
            let v = t.text();
            if v.is_empty() && i.pre.borrow().is_empty() {
                (i.sink)(Ev::CmdLeave);
            } else {
                (i.sink)(Ev::Text(i.full()));
            }
        });
        let keys = gtk::EventControllerKey::new();
        // before GtkText's own bindings (Ctrl-a selects all, Tab moves focus, …)
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(&inner);
        keys.connect_key_pressed(move |_, key, _, mods| match w.upgrade() {
            Some(i) if i.key(key, mods) => glib::Propagation::Stop,
            _ => glib::Propagation::Proceed,
        });
        text.add_controller(keys);

        // a press on the blurred page (outside the card) closes the palette
        let click = gtk::GestureClick::new();
        click.set_button(0);
        let w = Rc::downgrade(&inner);
        click.connect_pressed(move |_, _, x, y| {
            let Some(i) = w.upgrade() else { return };
            let inside = i.pop.compute_bounds(&i.root).is_some_and(|b| {
                b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32))
            });
            if !inside {
                (i.sink)(Ev::CmdLeave);
            }
        });
        root.add_controller(click);
        Palette(inner)
    }

    /// The overlay child: fills the content area.
    pub fn widget(&self) -> gtk::Widget {
        self.0.root.clone().upcast()
    }

    pub fn open(&self, text: &str, compact: bool) {
        let i = &self.0;
        i.compact.set(compact);
        let r = &i.root;
        if compact {
            r.add_css_class("compact");
            r.set_halign(gtk::Align::Center);
            r.set_valign(gtk::Align::End);
            r.set_margin_bottom(18);
            i.pop.set_origin(Origin::Bottom);
        } else {
            r.remove_css_class("compact");
            r.set_halign(gtk::Align::Fill);
            r.set_valign(gtk::Align::Fill);
            r.set_margin_bottom(0);
            i.pop.set_origin(Origin::Top);
        }
        r.set_can_target(true);
        i.set_count("");
        self.complete(None);
        i.pop.reveal();
        i.text.grab_focus_without_selecting();
        i.show(text);
        (i.sink)(Ev::Text(text.to_string()));
    }

    /// Spring out; the list stays until the next `open`, so the card doesn't snap first.
    pub fn close(&self) {
        self.0.root.set_can_target(false);
        self.0.pop.conceal(|| {});
    }

    /// Completion / history: replace the text, cursor at the end, not reported as typed.
    pub fn set_text(&self, text: &str) {
        self.0.show(text);
    }

    /// `{title, items: [[text, description]], cats: [category per item], sel}` or None.
    pub fn complete(&self, data: Option<&Value>) {
        let i = &self.0;
        let Some(v) = data else {
            i.last.replace(None);
            i.sel.set(None);
            for g in i.groups.take() {
                i.list.remove(&g.root);
            }
            i.list.set_visible(false);
            return;
        };
        let sel = v["sel"].as_i64().unwrap_or(-1);
        let key = (v["items"].clone(), v["cats"].clone());
        // same list, only `sel` moved: slide the highlight, don't rebuild
        if i.last.borrow().as_ref() == Some(&key) {
            return i.select(sel, true);
        }
        i.build(v);
        i.last.replace(Some(key));
        i.select(sel, false);
    }

    /// Site icons for rows with a web address, as WebKit's icon store answers.
    pub fn icons(&self, icons: &[(String, gdk::Texture)]) {
        let i = &self.0;
        let mut map = i.icons.borrow_mut();
        for (u, t) in icons {
            map.insert(u.clone(), t.clone());
        }
        for g in i.groups.borrow().iter() {
            for r in g.rows.iter().filter(|r| !r.url.is_empty() && r.icon.paintable().is_none()) {
                if let Some(t) = map.get(&r.url) {
                    r.set_icon(Some(t));
                }
            }
        }
    }

    /// Search position ("3 / 12") while typing a search.
    pub fn count(&self, text: &str) {
        self.0.set_count(text);
    }

    pub fn set_list_height(&self, h: ListHeight) {
        self.0.height.set(h);
        self.0.root.queue_allocate();
    }
}

impl Inner {
    fn full(&self) -> String {
        format!("{}{}", self.pre.borrow(), self.text.text())
    }

    fn set_count(&self, t: &str) {
        self.count.set_label(t);
        self.count.set_visible(!t.is_empty());
    }

    fn set_value(&self, v: &str, cursor: i32) {
        self.quiet.set(true);
        self.text.set_text(v);
        self.text.set_position(cursor);
        self.quiet.set(false);
    }

    /// Show `text`: the `:open -t ` style prefix out of sight, cursor at the end.
    fn show(&self, text: &str) {
        let pre = split_prefix(text);
        self.set_value(&text[pre.len()..], -1);
        self.tall.set(pre == ":bindings ");
        let hint = match pre {
            "" => None,
            ":bindings " => Some("Filter key bindings…  (Esc closes)"),
            ":profile-new " => Some("Name for the new profile…"),
            p if p.contains(" -t") || p.contains(" -w") => Some("Open in new tab…"),
            p if p.contains(" -b") => Some("Open in background tab…"),
            _ => Some("Search or enter address…"),
        };
        self.text.set_placeholder_text(hint);
        let search = self.compact.get();
        self.icon.set_label(if search || pre.starts_with(":open") {
            "🔍"
        } else if !pre.is_empty() {
            "›"
        } else {
            ":"
        });
        if search && text[pre.len()..].chars().nth(1).is_none() {
            self.set_count("");
        }
        *self.pre.borrow_mut() = pre.to_string();
        self.root.queue_allocate();
    }

    /// The palette.html keydown table (qutebrowser command-mode readline defaults).
    fn key(&self, k: gdk::Key, mods: gdk::ModifierType) -> bool {
        use gdk::Key;
        let c = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let a = mods.contains(gdk::ModifierType::ALT_MASK);
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        let (ctrl, alt) = (c && !a, a && !c);
        let s = &self.sink;
        let v: Vec<char> = self.text.text().chars().collect();
        let p = (self.text.position().max(0) as usize).min(v.len());
        let up = matches!(k, Key::Up | Key::KP_Up);
        let down = matches!(k, Key::Down | Key::KP_Down);
        match k {
            Key::Return | Key::KP_Enter | Key::ISO_Enter => {
                let pre = self.pre.borrow().clone();
                let blank = self.text.text().trim().is_empty();
                s(if pre == ":bindings " || (!pre.is_empty() && blank) { Ev::CmdLeave } else { Ev::Run(self.full()) });
            }
            Key::Escape => s(Ev::CmdLeave),
            Key::bracketleft if ctrl => s(Ev::CmdLeave),
            Key::BackSpace if !ctrl && !alt && v.is_empty() => s(Ev::CmdLeave),
            Key::Tab | Key::KP_Tab | Key::ISO_Left_Tab if !ctrl && !alt => {
                s(Ev::Complete(if shift || k == Key::ISO_Left_Tab { -1 } else { 1 }))
            }
            // arrows move in the list when one is shown, else browse history (Ctrl-p/n always do)
            _ if (up || down) && !ctrl && !alt && !self.groups.borrow().is_empty() => {
                s(Ev::Complete(if up { -1 } else { 1 }))
            }
            _ if up || (ctrl && k == Key::p) => s(Ev::Hist(-1)),
            _ if down || (ctrl && k == Key::n) => s(Ev::Hist(1)),
            Key::a if ctrl => self.move_to(self.lo(&v)),
            Key::e if ctrl => self.move_to(v.len()),
            Key::b if ctrl => self.move_to(p.saturating_sub(1)),
            Key::f if ctrl => self.move_to((p + 1).min(v.len())),
            Key::b if alt => self.move_to(word_back(&v, p)),
            Key::f if alt => self.move_to(word_fwd(&v, p)),
            Key::u if ctrl => self.edit(&v, 0, p, 0, true),
            Key::k if ctrl => self.edit(&v, p, v.len(), p, true),
            Key::w if ctrl => self.edit(&v, word_back(&v, p), p, word_back(&v, p), true),
            Key::d if alt => self.edit(&v, p, word_fwd(&v, p), p, true),
            Key::h if ctrl => {
                if p > 0 {
                    self.edit(&v, p - 1, p, p - 1, false)
                }
            }
            Key::y if ctrl => {
                let k = self.killed.borrow().clone();
                let n = k.chars().count();
                let mut v2: String = v[..p].iter().collect();
                v2.push_str(&k);
                v2.extend(&v[p..]);
                self.set_value(&v2, (p + n) as i32);
                s(Ev::Text(self.full()));
            }
            _ => return false,
        }
        true
    }

    /// Kills and Ctrl-a stop at the leading ":".
    fn lo(&self, v: &[char]) -> usize {
        usize::from(self.pre.borrow().is_empty() && v.first() == Some(&':'))
    }

    fn move_to(&self, p: usize) {
        self.text.set_position(p as i32);
    }

    fn edit(&self, v: &[char], from: usize, to: usize, cursor: usize, kill: bool) {
        let lo = self.lo(v);
        let (from, cursor) = (from.max(lo), cursor.max(lo));
        if from >= to {
            return;
        }
        if kill {
            *self.killed.borrow_mut() = v[from..to].iter().collect();
        }
        let rest: String = v[..from].iter().chain(&v[to..]).collect();
        self.set_value(&rest, cursor as i32);
        (self.sink)(Ev::Text(self.full()));
    }

    fn measure_root(&self, o: gtk::Orientation) -> (i32, i32, i32, i32) {
        if !self.compact.get() {
            return (0, 0, -1, -1);
        }
        match o {
            // the old search strip: clamp(240, area - 40, 560) with 6px around the pill
            gtk::Orientation::Horizontal => {
                let area = self.root.parent().map_or(600, |p| p.width());
                let w = (area - 40).clamp(240, 560) - 12;
                (w, w, -1, -1)
            }
            _ => {
                let (min, nat, _, _) = self.pop.measure(o, -1);
                (min, nat, -1, -1)
            }
        }
    }

    fn allocate_root(&self, w: i32, h: i32) {
        let (min_h, nat_h, _, _) = self.pop.measure(gtk::Orientation::Vertical, -1);
        self.pop.measure(gtk::Orientation::Horizontal, -1);
        if self.compact.get() {
            self.pop.allocate(w, h.max(min_h), -1, None);
            return;
        }
        let cw = 760.min((w as f64 * 0.86) as i32);
        let mut ch = self.bar.measure(gtk::Orientation::Vertical, -1).1;
        if self.list.is_visible() {
            let (_, list_nat, _, _) = self.list.measure(gtk::Orientation::Vertical, -1);
            let max = if self.tall.get() {
                (h as f64 * 0.7) as i32
            } else {
                match self.height.get() {
                    ListHeight::Percent(p) => (h as f64 * p / 100.0) as i32,
                    ListHeight::Px(px) => px,
                }
            };
            ch += GAP + list_nat.min(max);
        }
        let ch = ch.clamp(min_h, nat_h.max(min_h));
        let at = gtk::graphene::Point::new(((w - cw) / 2) as f32, (h as f64 * 0.1).round() as f32);
        self.pop.allocate(cw, ch, -1, Some(gtk::gsk::Transform::new().translate(&at)));
    }

    /// Replace the list, reusing the existing cards and rows (typing refilters every key).
    fn build(&self, v: &Value) {
        let empty = Vec::new();
        let items = v["items"].as_array().unwrap_or(&empty);
        let cats = v["cats"].as_array().unwrap_or(&empty);
        let title = v["title"].as_str().unwrap_or("");
        let cat_of = |i: usize| cats.get(i).and_then(Value::as_str).filter(|c| !c.is_empty()).unwrap_or(title);
        let mut count: HashMap<&str, usize> = HashMap::new();
        for c in cats.iter().filter_map(Value::as_str) {
            *count.entry(c).or_default() += 1;
        }
        let mut spans: Vec<(&str, usize, usize)> = Vec::new();
        for i in 0..items.len() {
            match spans.last_mut() {
                Some(s) if s.0 == cat_of(i) => s.2 = i + 1,
                _ => spans.push((cat_of(i), i, i + 1)),
            }
        }
        if let Some((g, r)) = self.sel.take() {
            if let Some(row) = self.groups.borrow().get(g).and_then(|g| g.rows.get(r)) {
                row.root.remove_css_class("sel");
            }
        }
        let icons = self.icons.borrow();
        let mut groups = self.groups.borrow_mut();
        let keep = spans.len().min(groups.len());
        for old in groups.drain(keep..) {
            self.list.remove(&old.root);
        }
        for (gi, &(cat, start, end)) in spans.iter().enumerate() {
            if gi == groups.len() {
                let g = Group::new();
                self.list.append(&g.root);
                groups.push(g);
            }
            let g = &mut groups[gi];
            g.start = start;
            g.cat.set_label(cat);
            g.cat.parent().unwrap().set_visible(!cat.is_empty());
            let n = count.get(cat).copied().unwrap_or(0);
            g.n.set_label(&n.to_string());
            g.n.set_visible(n > 0);
            for (ri, it) in items[start..end].iter().enumerate() {
                let a = it[0].as_str().unwrap_or("");
                let b = it[1].as_str().unwrap_or("");
                if ri == g.rows.len() {
                    let r = Row::new();
                    g.rows_box.append(&r.root);
                    g.rows.push(r);
                }
                g.rows[ri].set(a, b, &icons);
            }
            for r in g.rows.drain(end - start..) {
                g.rows_box.remove(&r.root);
            }
            // short cards keep their size; the long ones shrink and scroll
            if end - start <= 4 {
                g.root.add_css_class("small");
            } else {
                g.root.remove_css_class("small");
            }
            g.hl.reset();
            g.hl_w.set_visible(false);
            g.scroll.vadjustment().set_value(0.0);
        }
        self.list.set_visible(!groups.is_empty() && !self.compact.get());
        self.root.queue_allocate();
    }

    fn select(&self, sel: i64, slide: bool) {
        let groups = self.groups.borrow();
        let target = usize::try_from(sel).ok().and_then(|s| {
            groups.iter().position(|g| s >= g.start && s < g.start + g.rows.len()).map(|g| (g, s - groups[g].start))
        });
        let prev = self.sel.replace(target);
        if let Some((pg, pr)) = prev.filter(|p| Some(*p) != target) {
            if let Some(row) = groups.get(pg).and_then(|g| g.rows.get(pr)) {
                row.root.remove_css_class("sel");
            }
        }
        // into another card (or from none): no slide, the cards scroll on their own
        let same_card = prev.is_some() && prev.map(|p| p.0) == target.map(|t| t.0);
        if !same_card {
            if let Some(g) = prev.and_then(|p| groups.get(p.0)) {
                g.hl.reset();
                g.hl_w.set_visible(false);
            }
        }
        let Some((gi, ri)) = target else { return };
        let g = &groups[gi];
        g.rows[ri].root.add_css_class("sel");
        g.hl.go(&g.rows_box, &g.hl_w, ri, slide && same_card);
        g.hl.pending.set(Some((ri, slide)));
        scroll_pending(&g.rows_box, &g.scroll, &g.hl);
    }
}

impl Group {
    fn new() -> Self {
        let cat = gtk::Label::new(None);
        let n = gtk::Label::new(None);
        n.add_css_class("p-n");
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.add_css_class("p-cat");
        header.append(&cat);
        header.append(&n);
        let hl_w = gtk::Box::new(gtk::Orientation::Vertical, 0);
        hl_w.add_css_class("p-hl");
        hl_w.set_can_target(false);
        hl_w.set_visible(false);
        let rows_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        rows_box.append(&hl_w); // first: drawn under the rows
        let hl = Rc::new(Hl::default());
        let h = Rc::downgrade(&hl);
        set_layout(&rows_box, rows_measure, move |w, width, _| rows_allocate(w, width, h.upgrade().as_deref()));
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::External);
        scroll.set_propagate_natural_height(true);
        scroll.set_child(Some(&rows_box));
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("p-grp");
        root.set_overflow(gtk::Overflow::Hidden);
        root.append(&header);
        root.append(&scroll);
        // a new list lays out after `select`: bring the row into view once it has
        let (rb, sc, h) = (rows_box.downgrade(), scroll.downgrade(), Rc::downgrade(&hl));
        scroll.vadjustment().connect_changed(move |_| {
            if let (Some(rb), Some(sc), Some(h)) = (rb.upgrade(), sc.upgrade(), h.upgrade()) {
                scroll_pending(&rb, &sc, &h);
            }
        });
        Group { root, cat, n, scroll, rows_box, hl_w, rows: Vec::new(), start: 0, hl }
    }
}

impl Row {
    fn new() -> Self {
        let icon = gtk::Image::new();
        icon.set_pixel_size(16);
        icon.set_size_request(16, 16);
        icon.set_overflow(gtk::Overflow::Hidden);
        icon.add_css_class("p-fav");
        icon.set_visible(false);
        let a = gtk::Label::new(None);
        a.add_css_class("p-a");
        let b = gtk::Label::new(None);
        b.add_css_class("p-b");
        for l in [&a, &b] {
            l.set_xalign(0.0);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_single_line_mode(true);
        }
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("p-row");
        root.append(&icon);
        root.append(&a);
        root.append(&b);
        set_layout(&root, row_measure, row_allocate);
        Row { root, icon, a, b, url: String::new() }
    }

    fn set(&mut self, a: &str, b: &str, icons: &HashMap<String, gdk::Texture>) {
        for (l, t) in [(&self.a, a), (&self.b, b)] {
            if l.label() != t {
                l.set_label(t);
            }
        }
        let url = [a, b].into_iter().find(|x| x.starts_with("http://") || x.starts_with("https://")).unwrap_or("");
        if url != self.url || (self.icon.paintable().is_none() && icons.contains_key(url)) {
            self.url = url.to_string();
            self.icon.set_visible(!url.is_empty());
            self.set_icon(icons.get(url));
        }
    }

    fn set_icon(&self, t: Option<&gdk::Texture>) {
        self.icon.set_paintable(t);
        if t.is_some() {
            self.icon.remove_css_class("fi");
        } else {
            self.icon.add_css_class("fi");
        }
    }
}

impl Hl {
    fn reset(&self) {
        if let Some(a) = self.anim.take() {
            a.pause();
        }
        if let Some(a) = self.scroll_anim.take() {
            a.pause();
        }
        self.target.set(None);
        self.pending.set(None);
        self.scroll_to.set(None);
    }

    /// Move the highlight to row `i`: slide from where it is drawn, or jump.
    fn go(self: &Rc<Self>, rows_box: &gtk::Box, hl_w: &gtk::Box, i: usize, slide: bool) {
        if let Some(a) = self.anim.take() {
            a.pause();
        }
        self.target.set(Some(i));
        if slide && hl_w.is_visible() {
            self.from.set(self.drawn.get());
            self.p.set(0.0);
            let (me, rb) = (Rc::downgrade(self), rows_box.downgrade());
            let a = motion::spring(rows_box, 0.0, 1.0, motion::layout(), move |v| {
                if let (Some(me), Some(rb)) = (me.upgrade(), rb.upgrade()) {
                    me.p.set(v);
                    rb.queue_allocate();
                }
            });
            self.anim.replace(Some(a));
        } else {
            self.p.set(1.0);
        }
        hl_w.set_visible(true);
        rows_box.queue_allocate();
    }
}

/// Bring the pending row into view (nearest edge), once the card's scroll range is current.
fn scroll_pending(rows_box: &gtk::Box, scroll: &gtk::ScrolledWindow, hl: &Rc<Hl>) {
    let Some((i, smooth)) = hl.pending.get() else { return };
    let adj = scroll.vadjustment();
    let (mut y, mut h, mut total) = (0.0, 0.0, 0.0);
    for (n, r) in children(rows_box).skip(1).enumerate() {
        let rh = r.measure(gtk::Orientation::Vertical, -1).1 as f64;
        if n == i {
            (y, h) = (total, rh);
        }
        total += rh;
    }
    let page = adj.page_size();
    if page <= 0.0 || (adj.upper() - total).abs() > 0.5 {
        return; // not laid out yet: the adjustment's `changed` calls again
    }
    hl.pending.set(None);
    let cur = hl.scroll_to.get().unwrap_or(adj.value());
    let to = if y < cur {
        y
    } else if y + h > cur + page {
        y + h - page
    } else {
        return;
    };
    let to = to.clamp(0.0, (total - page).max(0.0));
    if let Some(a) = hl.scroll_anim.take() {
        a.pause();
    }
    if !smooth {
        hl.scroll_to.set(None);
        adj.set_value(to);
        return;
    }
    hl.scroll_to.set(Some(to));
    let me = Rc::downgrade(hl);
    let adj2 = adj.clone();
    let a = motion::spring(scroll, adj.value(), to, motion::layout(), move |v| adj2.set_value(v));
    a.connect_done(move |_| {
        if let Some(me) = me.upgrade() {
            me.scroll_to.set(None);
        }
    });
    hl.scroll_anim.replace(Some(a));
}

/// Split off the hidden command prefix: `^:(?:open(?: -[tbw])*|bindings|profile-new) `.
fn split_prefix(text: &str) -> &str {
    let Some(rest) = text.strip_prefix(':') else { return "" };
    for word in ["bindings ", "profile-new "] {
        if rest.starts_with(word) {
            return &text[..1 + word.len()];
        }
    }
    let Some(mut opts) = rest.strip_prefix("open") else { return "" };
    let mut ends = vec![1 + 4];
    while let Some(r) = opts.strip_prefix(" -").filter(|r| r.starts_with(['t', 'b', 'w'])) {
        opts = &r[1..];
        ends.push(ends.last().unwrap() + 3);
    }
    // the longest run of flags followed by a space (the regex backtracks the same way)
    ends.into_iter().rev().find(|&e| text[e..].starts_with(' ')).map_or("", |e| &text[..e + 1])
}

/// Word = run of non-space (readline).
fn word_back(v: &[char], mut p: usize) -> usize {
    while p > 0 && v[p - 1] == ' ' {
        p -= 1;
    }
    while p > 0 && v[p - 1] != ' ' {
        p -= 1;
    }
    p
}

fn word_fwd(v: &[char], mut p: usize) -> usize {
    while p < v.len() && v[p] == ' ' {
        p += 1;
    }
    while p < v.len() && v[p] != ' ' {
        p += 1;
    }
    p
}

fn children(w: &impl IsA<gtk::Widget>) -> impl Iterator<Item = gtk::Widget> {
    std::iter::successors(w.first_child(), |c| c.next_sibling())
}

fn translate(x: f64, y: f64) -> Option<gtk::gsk::Transform> {
    Some(gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(x as f32, y as f32)))
}

/// Cards stacked with a gap. Short cards (`.small`) keep their height; when the list is
/// capped the long ones shrink in proportion to their size, down to ~9em (a header and
/// about three rows), like the old flex column.
fn list_measure(w: &gtk::Widget, o: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
    let (mut min, mut nat) = (0, 0);
    let mut n = 0;
    for c in children(w).filter(|c| c.is_visible()) {
        let (cmin, cnat, _, _) = c.measure(o, -1);
        if o == gtk::Orientation::Horizontal {
            (min, nat) = (min.max(cmin), nat.max(cnat));
        } else {
            min += group_min(w, &c, cnat);
            nat += cnat;
            n += 1;
        }
    }
    if n > 1 {
        min += GAP * (n - 1);
        nat += GAP * (n - 1);
    }
    (min, nat, -1, -1)
}

fn group_min(list: &gtk::Widget, g: &gtk::Widget, nat: i32) -> i32 {
    if g.has_css_class("small") {
        return nat;
    }
    let font = list.pango_context().font_description();
    let em = font.map_or(16.0, |f| {
        let s = f.size() as f64 / gtk::pango::SCALE as f64;
        if f.is_size_absolute() { s } else { s * 96.0 / 72.0 }
    });
    nat.min((9.0 * em) as i32)
}

fn list_allocate(w: &gtk::Widget, width: i32, height: i32) {
    let kids: Vec<gtk::Widget> = children(w).filter(|c| c.is_visible()).collect();
    let mut sizes: Vec<(i32, i32)> = kids
        .iter()
        .map(|c| {
            let nat = c.measure(gtk::Orientation::Vertical, -1).1;
            (group_min(w, c, nat), nat)
        })
        .collect();
    let gaps = GAP * (kids.len() as i32 - 1).max(0);
    let mut h: Vec<i32> = sizes.iter().map(|s| s.1).collect();
    // shrink the cards still above their minimum, weighted by natural height
    for _ in 0..kids.len() {
        let over = h.iter().sum::<i32>() + gaps - height;
        let weight: i32 = sizes.iter().zip(&h).filter(|(s, h)| **h > s.0).map(|(s, _)| s.1).sum();
        if over <= 0 || weight == 0 {
            break;
        }
        for (s, h) in sizes.iter_mut().zip(h.iter_mut()).filter(|(s, h)| **h > s.0) {
            let cut = (over as f64 * s.1 as f64 / weight as f64).ceil() as i32;
            *h = (*h - cut).max(s.0);
        }
    }
    let mut y = 0;
    for (c, h) in kids.iter().zip(h) {
        c.allocate(width, h, -1, translate(0.0, y as f64));
        y += h + GAP;
    }
}

/// Rows stacked; the first child is the highlight, placed between its slide's ends.
fn rows_measure(w: &gtk::Widget, o: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
    let (mut min, mut nat) = (0, 0);
    for c in children(w).skip(1) {
        let (cmin, cnat, _, _) = c.measure(o, -1);
        if o == gtk::Orientation::Horizontal {
            (min, nat) = (min.max(cmin), nat.max(cnat));
        } else {
            nat += cnat;
        }
    }
    if o == gtk::Orientation::Vertical {
        min = nat;
    }
    (min, nat, -1, -1)
}

fn rows_allocate(w: &gtk::Widget, width: i32, hl: Option<&Hl>) {
    let mut y = 0.0;
    let mut to = None;
    for (i, c) in children(w).skip(1).enumerate() {
        let h = c.measure(gtk::Orientation::Vertical, -1).1;
        c.allocate(width, h, -1, translate(0.0, y));
        if hl.and_then(|hl| hl.target.get()) == Some(i) {
            to = Some((y, h as f64));
        }
        y += h as f64;
    }
    let (Some(hl), Some(hl_w)) = (hl, w.first_child()) else { return };
    hl_w.measure(gtk::Orientation::Vertical, -1);
    let Some(to) = to.filter(|_| hl_w.is_visible()) else { return };
    let (from, p) = (hl.from.get(), hl.p.get());
    let cur = if p >= 1.0 { to } else { (from.0 + (to.0 - from.0) * p, from.1 + (to.1 - from.1) * p) };
    hl.drawn.set(cur);
    hl_w.allocate(width, cur.1.round().max(0.0) as i32, -1, translate(0.0, cur.0));
}

/// [icon 16px] text (60% of the row) description (the rest), 10px apart.
fn row_measure(w: &gtk::Widget, o: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
    let (mut min, mut nat) = (0, 0);
    for c in children(w).filter(|c| c.is_visible()) {
        let (cmin, cnat, _, _) = c.measure(o, -1);
        if o == gtk::Orientation::Horizontal {
            (min, nat) = (min + cmin + 10, nat + cnat + 10);
        } else {
            (min, nat) = (min.max(cmin), nat.max(cnat));
        }
    }
    (min, nat, -1, -1)
}

fn row_allocate(w: &gtk::Widget, width: i32, height: i32) {
    let Some(icon) = w.first_child() else { return };
    let Some(a) = icon.next_sibling() else { return };
    let Some(b) = a.next_sibling() else { return };
    let mut x = 0;
    if icon.is_visible() {
        icon.measure(gtk::Orientation::Horizontal, -1);
        icon.allocate(16, 16, -1, translate(0.0, ((height - 16) / 2) as f64));
        x = 26;
    }
    let (amin, ..) = a.measure(gtk::Orientation::Horizontal, -1);
    let (bmin, ..) = b.measure(gtk::Orientation::Horizontal, -1);
    let aw = ((width as f64 * 0.6) as i32).min(width - x - 10 - bmin).max(amin);
    let bw = (width - x - aw - 10).max(bmin);
    for (c, cx, cw) in [(&a, x, aw), (&b, x + aw + 10, bw)] {
        let h = c.measure(gtk::Orientation::Vertical, -1).1;
        c.allocate(cw, h, -1, translate(cx as f64, ((height - h) / 2) as f64));
    }
}

type MeasureFn = Box<dyn Fn(&gtk::Widget, gtk::Orientation, i32) -> (i32, i32, i32, i32)>;
type AllocFn = Box<dyn Fn(&gtk::Widget, i32, i32)>;

/// Give `w` a layout made of two closures (gtk4-rs has no GtkCustomLayout binding).
fn set_layout(
    w: &impl IsA<gtk::Widget>,
    measure: impl Fn(&gtk::Widget, gtk::Orientation, i32) -> (i32, i32, i32, i32) + 'static,
    allocate: impl Fn(&gtk::Widget, i32, i32) + 'static,
) {
    let l: FnLayout = glib::Object::new();
    let _ = l.imp().measure.set(Box::new(measure));
    let _ = l.imp().alloc.set(Box::new(allocate));
    w.set_layout_manager(Some(l));
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct FnLayout {
        pub measure: OnceCell<MeasureFn>,
        pub alloc: OnceCell<AllocFn>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FnLayout {
        const NAME: &'static str = "NovaPaletteLayout";
        type Type = super::FnLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for FnLayout {}

    impl LayoutManagerImpl for FnLayout {
        fn request_mode(&self, _: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, w: &gtk::Widget, o: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            self.measure.get().map_or((0, 0, -1, -1), |m| m(w, o, for_size))
        }

        fn allocate(&self, w: &gtk::Widget, width: i32, height: i32, _: i32) {
            if let Some(a) = self.alloc.get() {
                a(w, width, height);
            }
        }
    }
}

glib::wrapper! {
    pub struct FnLayout(ObjectSubclass<imp::FnLayout>) @extends gtk::LayoutManager;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_like_the_html_regex() {
        assert_eq!(split_prefix(":open "), ":open ");
        assert_eq!(split_prefix(":open -t "), ":open -t ");
        assert_eq!(split_prefix(":open -t -b x"), ":open -t -b ");
        assert_eq!(split_prefix(":open -tx"), ":open ");
        assert_eq!(split_prefix(":open -t"), ":open ");
        assert_eq!(split_prefix(":open"), "");
        assert_eq!(split_prefix(":opener "), "");
        assert_eq!(split_prefix(":bindings g"), ":bindings ");
        assert_eq!(split_prefix(":profile-new "), ":profile-new ");
        assert_eq!(split_prefix(":tab-close"), "");
        assert_eq!(split_prefix("/open "), "");
    }

    #[test]
    fn readline_words() {
        let v: Vec<char> = ":open foo  bar".chars().collect();
        assert_eq!(word_back(&v, v.len()), 11);
        assert_eq!(word_back(&v, 11), 6);
        assert_eq!(word_fwd(&v, 9), 14);
        assert_eq!(word_fwd(&v, 0), 5);
    }

    /// Real widgets in a window on screen: keys, a 300-row list, the selection.
    /// `cargo test palette::tests::live -- --ignored --nocapture` (writes /tmp/palette-live.png)
    #[test]
    #[ignore = "needs a display"]
    fn live() {
        use gdk::{Key, ModifierType as M};
        use std::time::Instant;
        adw::init().unwrap();
        let win = gtk::Window::new();
        win.add_css_class(super::super::style::WINDOW_CLASS);
        win.set_default_size(1200, 900);
        super::super::style::install(&WidgetExt::display(&win));
        super::super::style::apply(&crate::theme::vars(&crate::config::get()));
        let evs: Rc<RefCell<Vec<Ev>>> = Rc::default();
        let e = evs.clone();
        let p = Palette::new(Rc::new(move |ev| e.borrow_mut().push(ev)));
        let o = gtk::Overlay::new();
        o.set_child(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
        o.add_overlay(&p.widget());
        win.set_child(Some(&o));
        win.present();
        let ctx = glib::MainContext::default();
        let pump = |ms: u64| {
            let t = Instant::now();
            while t.elapsed().as_millis() < ms as u128 {
                ctx.iteration(false);
            }
        };
        let painted = Rc::new(Cell::new(false));
        pump(300);
        let fc = win.frame_clock().unwrap();
        let pd = painted.clone();
        fc.connect_after_paint(move |_| pd.set(true));
        let frame = || {
            painted.set(false);
            while !painted.get() {
                ctx.iteration(true);
            }
        };
        let take = || std::mem::take(&mut *evs.borrow_mut());

        p.open(":open -t ", false);
        assert_eq!(take(), [Ev::Text(":open -t ".into())]);
        assert_eq!(p.0.text.text(), "");
        assert_eq!(p.0.icon.label(), "🔍");
        // set_text is quiet
        p.set_text(":open -t abc def");
        assert!(take().is_empty());
        let key = |k, m| assert!(p.0.key(k, m), "{k:?} not handled");
        key(Key::w, M::CONTROL_MASK);
        assert_eq!(take(), [Ev::Text(":open -t abc ".into())]);
        key(Key::y, M::CONTROL_MASK);
        assert_eq!(take(), [Ev::Text(":open -t abc def".into())]);
        key(Key::a, M::CONTROL_MASK);
        assert_eq!(p.0.text.position(), 0);
        key(Key::k, M::CONTROL_MASK);
        assert_eq!(take(), [Ev::Text(":open -t ".into())]);
        // emptied address: Enter / Backspace leave
        key(Key::Return, M::empty());
        key(Key::BackSpace, M::empty());
        assert_eq!(take(), [Ev::CmdLeave, Ev::CmdLeave]);
        key(Key::Up, M::empty());
        key(Key::n, M::CONTROL_MASK);
        key(Key::Tab, M::empty());
        key(Key::ISO_Left_Tab, M::SHIFT_MASK);
        assert_eq!(take(), [Ev::Hist(-1), Ev::Hist(1), Ev::Complete(1), Ev::Complete(-1)]);

        p.open(":", false);
        take();
        // Ctrl-u stops at the leading ":"; deleting the ":" by typing leaves
        p.0.text.set_text(":abc");
        take();
        p.0.text.set_position(-1);
        key(Key::u, M::CONTROL_MASK);
        assert_eq!(take(), [Ev::Text(":".into())]);
        p.0.text.delete_text(0, -1);
        assert_eq!(take(), [Ev::CmdLeave]);

        let list = |n: usize, q: &str, sel: i64| {
            let items: Vec<Value> = (0..n)
                .map(|i| serde_json::json!([format!("https://site{i}.example/{q}"), format!("Page {i} {q}")]))
                .collect();
            let cats: Vec<&str> = (0..n).map(|i| if i < 3 { "Bookmarks" } else { "History" }).collect();
            serde_json::json!({"title": "", "items": items, "cats": cats, "sel": sel})
        };
        p.open(":open ", false);
        pump(400);
        let mut worst = 0u128;
        for (i, q) in ["", "g", "gi", "git", "gith", "githu", "github"].iter().enumerate() {
            let t = Instant::now();
            p.complete(Some(&list(300 - i * 20, q, -1)));
            frame();
            worst = worst.max(t.elapsed().as_micros());
        }
        println!("refilter 180-300 rows + frame: worst {worst} µs");
        let t = Instant::now();
        p.complete(Some(&list(180, "github", 0)));
        frame();
        println!("sel move: {} µs", t.elapsed().as_micros());
        key(Key::Down, M::empty());
        assert_eq!(take().last(), Some(&Ev::Complete(1)));
        for sel in [1, 2, 3, 40] {
            p.complete(Some(&list(180, "github", sel)));
            pump(60);
        }
        pump(600);
        let gs = p.0.groups.borrow();
        assert_eq!(gs.len(), 2);
        assert_eq!(gs[1].rows.len(), 177);
        assert!(gs[0].root.has_css_class("small") && !gs[1].root.has_css_class("small"));
        assert!(gs[1].rows[37].root.has_css_class("sel"));
        let adj = gs[1].scroll.vadjustment();
        let r = gs[1].rows[37].root.compute_bounds(&gs[1].rows_box).unwrap();
        let (y, h) = (r.y() as f64, r.height() as f64);
        assert!(y >= adj.value() - 0.5 && y + h <= adj.value() + adj.page_size() + 0.5, "row 40 not in view");
        let hl = gs[1].hl_w.compute_bounds(&gs[1].rows_box).unwrap();
        assert!((hl.y() as f64 - y).abs() < 1.0, "highlight not on row 40");
        drop(gs);
        p.icons(&[]);
        let snap = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(&win)).snapshot(&snap, win.width() as f64, win.height() as f64);
        let r = win.native().and_then(|n| n.renderer()).unwrap();
        r.render_texture(&snap.to_node().unwrap(), None).save_to_png("/tmp/palette-live.png").unwrap();
        win.destroy();
    }
}
