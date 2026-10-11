//! In-process DevTools calls (S5b/S6): fire-and-forget via `execute_dev_tools_method`,
//! and request/response via `send_dev_tools_message` + one observer per browser.
use cef::*;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

type Callback = Box<dyn FnOnce(Option<Value>)>;

thread_local! {
    /// (browser id, message id) → callback, answered on the UI thread.
    static PENDING: RefCell<HashMap<(i32, i32), Callback>> = RefCell::new(HashMap::new());
    /// Keeps each browser's observer registered (dropping a Registration removes it).
    static OBSERVERS: RefCell<HashMap<i32, Registration>> = RefCell::new(HashMap::new());
    /// Our ids start high so they never meet execute_dev_tools_method's auto ids.
    static NEXT_ID: Cell<i32> = const { Cell::new(1 << 24) };
}

wrap_dev_tools_message_observer! {
    struct Observer {}
    impl DevToolsMessageObserver {
        fn on_dev_tools_method_result(&self, browser: Option<&mut Browser>, message_id: ::std::os::raw::c_int, success: ::std::os::raw::c_int, result: Option<&[u8]>) {
            let Some(b) = browser else { return };
            let Some(cb) = PENDING.with_borrow_mut(|p| p.remove(&(b.identifier(), message_id))) else { return };
            let value = result.and_then(|r| serde_json::from_slice(r).ok());
            cb(if success != 0 { value } else { None });
        }
        fn on_dev_tools_event(&self, browser: Option<&mut Browser>, method: Option<&CefString>, params: Option<&[u8]>) {
            let (Some(b), Some(m)) = (browser, method) else { return };
            if m.to_string() == "Runtime.bindingCalled" {
                if let Some(p) = params.and_then(|p| serde_json::from_slice::<Value>(p).ok()) {
                    crate::adblock::on_binding(b, &p);
                }
            }
        }
    }
}

/// Drop a closed browser's observer and its unanswered callbacks.
pub fn forget(browser: &Browser) {
    let id = browser.identifier();
    OBSERVERS.with_borrow_mut(|o| o.remove(&id));
    PENDING.with_borrow_mut(|p| p.retain(|k, _| k.0 != id));
}

/// Call a DevTools method; `cb` gets the `result` object, or None on error.
pub fn call(
    browser: &Browser,
    method: &str,
    params: Value,
    cb: impl FnOnce(Option<Value>) + 'static,
) {
    let Some(host) = browser.host() else {
        return cb(None);
    };
    let bid = browser.identifier();
    if !OBSERVERS.with_borrow(|o| o.contains_key(&bid)) {
        if let Some(r) = host.add_dev_tools_message_observer(Some(&mut Observer::new())) {
            OBSERVERS.with_borrow_mut(|o| o.insert(bid, r));
        }
    }
    let id = NEXT_ID.replace(NEXT_ID.get() + 1);
    PENDING.with_borrow_mut(|p| p.insert((bid, id), Box::new(cb)));
    let msg = json!({ "id": id, "method": method, "params": params }).to_string();
    if host.send_dev_tools_message(Some(msg.as_bytes())) == 0 {
        if let Some(cb) = PENDING.with_borrow_mut(|p| p.remove(&(bid, id))) {
            cb(None);
        }
    }
}

/// Fire and forget.
pub fn send(browser: &Browser, method: &str, params: Value) {
    call(browser, method, params, |_| {});
}

/// Run `js` in an isolated world of the main frame (page scripts can't see or
/// patch our globals). `cb` gets (context id, returned value); reuse the id
/// with `eval_in` for later calls in the same world.
pub fn eval_isolated(
    browser: &Browser,
    js: String,
    cb: impl FnOnce(Option<(i64, Value)>) + 'static,
) {
    let b = browser.clone();
    call(browser, "Page.getFrameTree", json!({}), move |tree| {
        let Some(frame) = tree
            .as_ref()
            .and_then(|t| t["frameTree"]["frame"]["id"].as_str())
        else {
            return cb(None);
        };
        let params = json!({ "frameId": frame, "worldName": "novabrowser" });
        let b2 = b.clone();
        call(&b, "Page.createIsolatedWorld", params, move |world| {
            let Some(ctx) = world.and_then(|w| w["executionContextId"].as_i64()) else {
                return cb(None);
            };
            eval_in(&b2, ctx, js, move |v| cb(v.map(|v| (ctx, v))));
        });
    });
}

/// Evaluate `js` in context `ctx`; `cb` gets its JSON value, or None on an exception.
pub fn eval_in(browser: &Browser, ctx: i64, js: String, cb: impl FnOnce(Option<Value>) + 'static) {
    let params =
        json!({ "expression": js, "contextId": ctx, "returnByValue": true, "awaitPromise": true });
    call(browser, "Runtime.evaluate", params, move |r| {
        let r = r.filter(|r| r.get("exceptionDetails").is_none());
        cb(r.map(|r| r["result"]["value"].clone()));
    });
}

/// A trusted mouse-wheel event at the viewport centre: scrolls whatever is
/// under it (inner scroll containers too), with the page's smooth scrolling.
pub fn wheel(browser: &Browser, dx: f64, dy: f64) {
    let (w, h) = crate::window::content_size();
    let p = json!({ "type": "mouseWheel", "x": w / 2.0, "y": h / 2.0, "deltaX": dx, "deltaY": dy });
    send(browser, "Input.dispatchMouseEvent", p);
}

/// A trusted click (press + release) at viewport CSS px; `modifiers` per CDP (2 = Ctrl).
pub fn click(browser: &Browser, x: f64, y: f64, button: &str, modifiers: i32) {
    for t in ["mouseMoved", "mousePressed", "mouseReleased"] {
        let p = json!({ "type": t, "x": x, "y": y, "button": if t == "mouseMoved" { "none" } else { button },
                        "clickCount": 1, "modifiers": modifiers });
        send(browser, "Input.dispatchMouseEvent", p);
    }
}

pub fn hover(browser: &Browser, x: f64, y: f64) {
    send(
        browser,
        "Input.dispatchMouseEvent",
        json!({ "type": "mouseMoved", "x": x, "y": y }),
    );
}
