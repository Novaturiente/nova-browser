//! Floating cards over the page: the command palette (URL box), messages, questions,
//! which-key, the tab tooltip and the tab menu. Each is a UI page in a CEF browser view
//! placed as a window overlay.
//!
//! CEF can only show these as opaque rectangles: a windowed browser can't be transparent
//! (tested), and anything the Views layer paints itself sits under the page (spike: an
//! overlay Panel, even opaque, never showed). So each overlay is a solid card exactly the
//! size of its view, and the view's own background is the card colour (`ui_settings`):
//! whenever CEF shows the view before its page has painted (first show, a new size), it
//! shows that colour, which looks like the empty card. With CEF's default, opaque white,
//! that was the white flash.
//!
//! Show: the page is filled, the view placed and made visible, and the page fades its
//! content in (ui/card.html). Hide: the page fades its content out first, then the view
//! is hidden, so the frame it keeps (and shows first next time) is the empty card.
//! The page reports its natural size ("size w h"), and the card follows it.

use cef::*;
use std::cell::Cell;
use std::rc::Rc;

/// How long the content fades out before the view is hidden (card.html .out: 100 ms).
const HIDE_MS: i64 = 110;

/// Where on its anchor a card sits: `Start` = anchor is its left / top edge.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Align {
    Start,
    Center,
    End,
}

/// A point in window coordinates and which part of the card goes there.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Anchor {
    pub x: i32,
    pub y: i32,
    pub h: Align,
    pub v: Align,
}

impl Anchor {
    pub const fn new(x: i32, y: i32, h: Align, v: Align) -> Anchor {
        Anchor { x, y, h, v }
    }
}

/// The card's rect for `size` at `a`, kept inside a `win` (width, height) window.
pub fn place(a: Anchor, size: (i32, i32), win: (i32, i32)) -> Rect {
    let (w, h) = (size.0.max(1), size.1.max(1));
    let x = match a.h {
        Align::Start => a.x,
        Align::Center => a.x - w / 2,
        Align::End => a.x - w,
    };
    let y = match a.v {
        Align::Start => a.y,
        Align::Center => a.y - h / 2,
        Align::End => a.y - h,
    };
    Rect {
        x: x.min(win.0 - w - 4).max(4),
        y: y.min(win.1 - h - 4).max(0),
        width: w,
        height: h,
    }
}

/// cef::Rect has no PartialEq.
fn xywh(r: &Rect) -> (i32, i32, i32, i32) {
    (r.x, r.y, r.width, r.height)
}

#[derive(Clone)]
pub struct Overlay {
    pub view: BrowserView,
    ctl: OverlayController,
    window: Window,
    /// Bumped on every show and hide: a delayed hide or focus only applies if current.
    gen: Rc<Cell<u64>>,
    anchor: Rc<Cell<Anchor>>,
    size: Rc<Cell<(i32, i32)>>,
}

impl Overlay {
    /// `takes_keys`: the overlay can hold keyboard focus (palette, tab menu).
    pub fn new(window: &Window, view: BrowserView, takes_keys: bool) -> Overlay {
        let ctl = window
            .add_overlay_view(Some(&mut View::from(&view)), DockingMode::CUSTOM, takes_keys as i32)
            .expect("overlay");
        Overlay {
            view,
            ctl,
            window: window.clone(),
            gen: Rc::default(),
            anchor: Rc::new(Cell::new(Anchor::new(0, 0, Align::Start, Align::Start))),
            size: Rc::new(Cell::new((1, 1))),
        }
    }

    pub fn id(&self) -> Option<i32> {
        self.view.browser().map(|b| b.identifier())
    }

    pub fn shown(&self) -> bool {
        self.ctl.is_visible() != 0
    }

    pub fn js(&self, js: &str) {
        if let Some(f) = self.view.browser().and_then(|b| b.main_frame()) {
            f.execute_java_script(Some(&CefString::from(js)), None, 0);
        }
    }

    fn bump(&self) -> u64 {
        self.gen.set(self.gen.get() + 1);
        self.gen.get()
    }

    fn place(&self) {
        let b = self.window.bounds();
        let r = place(self.anchor.get(), self.size.get(), (b.width, b.height));
        if xywh(&self.ctl.bounds()) != xywh(&r) {
            self.ctl.set_bounds(Some(&r));
        }
    }

    /// Fill the page with `js` and show the card at `anchor`, `size` until the page
    /// reports its own. Already shown: it moves and refills without fading out.
    pub fn show(&self, anchor: Anchor, size: (i32, i32), js: &str) {
        self.bump();
        self.anchor.set(anchor);
        self.size.set(size);
        self.js(&format!("{js}; nova.in(); nova.fit()"));
        self.place();
        if !self.shown() {
            self.ctl.set_visible(1);
        }
    }

    /// Move the card (window resized, completion grew) without refilling it.
    pub fn move_to(&self, anchor: Anchor) {
        self.anchor.set(anchor);
        if self.shown() {
            self.place();
        }
    }

    /// "size w h" from the page: its content's natural size. `own_width`: the card takes
    /// the page's width (the toast); else the width is ours, and a report measured at
    /// another width (the view not resized yet) is stale and dropped.
    pub fn fit(&self, w: i32, h: i32, own_width: bool) {
        let (cur_w, _) = self.size.get();
        if !own_width && w != cur_w {
            return;
        }
        self.size.set((w, h));
        if self.shown() {
            self.place();
        }
    }

    /// Fade the content out, then hide the view.
    pub fn hide(&self) {
        if !self.shown() {
            return;
        }
        let gen = self.bump();
        self.js("nova.out()");
        post_delayed_task(ThreadId::UI, Some(&mut HideOverlay::new(self.clone(), gen)), HIDE_MS);
    }

    /// Give the card keyboard focus. A just-shown view ignores request_focus, and the
    /// first show after start can take a few frames more, so retry until it has it.
    pub fn focus(&self) {
        let gen = self.gen.get();
        post_task(ThreadId::UI, Some(&mut FocusOverlay::new(self.clone(), gen, 20)));
    }

    /// Unpainted areas show this (ARGB) instead of the creation-time card colour.
    pub fn set_color(&self, argb: u32) {
        self.view.set_background_color(argb);
    }
}

wrap_task! {
    struct HideOverlay { overlay: Overlay, gen: u64 }
    impl Task {
        fn execute(&self) {
            if self.overlay.gen.get() == self.gen {
                self.overlay.ctl.set_visible(0);
            }
        }
    }
}

wrap_task! {
    struct FocusOverlay { overlay: Overlay, gen: u64, tries: u32 }
    impl Task {
        fn execute(&self) {
            let o = &self.overlay;
            if o.gen.get() != self.gen || !o.shown() || o.view.has_focus() != 0 {
                return;
            }
            o.view.request_focus();
            if self.tries > 0 {
                post_delayed_task(ThreadId::UI, Some(&mut FocusOverlay::new(o.clone(), self.gen, self.tries - 1)), 30);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_follows_anchor_and_stays_in_window() {
        let win = (1000, 800);
        let top = Anchor::new(500, 40, Align::Center, Align::Start);
        assert_eq!(xywh(&place(top, (300, 50), win)), (350, 40, 300, 50));
        let corner = Anchor::new(992, 792, Align::End, Align::End);
        assert_eq!(xywh(&place(corner, (400, 200), win)), (592, 592, 400, 200));
        // a tooltip at the right edge is pushed back in (4 px margin)
        let right = Anchor::new(900, 40, Align::Start, Align::Start);
        assert_eq!(place(right, (440, 60), win).x, 556);
        // wider than the window: pinned to the left margin
        assert_eq!(place(top, (2000, 50), win).x, 4);
    }
}
