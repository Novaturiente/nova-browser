//! Native GTK browser chrome: tab bar + status, command palette, which-key, tooltip,
//! tab menu, password list, prompt and toast. Replaces the HTML views (`ui/*.html`)
//! one for one; `window.rs` drives it with typed calls and gets user actions back as
//! `Ev` values through a `Sink` (was `nova.send("…")` strings).
pub mod keyhint;
pub mod motion;
pub mod palette;
pub mod popup;
pub mod prompt;
pub mod style;
pub mod tabbar;
pub mod toast;

use gtk::{gdk, glib};
use std::rc::Rc;

/// A user action in the chrome, for `window.rs` (always the window that owns the widget).
#[derive(Debug, Clone, PartialEq)]
pub enum Ev {
    /// press on tab i: focus it
    Tab(usize),
    /// middle-click or the × on tab i
    Close(usize),
    /// a drag dropped the active tab at index i (0-based, final position)
    Move(usize),
    /// right-click on tab i: (i, x, bottom) in the tab bar widget's coordinates
    Menu(usize, i32, i32),
    /// hover on tab i long enough: (i, left, bottom, right) in tab bar coordinates; None hides
    Tip(Option<(usize, i32, i32, i32)>),
    /// click on the downloads pill
    Downloads,
    /// palette text changed (also sent once on open with the initial text)
    Text(String),
    /// Enter in the palette
    Run(String),
    /// Tab / Shift-Tab / Down / Up with a list: move the selection by n
    Complete(i64),
    /// Up / Down without a list, Ctrl-n / Ctrl-p: history step by n
    Hist(i64),
    /// Escape, Ctrl-[, Backspace on empty text, click outside the card
    CmdLeave,
    /// tab menu pick: (tab, action) from the `MENU` table
    MenuAct(usize, &'static str),
    /// tab menu dismissed
    MenuClose,
    /// click on password suggestion row i
    PwPick(usize),
    /// click on a prompt answer button: its key ("y", "n", "A", "N")
    Answer(String),
}

pub type Sink = Rc<dyn Fn(Ev)>;

/// One tab as the tab bar shows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TabItem {
    /// stable per tab (survives moves): animate open, close and reorder by it
    pub key: u64,
    /// formatted title (`tabs.title.format`)
    pub label: String,
    /// favicon (a refcounted texture: cheap to clone on every refresh)
    pub icon: Option<gdk::Texture>,
    pub active: bool,
    pub loading: bool,
    /// 0..1
    pub progress: f64,
    pub crashed: bool,
    pub pinned: bool,
    pub asleep: bool,
}

/// Status pills at the end of the tab bar, and the load progress line.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    /// named profile or "private"; "" = no pill
    pub profile: String,
    /// "-- INSERT --" style label ("" in normal mode)
    pub mode: String,
    /// `normal`, `insert`, `passthrough`, `command`, … (CSS class for the colours)
    pub modename: &'static str,
    /// command mode: the palette is open, the mode pill hides
    pub cmd: bool,
    /// pending keys / typed hint chars
    pub keys: String,
    /// downloads pill text ("" = none) and class (`run`, `done`, `fail`, "")
    pub dl: String,
    pub dlc: &'static str,
    /// active tab loading and its progress (0..1)
    pub loading: bool,
    pub progress: f64,
}

/// Completion list height (`completion.height`): a share of the window or pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ListHeight {
    Percent(f64),
    Px(i32),
}

/// Decode a `data:image/…;base64,` URL (favicons) into a texture.
pub fn texture_from_data_url(url: &str) -> Option<gdk::Texture> {
    let b64 = url.strip_prefix("data:")?.split_once(";base64,")?.1;
    let bytes = glib::base64_decode(b64);
    gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)).ok()
}

/// All chrome widgets of one window (cheap to clone: GObject refs / Rc handles).
#[derive(Clone)]
pub struct Chrome {
    pub tabbar: tabbar::TabBar,
    pub palette: palette::Palette,
    pub keyhint: keyhint::KeyHint,
    pub tip: popup::Tip,
    pub menu: popup::Menu,
    pub suggest: popup::Suggest,
    pub prompt: prompt::Prompt,
    pub toast: toast::Toast,
}

impl Chrome {
    pub fn new(sink: Sink) -> Self {
        Chrome {
            tabbar: tabbar::TabBar::new(sink.clone()),
            palette: palette::Palette::new(sink.clone()),
            keyhint: keyhint::KeyHint::new(),
            tip: popup::Tip::new(),
            menu: popup::Menu::new(sink.clone()),
            suggest: popup::Suggest::new(sink.clone()),
            prompt: prompt::Prompt::new(sink),
            toast: toast::Toast::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn data_url_rejects_garbage() {
        assert!(super::texture_from_data_url("").is_none());
        assert!(super::texture_from_data_url("http://x/y.png").is_none());
    }
}
