//! aria2 download backend: the engine-free part (aria2c process, RPC, polling, session
//! file) lives in `nova_core::aria2`; this file only gathers what CEF knows about a
//! download (the tab's cookies, user agent and referer) before handing it over.
use cef::*;
pub use nova_core::aria2::{enabled, remove, stop};
use serde_json::json;

pub fn start() {
    nova_core::aria2::start("chromium", crate::downloads::aria2_update, crate::downloads::aria2_added);
}

fn dir() -> String {
    crate::downloads::dir().to_string_lossy().into_owned()
}

/// Hand `url` to aria2 with the cookies, user agent and referer the tab would
/// send. Async: the result arrives as `downloads::aria2_added(id, …)`.
pub fn add(browser: &Browser, url: String, id: u64) {
    let referer = browser
        .main_frame()
        .and_then(|f| crate::net::referrer(&CefString::from(&f.url()).to_string(), &url))
        .unwrap_or_default();
    let b = browser.clone();
    crate::devtools::call(
        browser,
        "Network.getCookies",
        json!({ "urls": [url] }),
        move |r| {
            let cookies: Vec<String> = r
                .as_ref()
                .and_then(|r| r["cookies"].as_array())
                .into_iter()
                .flatten()
                .filter_map(|c| Some(format!("{}={}", c["name"].as_str()?, c["value"].as_str()?)))
                .collect();
            let expr = json!({ "expression": "navigator.userAgent", "returnByValue": true });
            crate::devtools::call(&b, "Runtime.evaluate", expr, move |ua| {
                let ua = ua.as_ref().and_then(|u| u["result"]["value"].as_str()).map(str::to_string);
                let mut header = Vec::new();
                if !cookies.is_empty() {
                    header.push(format!("Cookie: {}", cookies.join("; ")));
                }
                if !referer.is_empty() {
                    header.push(format!("Referer: {referer}"));
                }
                nova_core::aria2::add(url, id, &dir(), header, ua);
            });
        },
    );
}

/// A magnet: link (any thread).
pub fn add_magnet(url: String, id: u64) {
    nova_core::aria2::add(url, id, &dir(), Vec::new(), None);
}
