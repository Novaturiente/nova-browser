//! Modes and key dispatch, decided in `on_pre_key_event` (PLAN.md §3 Keys).
use crate::keys::{self, Key, Keymap, Mods, Outcome, Parser};
use crate::{commands, config, hints, window};
use std::cell::RefCell;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Normal,
    Insert,
    Passthrough,
    Command,
    Hint,
    Prompt,
}

struct State {
    mode: Mode,
    parser: Parser,
    normal: Keymap,
    insert: Keymap,
    passthrough: Keymap,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State {
        mode: Mode::Normal,
        parser: Parser::default(),
        normal: Keymap::new(),
        insert: Keymap::new(),
        passthrough: Keymap::new(),
    });
}

/// (Re)build the keymaps: `bindings.default`, then `bindings.commands`, then
/// config.ts bind()/unbind() in order. Bad key strings are reported, not fatal.
/// Every binding in apply order: defaults, then `bindings.commands`, then config.ts
/// bind/unbind. (mode, key, command or None = unbind, set by the user).
fn all_binds(cfg: &config::Config) -> Vec<(String, String, Option<String>, bool)> {
    let mut binds = Vec::new();
    for setting in ["bindings.default", "bindings.commands"] {
        for (mode, map) in cfg.get(setting).as_object().into_iter().flatten() {
            for (key, cmd) in map.as_object().into_iter().flatten() {
                // an empty/null command in bindings.commands unbinds (qutebrowser)
                let cmd = cmd.as_str().filter(|c| !c.is_empty()).map(str::to_string);
                binds.push((mode.clone(), key.clone(), cmd, setting == "bindings.commands"));
            }
        }
    }
    binds.extend(cfg.binds.iter().map(|(m, k, c)| (m.clone(), k.clone(), c.clone(), true)));
    binds
}

/// Effective bindings for `:bindings`: (mode, key in canonical notation, command,
/// set by the user). Later binds replace earlier ones for the same key; unbinds drop it.
pub fn bindings() -> Vec<(String, String, String, bool)> {
    let mut out: Vec<(String, String, String, bool)> = Vec::new();
    for (mode, keystr, cmd, user) in all_binds(&config::get()) {
        let Ok(seq) = keys::parse_keystring(&keystr) else { continue };
        let key: String = seq.iter().map(|k| k.to_string()).collect();
        let at = out.iter().position(|(m, k, _, _)| *m == mode && *k == key);
        match (at, cmd) {
            (Some(i), Some(c)) => out[i] = (mode, key, c, user),
            (None, Some(c)) => out.push((mode, key, c, user)),
            (Some(i), None) => {
                out.remove(i);
            }
            (None, None) => {}
        }
    }
    out
}

pub fn init() {
    let cfg = config::get();
    let binds: Vec<(String, String, Option<String>)> =
        all_binds(&cfg).into_iter().map(|(m, k, c, _)| (m, k, c)).collect();
    S.with_borrow_mut(|s| {
        s.parser.clear();
        s.normal = Keymap::new();
        s.insert = Keymap::new();
        s.passthrough = Keymap::new();
        for (mode, keystr, cmd) in &binds {
            let map = match mode.as_str() {
                "normal" => &mut s.normal,
                "insert" => &mut s.insert,
                "passthrough" => &mut s.passthrough,
                _ => continue, // command/hint/prompt keys are fixed in the UI for now
            };
            match (keys::parse_keystring(keystr), cmd) {
                (Ok(seq), Some(cmd)) => map.bind(&seq, cmd),
                (Ok(seq), None) => {
                    map.unbind(&seq);
                }
                (Err(e), _) => eprintln!("novabrowser: bad binding {keystr:?}: {e}"),
            }
        }
    });
}

pub fn mode() -> Mode {
    S.with_borrow(|s| s.mode)
}

pub fn pending() -> String {
    S.with_borrow(|s| s.parser.pending())
}

pub fn enter(m: Mode) {
    let old = S.with_borrow_mut(|s| {
        s.parser.clear();
        std::mem::replace(&mut s.mode, m)
    });
    if old == Mode::Hint && m != Mode::Hint {
        hints::on_leave();
    }
    if old != m {
        window::refresh();
    }
}

pub fn leave() {
    if mode() == Mode::Command {
        window::close_cmdline();
    } else {
        enter(Mode::Normal);
    }
}

pub fn clear_keychain() {
    S.with_borrow_mut(|s| s.parser.clear());
    window::refresh();
}

/// Focus moved inside the active tab (renderer `on_focused_node_changed`).
/// qutebrowser `input.insert_mode.auto_enter` / `auto_leave`.
pub fn on_focus(editable: bool) {
    let cfg = config::get();
    match (mode(), editable) {
        (Mode::Normal, true) if cfg.bool("input.insert_mode.auto_enter") => enter(Mode::Insert),
        (Mode::Insert, false) if cfg.bool("input.insert_mode.auto_leave") => enter(Mode::Normal),
        _ => {}
    }
}

enum Act {
    Pass,
    Eat,
    Run(String, Option<u32>),
    Hint(String),
    Prompt(String),
}

/// A RAWKEYDOWN for a web tab. Returns true to consume it (the page sees nothing).
pub fn on_key(vk: i32, character: u16, cef_mods: u32) -> bool {
    let Some(key) = keys::key_from_cef(vk, character, cef_mods) else {
        return false;
    };
    let act = S.with_borrow_mut(|s| match s.mode {
        Mode::Normal => match s.parser.handle(&key, &s.normal, true) {
            Outcome::Pending => Act::Eat,
            Outcome::Run { command, count } => Act::Run(command, count),
            // input.forward_unbound_keys = auto: unbound plain characters are eaten, others reach the page
            Outcome::NoMatch if is_plain_char(&key) => Act::Eat,
            Outcome::NoMatch => Act::Pass,
        },
        // Only single-key bindings here, so typing "jk" in a form never triggers anything.
        Mode::Insert => single(&s.insert, &key),
        Mode::Passthrough => single(&s.passthrough, &key),
        Mode::Command => Act::Pass,
        Mode::Prompt => Act::Prompt(key.name.clone()),
        // shortcut: hint-mode keys are fixed (Escape leaves, label chars, Backspace); bindings come with config
        Mode::Hint => Act::Hint(key.name.clone()),
    });
    match act {
        Act::Pass => {
            window::refresh();
            false
        } // a NoMatch may have cleared a shown keychain
        Act::Eat => {
            window::refresh();
            true
        }
        Act::Hint(name) if name == "Escape" => {
            enter(Mode::Normal);
            true
        }
        Act::Hint(name) => {
            hints::on_key(&name);
            true
        }
        Act::Prompt(name) => {
            crate::permissions::on_key(&name);
            true
        }
        Act::Run(cmd, count) => {
            commands::run(&cmd, count);
            window::refresh();
            true
        }
    }
}

fn single(map: &Keymap, key: &Key) -> Act {
    match map.get(std::slice::from_ref(key)) {
        Some(cmd) => Act::Run(cmd.to_string(), None),
        None => Act::Pass,
    }
}

fn is_plain_char(key: &Key) -> bool {
    key.name.chars().count() == 1
        && !key.mods.contains(Mods::CTRL)
        && !key.mods.contains(Mods::ALT)
        && !key.mods.contains(Mods::META)
}

/// For the status bar colour (`colors.statusbar.<mode>`).
pub fn mode_name() -> &'static str {
    match mode() {
        Mode::Normal | Mode::Hint | Mode::Prompt => "normal",
        Mode::Insert => "insert",
        Mode::Passthrough => "passthrough",
        Mode::Command => "command",
    }
}

pub fn mode_label() -> &'static str {
    match mode() {
        Mode::Normal => "",
        Mode::Insert => "-- INSERT MODE --",
        Mode::Passthrough => "-- PASSTHROUGH MODE --",
        Mode::Command => "",
        Mode::Hint => "-- HINT MODE --",
        Mode::Prompt => "-- PROMPT --",
    }
}
