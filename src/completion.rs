//! `:` command line: completion (commands, history) and command history.
//! The status bar `<input>` edits text natively and reports every change here.
use crate::{commands, history, window};
use std::cell::RefCell;

#[derive(Default)]
struct State {
    /// Current command line text, `:` included.
    text: String,
    /// Text before the completed part, e.g. `:open -t `.
    prefix: String,
    title: &'static str,
    items: Vec<(String, String)>,
    sel: Option<usize>,
    /// History query whose answer we still want.
    query: u64,
    /// Typed command lines, oldest first.
    hist: Vec<String>,
    /// While browsing history: (index into hist, text typed before browsing).
    hist_pos: Option<(usize, String)>,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State::default());
}

/// The text changed by typing: recompute completion, stop browsing history.
pub fn on_text(text: &str) {
    S.with_borrow_mut(|s| s.hist_pos = None);
    complete(text);
}

fn complete(text: &str) {
    let body = text.strip_prefix(':').unwrap_or(text);
    S.with_borrow_mut(|s| {
        s.text = text.to_string();
        s.sel = None;
        s.query = 0;
    });
    match body.split_once(' ') {
        None => {
            let items = commands::NAMES
                .iter()
                .filter(|n| n.contains(body))
                .map(|n| (n.to_string(), String::new()))
                .collect();
            set(":".into(), "Commands", items);
        }
        Some(("open", rest)) => {
            // flags (-t, -b, …) stay in the prefix
            let mut arg = rest;
            while let Some((_, tail)) = arg.split_once(' ').filter(|(f, _)| f.starts_with('-')) {
                arg = tail;
            }
            let prefix = text[..text.len() - arg.len()].to_string();
            let id = history::query(arg);
            S.with_borrow_mut(|s| {
                s.prefix = prefix;
                s.query = id;
            });
        }
        Some(_) => set(String::new(), "", Vec::new()),
    }
}

/// Answer from the history thread; stale answers are dropped.
pub fn history_results(id: u64, items: Vec<(String, String)>) {
    let (wanted, prefix) = S.with_borrow(|s| (s.query == id, s.prefix.clone()));
    if wanted {
        set(prefix, "History", items);
    }
}

fn set(prefix: String, title: &'static str, items: Vec<(String, String)>) {
    S.with_borrow_mut(|s| {
        s.prefix = prefix;
        s.title = title;
        s.items = items;
        s.sel = None;
    });
    render();
}

fn render() {
    let json = S.with_borrow(|s| {
        if s.items.is_empty() {
            return None;
        }
        let rows: Vec<String> = s
            .items
            .iter()
            .map(|(a, b)| format!("[{},{}]", window::js_str(a), window::js_str(b)))
            .collect();
        Some(format!(
            "{{\"title\":{},\"items\":[{}],\"sel\":{}}}",
            window::js_str(s.title),
            rows.join(","),
            s.sel.map_or(-1, |i| i as i64)
        ))
    });
    window::show_completion(json.as_deref());
}

/// Tab / Shift-Tab: select the next/previous item and put it on the command line.
pub fn select(by: i64) {
    let text = S.with_borrow_mut(|s| {
        let n = s.items.len() as i64;
        if n == 0 {
            return None;
        }
        let i = match s.sel {
            None if by > 0 => 0,
            None => n - 1,
            Some(i) => (i as i64 + by).rem_euclid(n),
        } as usize;
        s.sel = Some(i);
        s.text = format!("{}{}", s.prefix, s.items[i].0);
        Some(s.text.clone())
    });
    if let Some(t) = text {
        window::set_cmdline_text(&t);
        render();
    }
}

pub fn set_cmd_history(cmds: Vec<String>) {
    S.with_borrow_mut(|s| s.hist = cmds);
}

/// A command line was run from `:`: remember it (skip immediate repeats).
pub fn record(line: &str) {
    let new = S.with_borrow_mut(|s| {
        let new = s.hist.last().map(String::as_str) != Some(line);
        if new {
            s.hist.push(line.to_string());
        }
        new
    });
    if new {
        history::cmd(line);
    }
}

/// Up / Down: older / newer command lines starting with what was typed (qutebrowser).
pub fn history_step(by: i64) {
    if let Some(t) = S.with_borrow_mut(|s| step(s, by)) {
        window::set_cmdline_text(&t);
        complete(&t);
    }
}

/// The new command line text, or None at the oldest match.
fn step(s: &mut State, by: i64) -> Option<String> {
    let (pos, typed) = s.hist_pos.clone().unwrap_or((s.hist.len(), s.text.clone()));
    let mut i = pos as i64;
    loop {
        i += by;
        if i < 0 {
            return None;
        }
        if i as usize >= s.hist.len() {
            s.hist_pos = None;
            return Some(typed);
        }
        let h = &s.hist[i as usize];
        if h.starts_with(&typed) && *h != s.text {
            s.hist_pos = Some((i as usize, typed));
            return Some(h.clone());
        }
    }
}

/// Command line closed.
pub fn reset() {
    S.with_borrow_mut(|s| {
        s.items.clear();
        s.sel = None;
        s.query = 0;
        s.hist_pos = None;
    });
    window::show_completion(None);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hist_test(hist: &[&str], typed: &str, steps: &[i64]) -> Vec<String> {
        let mut s = State {
            hist: hist.iter().map(|h| h.to_string()).collect(),
            text: typed.into(),
            ..Default::default()
        };
        steps
            .iter()
            .map(|&by| {
                if let Some(t) = step(&mut s, by) {
                    s.text = t;
                }
                s.text.clone()
            })
            .collect()
    }

    #[test]
    fn history_prefix_navigation() {
        let h = [":open a", ":tab-close", ":open b", ":open b", ":zoom 50"];
        assert_eq!(
            hist_test(&h, ":open", &[-1, -1, -1, 1, 1]),
            [":open b", ":open a", ":open a", ":open b", ":open"]
        );
        assert_eq!(
            hist_test(&h, ":", &[-1, -1, -1]),
            [":zoom 50", ":open b", ":tab-close"]
        );
    }
}
