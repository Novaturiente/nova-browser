//! Message toast (toast.html): a pill at the bottom centre sized to its text, with a
//! round level icon. A new message while shown updates in place; the width springs.
use super::motion::{self, Origin, Pop};
use gtk::glib;
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub const CSS: &str = "
window.nova .nova-toast {
  border-radius: 999px;
  font: var(--fonts-messages-info, var(--fonts-statusbar));
  background: var(--colors-messages-info-bg);
  color: var(--colors-messages-info-fg);
  border: 1px solid var(--colors-completion-category-border-top);
  box-shadow: 0 2px 5px rgba(0,0,0,.35);
  transition: background-color 160ms ease-out, color 160ms ease-out;
}
window.nova .nova-toast.warning {
  background: var(--colors-messages-warning-bg);
  color: var(--colors-messages-warning-fg);
}
window.nova .nova-toast.error {
  background: var(--colors-messages-error-bg);
  color: var(--colors-messages-error-fg);
}
window.nova .nova-toast > viewport > box { min-height: 32px; padding: 0 18px 0 8px; }
window.nova .nova-toast .icon {
  min-width: 22px; min-height: 22px; border-radius: 999px; font-weight: bold;
  background: color-mix(in srgb, currentColor 18%, transparent);
}
";

#[derive(Clone)]
pub struct Toast(Rc<Inner>);

struct Inner {
    pop: Pop,
    /// the pill: caps the text's natural width (scrollbars never)
    pill: gtk::ScrolledWindow,
    body: gtk::Box,
    icon: gtk::Label,
    msg: gtk::Label,
    level: RefCell<String>,
    anim: RefCell<Option<adw::SpringAnimation>>,
}

impl Toast {
    pub fn new() -> Self {
        let icon = gtk::Label::new(None);
        icon.add_css_class("icon");
        icon.set_valign(gtk::Align::Center);
        let msg = gtk::Label::new(None);
        msg.set_ellipsize(gtk::pango::EllipsizeMode::End);
        msg.set_xalign(0.0);
        msg.set_hexpand(true);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        body.append(&icon);
        body.append(&msg);
        let pill = gtk::ScrolledWindow::new();
        pill.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Never);
        pill.set_propagate_natural_width(true);
        pill.set_propagate_natural_height(true);
        pill.set_child(Some(&body));
        pill.add_css_class("nova-toast");
        let pop = Pop::new(&pill, Origin::Bottom, false);
        Toast(Rc::new(Inner {
            pop,
            pill,
            body,
            icon,
            msg,
            level: RefCell::default(),
            anim: RefCell::default(),
        }))
    }
    pub fn widget(&self) -> gtk::Widget {
        self.0.pop.clone().upcast()
    }
    /// level: info / warning / error / find ("query\tpos / total")
    pub fn show(&self, level: &str, text: &str) {
        let i = &self.0;
        let old_level = i.level.replace(level.to_string());
        if old_level != level {
            if !old_level.is_empty() {
                i.pill.remove_css_class(&old_level);
            }
            i.pill.add_css_class(level);
        }
        i.icon.set_text(match level {
            "warning" => "!",
            "error" => "\u{d7}",
            "find" => "🔍",
            _ => "i",
        });
        match text.split_once('\t').filter(|_| level == "find") {
            Some((q, pos)) => i.msg.set_markup(&format!(
                "{} <span alpha=\"75%\" font_features=\"tnum\">{}</span>",
                glib::markup_escape_text(q),
                glib::markup_escape_text(pos)
            )),
            None => i.msg.set_text(text),
        }
        // content width cap: the window's content minus 40, minus the pill's border
        let cap = i.pop.parent().map_or(0, |p| p.width()) - 42;
        let cap = if cap > 0 { cap.max(160) } else { -1 };
        if let Some(a) = i.anim.take() {
            a.pause();
        }
        let from = i.body.width();
        if !i.pop.is_shown() || from <= 0 {
            i.body.set_width_request(-1);
            i.pill.set_max_content_width(cap);
            return i.pop.reveal();
        }
        // spring the width from what is on screen to the new text's natural width
        i.body.set_width_request(-1);
        let nat = i.body.measure(gtk::Orientation::Horizontal, -1).1;
        let to = if cap > 0 { nat.min(cap) } else { nat };
        if to == from {
            return i.pill.set_max_content_width(cap);
        }
        let (body, pill) = (i.body.clone(), i.pill.clone());
        let step = move |v: f64| {
            let v = v.round().max(1.0) as i32;
            body.set_width_request(v);
            pill.set_max_content_width(v);
        };
        step(from as f64);
        let a = motion::spring(&i.pill, from as f64, to as f64, motion::layout(), step);
        let (body, pill) = (i.body.clone(), i.pill.clone());
        a.connect_done(move |_| {
            body.set_width_request(-1);
            pill.set_max_content_width(cap);
        });
        i.anim.replace(Some(a));
    }
    pub fn hide(&self) {
        self.0.pop.conceal(|| {});
    }
}
