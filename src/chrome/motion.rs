//! Motion for the native chrome: spring presets (libadwaita springs, Apple-like feel) and
//! `Pop`, a one-child container that shows and hides its child with opacity + scale +
//! a short slide. A hidden or hiding `Pop` never takes clicks (`can_target` off), so a
//! fading overlay can't swallow a click meant for the page.
use adw::prelude::*;
use gtk::glib;
use gtk::subclass::prelude::*;
use std::cell::{Cell, RefCell};

/// Opening: quick with a slight overshoot.
pub fn snappy() -> adw::SpringParams {
    adw::SpringParams::new(0.78, 1.0, 420.0)
}

/// Closing and settling: critically damped, no bounce.
pub fn smooth() -> adw::SpringParams {
    adw::SpringParams::new(1.0, 1.0, 380.0)
}

/// Layout moves (tabs sliding, widths): fast, barely any overshoot.
pub fn layout() -> adw::SpringParams {
    adw::SpringParams::new(0.9, 1.0, 520.0)
}

/// Run a spring on `widget` from `from` to `to`, calling `on_value` every frame. The
/// returned animation is already playing; keep it to pause or retarget. Skipped at once
/// when animations are off or `widget` is unmapped (libadwaita does that; a widget made
/// visible inside a mapped parent maps at once).
pub fn spring(
    widget: &impl IsA<gtk::Widget>,
    from: f64,
    to: f64,
    params: adw::SpringParams,
    on_value: impl Fn(f64) + 'static,
) -> adw::SpringAnimation {
    spring_then(widget, from, to, params, on_value, || {})
}

/// `spring`, then `done` once it settles. Connected before it plays: a skipped animation
/// finishes inside `play()`.
pub fn spring_then(
    widget: &impl IsA<gtk::Widget>,
    from: f64,
    to: f64,
    params: adw::SpringParams,
    on_value: impl Fn(f64) + 'static,
    done: impl Fn() + 'static,
) -> adw::SpringAnimation {
    let target = adw::CallbackAnimationTarget::new(on_value);
    let a = adw::SpringAnimation::new(widget, from, to, params, target);
    a.set_clamp(false);
    a.connect_done(move |_| done());
    a.play();
    a
}

thread_local! {
    /// The running `fade` per widget, so a new fade replaces it (its `done` never runs).
    static FADES: RefCell<std::collections::HashMap<usize, adw::SpringAnimation>> = RefCell::default();
}

/// Fade a plain widget's opacity to `to` (shown first when fading in); hides it after a
/// fade to 0, then runs `done`. Replaces a fade still running on `widget`.
pub fn fade(widget: &impl IsA<gtk::Widget>, to: f64, done: impl Fn() + 'static) {
    let w: gtk::Widget = widget.clone().upcast();
    let key = w.as_ptr() as usize;
    if let Some(a) = FADES.with_borrow_mut(|f| f.remove(&key)) {
        a.pause();
    }
    if to == 0.0 && !w.is_visible() {
        return done();
    }
    if to > 0.0 && !w.is_visible() {
        w.set_opacity(0.0);
        w.set_visible(true);
    }
    let (w2, w3) = (w.clone(), w.clone());
    let a = spring_then(&w, w.opacity(), to, smooth(), move |v| w2.set_opacity(v.clamp(0.0, 1.0)), move || {
        FADES.with_borrow_mut(|f| f.remove(&key));
        if to == 0.0 {
            w3.set_visible(false);
        }
        done();
    });
    if a.state() != adw::AnimationState::Finished {
        FADES.with_borrow_mut(|f| f.insert(key, a));
    }
}

/// Where a `Pop` grows from (and slides toward when shown).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Origin {
    /// drops down from above (prompt, menus under the tab bar)
    #[default]
    Top,
    /// rises from below (toast, search pill, which-key)
    Bottom,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Pop {
        pub child: RefCell<Option<gtk::Widget>>,
        /// 0 = hidden, 1 = shown; a spring may overshoot past 1
        pub t: Cell<f64>,
        pub origin: Cell<Origin>,
        pub shown: Cell<bool>,
        pub targetable: Cell<bool>,
        pub anim: RefCell<Option<adw::SpringAnimation>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Pop {
        const NAME: &'static str = "NovaPop";
        type Type = super::Pop;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Pop {
        fn dispose(&self) {
            if let Some(c) = self.child.take() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for Pop {
        fn measure(&self, o: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            match self.child.borrow().as_ref() {
                // overlays ask for height with no width: answer for the natural width, or a
                // wrapping label reports its height at its narrowest (one char per line)
                Some(c) if o == gtk::Orientation::Vertical && for_size < 0 => {
                    c.measure(o, c.measure(gtk::Orientation::Horizontal, -1).1)
                }
                Some(c) => c.measure(o, for_size),
                None => (0, 0, -1, -1),
            }
        }

        fn size_allocate(&self, w: i32, h: i32, baseline: i32) {
            let Some(c) = self.child.borrow().clone() else { return };
            let t = self.t.get();
            let (wf, hf) = (w as f32, h as f32);
            let s = (0.94 + 0.06 * t) as f32;
            let dy = match self.origin.get() {
                Origin::Top => -10.0 * (1.0 - t) as f32,
                Origin::Bottom => 10.0 * (1.0 - t) as f32,
            };
            let tr = gtk::gsk::Transform::new()
                .translate(&gtk::graphene::Point::new(wf / 2.0, hf / 2.0 + dy))
                .scale(s, s)
                .translate(&gtk::graphene::Point::new(-wf / 2.0, -hf / 2.0));
            c.allocate(w, h, baseline, Some(tr));
        }
    }
}

glib::wrapper! {
    pub struct Pop(ObjectSubclass<imp::Pop>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Pop {
    /// A hidden pop around `child`. `targetable`: whether it takes clicks while shown.
    pub fn new(child: &impl IsA<gtk::Widget>, origin: Origin, targetable: bool) -> Self {
        let p: Self = glib::Object::new();
        child.set_parent(&p);
        let imp = p.imp();
        imp.child.replace(Some(child.clone().upcast()));
        imp.origin.set(origin);
        imp.targetable.set(targetable);
        p.set_visible(false);
        p.set_can_target(false);
        p.set_opacity(0.0);
        p
    }

    pub fn set_origin(&self, o: Origin) {
        self.imp().origin.set(o);
    }

    /// Shown, or showing (not hiding).
    pub fn is_shown(&self) -> bool {
        self.imp().shown.get()
    }

    fn set_t(&self, t: f64) {
        self.imp().t.set(t);
        self.set_opacity(t.clamp(0.0, 1.0));
        self.queue_allocate();
    }

    fn animate(&self, to: f64, params: adw::SpringParams, done: impl Fn() + 'static) {
        if let Some(a) = self.imp().anim.take() {
            a.pause();
        }
        let me = self.downgrade();
        let a = spring_then(self, self.imp().t.get(), to, params, move |v| {
            if let Some(p) = me.upgrade() {
                p.set_t(v);
            }
        }, done);
        self.imp().anim.replace(Some(a));
    }

    /// Spring in. Already shown: nothing.
    pub fn reveal(&self) {
        if self.is_shown() {
            return;
        }
        let imp = self.imp();
        imp.shown.set(true);
        self.set_can_target(imp.targetable.get());
        if !self.is_visible() {
            self.set_t(0.0);
            self.set_visible(true);
        }
        self.animate(1.0, snappy(), || {});
    }

    /// Spring out, hide, then run `done`. Takes no clicks from the first frame.
    pub fn conceal(&self, done: impl Fn() + 'static) {
        let imp = self.imp();
        imp.shown.set(false);
        self.set_can_target(false);
        if !self.is_visible() {
            return done();
        }
        let me = self.downgrade();
        self.animate(0.0, smooth(), move || {
            if let Some(p) = me.upgrade() {
                if !p.is_shown() {
                    p.set_visible(false);
                }
            }
            done();
        });
    }
}
