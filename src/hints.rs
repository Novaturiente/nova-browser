//! `hint` command and hint mode (qutebrowser browser/hints.py). Labels are made
//! here; finding, drawing and picking elements runs in `hints.js` inside an
//! isolated world of the page.
use crate::modes::{self, Mode};
use crate::{clipboard, commands, devtools, window};
use cef::*;
use serde_json::Value;
use std::cell::RefCell;

const JS: &str = include_str!("hints.js");
// shortcut: qutebrowser defaults + the user's hints.chars/uppercase; config.ts (Phase 5) replaces these
const CHARS: &str = "asdfghjkl";
const MIN_CHARS: usize = 1;
const UPPERCASE: bool = true;

/// qutebrowser `hints.selectors` defaults.
fn selector(group: &str) -> Option<&'static str> {
    Some(match group {
        "all" => "a, area, textarea, select, input:not([type=\"hidden\"]), button, frame, iframe, img, link, summary, \
                  [contenteditable]:not([contenteditable=\"false\"]), [onclick], [onmousedown], [role=\"link\"], \
                  [role=\"option\"], [role=\"button\"], [role=\"tab\"], [role=\"checkbox\"], [role=\"switch\"], \
                  [role=\"menuitem\"], [role=\"menuitemcheckbox\"], [role=\"menuitemradio\"], [role=\"treeitem\"], \
                  [aria-haspopup], [ng-click], [ngClick], [data-ng-click], [x-ng-click], [tabindex]:not([tabindex=\"-1\"])",
        "links" => "a[href], area[href], link[href], [role=\"link\"][href]",
        "images" => "img",
        "media" => "audio, img, video",
        "url" => "[src], [href]",
        "inputs" => "input[type=\"text\"], input[type=\"date\"], input[type=\"datetime-local\"], input[type=\"email\"], \
                     input[type=\"month\"], input[type=\"number\"], input[type=\"password\"], input[type=\"search\"], \
                     input[type=\"tel\"], input[type=\"time\"], input[type=\"url\"], input[type=\"week\"], input:not([type]), \
                     [contenteditable]:not([contenteditable=\"false\"]), textarea",
        _ => return None,
    })
}

struct Session {
    browser: Browser,
    ctx: i64,
    labels: Vec<String>,
    typed: String,
    target: String,
    args: Vec<String>,
    rapid: bool,
}

thread_local! {
    static S: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// `hint [--rapid] [--first] [group] [target] [args...]`
pub fn start(cmd: &commands::Cmd) -> Result<(), String> {
    // Flags may follow the group (`hint inputs --first`); only group and target are
    // checked, so spawn args like `mpv -f` stay intact.
    let is_flag = |a: &str, s: &str, l: &str| a == s || a == l;
    let mut flags = cmd.flags.clone();
    let mut pos: Vec<String> = Vec::new();
    let mut it = cmd.args.iter();
    for a in it.by_ref() {
        if is_flag(a, "-r", "--rapid") || is_flag(a, "-f", "--first") {
            flags.push(a.clone());
        } else {
            pos.push(a.clone());
            if pos.len() == 2 {
                break;
            }
        }
    }
    let args: Vec<String> = it.cloned().collect();
    let group = pos.first().map_or("all", String::as_str);
    let sel = selector(group).ok_or_else(|| format!("unknown hint group {group}"))?;
    let target = pos.get(1).cloned().unwrap_or_else(|| "normal".into());
    let rapid = flags.iter().any(|f| is_flag(f, "-r", "--rapid"));
    let first = flags.iter().any(|f| is_flag(f, "-f", "--first"));
    let browser = window::active_browser().ok_or("no active tab")?;
    let js = format!(
        "{JS}; __nova.collect({})",
        serde_json::to_string(sel).unwrap()
    );
    let b = browser.clone();
    devtools::eval_isolated(&browser, js, move |r| {
        let Some((ctx, n)) = r else {
            return window::message("error", "hints: page script failed");
        };
        let n = n.as_u64().unwrap_or(0) as usize;
        if n == 0 {
            return window::message("error", "No elements found.");
        }
        if first {
            return devtools::eval_in(&b, ctx, "__nova.focusFirst()".into(), |_| {});
        }
        let labels = scattered(n, CHARS, MIN_CHARS);
        let draw = format!(
            "__nova.draw({}, {UPPERCASE})",
            serde_json::to_string(&labels).unwrap()
        );
        devtools::eval_in(&b, ctx, draw, |_| {});
        S.set(Some(Session {
            browser: b,
            ctx,
            labels,
            typed: String::new(),
            target,
            args,
            rapid,
        }));
        modes::enter(Mode::Hint);
    });
    Ok(())
}

/// Keys in hint mode: label chars filter, Backspace un-types, anything else is eaten.
pub fn on_key(name: &str) {
    let action = S.with_borrow_mut(|s| {
        let s = s.as_mut()?;
        match name {
            "Backspace" => {
                s.typed.pop();
            }
            n if n.chars().count() == 1 && CHARS.contains(&n.to_lowercase()) => {
                s.typed.push_str(&n.to_lowercase())
            }
            _ => return None,
        }
        let matching: Vec<usize> = (0..s.labels.len())
            .filter(|&i| s.labels[i].starts_with(&s.typed))
            .collect();
        Some((s.browser.clone(), s.ctx, s.typed.clone(), matching))
    });
    let Some((b, ctx, typed, matching)) = action else {
        return;
    };
    match matching.as_slice() {
        [] => {
            // qutebrowser: a non-matching key leaves hint mode
            window::message("info", "No matching hint.");
            modes::enter(Mode::Normal);
        }
        // hints.auto_follow = unique-match
        [i] => follow(&b, ctx, *i),
        _ => {
            let js = format!(
                "__nova.filter({}, {UPPERCASE})",
                serde_json::to_string(&typed).unwrap()
            );
            devtools::eval_in(&b, ctx, js, |_| {});
        }
    }
    window::refresh();
}

pub fn typed() -> String {
    S.with_borrow(|s| s.as_ref().map(|s| s.typed.clone()).unwrap_or_default())
}

fn follow(b: &Browser, ctx: i64, i: usize) {
    let b2 = b.clone();
    devtools::eval_in(b, ctx, format!("__nova.pick({i})"), move |info| {
        let Some(info) = info else {
            return window::message("error", "hints: element is gone");
        };
        let (target, args, rapid) = S
            .with_borrow(|s| {
                s.as_ref()
                    .map(|s| (s.target.clone(), s.args.clone(), s.rapid))
            })
            .unwrap_or_default();
        if rapid {
            S.with_borrow_mut(|s| s.as_mut().map(|s| s.typed.clear()));
            devtools::eval_in(&b2, ctx, format!("__nova.filter('', {UPPERCASE})"), |_| {});
        } else {
            modes::enter(Mode::Normal); // clears the labels (on_leave)
        }
        if let Err(e) = act(&b2, &target, &args, &info) {
            window::message("error", &e);
        }
    });
}

/// Run `target` on the picked element (`info` from hints.js `pick`).
fn act(b: &Browser, target: &str, args: &[String], info: &Value) -> Result<(), String> {
    let (x, y) = (
        info["x"].as_f64().unwrap_or(0.0),
        info["y"].as_f64().unwrap_or(0.0),
    );
    let href = info["href"].as_str();
    let url = href.or(info["src"].as_str());
    let need_url = || {
        url.map(str::to_string)
            .ok_or_else(|| "hint: element has no URL".to_string())
    };
    match target {
        "normal" => devtools::click(b, x, y, "left", 0),
        "current" => devtools::click(b, x, y, "left", 0),
        // qutebrowser tabs.background = true: `tab` opens in the background
        "tab" | "tab-bg" | "tab-fg" | "window" => match href {
            Some(u) => window::open(u, target == "tab-fg" || target == "window"),
            // shortcut: non-link elements just get a click; qutebrowser fakes ctrl+click
            None => devtools::click(b, x, y, "left", 0),
        },
        "hover" => devtools::hover(b, x, y),
        "yank" | "yank-primary" => {
            let u = need_url()?;
            clipboard::write(&u, target == "yank-primary");
            window::message(
                "info",
                &format!(
                    "Yanked URL to {}: {u}",
                    if target == "yank" {
                        "clipboard"
                    } else {
                        "primary selection"
                    }
                ),
            );
        }
        "fill" => window::open_cmdline(&args.join(" ").replace("{hint-url}", &need_url()?)),
        "spawn" => {
            let u = need_url()?;
            let argv: Vec<String> = args.iter().map(|a| a.replace("{hint-url}", &u)).collect();
            commands::spawn(&argv)?;
        }
        "run" => commands::run(&args.join(" ").replace("{hint-url}", &need_url()?), None),
        t => return Err(format!("hint target {t} not implemented yet")),
    }
    Ok(())
}

/// Hint mode ended (Escape, tab switch, follow): remove the labels.
pub fn on_leave() {
    if let Some(s) = S.take() {
        devtools::eval_in(&s.browser, s.ctx, "__nova.clear()".into(), |_| {});
    }
}

/// qutebrowser `_hint_scattered`: shortest labels first, spread over the first char.
pub fn scattered(n: usize, chars: &str, min_chars: usize) -> Vec<String> {
    let chars: Vec<char> = chars.chars().collect();
    let base = chars.len();
    let mut needed = 1; // ceil_log(n, base), at least 1
    let mut accum = base;
    while accum < n {
        needed += 1;
        accum *= base;
    }
    let needed = needed.max(min_chars);
    let short = if needed > min_chars && needed > 1 {
        (base.pow(needed as u32) - n) / (base - 1)
    } else {
        0
    };
    let mut out: Vec<String> = Vec::with_capacity(n);
    if needed > 1 {
        out.extend((0..short).map(|i| to_label(i, &chars, needed - 1)));
    }
    let start = short * base;
    out.extend((start..start + n - short).map(|i| to_label(i, &chars, needed)));
    // _shuffle_hints
    let mut buckets = vec![Vec::new(); base];
    for (i, l) in out.into_iter().enumerate() {
        buckets[i % base].push(l);
    }
    buckets.concat()
}

fn to_label(mut n: usize, chars: &[char], digits: usize) -> String {
    let base = chars.len();
    let mut s = Vec::new();
    loop {
        s.insert(0, chars[n % base]);
        n /= base;
        if n == 0 {
            break;
        }
    }
    while s.len() < digits {
        s.insert(0, chars[0]);
    }
    s.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scattered_matches_qutebrowser() {
        // Values from qutebrowser HintManager._hint_scattered with these inputs.
        assert_eq!(scattered(3, "asdf", 1), ["a", "s", "d"]);
        assert_eq!(scattered(5, "abc", 1), ["a", "cb", "b", "cc", "ca"]);
        assert_eq!(
            scattered(12, "asdfghjkl", 1),
            ["a", "ls", "s", "ld", "d", "lf", "f", "g", "h", "j", "k", "la"]
        );
        let many = scattered(100, "asdfghjkl", 1);
        assert_eq!(many.len(), 100);
        // prefix-free and unique, so a fully typed label is always a unique match
        for (i, a) in many.iter().enumerate() {
            for (j, b) in many.iter().enumerate() {
                assert!(i == j || !b.starts_with(a.as_str()), "{a} prefixes {b}");
            }
        }
        assert_eq!(scattered(2, "asdf", 2), ["aa", "as"]);
    }
}
