//! Downloads (PLAN §5.4): straight into `downloads.location.directory` (default
//! ~/Downloads), no prompt, unique names; start/finish/failure as messages.
//! `download-open` opens the newest finished file with `downloads.open_dispatcher`
//! or xdg-open; `download-cancel` stops the newest running one.
//! shortcut: no download bar or list UI, and `downloads.location.prompt` is ignored
//! (always saves); add a list if several parallel downloads get confusing.
use crate::{config, window};
use cef::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

struct Dl {
    id: u32,
    path: PathBuf,
    done: bool,
    failed: bool,
    cb: Option<DownloadItemCallback>,
}

thread_local! {
    /// Oldest first.
    static LIST: RefCell<Vec<Dl>> = const { RefCell::new(Vec::new()) };
}

fn dir() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    match config::get().str("downloads.location.directory") {
        d if d.is_empty() => home.join("Downloads"),
        d => match d.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(d),
        },
    }
}

/// `name` in `dir`, or `name (1).ext`, `name (2).ext`, … if taken (also by a running download).
fn unique(dir: &Path, name: &str, taken: &[PathBuf]) -> PathBuf {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    let free = |p: &PathBuf| !p.exists() && !taken.contains(p);
    let mut p = dir.join(name);
    let mut i = 1;
    while !free(&p) {
        p = dir.join(format!("{stem} ({i}){ext}"));
        i += 1;
    }
    p
}

/// Untrusted server-suggested name → a plain file name.
fn sanitize(name: &str) -> String {
    let n: String = name
        .chars()
        .map(|c| if c == '/' || c == '\0' { '_' } else { c })
        .collect();
    let n = n.trim().trim_start_matches('.').to_string();
    if n.is_empty() {
        "download".into()
    } else {
        n
    }
}

pub fn before(item: &DownloadItem, suggested: &str, cb: &BeforeDownloadCallback) {
    let d = dir();
    if let Err(e) = std::fs::create_dir_all(&d) {
        return window::message("error", &format!("download: {}: {e}", d.display()));
    }
    let taken: Vec<PathBuf> = LIST.with_borrow(|l| {
        l.iter()
            .filter(|x| !x.done)
            .map(|x| x.path.clone())
            .collect()
    });
    let path = unique(&d, &sanitize(suggested), &taken);
    LIST.with_borrow_mut(|l| {
        l.push(Dl {
            id: item.id(),
            path: path.clone(),
            done: false,
            failed: false,
            cb: None,
        })
    });
    window::message("info", &format!("Downloading {}", path.display()));
    cb.cont(
        Some(&CefString::from(path.to_str().unwrap_or("download"))),
        0,
    );
}

pub fn updated(item: &DownloadItem, cb: Option<&DownloadItemCallback>) {
    let id = item.id();
    let (complete, failed) = (
        item.is_complete() != 0,
        item.is_canceled() != 0 || item.is_interrupted() != 0,
    );
    let report = LIST.with_borrow_mut(|l| {
        let d = l.iter_mut().find(|d| d.id == id)?;
        if d.done {
            return None;
        }
        d.cb = cb.cloned();
        if complete || failed {
            d.done = true;
            d.failed = failed;
            d.cb = None;
            return Some(d.path.clone());
        }
        None
    });
    match report {
        Some(p) if failed => window::message("error", &format!("Download failed: {}", p.display())),
        Some(p) => window::message("info", &format!("Download finished: {}", p.display())),
        None => {}
    }
}

/// `download-open [cmd…]`: newest finished download, with `cmd` or the dispatcher/xdg-open.
pub fn open(cmdline: &[String]) -> Result<(), String> {
    let path = LIST
        .with_borrow(|l| {
            l.iter()
                .rev()
                .find(|d| d.done && !d.failed)
                .map(|d| d.path.clone())
        })
        .ok_or("No finished download")?;
    let p = path.to_string_lossy().to_string();
    let argv: Vec<String> = if !cmdline.is_empty() {
        cmdline
            .iter()
            .map(|a| a.replace("{}", &p))
            .chain((!cmdline.iter().any(|a| a.contains("{}"))).then(|| p.clone()))
            .collect()
    } else {
        match config::get().str("downloads.open_dispatcher") {
            d if d.is_empty() => vec!["xdg-open".into(), p],
            d => vec![d, p],
        }
    };
    crate::commands::spawn(&argv)
}

/// `download-cancel`: the newest running download.
pub fn cancel() -> Result<(), String> {
    let cb = LIST.with_borrow(|l| l.iter().rev().find(|d| !d.done).and_then(|d| d.cb.clone()));
    cb.ok_or("No running download")?.cancel();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_names_and_sanitizing() {
        let d = std::env::temp_dir().join(format!("nova-dl-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.pdf"), "").unwrap();
        assert_eq!(unique(&d, "a.pdf", &[]), d.join("a (1).pdf"));
        assert_eq!(
            unique(&d, "a.pdf", &[d.join("a (1).pdf")]),
            d.join("a (2).pdf")
        );
        assert_eq!(unique(&d, "b", &[]), d.join("b"));
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(sanitize("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize(".bashrc"), "bashrc");
        assert_eq!(sanitize(""), "download");
    }
}
