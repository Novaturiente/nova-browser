//! Brave's adblock engine (adblock-rust): network blocking on the CEF IO
//! thread, plus per-site cosmetic CSS and uBlock scriptlets injected into the
//! page's main world before its own scripts. Lists are fetched in the
//! background, cached on disk, and refreshed when older than a day.
use crate::{config, devtools, urlmatch::UrlPattern, window};
use adblock::{
    lists::{FilterSet, ParseOptions},
    request::Request,
    resources::Resource,
    Engine,
};
use cef::*;
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, SystemTime},
};

static ENGINE: RwLock<Option<Arc<Engine>>> = RwLock::new(None);
/// One update at a time (daily timer vs `:adblock-update`).
static UPDATING: Mutex<()> = Mutex::new(());
const MAX_AGE: Duration = Duration::from_secs(24 * 3600);

thread_local! {
    /// browser id -> identifier of its Page.addScriptToEvaluateOnNewDocument script.
    static SCRIPTS: RefCell<HashMap<i32, String>> = RefCell::new(HashMap::new());
}

fn dir() -> PathBuf {
    crate::xdg_dir("XDG_DATA_HOME", ".local/share").join("adblock")
}

/// uBlock Origin scriptlets, built by scripts/gen-adblock-resources.mjs.
/// shortcut: bundled at build time, so new scriptlets need a rebuild (rules in the lists update daily).
fn resources() -> Vec<Resource> {
    serde_json::from_str(include_str!("adblock-resources.json")).expect("bundled adblock resources")
}

fn engine() -> Option<Arc<Engine>> {
    ENGINE.read().unwrap().clone()
}

fn enabled(cfg: &config::Config, url: &str) -> bool {
    cfg.get_for("content.blocking.enabled", url)
        .as_bool()
        .unwrap_or(false)
        && matches!(
            cfg.str("content.blocking.method").as_str(),
            "auto" | "adblock" | "both"
        )
}

fn lists_key(cfg: &config::Config) -> String {
    cfg.list("content.blocking.adblock.lists").join("\n")
}

wrap_task! {
    struct Message { level: String, text: String }
    impl Task {
        fn execute(&self) { window::message(&self.level, &self.text); }
    }
}

fn report(level: &str, text: String) {
    post_task(
        ThreadId::UI,
        Some(&mut Message::new(level.to_string(), text)),
    );
}

/// Load the cached engine, then keep the lists fresh. Call after CEF init.
pub fn start() {
    std::thread::Builder::new()
        .name("adblock".into())
        .spawn(|| {
            let fresh = load_cache();
            if !fresh {
                update(false);
            }
            loop {
                std::thread::sleep(Duration::from_secs(3600));
                if !load_cache_fresh() {
                    update(false);
                }
            }
        })
        .expect("spawn adblock thread");
}

/// `:adblock-update`
pub fn update_now() {
    std::thread::spawn(|| update(true));
}

/// Install the cached engine if there is one. True if it is fresh (same lists, < 1 day old).
fn load_cache() -> bool {
    let d = dir();
    let Ok(bytes) = std::fs::read(d.join("engine.dat")) else {
        return false;
    };
    let mut e = Engine::default();
    if e.deserialize(&bytes).is_err() {
        eprintln!("novabrowser: adblock cache unreadable, rebuilding");
        return false;
    }
    e.use_resources(resources());
    *ENGINE.write().unwrap() = Some(Arc::new(e));
    load_cache_fresh()
}

fn load_cache_fresh() -> bool {
    let d = dir();
    let same_lists =
        std::fs::read_to_string(d.join("lists.txt")).is_ok_and(|k| k == lists_key(&config::get()));
    let age = std::fs::metadata(d.join("engine.dat"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok());
    same_lists && age.is_some_and(|a| a < MAX_AGE)
}

fn fetch(url: &str) -> Result<String, String> {
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::read_to_string(path).map_err(|e| e.to_string());
    }
    // shortcut: curl instead of an HTTP crate; swap if curl ever isn't on the target systems.
    let out = Command::new("curl")
        .args(["-fsSL", "--compressed", "--max-time", "120", url])
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

/// Download every list (falling back to its last copy), build, cache, install.
fn update(verbose: bool) {
    let Ok(_lock) = UPDATING.try_lock() else {
        if verbose {
            report("info", "adblock: update already running".into());
        }
        return;
    };
    let cfg = config::get();
    let urls = cfg.list("content.blocking.adblock.lists");
    let d = dir();
    if let Err(e) = std::fs::create_dir_all(&d) {
        return report("error", format!("adblock: {}: {e}", d.display()));
    }
    let mut set = FilterSet::new(false);
    let mut failed = Vec::new();
    let mut rules = 0;
    for url in &urls {
        let copy = d.join(format!("list-{:016x}.txt", fnv(url)));
        let text = match fetch(url) {
            Ok(t) => {
                let _ = write_atomic(&copy, t.as_bytes());
                t
            }
            Err(e) => {
                eprintln!("novabrowser: adblock: {url}: {e}");
                failed.push(url.as_str());
                match std::fs::read_to_string(&copy) {
                    Ok(t) => t,
                    Err(_) => continue,
                }
            }
        };
        rules += text.lines().count();
        set.add_filter_list(text, ParseOptions::default());
    }
    let mut e = Engine::new_with_filter_set(set);
    e.use_resources(resources());
    let ok = write_atomic(&d.join("engine.dat"), &e.serialize()).is_ok()
        && write_atomic(&d.join("lists.txt"), lists_key(&cfg).as_bytes()).is_ok();
    if !ok {
        eprintln!(
            "novabrowser: adblock: could not write cache in {}",
            d.display()
        );
    }
    *ENGINE.write().unwrap() = Some(Arc::new(e));
    let msg = format!(
        "adblock: {} lists, {rules} lines",
        urls.len() - failed.len()
    );
    if !failed.is_empty() {
        report(
            "warning",
            format!("{msg}; failed (kept old copy): {}", failed.join(", ")),
        );
    } else if verbose {
        report("info", msg);
    } else {
        eprintln!("novabrowser: {msg}");
    }
}

fn write_atomic(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(tmp, path)
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

fn request_type(t: ResourceType) -> &'static str {
    match t {
        ResourceType::MAIN_FRAME => "document",
        ResourceType::SUB_FRAME => "subdocument",
        ResourceType::STYLESHEET => "stylesheet",
        ResourceType::SCRIPT => "script",
        ResourceType::IMAGE | ResourceType::FAVICON => "image",
        ResourceType::FONT_RESOURCE => "font",
        ResourceType::OBJECT | ResourceType::PLUGIN_RESOURCE => "object",
        ResourceType::MEDIA => "media",
        ResourceType::XHR => "xmlhttprequest",
        ResourceType::PING => "ping",
        ResourceType::CSP_REPORT => "csp_report",
        _ => "other",
    }
}

/// IO thread: should this subresource request be cancelled? `source` = the tab's page URL.
pub fn should_block(url: &str, source: &str, t: ResourceType) -> bool {
    if t == ResourceType::MAIN_FRAME {
        return false; // like qutebrowser: never block the page you asked for
    }
    let Some(e) = engine() else { return false };
    let cfg = config::get();
    if !enabled(&cfg, source) {
        return false;
    }
    // shortcut: whitelist patterns parsed per request; cache them if the list ever gets long.
    for p in cfg.list("content.blocking.whitelist") {
        if UrlPattern::parse(&p).is_ok_and(|p| p.matches(url)) {
            return false;
        }
    }
    let Ok(req) = Request::new(url, source, request_type(t), "get") else {
        return false;
    };
    e.check_network_request(&req).should_block()
}

/// UI thread, before each main-frame navigation: swap the tab's injected script
/// for one carrying this site's hide CSS and scriptlets.
pub fn on_navigate(browser: &Browser, url: &str) {
    let bid = browser.identifier();
    if let Some(old) = SCRIPTS.with_borrow_mut(|s| s.remove(&bid)) {
        devtools::send(
            browser,
            "Page.removeScriptToEvaluateOnNewDocument",
            json!({ "identifier": old }),
        );
    }
    let Some(e) = engine() else { return };
    if !url.starts_with("http") || !enabled(&config::get(), url) {
        return;
    }
    let Some(js) = page_script(&e, url) else {
        return;
    };
    // New-document scripts only run while the Page domain is enabled in this session.
    devtools::send(browser, "Page.enable", json!({}));
    devtools::call(
        browser,
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": js }),
        move |r| {
            if let Some(id) = r
                .as_ref()
                .and_then(|r| r.get("identifier"))
                .and_then(Value::as_str)
            {
                SCRIPTS.with_borrow_mut(|s| s.insert(bid, id.to_string()));
            }
        },
    );
}

pub fn forget(browser: &Browser) {
    SCRIPTS.with_borrow_mut(|s| s.remove(&browser.identifier()));
}

/// UI thread, main frame committed (on_load_start): start the generic-hiding scanner in
/// a fresh "novabrowser" isolated world with its binding. Bound by context id: binding by
/// world name only reaches new contexts while Runtime is enabled, and Runtime.enable is
/// visible to anti-bot scripts (spike 2026-10-11).
/// shortcut: main frame only, and it starts after commit, so a generic ad can flash briefly.
pub fn on_load_start(browser: &Browser, url: &str) {
    if engine().is_none() || !url.starts_with("http") || !enabled(&config::get(), url) {
        return;
    }
    let b = browser.clone();
    devtools::call(browser, "Page.getFrameTree", json!({}), move |r| {
        let Some(frame) = r
            .as_ref()
            .and_then(|r| r["frameTree"]["frame"]["id"].as_str())
            .map(str::to_string)
        else {
            return;
        };
        let b2 = b.clone();
        devtools::call(
            &b,
            "Page.createIsolatedWorld",
            json!({ "frameId": frame, "worldName": "novabrowser" }),
            move |r| {
                let Some(ctx) = r.as_ref().and_then(|r| r["executionContextId"].as_i64()) else {
                    return;
                };
                devtools::send(
                    &b2,
                    "Runtime.addBinding",
                    json!({ "name": "__novaClassIds", "executionContextId": ctx }),
                );
                devtools::send(
                    &b2,
                    "Runtime.evaluate",
                    json!({ "expression": include_str!("adblock-scan.js"), "contextId": ctx }),
                );
            },
        );
    });
}

/// `Runtime.bindingCalled` from adblock-scan.js: new class names / ids on a page.
/// Answers with the generic hide rules that match them (adblock-rust's
/// hidden_class_id_selectors, as Brave does).
pub fn on_binding(browser: &Browser, params: &Value) {
    if params["name"] != "__novaClassIds" {
        return;
    }
    let (Some(ctx), Some(payload)) = (
        params["executionContextId"].as_i64(),
        params["payload"].as_str(),
    ) else {
        return;
    };
    let Ok(p) = serde_json::from_str::<Value>(payload) else {
        return;
    };
    let url = p["u"].as_str().unwrap_or("");
    let Some(e) = engine() else { return };
    if !url.starts_with("http") || !enabled(&config::get(), url) {
        return;
    }
    let r = e.url_cosmetic_resources(url);
    if r.generichide {
        return;
    }
    let strs = |k: &str| {
        p[k].as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let sels = e.hidden_class_id_selectors(strs("c"), strs("i"), &r.exceptions);
    if sels.is_empty() {
        return;
    }
    let css: Vec<String> = sels
        .iter()
        .map(|s| format!("{s}{{display:none!important}}"))
        .collect();
    devtools::send(
        browser,
        "Runtime.evaluate",
        json!({ "expression": format!("globalThis.__novaHide({})", serde_json::to_string(&css.join("\n")).unwrap()), "contextId": ctx }),
    );
}

/// shortcut: no generic class/id hiding (needs a DOM scan round trip) and no
/// procedural filters (:has-text etc.); specific hides + scriptlets only.
fn page_script(e: &Engine, url: &str) -> Option<String> {
    let r = e.url_cosmetic_resources(url);
    if r.hide_selectors.is_empty() && r.injected_script.is_empty() {
        return None;
    }
    let host = crate::commands::host(url);
    // One rule per selector, so one invalid selector only drops itself.
    let css: String = r
        .hide_selectors
        .iter()
        .map(|s| format!("{s}{{display:none!important}}\n"))
        .collect();
    // Hostname guard: the script stays registered for same-tab navigations and runs in frames.
    // Constructable stylesheet: not subject to the page's CSP, unlike a <style> element.
    Some(format!(
        "if(location.hostname==={host}){{try{{{script}\n}}catch(e){{}}\
         try{{const s=new CSSStyleSheet();s.replaceSync({css});\
         document.adoptedStyleSheets=[...document.adoptedStyleSheets,s]}}catch(e){{}}}}",
        host = serde_json::to_string(&host).unwrap(),
        script = r.injected_script,
        css = serde_json::to_string(&css).unwrap(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_with(rules: &str) -> Engine {
        let mut set = FilterSet::new(false);
        set.add_filter_list(rules.to_string(), ParseOptions::default());
        let mut e = Engine::new_with_filter_set(set);
        e.use_resources(resources());
        e
    }

    #[test]
    fn network_and_cosmetic() {
        let e = engine_with(
            "||ads.example^\n@@||ads.example/ok.js\nexample.com##.banner\n\
             youtube.com##+js(set-constant, ytInitialPlayerResponse.adPlacements, undefined)\n",
        );
        let check = |u: &str| {
            e.check_network_request(
                &Request::new(u, "https://site.test/", "script", "get").unwrap(),
            )
            .should_block()
        };
        assert!(check("https://ads.example/x.js"));
        assert!(!check("https://ads.example/ok.js"));
        assert!(!check("https://cdn.example/x.js"));
        let js = page_script(&e, "https://example.com/a").unwrap();
        assert!(js.contains(".banner{display:none!important}") && js.contains("\"example.com\""));
        let yt = page_script(&e, "https://www.youtube.com/watch?v=x").unwrap();
        assert!(
            yt.contains("adPlacements"),
            "scriptlet resolved from bundled resources"
        );
        assert!(page_script(&e, "https://other.test/").is_none());
    }

    #[test]
    fn engine_roundtrips_through_cache_format() {
        let e = engine_with("||ads.example^\n");
        let mut back = Engine::default();
        back.deserialize(&e.serialize()).unwrap();
        let req = Request::new(
            "https://ads.example/a.js",
            "https://site.test/",
            "script",
            "get",
        )
        .unwrap();
        assert!(back.check_network_request(&req).should_block());
    }
}
