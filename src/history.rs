//! History store on its own thread (PLAN.md §5.2): visits, command history and
//! completion queries. The UI thread only sends messages and never waits;
//! results come back with `post_task`.
use cef::*;
use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub const LIMIT: usize = 200;
const SHORT_SCAN: i64 = 20_000;

enum Msg {
    Visit(String),
    Title(String, String),
    Cmd(String),
    Query(u64, String),
}

static TX: OnceLock<mpsc::Sender<Msg>> = OnceLock::new();
/// Newest query id: older queued queries are skipped, so fast typing never queues work.
static LATEST: AtomicU64 = AtomicU64::new(0);

wrap_task! {
    struct Results { id: u64, items: Vec<(String, String)> }
    impl Task {
        fn execute(&self) { crate::completion::history_results(self.id, self.items.clone()); }
    }
}

wrap_task! {
    struct CmdsLoaded { cmds: Vec<String> }
    impl Task {
        fn execute(&self) { crate::completion::set_cmd_history(self.cmds.clone()); }
    }
}

pub fn start(path: PathBuf) {
    let (tx, rx) = mpsc::channel();
    TX.set(tx).ok();
    std::thread::Builder::new()
        .name("history".into())
        .spawn(move || {
            let db = match open(&path) {
                Ok(db) => db,
                Err(e) => return eprintln!("novabrowser: history: can't open {path:?}: {e}"),
            };
            match cmd_history(&db) {
                Ok(cmds) => {
                    post_task(ThreadId::UI, Some(&mut CmdsLoaded::new(cmds)));
                }
                Err(e) => eprintln!("novabrowser: history: {e}"),
            };
            for msg in rx {
                if let Err(e) = handle(&db, msg) {
                    eprintln!("novabrowser: history: {e}");
                }
            }
        })
        .expect("spawn history thread");
}

fn send(msg: Msg) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(msg);
    }
}

/// A main-frame navigation. Only web and file URLs are kept.
pub fn visit(url: &str) {
    if ["http:", "https:", "file:"]
        .iter()
        .any(|s| url.starts_with(s))
    {
        send(Msg::Visit(url.to_string()));
    }
}
pub fn title(url: &str, title: &str) {
    send(Msg::Title(url.to_string(), title.to_string()));
}
pub fn cmd(line: &str) {
    send(Msg::Cmd(line.to_string()));
}
/// Ask for history matching `text`; the answer arrives as `completion::history_results(id, …)`.
pub fn query(text: &str) -> u64 {
    let id = LATEST.fetch_add(1, Ordering::SeqCst) + 1;
    send(Msg::Query(id, text.to_string()));
    id
}

pub fn open(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let db = Connection::open(path)?;
    db.execute_batch(
        // `id` order = recency: a visit re-inserts its row with the next id, so the
        // trigram index can return newest matches first and stop at LIMIT.
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
         CREATE TABLE IF NOT EXISTS history(
             id INTEGER PRIMARY KEY, url TEXT NOT NULL UNIQUE, title TEXT NOT NULL DEFAULT '',
             last_atime INTEGER NOT NULL, visits INTEGER NOT NULL DEFAULT 1);
         CREATE VIRTUAL TABLE IF NOT EXISTS history_fts USING fts5(
             url, title, tokenize='trigram', content='history', content_rowid='id');
         CREATE TRIGGER IF NOT EXISTS history_ai AFTER INSERT ON history BEGIN
             INSERT INTO history_fts(rowid, url, title) VALUES (new.id, new.url, new.title); END;
         CREATE TRIGGER IF NOT EXISTS history_ad AFTER DELETE ON history BEGIN
             INSERT INTO history_fts(history_fts, rowid, url, title) VALUES ('delete', old.id, old.url, old.title); END;
         CREATE TRIGGER IF NOT EXISTS history_au AFTER UPDATE ON history BEGIN
             INSERT INTO history_fts(history_fts, rowid, url, title) VALUES ('delete', old.id, old.url, old.title);
             INSERT INTO history_fts(rowid, url, title) VALUES (new.id, new.url, new.title); END;
         CREATE TABLE IF NOT EXISTS cmd_history(id INTEGER PRIMARY KEY, cmd TEXT NOT NULL);",
    )?;
    Ok(db)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn handle(db: &Connection, msg: Msg) -> rusqlite::Result<()> {
    match msg {
        // shortcut: redirects are recorded too (qutebrowser skips them); filter if completion gets noisy
        Msg::Visit(url) => visit_row(db, &url, now())?,
        Msg::Title(url, title) => {
            db.prepare_cached("UPDATE history SET title = ?2 WHERE url = ?1")?
                .execute(params![url, title])?;
        }
        Msg::Cmd(line) => {
            db.prepare_cached("INSERT INTO cmd_history(cmd) VALUES (?1)")?
                .execute(params![line])?;
        }
        Msg::Query(id, text) => {
            if id != LATEST.load(Ordering::SeqCst) {
                return Ok(());
            }
            let t = Instant::now();
            let items = search(db, &text, LIMIT)?;
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            if ms > 16.0 {
                eprintln!("novabrowser: history: slow query {text:?}: {ms:.1} ms");
            }
            post_task(ThreadId::UI, Some(&mut Results::new(id, items)));
        }
    }
    Ok(())
}

/// Move `url` to the newest id, keeping its title and counting the visit.
fn visit_row(db: &Connection, url: &str, atime: i64) -> rusqlite::Result<()> {
    let old: Option<(String, i64)> = db
        .prepare_cached("SELECT title, visits FROM history WHERE url = ?1")?
        .query_row(params![url], |r| Ok((r.get(0)?, r.get(1)?)))
        .ok();
    let (title, visits) = old.unwrap_or_default();
    db.execute_batch("BEGIN")?;
    db.prepare_cached("DELETE FROM history WHERE url = ?1")?
        .execute(params![url])?;
    db.prepare_cached(
        "INSERT INTO history(url, title, last_atime, visits) VALUES (?1, ?2, ?3, ?4)",
    )?
    .execute(params![url, title, atime, visits + 1])?;
    db.execute_batch("COMMIT")
}

fn cmd_history(db: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut st = db.prepare(
        "SELECT cmd FROM (SELECT id, cmd FROM cmd_history ORDER BY id DESC LIMIT 1000) ORDER BY id",
    )?;
    let rows = st.query_map([], |r| r.get(0))?;
    rows.collect()
}

/// Newest first; every word must be in the URL or the title (qutebrowser's history
/// model). Words of 3+ chars go through the trigram index; shorter ones (which it
/// can't index) are LIKE filters on top.
pub fn search(
    db: &Connection,
    text: &str,
    limit: usize,
) -> rusqlite::Result<Vec<(String, String)>> {
    let (long, short): (Vec<&str>, Vec<&str>) = text
        .split_whitespace()
        .partition(|w| w.chars().count() >= 3);
    let mut args: Vec<String> = Vec::new();
    if !long.is_empty() {
        args.push(
            long.iter()
                .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" AND "),
        );
    }
    let mut conds = Vec::new();
    for w in short {
        args.push(format!(
            "%{}%",
            w.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        ));
        let i = args.len();
        conds.push(format!(
            "(h.url LIKE ?{i} ESCAPE '\\' OR h.title LIKE ?{i} ESCAPE '\\')"
        ));
    }
    let sql = if long.is_empty() {
        // The index can't serve 1-2 char words: scan only the newest SHORT_SCAN rows (~5 ms),
        // these queries are transient while typing.
        conds.push(format!(
            "h.id > (SELECT coalesce(max(id), 0) FROM history) - {SHORT_SCAN}"
        ));
        format!(
            "SELECT h.url, h.title FROM history h WHERE {} ORDER BY h.id DESC LIMIT {limit}",
            conds.join(" AND ")
        )
    } else {
        let more: String = conds.iter().map(|c| format!(" AND {c}")).collect();
        format!(
            "SELECT h.url, h.title FROM history_fts f JOIN history h ON h.id = f.rowid
             WHERE history_fts MATCH ?1{more} ORDER BY f.rowid DESC LIMIT {limit}"
        )
    };
    let mut st = db.prepare_cached(&sql)?;
    let rows = st.query_map(rusqlite::params_from_iter(&args), |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(n: i64) -> Connection {
        let mut db = open(std::path::Path::new(":memory:")).unwrap();
        let tx = db.transaction().unwrap();
        {
            let mut st = tx
                .prepare("INSERT INTO history(id, url, title, last_atime) VALUES (?3, ?1, ?2, ?3)")
                .unwrap();
            for i in 0..n {
                st.execute(params![
                    format!(
                        "https://site{}.example.com/articles/{i}/some-slug-text",
                        i % 5000
                    ),
                    format!("Article {i} about topic {} - Site {}", i % 300, i % 5000),
                    i
                ])
                .unwrap();
            }
        }
        tx.commit().unwrap();
        db
    }

    #[test]
    fn search_matches_all_words_newest_first_and_escapes() {
        let db = open(std::path::Path::new(":memory:")).unwrap();
        for (u, t) in [
            ("https://a.com/x", "Rust lang"),
            ("https://b.com/rust", "B"),
            ("https://c.com/100%_", "C"),
        ] {
            visit_row(&db, u, 0).unwrap();
            db.execute(
                "UPDATE history SET title = ?2 WHERE url = ?1",
                params![u, t],
            )
            .unwrap();
        }
        let urls = |q: &str| {
            search(&db, q, 10)
                .unwrap()
                .into_iter()
                .map(|r| r.0)
                .collect::<Vec<_>>()
        };
        assert_eq!(urls("rust"), ["https://b.com/rust", "https://a.com/x"]);
        assert_eq!(urls("RUST a.com"), ["https://a.com/x"]);
        assert_eq!(urls("%_"), ["https://c.com/100%_"]);
        assert_eq!(urls("_"), ["https://c.com/100%_"]);
        assert_eq!(urls("").len(), 3);
        assert_eq!(urls("rust x"), ["https://a.com/x"]); // trigram + short word
        assert_eq!(urls("\"rust"), Vec::<String>::new()); // quote can't break the MATCH syntax
                                                          // a revisit moves to the front and keeps title + count
        visit_row(&db, "https://a.com/x", 9).unwrap();
        assert_eq!(urls("rust"), ["https://a.com/x", "https://b.com/rust"]);
        let (t, v): (String, i64) = db
            .query_row(
                "SELECT title, visits FROM history WHERE url = 'https://a.com/x'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((t.as_str(), v), ("Rust lang", 2));
    }

    /// Phase 3 exit check (PLAN.md §8): completion under 16 ms over 100k rows.
    #[test]
    fn search_100k_rows_under_16ms() {
        let db = fake(100_000);
        for q in [
            "",
            "site42",
            "topic 7",
            "slug article",
            "nomatchanywhere",
            "site4999 99999",
            "site 7",
            "w",
            "zq",
            "x y",
        ] {
            search(&db, q, LIMIT).unwrap(); // warm the statement cache
            let t = Instant::now();
            let n = search(&db, q, LIMIT).unwrap().len();
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            eprintln!("query {q:?}: {n} rows in {ms:.2} ms");
            assert!(ms < 16.0, "{q:?} took {ms:.2} ms");
        }
    }
}
