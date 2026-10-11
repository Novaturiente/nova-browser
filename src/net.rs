//! Per-request policy (CEF IO thread): cookies, request headers, HTTPS-only.
use crate::config;
use cef::*;
use serde_json::Value;
use std::collections::BTreeSet;
use std::net::IpAddr;
use std::sync::{Mutex, RwLock};

/// `host` or `host:port` lines from <config>/https_excludes: left on http.
static HTTPS_EXCLUDES: RwLock<Vec<String>> = RwLock::new(Vec::new());

pub fn excludes_file() -> std::path::PathBuf {
    config::path().with_file_name("https_excludes")
}

/// (Re)read https_excludes; at start and after `https-toggle` / `config-source`.
pub fn load_https_excludes() {
    let lines = std::fs::read_to_string(excludes_file()).unwrap_or_default();
    *HTTPS_EXCLUDES.write().unwrap() = lines
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();
}

/// "host:port" (port only if explicit) and the bare host of an http(s) URL.
pub fn authority(url: &str) -> Option<(&str, &str)> {
    let rest = url.split_once("://")?.1;
    let auth = rest.split(['/', '?', '#']).next()?;
    let auth = auth.rsplit_once('@').map_or(auth, |(_, a)| a);
    let host = match auth.strip_prefix('[') {
        Some(v6) => v6.split(']').next()?,
        None => auth.split(':').next()?,
    };
    Some((auth, host))
}

/// Hosts that can't hold a public certificate (same rules as the old https_only.py).
fn is_local(host: &str) -> bool {
    if host.is_empty() || host == "localhost" || !host.contains('.') && !host.contains(':') {
        return true;
    }
    if [".local", ".lan", ".internal", ".home", ".arpa", ".ts.net"]
        .iter()
        .any(|s| host.ends_with(s))
    {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(a)) => {
            let o = a.octets();
            a.is_private()
                || a.is_loopback()
                || a.is_link_local()
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // Tailscale 100.64/10
        }
        Ok(IpAddr::V6(a)) => {
            a.is_loopback()
                || (a.segments()[0] & 0xfe00) == 0xfc00
                || (a.segments()[0] & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

/// HTTPS-only: the https:// URL to load instead, if this request should be upgraded.
pub fn https_upgrade(url: &str) -> Option<String> {
    let rest = url.strip_prefix("http://")?;
    let (auth, host) = authority(url)?;
    let ex = HTTPS_EXCLUDES.read().unwrap();
    if is_local(host) || ex.iter().any(|e| e == host || e == auth) {
        return None;
    }
    Some(format!("https://{rest}"))
}

/// Registrable domain (eTLD+1), via the adblock crate's public suffix list.
fn site(url: &str) -> Option<String> {
    adblock::url_parser::parse_url(url).map(|u| u.domain().to_string())
}

// ---- Cookies, Chromium side. Its third-party blocking also covers document.cookie
// and storage, which our CookieAccessFilter (network only) can't see. Exceptions
// must be exact (third-party origin, page origin) pairs: CEF can't express "any
// third party on this page" (spike 2026-10-11), so pairs are added as seen.

/// Pairs we allowed via set_content_setting; Chromium persists them in the profile,
/// so we keep our own list to undo them when the config stops asking for them.
static PAIRS: Mutex<BTreeSet<(String, String)>> = Mutex::new(BTreeSet::new());

fn pairs_file() -> std::path::PathBuf {
    crate::xdg_dir("XDG_DATA_HOME", ".local/share").join("cookie_exceptions.json")
}

fn save_pairs(p: &BTreeSet<(String, String)>) {
    let f = pairs_file();
    let text = serde_json::to_string(p).unwrap();
    if let Err(e) = crate::write_atomic(&f, text.as_bytes()) {
        eprintln!("novabrowser: {}: {e}", f.display());
    }
}

fn origin(url: &str) -> Option<String> {
    let scheme = url.split_once("://")?.0;
    Some(format!("{scheme}://{}/", authority(url)?.0))
}

/// UI thread, at start and after any config change: map `content.cookies.accept`
/// onto Chromium's cookie controls, and drop exceptions the config no longer wants.
pub fn apply_cookie_policy() {
    let Some(ctx) = request_context_get_global_context() else {
        return;
    };
    let cfg = config::get();
    let global = cfg.str("content.cookies.accept");
    let v = value_create().expect("cef value");
    v.set_int(matches!(global.as_str(), "no-3rdparty" | "no-unknown-3rdparty") as i32);
    crate::set_pref(&ctx, "profile.cookie_controls_mode", v);
    // shortcut: per-site overrides under a global `never` aren't mapped (they'd need allow pairs too).
    let default = if global == "never" {
        ContentSettingValues::BLOCK
    } else {
        ContentSettingValues::DEFAULT
    };
    ctx.set_content_setting(None, None, ContentSettingTypes::COOKIES, default);
    let mut pairs = PAIRS.lock().unwrap();
    if pairs.is_empty() {
        *pairs = std::fs::read_to_string(pairs_file())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
    }
    let before = pairs.len();
    pairs.retain(|(req, page)| {
        let keep = global_blocks_third_party(&cfg) && page_allows_all(&cfg, page);
        if !keep {
            ctx.set_content_setting(
                Some(&req.as_str().into()),
                Some(&page.as_str().into()),
                ContentSettingTypes::COOKIES,
                ContentSettingValues::DEFAULT,
            );
        }
        keep
    });
    if pairs.len() != before {
        save_pairs(&pairs);
    }
}

fn global_blocks_third_party(cfg: &config::Config) -> bool {
    matches!(
        cfg.str("content.cookies.accept").as_str(),
        "no-3rdparty" | "no-unknown-3rdparty"
    )
}

fn page_allows_all(cfg: &config::Config, page: &str) -> bool {
    cfg.get_for("content.cookies.accept", page).as_str() == Some("all")
}

wrap_task! {
    struct AllowPair { req: String, page: String }
    impl Task {
        fn execute(&self) {
            if let Some(ctx) = request_context_get_global_context() {
                ctx.set_content_setting(Some(&self.req.as_str().into()), Some(&self.page.as_str().into()), ContentSettingTypes::COOKIES, ContentSettingValues::ALLOW);
            }
            save_pairs(&PAIRS.lock().unwrap());
        }
    }
}

/// IO thread, every subresource request: on a page whose pattern says `all` while
/// the global setting blocks third parties, allow this (third party, page) pair.
/// shortcut: the pair's very first request goes out before the exception exists
/// (no cookie on it); later ones and later visits are fine.
pub fn note_request(url: &str, page: &str) {
    let cfg = config::get();
    if page.is_empty() || !global_blocks_third_party(&cfg) || !page_allows_all(&cfg, page) {
        return;
    }
    if matches!((site(url), site(page)), (Some(a), Some(b)) if a == b) {
        return;
    }
    let (Some(req), Some(pg)) = (origin(url), origin(page)) else {
        return;
    };
    if PAIRS.lock().unwrap().insert((req.clone(), pg.clone())) {
        post_task(ThreadId::UI, Some(&mut AllowPair::new(req, pg)));
    }
}

/// `content.cookies.accept`, looked up for the tab's page (`first_party`) like qutebrowser.
/// Network cookies; Chromium's own blocking (apply_cookie_policy) adds document.cookie and storage.
pub fn cookie_allowed(url: &str, first_party: &str) -> bool {
    let first_party = if first_party.is_empty() {
        url
    } else {
        first_party
    };
    match config::get()
        .get_for("content.cookies.accept", first_party)
        .as_str()
    {
        Some("never") => false,
        // shortcut: no-unknown-3rdparty treated as no-3rdparty (no "already has cookies" lookup).
        Some("no-3rdparty" | "no-unknown-3rdparty") => {
            matches!((site(url), site(first_party)), (Some(a), Some(b)) if a == b)
        }
        _ => true,
    }
}

/// `content.headers.*`, looked up for the request URL like qutebrowser.
/// Accept-Language is global (`language_list`): Chromium's network layer overwrites the header.
pub fn set_headers(req: &mut Request, url: &str) {
    let cfg = config::get();
    let set = |k: &str, v: &str| req.set_header_by_name(Some(&k.into()), Some(&v.into()), 1);
    if let Value::Object(h) = cfg.get_for("content.headers.custom", url) {
        for (k, v) in h {
            if let Some(v) = v.as_str() {
                set(&k, v);
            }
        }
    }
    match cfg.get_for("content.headers.do_not_track", url) {
        Value::Bool(true) => set("DNT", "1"),
        Value::Bool(false) => set("DNT", "0"),
        _ => {}
    }
}

/// "en-US,en;q=0.9" -> "en-US,en": CEF wants bare languages and adds q-values itself.
pub fn language_list(header: &str) -> String {
    header
        .split(',')
        .map(|l| l.split(';').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    #[test]
    fn site_is_registrable_domain() {
        assert_eq!(
            super::site("https://a.b.example.co.uk/x").as_deref(),
            Some("example.co.uk")
        );
        assert_eq!(
            super::site("https://accounts.google.com/").as_deref(),
            Some("google.com")
        );
        assert_eq!(
            super::language_list("en-US,en;q=0.9, de ;q=0.5"),
            "en-US,en,de"
        );
    }

    #[test]
    fn https_upgrade_rules() {
        use super::https_upgrade as up;
        assert_eq!(
            up("http://example.com/a?b").as_deref(),
            Some("https://example.com/a?b")
        );
        assert_eq!(up("https://example.com/"), None);
        for local in [
            "http://localhost:3000/",
            "http://novahome:2283/",
            "http://192.168.1.1/",
            "http://10.0.0.5/",
            "http://100.88.215.101:8080/",
            "http://nas.local/",
            "http://x.tail9d0106.ts.net/",
            "http://[::1]:80/",
            "http://127.0.0.1/",
            "http://u:p@router/",
        ] {
            assert_eq!(up(local), None, "{local}");
        }
        assert!(up("http://100.128.0.1/").is_some(), "outside 100.64/10");
        *super::HTTPS_EXCLUDES.write().unwrap() =
            vec!["old.example".into(), "port.example:8080".into()];
        assert_eq!(up("http://old.example/x"), None);
        assert_eq!(up("http://port.example:8080/"), None);
        assert!(up("http://port.example/").is_some());
    }
}
