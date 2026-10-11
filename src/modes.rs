//! Modes and key dispatch, decided in `on_pre_key_event` (PLAN.md §3 Keys).
use crate::keys::{self, Key, Keymap, Mods, Outcome, Parser};
use crate::{bindings, commands, hints, window};
use std::cell::RefCell;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Normal,
    Insert,
    Passthrough,
    Command,
    Hint,
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

/// Load the default bindings; bad key strings are reported, not fatal.
pub fn init() {
    S.with_borrow_mut(|s| {
        for (mode, keystr, cmd) in bindings::DEFAULTS {
            let map = match *mode {
                "normal" => &mut s.normal,
                "insert" => &mut s.insert,
                _ => &mut s.passthrough,
            };
            match keys::parse_keystring(keystr) {
                Ok(seq) => map.bind(&seq, cmd),
                Err(e) => eprintln!("novabrowser: bad default binding {keystr:?}: {e}"),
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
/// qutebrowser `input.insert_mode.auto_enter/auto_leave/auto_load` = true.
pub fn on_focus(editable: bool) {
    match (mode(), editable) {
        (Mode::Normal, true) => enter(Mode::Insert),
        (Mode::Insert, false) => enter(Mode::Normal),
        _ => {}
    }
}

enum Act {
    Pass,
    Eat,
    Run(String, Option<u32>),
    Hint(String),
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

pub fn mode_label() -> &'static str {
    match mode() {
        Mode::Normal => "",
        Mode::Insert => "-- INSERT MODE --",
        Mode::Passthrough => "-- PASSTHROUGH MODE --",
        Mode::Command => "",
        Mode::Hint => "-- HINT MODE --",
    }
}
