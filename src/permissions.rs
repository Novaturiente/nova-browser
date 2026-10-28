//! Site permissions (S7): qutebrowser's `content.*` true/false/ask settings, a
//! y/n/A/N prompt in the tab bar for "ask", and A/N answers saved per origin in
//! <data>/permissions.json (they win over config.ts, like qutebrowser's autoconfig).
use crate::{config, modes, window};
use cef::*;
use serde_json::{Map, Value};
use std::{cell::RefCell, collections::VecDeque, path::PathBuf};

/// CEF_PERMISSION_TYPE_* bit -> (setting, words for the prompt). Bits not listed are denied
/// silently (qutebrowser has no setting for them either), except MULTIPLE_DOWNLOADS.
const PROMPT_BITS: &[(u32, &str, &str)] = &[
    (16, "content.javascript.clipboard", "read your clipboard"),
    (64, "content.persistent_storage", "store data persistently"),
    (256, "content.geolocation", "see your location"),
    (32768, "content.notifications.enabled", "show notifications"),
    (131072, "content.mouse_lock", "lock your mouse pointer"),
    (
        524288,
        "content.register_protocol_handler",
        "open links of a protocol",
    ),
];
const MULTIPLE_DOWNLOADS: u32 = 16384;
/// CEF_MEDIA_PERMISSION_* bit -> setting.
const MEDIA_BITS: &[(u32, &str, &str)] = &[
    (1, "content.media.audio_capture", "use your microphone"),
    (2, "content.media.video_capture", "use your camera"),
    (4, "content.desktop_capture", "capture your screen audio"),
    (8, "content.desktop_capture", "capture your screen"),
];


enum Callback {
    Prompt(u64, PermissionPromptCallback),
    Media(u32, MediaAccessCallback),
    /// alert / confirm / prompt() / leave-page; the String is what "yes" returns to prompt().
    Dialog(JsdialogCallback, String),
    /// our own yes/no questions (passwords.rs save/update); id for expiry
    Ask(u64, std::rc::Rc<dyn Fn(bool)>),
    /// hung renderer: yes = kill it (crash page, `r` reloads), no = keep waiting
    Hang(UnresponsiveProcessCallback),
}

struct Pending {
    browser: i32,
    origin: String,
    settings: Vec<&'static str>,
    text: String,
    cb: Callback,
}

thread_local! {
    static QUEUE: RefCell<VecDeque<Pending>> = const { RefCell::new(VecDeque::new()) };
    static SAVED: RefCell<Option<Map<String, Value>>> = const { RefCell::new(None) };
    static ASK_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

wrap_task! {
    struct ExpireAsk { id: u64 }
    impl Task {
        fn execute(&self) {
            QUEUE.with_borrow_mut(|q| q.retain(|p| !matches!(p.cb, Callback::Ask(i, _) if i == self.id)));
            show();
        }
    }
}

fn file() -> PathBuf {
    crate::xdg_dir("XDG_DATA_HOME", ".local/share").join("permissions.json")
}

fn saved(origin: &str, setting: &str) -> Option<bool> {
    SAVED.with_borrow_mut(|s| {
        let s = s.get_or_insert_with(|| {
            std::fs::read_to_string(file())
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default()
        });
        s.get(origin)?.get(setting)?.as_bool()
    })
}

fn save(origin: &str, settings: &[&str], allow: bool) {
    let text = SAVED.with_borrow_mut(|s| {
        let s = s.get_or_insert_with(Map::new);
        let site = s.entry(origin).or_insert_with(|| Value::Object(Map::new()));
        for n in settings {
            site[*n] = Value::Bool(allow);
        }
        serde_json::to_string_pretty(s).unwrap()
    });
    let f = file();
    if let Err(e) = crate::write_atomic(&f, text.as_bytes()) {
        window::message("error", &format!("{}: {e}", f.display()));
    }
}

#[derive(Debug, PartialEq)]
enum Decision {
    Allow,
    Deny,
    Ask,
}

/// Saved answer, else the setting for this origin: true / false / "ask".
fn decide(origin: &str, setting: &str) -> Decision {
    if let Some(b) = saved(origin, setting) {
        return if b { Decision::Allow } else { Decision::Deny };
    }
    let v = config::get().get_for(setting, origin);
    match (setting, v.as_str(), v.as_bool()) {
        (_, _, Some(true)) | ("content.javascript.clipboard", Some("access-paste"), _) => {
            Decision::Allow
        }
        (_, Some("ask"), _) => Decision::Ask,
        _ => Decision::Deny, // false, and clipboard none/access (write-only)
    }
}

/// Combine the per-setting decisions of one request (CEF answers all bits at once).
fn combine(ds: &[Decision]) -> Decision {
    if ds.contains(&Decision::Deny) {
        Decision::Deny
    } else if ds.contains(&Decision::Ask) {
        Decision::Ask
    } else {
        Decision::Allow
    }
}

fn answer(cb: &Callback, allow: bool) {
    match cb {
        Callback::Prompt(_, c) => c.cont(if allow {
            PermissionRequestResult::ACCEPT
        } else {
            PermissionRequestResult::DENY
        }),
        Callback::Media(bits, c) if allow => c.cont(*bits),
        Callback::Media(_, c) => c.cancel(),
        Callback::Dialog(c, input) => c.cont(allow as i32, Some(&input.as_str().into())),
        Callback::Ask(_, f) => f(allow),
        Callback::Hang(c) if allow => c.terminate(),
        Callback::Hang(c) => c.wait(),
    }
}

/// `on_render_process_unresponsive` (Chromium waits ~15 s of no input acks first).
pub fn hung(browser: &Browser, cb: &UnresponsiveProcessCallback) {
    QUEUE.with_borrow_mut(|q| {
        q.push_back(Pending {
            browser: browser.identifier(),
            origin: String::new(),
            settings: Vec::new(),
            text: "This page isn't responding. Kill it? [y]es [n]o, keep waiting".into(),
            cb: Callback::Hang(cb.clone()),
        })
    });
    show_later();
}

/// The page recovered: its hang question is moot.
pub fn responsive(browser: &Browser) {
    let id = browser.identifier();
    QUEUE
        .with_borrow_mut(|q| q.retain(|p| !(p.browser == id && matches!(p.cb, Callback::Hang(_)))));
    show_later();
}

/// `on_show_permission_prompt`. Always answers (now or after the prompt): a
/// returned 0 would leave the page's request hanging (S7).
pub fn request(
    browser: &Browser,
    prompt_id: u64,
    origin: &str,
    bits: u32,
    cb: &PermissionPromptCallback,
) {
    let wanted: Vec<_> = PROMPT_BITS.iter().filter(|(b, ..)| bits & b != 0).collect();
    let known = wanted
        .iter()
        .fold(MULTIPLE_DOWNLOADS, |acc, (b, ..)| acc | b);
    let ds: Vec<_> = if bits & !known != 0 {
        vec![Decision::Deny]
    } else {
        wanted.iter().map(|(_, s, _)| decide(origin, s)).collect()
    };
    let what: Vec<_> = wanted.iter().map(|(.., w)| *w).collect();
    let settings = wanted.iter().map(|(_, s, _)| *s).collect();
    handle(
        browser,
        origin,
        settings,
        &what,
        combine(&ds),
        Callback::Prompt(prompt_id, cb.clone()),
    );
}

/// `on_request_media_access_permission`.
pub fn request_media(browser: &Browser, origin: &str, bits: u32, cb: &MediaAccessCallback) {
    let wanted: Vec<_> = MEDIA_BITS.iter().filter(|(b, ..)| bits & b != 0).collect();
    let ds: Vec<_> = wanted.iter().map(|(_, s, _)| decide(origin, s)).collect();
    let what: Vec<_> = wanted.iter().map(|(.., w)| *w).collect();
    let mut settings: Vec<&'static str> = wanted.iter().map(|(_, s, _)| *s).collect();
    settings.dedup();
    handle(
        browser,
        origin,
        settings,
        &what,
        combine(&ds),
        Callback::Media(bits, cb.clone()),
    );
}

fn handle(
    browser: &Browser,
    origin: &str,
    settings: Vec<&'static str>,
    what: &[&str],
    d: Decision,
    cb: Callback,
) {
    match d {
        Decision::Allow => answer(&cb, true),
        Decision::Deny => answer(&cb, false),
        Decision::Ask => {
            let text = format!(
                "Allow {origin} to {}? [y]es [n]o [A]lways [N]ever",
                what.join(" and ")
            );
            QUEUE.with_borrow_mut(|q| {
                q.push_back(Pending {
                    browser: browser.identifier(),
                    origin: origin.to_string(),
                    settings,
                    text,
                    cb,
                })
            });
            show_later();
        }
    }
}

/// JS dialogs share the prompt queue: without a handler the page would hang (S7).
/// shortcut: prompt() can't take typed text; `y` returns its default value.
pub fn dialog(
    browser: &Browser,
    origin: &str,
    kind: JsdialogType,
    message: &str,
    default: &str,
    cb: &JsdialogCallback,
) {
    let host = crate::commands::host(origin).to_string();
    let (keys, input) = if kind == JsdialogType::ALERT {
        ("[y]/[n] to close", String::new())
    } else if kind == JsdialogType::PROMPT {
        ("[y]es (answer \"{default}\") [n]o", default.to_string())
    } else {
        ("[y]es [n]o", String::new())
    };
    let keys = keys.replace("{default}", default);
    let text = format!("{host} says: {message} {keys}");
    let cb = Callback::Dialog(cb.clone(), input);
    QUEUE.with_borrow_mut(|q| {
        q.push_back(Pending {
            browser: browser.identifier(),
            origin: origin.to_string(),
            settings: Vec::new(),
            text,
            cb,
        })
    });
    show_later();
}

/// A yes/no question in the prompt queue; `f` gets the answer (Escape = no).
/// The closure may hold a password: a newer question for the tab replaces it, and an
/// unanswered one is dropped (= no) after 5 minutes.
pub fn ask(browser: &Browser, text: String, f: impl Fn(bool) + 'static) {
    let id = ASK_ID.replace(ASK_ID.get() + 1);
    let bid = browser.identifier();
    post_delayed_task(ThreadId::UI, Some(&mut ExpireAsk::new(id)), 5 * 60_000);
    QUEUE.with_borrow_mut(|q| {
        q.retain(|p| !(p.browser == bid && matches!(p.cb, Callback::Ask(..))));
        q.push_back(Pending {
            browser: browser.identifier(),
            origin: String::new(),
            settings: Vec::new(),
            text,
            cb: Callback::Ask(id, std::rc::Rc::new(f)),
        })
    });
    show_later();
}

/// Navigating away from a page with a beforeunload handler.
pub fn leave_page(browser: &Browser, is_reload: bool, cb: &JsdialogCallback) {
    let text = format!(
        "{} this page? Changes you made may not be saved. [y]es [n]o",
        if is_reload { "Reload" } else { "Leave" }
    );
    let cb = Callback::Dialog(cb.clone(), String::new());
    QUEUE.with_borrow_mut(|q| {
        q.push_back(Pending {
            browser: browser.identifier(),
            origin: String::new(),
            settings: Vec::new(),
            text,
            cb,
        })
    });
    show_later();
}

/// CEF closed this tab's dialogs itself (navigation, reload).
pub fn reset_dialogs(browser: &Browser) {
    let id = browser.identifier();
    QUEUE.with_borrow_mut(|q| {
        q.retain(|p| !(p.browser == id && matches!(p.cb, Callback::Dialog(..))))
    });
    show_later();
}

wrap_task! {
    struct Show {}
    impl Task {
        fn execute(&self) { show(); }
    }
}

/// `show()` from a CEF callback: CEF may call us synchronously from inside
/// window code that holds its state (e.g. leave-page dialog during load_url).
fn show_later() {
    post_task(ThreadId::UI, Some(&mut Show::new()));
}

/// Show the oldest open prompt, or leave prompt mode when there is none.
fn show() {
    let text = QUEUE.with_borrow(|q| q.front().map(|p| p.text.clone()));
    let on = text.is_some();
    window::set_prompt(text);
    match (on, modes::mode() == modes::Mode::Prompt) {
        (true, false) => {
            if modes::mode() == modes::Mode::Command {
                window::close_cmdline();
            }
            modes::enter(modes::Mode::Prompt);
            window::focus_active();
        }
        (false, true) => {
            modes::enter(modes::Mode::Normal);
            window::focus_active();
        }
        _ => {}
    }
}

/// Prompt-mode key: y / n / A / N, Escape = no. Other keys are eaten.
pub fn on_key(name: &str) {
    let (allow, keep) = match name {
        "y" => (true, false),
        "n" | "Escape" => (false, false),
        "A" => (true, true),
        "N" => (false, true),
        _ => return,
    };
    let Some(p) = QUEUE.with_borrow_mut(|q| q.pop_front()) else {
        return show();
    };
    if keep && !p.settings.is_empty() {
        save(&p.origin, &p.settings, allow);
    }
    answer(&p.cb, allow);
    show();
}

/// CEF dropped a prompt (navigation, tab closed): forget it without answering.
pub fn dismissed(prompt_id: u64) {
    QUEUE.with_borrow_mut(|q| {
        q.retain(|p| !matches!(p.cb, Callback::Prompt(id, _) if id == prompt_id))
    });
    show_later();
}

/// Tab closed: drop its open prompts (media requests have no dismiss callback).
pub fn forget(browser: &Browser) {
    let id = browser.identifier();
    QUEUE.with_borrow_mut(|q| q.retain(|p| p.browser != id));
    show_later();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_any_deny_wins_then_ask() {
        use Decision::*;
        assert_eq!(combine(&[Allow, Ask]), Ask);
        assert_eq!(combine(&[Ask, Deny]), Deny);
        assert_eq!(combine(&[Allow, Allow]), Allow);
        assert_eq!(combine(&[]), Allow);
    }

    #[test]
    fn bits_map_to_shared_settings() {
        for b in PROMPT_BITS.iter().chain(MEDIA_BITS) {
            assert!(nova_core::permissions::is_setting(b.1), "{}", b.1);
        }
    }
}
