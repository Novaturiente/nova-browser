//! In-process DevTools calls (`execute_dev_tools_method`, S5b/S6).
use cef::*;

fn call(browser: &Browser, method: &str, params: &DictionaryValue) {
    if let Some(h) = browser.host() {
        h.execute_dev_tools_method(0, Some(&CefString::from(method)), Some(&mut params.clone()));
    }
}

/// A trusted mouse-wheel event at the viewport centre: scrolls whatever is
/// under it (inner scroll containers too), with the page's smooth scrolling.
pub fn wheel(browser: &Browser, dx: f64, dy: f64) {
    let (w, h) = crate::window::content_size();
    let p = dictionary_value_create().expect("dictionary");
    let set = |k: &str, v: f64| p.set_double(Some(&CefString::from(k)), v);
    p.set_string(
        Some(&CefString::from("type")),
        Some(&CefString::from("mouseWheel")),
    );
    set("x", w / 2.0);
    set("y", h / 2.0);
    set("deltaX", dx);
    set("deltaY", dy);
    call(browser, "Input.dispatchMouseEvent", &p);
}
