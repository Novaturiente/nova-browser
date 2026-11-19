//! GTK CSS for the chrome. Two providers on the display: the static sheet (each
//! component's `CSS`) and the theme: every `theme::vars` entry (`colors.*`, `fonts.*`,
//! paddings) as a CSS custom property on `window.nova`, so the component sheets use
//! `var(--colors-tabs-bar-bg)` exactly like the old `ui/*.html` pages did.
use gtk::gdk;
use serde_json::{Map, Value};
use std::cell::RefCell;

thread_local! {
    static THEME: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
}

/// CSS class on every browser window; the theme variables hang off it.
pub const WINDOW_CLASS: &str = "nova";

/// Shared look for overlay cards (menus, tooltip, prompt, toast, which-key, palette).
const BASE: &str = "
window.nova .nova-card {
  border-radius: 12px;
  box-shadow: 0 10px 30px rgba(0,0,0,.35), 0 0 0 1px rgba(255,255,255,.06);
}
window.nova .nova-backdrop { filter: blur(10px) brightness(.45); }
window.nova .nova-backdrop-bg { background: black; }
";

/// Install the static sheet once per display. Call before the first window shows.
pub fn install(display: &gdk::Display) {
    THEME.with_borrow_mut(|t| {
        if t.is_some() {
            return;
        }
        let css = gtk::CssProvider::new();
        let sheet = [
            BASE,
            super::tabbar::CSS,
            super::palette::CSS,
            super::popup::CSS,
            super::prompt::CSS,
            super::toast::CSS,
            super::keyhint::CSS,
            // behind the web views: a page whose first frame doesn't cover the view yet (a new
            // window still settling its size) shows this, not the light GTK window background
            &format!("window.nova .nova-pages {{ background-color: {}; }}\n", UNPAINTED),
        ]
        .concat();
        css.load_from_string(&sheet);
        gtk::style_context_add_provider_for_display(display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        let theme = gtk::CssProvider::new();
        // above the sheet: a later rule with the same selector wins anyway, but keep it explicit
        gtk::style_context_add_provider_for_display(display, &theme, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
        *t = Some(theme);
    });
}

/// Behind a page that hasn't painted yet (dark: no white flash).
pub const UNPAINTED: &str = "#181a1b";

/// `window.nova { --name: value; … }` from `theme::vars`.
pub fn vars_css(vars: &Map<String, Value>) -> String {
    let mut out = String::from("window.nova {\n");
    for (k, v) in vars {
        let v = match v {
            Value::String(s) => s.clone(),
            v => v.to_string(),
        };
        // an empty or unparsable value would drop the whole block in some GTK versions
        if v.trim().is_empty() || v.contains(['{', '}', ';']) {
            continue;
        }
        out.push_str(&format!("  {k}: {v};\n"));
    }
    out.push_str("}\n");
    out
}

thread_local! {
    static LAST: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Re-theme every window (config load and `:set`). Unchanged: nothing (a reload
/// restyles every widget, and layout changes call this on each resize).
pub fn apply(vars: &Map<String, Value>) {
    let css = vars_css(vars);
    if LAST.with_borrow(|l| *l == css) {
        return;
    }
    THEME.with_borrow(|t| {
        if let Some(t) = t {
            t.load_from_string(&css);
        }
    });
    LAST.set(css);
}

#[cfg(test)]
mod tests {
    #[test]
    fn vars_become_custom_properties() {
        let mut m = serde_json::Map::new();
        m.insert("--colors-tabs-bar-bg".into(), "#222".into());
        m.insert("--bad".into(), "x; } window { color: red".into());
        m.insert("--empty".into(), "".into());
        let css = super::vars_css(&m);
        assert!(css.contains("--colors-tabs-bar-bg: #222;"));
        assert!(!css.contains("--bad") && !css.contains("--empty"));
    }
}
