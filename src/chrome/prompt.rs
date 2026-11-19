//! Permission / dialog question (prompt.html): a Spotlight-style card under the tab bar
//! ("?" badge + question), answer pills below (others, No, then Yes on the right). Keys
//! answer through prompt mode in window.rs; clicks send `Ev::Answer(key)`. A new question
//! while shown crossfades in place.
use super::motion::{Origin, Pop};
use super::{Ev, Sink};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub const CSS: &str = "
window.nova .nova-prompt {
  font: var(--fonts-prompts, var(--fonts-statusbar));
  color: var(--colors-prompts-fg);
}
window.nova .nova-prompt .card {
  padding: 10px 22px 10px 10px;
  border-radius: 28px;
  background: var(--colors-prompts-bg);
  border: 1px solid var(--colors-completion-category-border-top);
  box-shadow: 0 4px 14px rgba(0,0,0,.35);
}
window.nova .nova-prompt .badge {
  min-width: 36px; min-height: 36px; border-radius: 999px;
  font-weight: bold; font-size: 1.2em;
  color: var(--colors-tabs-indicator-start);
  background: color-mix(in srgb, var(--colors-tabs-indicator-start) 20%, transparent);
}
window.nova .nova-prompt .q { line-height: 20px; font-size: 1.1em; }
window.nova .nova-prompt button {
  font: inherit; font-weight: normal;
  min-width: 54px; min-height: 38px; padding: 0 20px; border-radius: 999px;
  border: 1px solid var(--colors-completion-category-border-top);
  background: var(--colors-prompts-bg);
  color: var(--colors-prompts-fg);
  box-shadow: 0 3px 10px rgba(0,0,0,.3);
  outline: none;
  transition: filter 140ms ease-out;
}
window.nova .nova-prompt button.no {
  color: var(--colors-messages-error-bg);
  background: color-mix(in srgb, var(--colors-messages-error-bg) 14%, var(--colors-prompts-bg));
  border-color: color-mix(in srgb, var(--colors-messages-error-bg) 35%, transparent);
}
window.nova .nova-prompt button.yes {
  font-weight: bold; border-color: transparent;
  background: var(--colors-tabs-indicator-start);
  color: var(--colors-tabs-bar-bg);
}
window.nova .nova-prompt button:hover { filter: brightness(1.12); }
window.nova .nova-prompt button .kbd { opacity: .55; margin-left: .5em; }
";

type Shown = (String, Vec<[&'static str; 2]>);

#[derive(Clone)]
pub struct Prompt(Rc<Inner>);

struct Inner {
    pop: Pop,
    /// one page per question; the old one crossfades out
    stack: gtk::Stack,
    sink: Sink,
    last: RefCell<Option<Shown>>,
}

impl Prompt {
    pub fn new(sink: Sink) -> Self {
        let stack = gtk::Stack::new();
        stack.set_hhomogeneous(false);
        stack.set_vhomogeneous(false);
        stack.set_interpolate_size(true);
        stack.set_transition_duration(160);
        stack.add_css_class("nova-prompt");
        let pop = Pop::new(&stack, Origin::Top, true);
        Prompt(Rc::new(Inner { pop, stack, sink, last: RefCell::default() }))
    }
    pub fn widget(&self) -> gtk::Widget {
        self.0.pop.clone().upcast()
    }
    /// opts: [key, label]
    pub fn show(&self, question: &str, opts: &[[&'static str; 2]]) {
        let i = &self.0;
        let shown = i.pop.is_shown();
        let same = i.last.borrow().as_ref().is_some_and(|(q, o)| q == question && o == opts);
        if !(same && shown) {
            i.last.replace(Some((question.to_string(), opts.to_vec())));
            let w = i.pop.parent().map_or(0, |p| p.width());
            i.stack.set_width_request(if w > 0 { (w - 24).clamp(240, 640) } else { 640 });
            let prev = i.stack.visible_child();
            // drop pages from earlier crossfades; keep the one on screen to fade from
            let mut c = i.stack.first_child();
            while let Some(w) = c {
                c = w.next_sibling();
                if !shown || Some(&w) != prev.as_ref() {
                    i.stack.remove(&w);
                }
            }
            let page = self.page(question, opts);
            i.stack.add_child(&page);
            i.stack.set_transition_type(if shown {
                gtk::StackTransitionType::Crossfade
            } else {
                gtk::StackTransitionType::None
            });
            i.stack.set_visible_child(&page);
        }
        i.pop.reveal();
    }
    pub fn hide(&self) {
        self.0.pop.conceal(|| {});
    }

    fn page(&self, question: &str, opts: &[[&'static str; 2]]) -> gtk::Box {
        let badge = gtk::Label::new(Some("?"));
        badge.add_css_class("badge");
        let q = gtk::Label::new(Some(question));
        q.add_css_class("q");
        q.set_xalign(0.0);
        q.set_hexpand(true);
        q.set_wrap(true);
        q.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        q.set_lines(3);
        q.set_ellipsize(gtk::pango::EllipsizeMode::End);
        // natural width from the stack, not the unwrapped text
        q.set_max_width_chars(1);
        let card = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        card.add_css_class("card");
        card.append(&badge);
        card.append(&q);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_halign(gtk::Align::Center);
        // order as prompt.html: other answers, then No, then Yes on the right
        let rank = |k: &str| match k {
            "y" => 2,
            "n" => 1,
            _ => 0,
        };
        let mut sorted: Vec<_> = opts.iter().collect();
        sorted.sort_by_key(|[k, _]| rank(k));
        for [key, label] in sorted {
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            content.set_halign(gtk::Align::Center);
            content.append(&gtk::Label::new(Some(label)));
            let kbd = gtk::Label::new(Some(key));
            kbd.add_css_class("kbd");
            content.append(&kbd);
            let b = gtk::Button::new();
            b.set_child(Some(&content));
            b.set_can_focus(false);
            b.set_focus_on_click(false);
            b.set_cursor_from_name(Some("pointer"));
            match *key {
                "y" => b.add_css_class("yes"),
                "n" => b.add_css_class("no"),
                _ => {}
            }
            let (sink, key) = (self.0.sink.clone(), key.to_string());
            b.connect_clicked(move |_| sink(Ev::Answer(key.clone())));
            row.append(&b);
        }

        let page = gtk::Box::new(gtk::Orientation::Vertical, 14);
        page.append(&card);
        page.append(&row);
        page
    }
}
