//! Running commands on CEF. Parsing, URL-or-search, zoom steps and the name list are
//! shared in `nova_core::commands`.
use crate::{clipboard, config, devtools, hints, modes, window};
use cef::*;
pub use nova_core::commands::{host, set_user, url_or_search, zoom_step, Cmd};
use nova_core::commands::tab_index;
use std::cell::RefCell;

/// `a ;; b` → commands, with {url}/{title}/{clipboard}/{primary} filled per token.
pub fn parse_line(line: &str) -> Result<Vec<Cmd>, String> {
    nova_core::commands::parse_line(line, &expand)
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
    crate::write_atomic(&file, (lines.join("\n") + "\n").as_bytes())
        .map_err(|e| format!("{}: {e}", file.display()))?;
    cfg.set_runtime_for(&format!("*://{host}/*"), D, &dark.to_string())?;
    window::config_changed();
    let still = dark
        && lines
            .iter()
            .any(|l| l.starts_with("*.") && host.ends_with(&l[1..]));
    let note = if still {
        " (a wildcard in darkmode_excludes still excludes it after a restart)"
    } else {
        ""
    };
    window::message(
        "info",
        &format!(
            "Dark mode {} for {host}{note}",
            if dark { "on" } else { "off" }
        ),
    );
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
    crate::write_atomic(&file, (lines.join("\n") + "\n").as_bytes())
        .map_err(|e| format!("{}: {e}", file.display()))?;
    crate::net::load_https_excludes();
    let rest = url.split_once("://").map_or(url.as_str(), |(_, r)| r);
    let target = if excluded {
        format!("https://{rest}")
    } else {
        format!("http://{rest}")
    };
    window::message(
        "info",
        &format!(
            "HTTPS-only {} for {auth}",
            if excluded { "on" } else { "off" }
        ),
    );
    run_one_line(&format!("open {target}"))
}

fn run_one_line(line: &str) -> Result<(), String> {
    run(line, None);
    Ok(())
}

thread_local! {
    /// True while a line typed into the command box/address bar runs.
    static FROM_BAR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A line the user entered in the command box: an `open` there counts as typed
/// (history ranks pages opened this way first).
pub fn run_from_bar(line: &str) {
    FROM_BAR.set(true);
    run(line, None);
    FROM_BAR.set(false);
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

/// Open `url` here, or in a new tab with -t / background with -b (-w = tab for now).
fn open_in(url: &str, cmd: &Cmd) -> Result<(), String> {
    if cmd.flag("-b", "--bg") {
        window::open(url, false);
    } else if cmd.flag("-w", "--window") {
        window::open_window(Some(url.to_string()));
    } else if cmd.flag("-t", "--tab") {
        window::open(url, true);
    } else {
        let b = window::active_browser().ok_or("no active tab")?;
        if let Some(f) = b.main_frame() {
            f.load_url(Some(&CefString::from(url)));
        }
    }
    Ok(())
}

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
            let typed = FROM_BAR.get() && !cmd.args.is_empty();
            let open = |url: String, current: bool, f: &dyn Fn(String)| {
                if typed {
                    window::typed_open(current, || f(url));
                } else {
                    f(url);
                }
            };
            // shortcut: -p (private) opens a tab until private windows exist
            if cmd.flag("-b", "--bg") {
                open(url, false, &|u| window::open(&u, false));
            } else if cmd.flag("-w", "--window") {
                open(url, false, &|u| window::open_window(Some(u)));
            } else if cmd.flag("-t", "--tab") || cmd.flag("-p", "--private") {
                open(url, false, &|u| window::open(&u, true));
            } else if let Some(f) = need()?.main_frame() {
                open(url, true, &|u| {
                    f.load_url(Some(&CefString::from(u.as_str())))
                });
            }
        }
        // shortcut: -o/-p/-n (which tab gets focus afterwards) ignored; add if missed
        "tab-close" => window::close_active(cmd.flag("-f", "--force")),
        "tab-pin" => window::pin(tab_index(cmd, count)),
        "tab-give" => window::give(tab_index(cmd, count)),
        "tab-hibernate" => window::hibernate(tab_index(cmd, count)),
        "tab-keep-awake" => window::keep_awake(tab_index(cmd, count)),
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
        // what the right-click menu's Paste does: works where pages swallow Ctrl-V (web terminals)
        "paste" => {
            let b = need()?;
            b.focused_frame()
                .or_else(|| b.main_frame())
                .ok_or("no frame")?
                .paste();
        }
        "back" => window::go(&need()?, -(n as i32)),
        "forward" => window::go(&need()?, n as i32),
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
        "spawn" => crate::spawn::spawn(
            &cmd.args,
            cmd.flag("-u", "--userscript"),
            cmd.flag("-o", "--output"),
            count,
        )?,
        // shortcut: -g (keys to the browser UI) not supported
        "fake-key" => crate::spawn::fake_key(&cmd.rest())?,
        "insert-text" => crate::spawn::insert_text(&cmd.rest())?,
        "yank" => yank(cmd, &need()?)?,
        "fullscreen" if cmd.flag("", "--leave") => {
            if let Some(h) = browser.and_then(|b| b.host()) {
                h.exit_fullscreen(1);
            }
        }
        "fullscreen" => window::toggle_fullscreen(),
        "devtools" => {
            let h = need()?.host().ok_or("no active tab")?;
            if h.has_dev_tools() != 0 {
                h.close_dev_tools();
            } else {
                // no client: our key bindings stay out of DevTools
                h.show_dev_tools(Some(&WindowInfo::default()), None, None, None);
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
        "close" | "quit" => {
            if cmd.flag("-s", "--save") {
                crate::session::save(None)?;
            }
            if cmd.name == "quit" {
                window::quit()
            } else {
                window::close_window()
            }
        }
        "quickmark-save" => {
            let (url, _) = window::active_url_title();
            window::open_cmdline(&format!(":quickmark-add {url} "));
        }
        "quickmark-add" => {
            let [url, name] = &cmd.args[..] else {
                return Err("quickmark-add: url and name required".into());
            };
            crate::marks::quickmark_add(url, name)?
        }
        "quickmark-load" => open_in(&crate::marks::quickmark_url(&cmd.rest())?, cmd)?,
        "quickmark-del" => {
            let (url, _) = window::active_url_title();
            crate::marks::quickmark_del(cmd.args.first().map(String::as_str), &url)?
        }
        "bookmark-add" => {
            let (cur_url, cur_title) = window::active_url_title();
            let url = cmd.args.first().cloned().unwrap_or(cur_url);
            let title = if cmd.args.len() > 1 {
                cmd.args[1..].join(" ")
            } else if cmd.args.is_empty() {
                cur_title
            } else {
                String::new()
            };
            crate::marks::bookmark_add(&url, &title, cmd.flag("", "--toggle"))?
        }
        "bookmark-load" => open_in(&url_or_search(&cmd.rest()), cmd)?,
        "bookmark-del" => {
            let url = if cmd.args.is_empty() {
                window::active_url_title().0
            } else {
                cmd.rest()
            };
            crate::marks::bookmark_del(&url)?
        }
        "bindings" => window::open_cmdline(&format!(":bindings {}", cmd.rest())),
        "profile-new" => match cmd.args.is_empty() {
            true => window::open_cmdline(":profile-new "),
            false => crate::profiles::create(&cmd.rest())?,
        },
        "profile-open" => match cmd.args.is_empty() {
            true => window::open_cmdline(":profile-open "),
            false => crate::profiles::open(&cmd.rest())?,
        },
        "settings" => window::open(
            &window::data_url(&crate::settings_page::page()).to_string(),
            true,
        ),
        "bookmarks" => window::open(&window::data_url(&crate::marks::page()).to_string(), true),
        "bookmark-list" => {
            let page = window::data_url(&crate::marks::page()).to_string();
            if cmd.flag("-j", "--jump") {
                open_in(&page, cmd)?
            } else {
                window::open(&page, true)
            }
        }
        "download-open" => crate::downloads::open(&cmd.args)?,
        "download-cancel" => {
            crate::downloads::cancel(cmd.args.first().and_then(|a| a.parse().ok()))?
        }
        "download-clear" => crate::downloads::clear(),
        "password-fill" => crate::passwords::fill_command(cmd.flag("-o", "--otp"))?,
        "history" => window::open(
            &window::data_url(&crate::history::page(Some(&cmd.rest()))).to_string(),
            true,
        ),
        "help" => window::open(&window::data_url(&crate::welcome::page()).to_string(), true),
        "downloads" => window::open(
            &window::data_url(&crate::downloads::page()).to_string(),
            true,
        ),
        "download" => {
            let url = match cmd.rest() {
                r if r.is_empty() => window::active_url_title().0,
                r if r.starts_with("magnet:") => r,
                r => url_or_search(&r),
            };
            let b = need()?;
            if url.starts_with("magnet:") {
                if !crate::downloads::magnet(url) {
                    return Err("magnet: links need aria2 (install aria2)".into());
                }
            } else {
                crate::downloads::start(&b, &url);
            }
        }
        "session-save" => crate::session::save(cmd.args.first().map(String::as_str))?,
        "session-load" => crate::session::load(
            cmd.args.first().ok_or("session-load: name required")?,
            cmd.flag("-c", "--clear"),
        )?,
        "session-delete" => {
            crate::session::delete(cmd.args.first().ok_or("session-delete: name required")?)?
        }
        "adblock-update" => crate::adblock::update_now(),
        "config-source" => {
            config::load().map_err(|e| format!("config unchanged: {e}"))?;
            crate::marks::reload();
            crate::net::load_https_excludes();
            window::config_changed();
            let ignored = config::get().ignored_per_site();
            if ignored.is_empty() {
                window::message("info", "config.ts loaded");
            } else {
                window::message(
                    "warning",
                    &format!(
                        "config.ts loaded; only global, ignored in pattern(): {}",
                        ignored.join(", ")
                    ),
                );
            }
        }
        "set" => {
            // `set name value` saves to autoconfig.json; -t/--temp only until restart;
            // `set name` shows the value
            let temp = cmd.flag("-t", "--temp");
            let pattern = cmd
                .flag("-u", "--pattern")
                .then(|| cmd.args.first().cloned())
                .flatten();
            let args = &cmd.args[pattern.is_some() as usize..];
            let (name, value) = match args {
                [n] => {
                    return {
                        window::message("info", &format!("{n} = {}", config::get().get(n)));
                        Ok(())
                    }
                }
                [n, rest @ ..] if !rest.is_empty() => (n, rest.join(" ")),
                _ => return Err("usage: set <setting> [value]".into()),
            };
            match &pattern {
                Some(p) => config::get().set_runtime_for(p, name, &value)?,
                None => set_user(name, &value, temp)?,
            }
            window::config_changed();
        }
        "config-cycle" => {
            // `config-cycle name v1 v2 …`: next value after the current one
            let [name, values @ ..] = cmd.args.as_slice() else {
                return Err("usage: config-cycle <setting> <values…>".into());
            };
            if values.len() < 2 {
                return Err("config-cycle needs at least two values".into());
            }
            let cfg = config::get();
            let cur = cfg.get(name);
            let cur = cur
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| cur.to_string());
            let i = values
                .iter()
                .position(|v| *v == cur)
                .map_or(0, |i| (i + 1) % values.len());
            set_user(name, &values[i], cmd.flag("-t", "--temp"))?;
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
    fn zoom_level_roundtrip() {
        assert!((level_to_pct(pct_to_level(150.)) - 150.).abs() < 1e-9);
    }
}