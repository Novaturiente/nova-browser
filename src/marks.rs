//! Quickmarks (`<config>/quickmarks`: `name url` per line) and bookmarks
//! (`<config>/bookmarks/urls`: `url title` per line) — qutebrowser's files, so the
//! user's existing ones can be copied over as they are.
use crate::{config, window};
use std::cell::RefCell;
use std::path::PathBuf;

#[derive(Default)]
struct Marks {
    loaded: bool,
    /// (name, url), file order.
    quick: Vec<(String, String)>,
    /// (url, title), file order.
    book: Vec<(String, String)>,
}

thread_local! {
    static M: RefCell<Marks> = RefCell::new(Marks::default());
}

fn quick_file() -> PathBuf {
    config::dir().join("quickmarks")
}
fn book_file() -> PathBuf {
    config::dir().join("bookmarks").join("urls")
}

/// `name url` / `url title`: split at the first (quickmarks: last) space.
pub fn parse(text: &str, name_first: bool) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let split = if name_first { l.rsplit_once(' ') } else { l.split_once(' ') };
            let (a, b) = split.unwrap_or((l, ""));
            (a.trim().to_string(), b.trim().to_string())
        })
        .collect()
}

fn with<R>(f: impl FnOnce(&mut Marks) -> R) -> R {
    M.with_borrow_mut(|m| {
        if !m.loaded {
            m.loaded = true;
            m.quick = parse(&std::fs::read_to_string(quick_file()).unwrap_or_default(), true);
            m.book = parse(&std::fs::read_to_string(book_file()).unwrap_or_default(), false);
        }
        f(m)
    })
}

fn save(path: PathBuf, rows: &[(String, String)]) -> Result<(), String> {
    let text: String = rows.iter().map(|(a, b)| format!("{a} {b}\n")).collect();
    std::fs::create_dir_all(path.parent().unwrap())
        .and_then(|_| crate::write_atomic(&path, text.as_bytes()))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Re-read both files on the next use (`:config-source`).
pub fn reload() {
    M.with_borrow_mut(|m| m.loaded = false);
}

pub fn quickmarks() -> Vec<(String, String)> {
    with(|m| m.quick.clone())
}
pub fn bookmarks() -> Vec<(String, String)> {
    with(|m| m.book.clone())
}

pub fn quickmark_url(name: &str) -> Result<String, String> {
    with(|m| m.quick.iter().find(|(n, _)| n == name).map(|(_, u)| u.clone()))
        .ok_or_else(|| format!("Quickmark '{name}' does not exist!"))
}

/// `quickmark-add url name`: replaces an existing mark of that name.
pub fn quickmark_add(url: &str, name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains(char::is_whitespace) {
        return Err(format!("Invalid quickmark name: '{name}'"));
    }
    let rows = with(|m| {
        match m.quick.iter_mut().find(|(n, _)| n == name) {
            Some(e) => e.1 = url.to_string(),
            None => m.quick.push((name.to_string(), url.to_string())),
        }
        m.quick.clone()
    });
    save(quick_file(), &rows)?;
    window::message("info", &format!("Added quickmark {name} for {url}"));
    Ok(())
}

/// `quickmark-del [name]`: without a name, the mark pointing at `current`.
pub fn quickmark_del(name: Option<&str>, current: &str) -> Result<(), String> {
    let rows = with(|m| {
        let i = m.quick.iter().position(|(n, u)| match name {
            Some(name) => n == name,
            None => u.trim_end_matches('/') == current.trim_end_matches('/'),
        })?;
        m.quick.remove(i);
        Some(m.quick.clone())
    })
    .ok_or_else(|| format!("Quickmark '{}' not found!", name.unwrap_or(current)))?;
    save(quick_file(), &rows)
}

/// `bookmark-add [url] [title]`; with `toggle`, an existing bookmark is removed instead.
pub fn bookmark_add(url: &str, title: &str, toggle: bool) -> Result<(), String> {
    let (rows, added) = with(|m| {
        if let Some(i) = m.book.iter().position(|(u, _)| u == url) {
            if !toggle {
                return Err(format!("Bookmark already exists: {url}"));
            }
            m.book.remove(i);
            return Ok((m.book.clone(), false));
        }
        m.book.push((url.to_string(), title.to_string()));
        Ok((m.book.clone(), true))
    })?;
    save(book_file(), &rows)?;
    let what = if added { "Bookmarked" } else { "Removed bookmark" };
    window::message("info", &format!("{what} {url}"));
    Ok(())
}

pub fn bookmark_del(url: &str) -> Result<(), String> {
    let rows = with(|m| {
        let i = m.book.iter().position(|(u, _)| u == url)?;
        m.book.remove(i);
        Some(m.book.clone())
    })
    .ok_or_else(|| format!("Bookmark '{url}' not found!"))?;
    save(book_file(), &rows)
}

/// Merge imported rows (import.rs); existing names/URLs win. Returns how many were new.
pub fn import(quick: &[(String, String)], book: &[(String, String)]) -> Result<(usize, usize), String> {
    let (q, b, nq, nb) = with(|m| {
        let (q0, b0) = (m.quick.len(), m.book.len());
        for (n, u) in quick {
            if !m.quick.iter().any(|(x, _)| x == n) {
                m.quick.push((n.clone(), u.clone()));
            }
        }
        for (u, t) in book {
            if !m.book.iter().any(|(x, _)| x == u) {
                m.book.push((u.clone(), t.clone()));
            }
        }
        (m.quick.clone(), m.book.clone(), m.quick.len() - q0, m.book.len() - b0)
    });
    save(quick_file(), &q)?;
    save(book_file(), &b)?;
    Ok((nq, nb))
}

/// Rows whose text or description contains every word of `pattern` (case-insensitive).
pub fn filter(rows: Vec<(String, String)>, pattern: &str) -> Vec<(String, String)> {
    let words: Vec<String> = pattern.split_whitespace().map(str::to_lowercase).collect();
    rows.into_iter()
        .filter(|(a, b)| {
            let hay = format!("{a} {b}").to_lowercase();
            words.iter().all(|w| hay.contains(w))
        })
        .collect()
}

/// `bookmark-list`: a local page with every quickmark and bookmark as a link.
pub fn list_page() -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
    let link = |u: &str, t: &str| {
        let safe = u.starts_with("http") || u.starts_with("file:") || u.starts_with("about:");
        let href = if safe { esc(u) } else { "#".into() };
        format!("<li><a href=\"{href}\">{}</a> <small>{}</small></li>", esc(if t.is_empty() { u } else { t }), esc(u))
    };
    let q: String = quickmarks().iter().map(|(n, u)| link(u, n)).collect();
    let b: String = bookmarks().iter().map(|(u, t)| link(u, t)).collect();
    format!(
        "<!doctype html><meta charset=utf-8><title>Bookmarks</title>\
         <style>body{{font:14px sans-serif;max-width:60em;margin:2em auto}}small{{opacity:.6}}li{{margin:.3em 0}}</style>\
         <h2>Quickmarks</h2><ul>{q}</ul><h2>Bookmarks</h2><ul>{b}</ul>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_qutebrowser_files() {
        assert_eq!(
            parse("klee https://a/b\n\n# c\nmy mark https://x", true),
            [("klee".into(), "https://a/b".into()), ("my mark".into(), "https://x".into())]
                as [(String, String); 2]
        );
        assert_eq!(
            parse("https://g/app?q=1 Google Gemini\nhttps://bare", false),
            [("https://g/app?q=1".into(), "Google Gemini".into()), ("https://bare".into(), String::new())]
                as [(String, String); 2]
        );
    }

    #[test]
    fn filter_needs_every_word() {
        let rows = vec![("https://gemini.google.com".into(), "Google Gemini".into()), ("https://x".into(), "Zoom".into())];
        assert_eq!(filter(rows.clone(), "gem GOOGLE").len(), 1);
        assert_eq!(filter(rows, "").len(), 2);
    }
}
