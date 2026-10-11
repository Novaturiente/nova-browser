//! Command line parsing and the Phase 2 command set (qutebrowser names).
use crate::{clipboard, config, devtools, hints, modes, window};
use cef::*;
use std::cell::RefCell;

/// One parsed command: name, `-x`/`--long` flags, positional args.
#[derive(Debug, PartialEq)]
pub struct Cmd {
    pub name: String,
    pub flags: Vec<String>,
    pub args: Vec<String>,
}

impl Cmd {
    fn flag(&self, short: &str, long: &str) -> bool {
        self.flags.iter().any(|f| f == short || f == long)
    }
    fn rest(&self) -> String {
        self.args.join(" ")
    }
}

/// Shell-like split: whitespace separates, '…' and "…" group, `\` escapes outside
/// '…' (qutebrowser; `fake-key \<` sends a literal `<`).
fn tokenize(s: &str) -> Result<Vec<String>, String> {
    let (mut out, mut cur, mut quote, mut has) = (vec![], String::new(), None, false);
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, c) if c.is_whitespace() => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (None, '\'' | '"') => {
                quote = Some(c);
                has = true;
            }
            (Some(q), c) if c == q => quote = None,
            (None | Some('"'), '\\') => {
                cur.extend(chars.next());
                has = true;
            }
            (_, c) => {
                cur.push(c);
                has = true;
            }
        }
    }
    if quote.is_some() {
        return Err(format!("unterminated quote in: {s}"));
    }
    if has {
        out.push(cur);
    }
    Ok(out)
}

/// `a ;; b` → ["a", "b"]; aliases expand on the first word, and an alias may
/// itself contain `;;` (qutebrowser expands before splitting).
pub fn parse_line(line: &str) -> Result<Vec<Cmd>, String> {
    let aliases = config::get().get("aliases");
    let mut out = Vec::new();
    split_expand(line.trim().trim_start_matches(':'), &aliases, 0, &mut out)?;
    out.iter().map(|p| parse_one(p)).collect()
}

fn split_expand(line: &str, aliases: &serde_json::Value, depth: u32, out: &mut Vec<String>) -> Result<(), String> {
    if depth > 8 {
        return Err("alias loop".into());
    }
    for part in line.split(";;").map(str::trim).filter(|p| !p.is_empty()) {
        let (first, tail) = part.split_once(char::is_whitespace).unwrap_or((part, ""));
        match aliases.get(first).and_then(|v| v.as_str()) {
            Some(target) => split_expand(&format!("{target} {tail}"), aliases, depth + 1, out)?,
            None => out.push(part.to_string()),
        }
    }
    Ok(())
}

fn parse_one(part: &str) -> Result<Cmd, String> {
    // Placeholders are filled per token, so a title with quotes or spaces stays one argument.
    let mut tokens = tokenize(part)?.into_iter().map(|t| expand(&t));
    let name = tokens.next().ok_or("empty command")?;
    let (mut flags, mut args, mut flags_done) = (vec![], vec![], false);
    for t in tokens {
        if !flags_done && t == "--" {
            flags_done = true;
        } else if !flags_done
            && args.is_empty()
            && t.len() > 1
            && t.starts_with('-')
            && !t[1..].starts_with(|c: char| c.is_ascii_digit())
        {
            if t.starts_with("--") {
                flags.push(t);
            } else {
                // -sr → -s -r
                flags.extend(t[1..].chars().map(|c| format!("-{c}")));
            }
        } else {
            args.push(t);
        }
    }
    Ok(Cmd { name, flags, args })
}

/// qutebrowser `url.searchengines` + `url.auto_search`: `g rust` uses engine `g`;
/// otherwise (naive) one word with a dot or localhost is a URL, the rest is searched.
pub fn url_or_search(text: &str) -> String {
    let cfg = config::get();
    let engines = cfg.get("url.searchengines");
    url_or_search_with(text, &engines, &cfg.str("url.auto_search"))
}

fn url_or_search_with(text: &str, engines: &serde_json::Value, auto_search: &str) -> String {
    let t = text.trim();
    if let Some((word, rest)) = t.split_once(' ') {
        if let Some(tpl) = engines.get(word).and_then(|v| v.as_str()).filter(|_| word != "DEFAULT") {
            return search_url(tpl, rest.trim());
        }
    }
    let has_scheme = t.contains("://")
        || ["about:", "file:", "data:", "chrome:"]
            .iter()
            .any(|p| t.starts_with(p));
    if has_scheme {
        return t.to_string();
    }
    // shortcut: "dns" and "schemeless" behave like "naive" (no DNS lookups)
    let looks_like_url = !t.contains(char::is_whitespace) && (t.starts_with("localhost") || t.contains('.'));
    if auto_search == "never" || looks_like_url {
        let scheme = if t.starts_with("localhost") {
            "http"
        } else {
            "https"
        };
        return format!("{scheme}://{t}");
    }
    let tpl = engines.get("DEFAULT").and_then(|v| v.as_str()).unwrap_or("https://duckduckgo.com/?q={}");
    search_url(tpl, t)
}

/// Fill a search engine template: `{}` quoted, `{unquoted}` raw (qutebrowser).
fn search_url(tpl: &str, term: &str) -> String {
    let q: String = term
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    tpl.replace("{unquoted}", term).replace("{semiquoted}", &q).replace("{quoted}", &q).replace("{}", &q)
}

/// `zoom.levels` as numbers ("110%" → 110).
fn zoom_levels() -> Vec<f64> {
    let mut v: Vec<f64> = config::get()
        .list("zoom.levels")
        .iter()
        .filter_map(|z| z.trim_end_matches('%').parse().ok())
        .collect();
    v.sort_by(f64::total_cmp);
    if v.is_empty() {
        v.push(100.0);
    }
    v
}

/// Next zoom level `steps` away from `current` percent (negative = out).
pub fn zoom_step(current: f64, steps: i32) -> f64 {
    zoom_step_in(&zoom_levels(), current, steps)
}

fn zoom_step_in(levels: &[f64], current: f64, steps: i32) -> f64 {
    let last = levels.len() as i32 - 1;
    // first level above (zoom in) / below (zoom out) the current one, then the remaining steps
    let target = if steps > 0 {
        levels
            .iter()
            .position(|&z| z > current + 0.5)
            .map_or(last, |i| i as i32 + steps - 1)
    } else {
        levels
            .iter()
            .rposition(|&z| z < current - 0.5)
            .map_or(0, |i| i as i32 + steps + 1)
    };
    levels[target.clamp(0, last) as usize]
}

/// CEF zoom level is log base 1.2 of the factor.
fn pct_to_level(pct: f64) -> f64 {
    (pct / 100.0).ln() / 1.2f64.ln()
}
fn level_to_pct(level: f64) -> f64 {
    100.0 * 1.2f64.powf(level)
}

thread_local! {
    /// {clipboard}, {primary} for the command line being run (read off-thread first).
    static CLIP: RefCell<(String, String)> = const { RefCell::new((String::new(), String::new())) };
}

fn expand(text: &str) -> String {
    if !text.contains('{') {
        return text.to_string();
    }
    let (url, title) = window::active_url_title();
    let (clip, primary) = CLIP.with_borrow(|c| c.clone());
    // shortcut: {url:pretty} and {url:yank} = {url}; add percent-decoding if it matters
    text.replace("{url:pretty}", &url)
        .replace("{url:yank}", &url)
        .replace("{url:host}", host(&url))
        .replace("{url}", &url)
        .replace("{title}", &title)
        .replace("{clipboard}", &clip)
        .replace("{primary}", &primary)
}

/// Host of a URL, without userinfo or port.
pub fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    let auth = auth.rsplit_once('@').map_or(auth, |(_, h)| h);
    if auth.starts_with('[') {
        return auth.split_inclusive(']').next().unwrap_or(auth);
    }
    auth.split(':').next().unwrap_or(auth)
}

/// `dark-toggle`: flip force-dark for this host now, and remember it in
/// `<config>/darkmode_excludes` (the file config.ts reads with `readLines`).
fn dark_toggle() -> Result<(), String> {
    const D: &str = "colors.webpage.darkmode.enabled";
    let (url, _) = window::active_url_title();
    let host = host(&url).to_string();
    if host.is_empty() {
        return Err("no host for this page".into());
    }
    let file = config::path().with_file_name("darkmode_excludes");
    let mut lines: Vec<String> = std::fs::read_to_string(&file)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    let cfg = config::get();
    let dark = !cfg.get_for(D, &url).as_bool().unwrap_or(false);
    if dark {
        lines.retain(|l| *l != host);
    } else if !lines.contains(&host) {
        lines.push(host.clone());
    }
    let tmp = file.with_extension("tmp");
    std::fs::write(&tmp, lines.join("\n") + "\n")
        .and_then(|_| std::fs::rename(&tmp, &file))
        .map_err(|e| format!("{}: {e}", file.display()))?;
    cfg.set_runtime_for(&format!("*://{host}/*"), D, &dark.to_string())?;
    window::config_changed();
    let still = dark && lines.iter().any(|l| l.starts_with("*.") && host.ends_with(&l[1..]));
    let note = if still { " (a wildcard in darkmode_excludes still excludes it after a restart)" } else { "" };
    window::message("info", &format!("Dark mode {} for {host}{note}", if dark { "on" } else { "off" }));
    Ok(())
}

/// `https-toggle`: add/remove this page's host (host:port if explicit) in
/// https_excludes, then reload it over the other scheme.
fn https_toggle() -> Result<(), String> {
    // The frame's URL, not the tab's last committed one: after a failed https upgrade they differ.
    let url = window::active_browser()
        .and_then(|b| b.main_frame())
        .map(|f| CefString::from(&f.url()).to_string())
        .unwrap_or_default();
    let (auth, _) = crate::net::authority(&url).ok_or("no host for this page")?;
    let file = crate::net::excludes_file();
    let mut lines: Vec<String> = std::fs::read_to_string(&file)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    let excluded = lines.iter().any(|l| l == auth);
    if excluded {
        lines.retain(|l| l != auth);
    } else {
        lines.push(auth.to_string());
    }
    let tmp = file.with_extension("tmp");
    std::fs::write(&tmp, lines.join("\n") + "\n")
        .and_then(|_| std::fs::rename(&tmp, &file))
        .map_err(|e| format!("{}: {e}", file.display()))?;
    crate::net::load_https_excludes();
    let rest = url.split_once("://").map_or(url.as_str(), |(_, r)| r);
    let target = if excluded { format!("https://{rest}") } else { format!("http://{rest}") };
    window::message("info", &format!("HTTPS-only {} for {auth}", if excluded { "on" } else { "off" }));
    run_one_line(&format!("open {target}"))
}

fn run_one_line(line: &str) -> Result<(), String> {
    run(line, None);
    Ok(())
}

/// Run a command line (`a ;; b`), reporting errors in the status bar.
/// `/text` and `?text` (from the command line) search forward / backward.
pub fn run(line: &str, count: Option<u32>) {
    let line = line.trim();
    if let Some(text) = line.strip_prefix('/') {
        return report("search", search(text, false));
    }
    if let Some(text) = line.strip_prefix('?') {
        return report("search", search(text, true));
    }
    if line.contains("{clipboard}") || line.contains("{primary}") {
        // reading the clipboard can block: do it on a thread, run when it's back
        return clipboard::expand_then_run(line, count);
    }
    run_parsed(line, count);
}

/// Run with `{clipboard}`/`{primary}` already read (clipboard.rs).
pub fn run_with_clipboard(line: &str, count: Option<u32>, clip: String, primary: String) {
    CLIP.set((clip, primary));
    run_parsed(line, count);
    CLIP.set((String::new(), String::new()));
}

fn report(name: &str, r: Result<(), String>) {
    if let Err(e) = r {
        window::message("error", &format!("{name}: {e}"));
    }
}

fn run_parsed(line: &str, count: Option<u32>) {
    let cmds = match parse_line(line) {
        Ok(c) => c,
        Err(e) => return window::message("error", &e),
    };
    for cmd in cmds {
        if let Err(e) = run_one(&cmd, count) {
            window::message("error", &format!("{}: {e}", cmd.name));
            return;
        }
    }
}

fn js(browser: &Browser, code: &str) {
    if let Some(f) = browser.main_frame() {
        f.execute_java_script(Some(&CefString::from(code)), None, 0);
    }
}

/// Scroll the element under the viewport centre (or the document) to `perc` percent.
const SCROLL_TO_PERC_JS: &str = "((p, x) => { let e = document.elementFromPoint(innerWidth / 2, innerHeight / 2);\
 const ok = (e) => x ? e.scrollWidth > e.clientWidth && /auto|scroll/.test(getComputedStyle(e).overflowX)\
 : e.scrollHeight > e.clientHeight && /auto|scroll/.test(getComputedStyle(e).overflowY);\
 while (e && e !== document.body && e !== document.documentElement && !ok(e)) e = e.parentElement;\
 if (!e || e === document.body || e === document.documentElement) e = document.scrollingElement || document.documentElement;\
 if (x) e.scrollLeft = (e.scrollWidth - e.clientWidth) * p / 100; else e.scrollTop = (e.scrollHeight - e.clientHeight) * p / 100; })";

thread_local! {
    /// Last search (text, backward), for `n` / `N`.
    static SEARCH: RefCell<(String, bool)> = const { RefCell::new((String::new(), false)) };
}

/// Find `text` in the active tab (Chromium find bar logic, highlighted by Chromium).
/// qutebrowser `search.ignore_case = smart`: case-sensitive only with an uppercase letter.
pub fn search(text: &str, backward: bool) -> Result<(), String> {
    let h = window::active_browser()
        .and_then(|b| b.host())
        .ok_or("no active tab")?;
    SEARCH.set((text.to_string(), backward));
    if text.is_empty() {
        h.stop_finding(1);
        return Ok(());
    }
    let case = text.chars().any(char::is_uppercase) as i32;
    h.find(Some(&CefString::from(text)), !backward as i32, case, 0);
    Ok(())
}

fn search_again(reverse: bool, n: u32) -> Result<(), String> {
    let (text, backward) = SEARCH.with_borrow(|s| s.clone());
    if text.is_empty() {
        return Err("no previous search".into());
    }
    let h = window::active_browser()
        .and_then(|b| b.host())
        .ok_or("no active tab")?;
    let case = text.chars().any(char::is_uppercase) as i32;
    for _ in 0..n {
        h.find(
            Some(&CefString::from(text.as_str())),
            (backward == reverse) as i32,
            case,
            1,
        );
    }
    Ok(())
}

/// `spawn cmd args…`: start a program, never wait for it on the UI thread.
pub fn spawn(argv: &[String]) -> Result<(), String> {
    crate::spawn::spawn(argv, false, false, None)
}

/// Implemented commands, for `:` completion. Keep in sync with `run_one`.
pub const NAMES: &[&str] = &[
    "adblock-update",
    "back",
    "clear-keychain",
    "close",
    "config-cycle",
    "config-source",
    "cmd-set-text",
    "dark-toggle",
    "https-toggle",
    "fake-key",
    "forward",
    "fullscreen",
    "hint",
    "insert-text",
    "message-error",
    "message-info",
    "message-warning",
    "mode-enter",
    "mode-leave",
    "nop",
    "open",
    "quit",
    "reload",
    "scroll",
    "scroll-page",
    "scroll-to-perc",
    "search",
    "search-next",
    "set",
    "search-prev",
    "spawn",
    "stop",
    "tab-clone",
    "tab-close",
    "tab-focus",
    "tab-move",
    "tab-mute",
    "tab-next",
    "tab-only",
    "tab-prev",
    "undo",
    "yank",
    "zoom",
    "zoom-in",
    "zoom-out",
];

fn run_one(cmd: &Cmd, count: Option<u32>) -> Result<(), String> {
    let n = count.unwrap_or(1);
    let browser = window::active_browser();
    let need = || browser.clone().ok_or_else(|| "no active tab".to_string());
    match cmd.name.as_str() {
        "nop" | "clear-keychain" => modes::clear_keychain(),
        "open" => {
            let url = if cmd.args.is_empty() {
                config::get().str("url.default_page")
            } else {
                url_or_search(&cmd.rest())
            };
            // shortcut: -w (window) and -p (private) open a tab until multi-window/private exist
            if cmd.flag("-b", "--bg") {
                window::open(&url, false);
            } else if cmd.flag("-t", "--tab")
                || cmd.flag("-w", "--window")
                || cmd.flag("-p", "--private")
            {
                window::open(&url, true);
            } else if let Some(f) = need()?.main_frame() {
                f.load_url(Some(&CefString::from(url.as_str())));
            }
        }
        // shortcut: -o/-p/-n (which tab gets focus afterwards) ignored; add if missed
        "tab-close" => window::close_active(),
        "tab-only" => window::close_others(),
        "tab-next" => window::cycle(n as i64),
        "tab-prev" => window::cycle(-(n as i64)),
        "tab-focus" => match cmd.args.first().map(String::as_str) {
            Some("last") => window::focus_last(),
            Some(i) => window::focus_index(i.parse::<i64>().map_err(|_| format!("bad index {i}"))?),
            None => window::focus_index(count.map(i64::from).unwrap_or(1)),
        },
        "tab-move" => match cmd.args.first().map(String::as_str) {
            Some("+") => window::move_active(n as i64, true),
            Some("-") => window::move_active(-(n as i64), true),
            Some(i) => window::move_active(
                i.parse::<i64>().map_err(|_| format!("bad index {i}"))?,
                false,
            ),
            None => window::move_active(count.map(i64::from).unwrap_or(1), false),
        },
        "tab-clone" => {
            let (url, _) = window::active_url_title();
            window::open(&url, !cmd.flag("-b", "--bg"));
        }
        "tab-mute" => {
            let b = need()?;
            let h = b.host().ok_or("no host")?;
            h.set_audio_muted((h.is_audio_muted() == 0) as i32);
            window::set_muted(&b, h.is_audio_muted() != 0);
            window::message(
                "info",
                if h.is_audio_muted() != 0 {
                    "muted"
                } else {
                    "unmuted"
                },
            );
        }
        "undo" => window::undo_close(),
        "back" => js(&need()?, &format!("history.go(-{n})")),
        "forward" => js(&need()?, &format!("history.go({n})")),
        "dark-toggle" => dark_toggle()?,
        "https-toggle" => https_toggle()?,
        "reload" => {
            let b = need()?;
            if !window::reload_crashed(&b) {
                if cmd.flag("-f", "--force") {
                    b.reload_ignore_cache()
                } else {
                    b.reload()
                }
            }
        }
        "stop" => need()?.stop_load(),
        "scroll" => {
            let step = 40.0 * n as f64;
            let (dx, dy) = match cmd.args.first().map(String::as_str) {
                Some("down") => (0.0, step),
                Some("up") => (0.0, -step),
                Some("left") => (-step, 0.0),
                Some("right") => (step, 0.0),
                Some("top") => {
                    return {
                        js(&need()?, &format!("{SCROLL_TO_PERC_JS}(0, false)"));
                        Ok(())
                    }
                }
                Some("bottom") => {
                    return {
                        js(&need()?, &format!("{SCROLL_TO_PERC_JS}(100, false)"));
                        Ok(())
                    }
                }
                d => return Err(format!("bad direction {d:?}")),
            };
            devtools::wheel(&need()?, dx, dy);
        }
        "scroll-page" => {
            let f = |i: usize| {
                cmd.args
                    .get(i)
                    .ok_or("usage: scroll-page x y")?
                    .parse::<f64>()
                    .map_err(|e| e.to_string())
            };
            let (w, h) = window::content_size();
            devtools::wheel(&need()?, f(0)? * w * n as f64, f(1)? * h * n as f64);
        }
        "scroll-to-perc" => {
            // qutebrowser: count is the percentage; no arg and no count = bottom (G)
            let p: f64 = match (cmd.args.first(), count) {
                (Some(a), _) => a.parse().map_err(|_| format!("bad percent {a}"))?,
                (None, Some(c)) => c as f64,
                (None, None) => 100.0,
            };
            js(
                &need()?,
                &format!(
                    "{SCROLL_TO_PERC_JS}({}, {})",
                    p.clamp(0.0, 100.0),
                    cmd.flag("-x", "--horizontal")
                ),
            );
        }
        "zoom-in" | "zoom-out" | "zoom" => {
            let h = need()?.host().ok_or("no host")?;
            let cur = level_to_pct(h.zoom_level());
            let pct = match cmd.name.as_str() {
                "zoom-in" => zoom_step(cur, n as i32),
                "zoom-out" => zoom_step(cur, -(n as i32)),
                _ => match (cmd.args.first(), count) {
                    (Some(a), _) => a
                        .trim_end_matches('%')
                        .parse()
                        .map_err(|_| format!("bad zoom {a}"))?,
                    (None, Some(c)) => c as f64,
                    (None, None) => 100.0,
                },
            };
            h.set_zoom_level(pct_to_level(pct));
            window::message("info", &format!("Zoom level: {pct}%"));
        }
        "search" => search(&cmd.rest(), cmd.flag("-r", "--reverse"))?,
        "search-next" => search_again(false, n)?,
        "search-prev" => search_again(true, n)?,
        "hint" => hints::start(cmd)?,
        // -d/--detach: children never block us, so it is the default
        "spawn" => crate::spawn::spawn(&cmd.args, cmd.flag("-u", "--userscript"), cmd.flag("-o", "--output"), count)?,
        // shortcut: -g (keys to the browser UI) not supported
        "fake-key" => crate::spawn::fake_key(&cmd.rest())?,
        "insert-text" => crate::spawn::insert_text(&cmd.rest())?,
        "yank" => yank(cmd, &need()?)?,
        "fullscreen" if cmd.flag("", "--leave") => {
            if let Some(h) = browser.and_then(|b| b.host()) {
                h.exit_fullscreen(1);
            }
        }
        "mode-enter" => modes::enter(match cmd.args.first().map(String::as_str) {
            Some("insert") => modes::Mode::Insert,
            Some("passthrough") => modes::Mode::Passthrough,
            Some("normal") => modes::Mode::Normal,
            m => return Err(format!("unsupported mode {m:?}")),
        }),
        "mode-leave" => modes::leave(),
        "cmd-set-text" => {
            let mut text = cmd.rest();
            if cmd.flag("-s", "--space") {
                text.push(' ');
            }
            if cmd.flag("-a", "--append") { /* nothing typed to append to yet */ }
            // -r (run on count) is ignored: count already drives the command
            window::open_cmdline(&text);
        }
        "message-info" | "message-warning" | "message-error" => {
            window::message(cmd.name.trim_start_matches("message-"), &cmd.rest());
        }
        "close" | "quit" => window::close_window(),
        "adblock-update" => crate::adblock::update_now(),
        "config-source" => {
            config::load().map_err(|e| format!("config unchanged: {e}"))?;
            crate::net::load_https_excludes();
            window::config_changed();
            let pending = config::get().pending().len();
            window::message("info", &format!("config.ts loaded ({pending} settings not built yet)"));
        }
        "set" => {
            // `set name value`; `set name` shows the value
            // -t (don't save) is the only behaviour: :set never writes a file yet
            let pattern = cmd.flag("-u", "--pattern").then(|| cmd.args.first().cloned()).flatten();
            let args = &cmd.args[pattern.is_some() as usize..];
            let (name, value) = match args {
                [n] => return {
                    window::message("info", &format!("{n} = {}", config::get().get(n)));
                    Ok(())
                },
                [n, rest @ ..] if !rest.is_empty() => (n, rest.join(" ")),
                _ => return Err("usage: set <setting> [value]".into()),
            };
            match &pattern {
                Some(p) => config::get().set_runtime_for(p, name, &value)?,
                None => config::get().set_runtime(name, &value)?,
            }
            window::config_changed();
        }
        "config-cycle" => {
            // `config-cycle name v1 v2 …`: next value after the current one
            let [name, values @ ..] = cmd.args.as_slice() else { return Err("usage: config-cycle <setting> <values…>".into()) };
            if values.len() < 2 {
                return Err("config-cycle needs at least two values".into());
            }
            let cfg = config::get();
            let cur = cfg.get(name);
            let cur = cur.as_str().map(str::to_string).unwrap_or_else(|| cur.to_string());
            let i = values.iter().position(|v| *v == cur).map_or(0, |i| (i + 1) % values.len());
            cfg.set_runtime(name, &values[i])?;
            window::config_changed();
        }
        other => return Err(format!("not implemented yet ({other})")),
    }
    Ok(())
}

/// `yank [url|title|domain|pretty-url|selection|inline <text>] [-s]`
fn yank(cmd: &Cmd, browser: &Browser) -> Result<(), String> {
    let primary = cmd.flag("-s", "--sel");
    let (url, title) = window::active_url_title();
    let what = cmd.args.first().map_or("url", String::as_str);
    let text =
        match what {
            "url" | "pretty-url" => url.clone(),
            "title" => title,
            "domain" => match url.split_once("://") {
                Some((scheme, _)) => format!(
                    "{scheme}://{}",
                    url[scheme.len() + 3..].split('/').next().unwrap_or("")
                ),
                None => url.clone(),
            },
            "inline" => cmd.args[1..].join(" "),
            "selection" => {
                devtools::eval_isolated(browser, "getSelection().toString()".into(), move |r| {
                    match r
                        .and_then(|(_, v)| v.as_str().map(str::to_string))
                        .filter(|t| !t.is_empty())
                    {
                        Some(t) => yanked(&t, "selection", primary),
                        None => window::message("error", "Nothing selected."),
                    }
                });
                return Ok(());
            }
            w => return Err(format!("unknown yank target {w}")),
        };
    yanked(&text, what, primary);
    Ok(())
}

fn yanked(text: &str, what: &str, primary: bool) {
    clipboard::write(text, primary);
    let to = if primary {
        "primary selection"
    } else {
        "clipboard"
    };
    let what = if what == "url" { "URL" } else { what };
    window::message("info", &format!("Yanked {what} to {to}: {text}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_expand_before_splitting() {
        let a = serde_json::json!({"t": "config-cycle tabs.position left top ;; message-info 'x'", "loop": "loop", "q": "close"});
        let mut out = Vec::new();
        split_expand("t ;; q", &a, 0, &mut out).unwrap();
        assert_eq!(out, ["config-cycle tabs.position left top", "message-info 'x'", "close"]);
        assert!(split_expand("loop", &a, 0, &mut Vec::new()).is_err());
    }

    #[test]
    fn url_host() {
        assert_eq!(
            host("https://user:pw@example.com:8080/x?y#z"),
            "example.com"
        );
        assert_eq!(host("http://[::1]:80/"), "[::1]");
        assert_eq!(host("file:///tmp/x"), "");
        assert_eq!(host("example.org"), "example.org");
    }

    #[test]
    fn tokenize_quotes() {
        assert_eq!(
            tokenize("a 'b c' \"d \\\" e\"").unwrap(),
            ["a", "b c", "d \" e"]
        );
        assert_eq!(tokenize("  x  ").unwrap(), ["x"]);
        assert_eq!(tokenize("''").unwrap(), [""]);
        assert_eq!(tokenize(r#"fake-key \< \a " ""#).unwrap(), ["fake-key", "<", "a", " "]);
        assert_eq!(tokenize(r"'\x'").unwrap(), [r"\x"]);
        assert!(tokenize("'open").is_err());
    }

    #[test]
    fn parse_flags_args_chain_alias() {
        let c = parse_line(":open -t example.com").unwrap();
        assert_eq!(
            c,
            [Cmd {
                name: "open".into(),
                flags: vec!["-t".into()],
                args: vec!["example.com".into()]
            }]
        );
        let c = &parse_line("cmd-set-text -sr :tab-focus").unwrap()[0];
        assert_eq!(
            (c.flags.clone(), c.args.clone()),
            (vec!["-s".into(), "-r".into()], vec![":tab-focus".into()])
        );
        let c = &parse_line("open -- -t").unwrap()[0];
        assert_eq!(c.args, ["-t"]);
        let c = &parse_line("tab-move -").unwrap()[0];
        assert_eq!(c.args, ["-"]);
        let c = &parse_line("scroll-page 0 -0.5").unwrap()[0];
        assert_eq!(c.args, ["0", "-0.5"]);
        let names: Vec<_> = parse_line("clear-keychain ;; search ;; fullscreen --leave")
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["clear-keychain", "search", "fullscreen"]);
        assert_eq!(parse_line("q").unwrap()[0].name, "close");
        assert_eq!(parse_line("wq").unwrap()[0].flags, ["--save"]);
    }

    #[test]
    fn urls_vs_search() {
        assert_eq!(url_or_search("example.com"), "https://example.com");
        assert_eq!(url_or_search("https://a.b/c?d"), "https://a.b/c?d");
        assert_eq!(url_or_search("localhost:8080"), "http://localhost:8080");
        assert_eq!(url_or_search("about:blank"), "about:blank");
        assert_eq!(
            url_or_search("rust lang"),
            "https://duckduckgo.com/?q=rust+lang"
        );
        assert_eq!(url_or_search("a&b"), "https://duckduckgo.com/?q=a%26b");
        let e = serde_json::json!({"DEFAULT": "https://s.x/?q={}", "g": "https://g.x/search?q={}", "raw": "https://r.x/{unquoted}"});
        assert_eq!(url_or_search_with("g rust lang", &e, "naive"), "https://g.x/search?q=rust+lang");
        assert_eq!(url_or_search_with("raw a/b", &e, "naive"), "https://r.x/a/b");
        assert_eq!(url_or_search_with("hello world", &e, "naive"), "https://s.x/?q=hello+world");
        assert_eq!(url_or_search_with("DEFAULT x", &e, "naive"), "https://s.x/?q=DEFAULT+x");
        assert_eq!(url_or_search_with("intranet", &e, "never"), "https://intranet");
    }

    #[test]
    fn zoom_steps() {
        assert_eq!(zoom_step(100., 1), 110.);
        assert_eq!(zoom_step(100., -1), 90.);
        assert_eq!(zoom_step(100., 3), 150.);
        assert_eq!(zoom_step(105., 1), 110.);
        assert_eq!(zoom_step(105., -1), 100.);
        assert_eq!(zoom_step(500., 1), 500.);
        assert_eq!(zoom_step(25., -1), 25.);
        assert!((level_to_pct(pct_to_level(150.)) - 150.).abs() < 1e-9);
    }
}
