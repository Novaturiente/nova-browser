//! Which-key (keyhint.html): the keys that can follow the pending prefix, in a card at
//! the bottom right. At most 380 px wide; taller lists are clipped to the window.
use super::motion::{Origin, Pop};
use gtk::prelude::*;

pub const CSS: &str = "
window.nova .nova-keyhint {
  background: var(--colors-keyhint-bg);
  color: var(--colors-keyhint-fg);
  font: var(--fonts-keyhint);
  padding: 8px 12px;
  border: 1px solid var(--colors-completion-category-border-top);
  border-radius: 14px;
}
window.nova .nova-keyhint .row { min-height: 1.6em; }
window.nova .nova-keyhint .k { min-width: 2.4em; }
window.nova .nova-keyhint .p { opacity: .5; }
window.nova .nova-keyhint .s { color: var(--colors-keyhint-suffix-fg); font-weight: bold; }
window.nova .nova-keyhint .c { opacity: .8; }
";

/// card padding + border, for the size caps (keyhint.html sized its view the same way)
const PAD_W: i32 = 26;
const PAD_H: i32 = 18;

#[derive(Clone)]
pub struct KeyHint(Pop, gtk::ScrolledWindow, gtk::Box);

impl KeyHint {
    pub fn new() -> Self {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let sw = gtk::ScrolledWindow::new();
        // External: no scrollbar, clipped like the page's overflow: hidden
        sw.set_policy(gtk::PolicyType::Never, gtk::PolicyType::External);
        sw.set_propagate_natural_width(true);
        sw.set_propagate_natural_height(true);
        sw.set_max_content_width(380 - PAD_W);
        sw.set_child(Some(&list));
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("nova-keyhint");
        card.append(&sw);
        KeyHint(Pop::new(&card, Origin::Bottom, false), sw, list)
    }
    pub fn widget(&self) -> gtk::Widget {
        self.0.clone().upcast()
    }
    /// rows: (rest of the keys, command)
    pub fn show(&self, prefix: &str, rows: &[(String, String)]) {
        let (sw, list) = (&self.1, &self.2);
        while let Some(c) = list.first_child() {
            list.remove(&c);
        }
        let label = |text: &str, class: &str| {
            let l = gtk::Label::new(Some(text));
            l.add_css_class(class);
            l.set_xalign(0.0);
            l
        };
        for (rest, cmd) in rows {
            let k = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            k.add_css_class("k");
            k.append(&label(prefix, "p"));
            k.append(&label(rest, "s"));
            let c = label(cmd, "c");
            c.set_ellipsize(gtk::pango::EllipsizeMode::End);
            c.set_hexpand(true);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
            row.add_css_class("row");
            row.append(&k);
            row.append(&c);
            list.append(&row);
        }
        // the window's content minus the 12 px margins above and below
        if let Some(p) = self.0.parent().filter(|p| p.height() > 0) {
            sw.set_max_content_height((p.height() - 24 - PAD_H).max(12));
            sw.set_max_content_width((p.width() - 24 - PAD_W).clamp(74, 380 - PAD_W));
        }
        sw.vadjustment().set_value(0.0);
        self.0.reveal();
    }
    pub fn hide(&self) {
        self.0.conceal(|| {});
    }
}
