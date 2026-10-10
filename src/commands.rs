//! Command line parsing and the Phase 2 command set (qutebrowser names).
use crate::{bindings, devtools, modes, window};
use cef::*;

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

/// Shell-like split: whitespace separates, '…' and "…" group, `\` escapes in "…".
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
            (Some('"'), '\\') => cur.extend(chars.next()),
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

/// `a ;; b` → ["a", "b"]; aliases expand on the first word.
pub fn parse_line(line: &str) -> Result<Vec<Cmd>, String> {
    let line = line.trim().trim_start_matches(':');
    line.split(";;")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(parse_one)
        .collect()
}

fn parse_one(part: &str) -> Result<Cmd, String> {
    let (first, tail) = part.split_once(char::is_whitespace).unwrap_or((part, ""));
    let expanded = match bindings::ALIASES.iter().find(|(a, _)| *a == first) {
        Some((_, target)) => format!("{target} {tail}"),
        None => part.to_string(),
    };
    let mut tokens = tokenize(&expanded)?.into_iter();
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

/// qutebrowser `url.auto_search = naive`: one word with a dot/colon/localhost is a URL.
pub fn url_or_search(text: &str) -> String {
    let t = text.trim();
    let has_scheme = t.contains("://")
        || ["about:", "file:", "data:", "chrome:"]
            .iter()
            .any(|p| t.starts_with(p));
    if has_scheme {
        return t.to_string();
    }
    if !t.contains(char::is_whitespace) && (t.starts_with("localhost") || t.contains('.')) {
        let scheme = if t.starts_with("localhost") {
            "http"
        } else {
            "https"
        };
        return format!("{scheme}://{t}");
    }
    // shortcut: fixed default engine until config.ts (Phase 5) brings url.searchengines
    let q: String = t
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("https://duckduckgo.com/?q={q}")
}

/// qutebrowser `zoom.levels` default.
const ZOOM_LEVELS: &[f64] = &[
    25., 33., 50., 67., 75., 90., 100., 110., 125., 150., 175., 200., 250., 300., 400., 500.,
];

/// Next zoom level `steps` away from `current` percent (negative = out).
pub fn zoom_step(current: f64, steps: i32) -> f64 {
    let last = ZOOM_LEVELS.len() as i32 - 1;
    // first level above (zoom in) / below (zoom out) the current one, then the remaining steps
    let target = if steps > 0 {
        ZOOM_LEVELS
            .iter()
            .position(|&z| z > current + 0.5)
            .map_or(last, |i| i as i32 + steps - 1)
    } else {
        ZOOM_LEVELS
            .iter()
            .rposition(|&z| z < current - 0.5)
            .map_or(0, |i| i as i32 + steps + 1)
    };
    ZOOM_LEVELS[target.clamp(0, last) as usize]
}

/// CEF zoom level is log base 1.2 of the factor.
fn pct_to_level(pct: f64) -> f64 {
    (pct / 100.0).ln() / 1.2f64.ln()
}
fn level_to_pct(level: f64) -> f64 {
    100.0 * 1.2f64.powf(level)
}

fn expand(text: &str) -> String {
    let (url, title) = window::active_url_title();
    // shortcut: {url:pretty} = {url}; {clipboard}/{primary} come with yank in Phase 4
    text.replace("{url:pretty}", &url)
        .replace("{url}", &url)
        .replace("{title}", &title)
}

/// Run a command line (`a ;; b`), reporting errors in the status bar.
pub fn run(line: &str, count: Option<u32>) {
    let cmds = match parse_line(&expand(line)) {
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

fn run_one(cmd: &Cmd, count: Option<u32>) -> Result<(), String> {
    let n = count.unwrap_or(1);
    let browser = window::active_browser();
    let need = || browser.clone().ok_or_else(|| "no active tab".to_string());
    match cmd.name.as_str() {
        "nop" | "clear-keychain" => modes::clear_keychain(),
        "open" => {
            let url = if cmd.args.is_empty() {
                "about:blank".to_string()
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
            let h = need()?.host().ok_or("no host")?;
            h.set_audio_muted((h.is_audio_muted() == 0) as i32);
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
        "search" if cmd.args.is_empty() => {
            if let Some(h) = browser.and_then(|b| b.host()) {
                h.stop_finding(1);
            }
        }
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
        other => return Err(format!("not implemented yet ({other})")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_quotes() {
        assert_eq!(
            tokenize("a 'b c' \"d \\\" e\"").unwrap(),
            ["a", "b c", "d \" e"]
        );
        assert_eq!(tokenize("  x  ").unwrap(), ["x"]);
        assert_eq!(tokenize("''").unwrap(), [""]);
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
