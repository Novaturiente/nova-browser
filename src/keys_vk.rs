//! GDK key -> Windows virtual key code, what Chromium's key events key on (a page's
//! `event.keyCode`, editing keys, shortcuts). Like Chromium's
//! ui/events/keycodes/keyboard_code_conversion_xkb.cc for the keys a US layout has.
//! shortcut: symbols map by their US-layout key; other layouts' punctuation may report
//! another keyCode (text input is unaffected: it goes by character).
use gtk::gdk;

pub fn vk(key: gdk::Key) -> i32 {
    let name = key.name().map(|n| n.to_string()).unwrap_or_default();
    by_name(&name).unwrap_or_else(|| by_char(key.to_unicode()))
}

fn by_name(name: &str) -> Option<i32> {
    Some(match name {
        "BackSpace" => 0x08,
        "Tab" | "ISO_Left_Tab" => 0x09,
        "Clear" | "KP_Begin" => 0x0C,
        "Return" | "KP_Enter" | "ISO_Enter" => 0x0D,
        "Shift_L" | "Shift_R" => 0x10,
        "Control_L" | "Control_R" => 0x11,
        "Alt_L" | "Alt_R" | "Meta_L" | "Meta_R" | "ISO_Level3_Shift" => 0x12,
        "Pause" => 0x13,
        "Caps_Lock" => 0x14,
        "Escape" => 0x1B,
        "space" | "KP_Space" => 0x20,
        "Page_Up" | "KP_Page_Up" | "Prior" | "KP_Prior" => 0x21,
        "Page_Down" | "KP_Page_Down" | "Next" | "KP_Next" => 0x22,
        "End" | "KP_End" => 0x23,
        "Home" | "KP_Home" => 0x24,
        "Left" | "KP_Left" => 0x25,
        "Up" | "KP_Up" => 0x26,
        "Right" | "KP_Right" => 0x27,
        "Down" | "KP_Down" => 0x28,
        "Select" => 0x29,
        "Print" => 0x2C,
        "Insert" | "KP_Insert" => 0x2D,
        "Delete" | "KP_Delete" => 0x2E,
        "Help" => 0x2F,
        "Super_L" => 0x5B,
        "Super_R" => 0x5C,
        "Menu" => 0x5D,
        "KP_0" => 0x60,
        "KP_1" => 0x61,
        "KP_2" => 0x62,
        "KP_3" => 0x63,
        "KP_4" => 0x64,
        "KP_5" => 0x65,
        "KP_6" => 0x66,
        "KP_7" => 0x67,
        "KP_8" => 0x68,
        "KP_9" => 0x69,
        "KP_Multiply" => 0x6A,
        "KP_Add" => 0x6B,
        "KP_Separator" => 0x6C,
        "KP_Subtract" => 0x6D,
        "KP_Decimal" => 0x6E,
        "KP_Divide" => 0x6F,
        "Num_Lock" => 0x90,
        "Scroll_Lock" => 0x91,
        "AudioMute" => 0xAD,
        "AudioLowerVolume" => 0xAE,
        "AudioRaiseVolume" => 0xAF,
        "AudioNext" => 0xB0,
        "AudioPrev" => 0xB1,
        "AudioStop" => 0xB2,
        "AudioPlay" | "AudioPause" => 0xB3,
        _ => {
            // F1..F24
            let n: i32 = name.strip_prefix('F')?.parse().ok()?;
            return (1..=24).contains(&n).then_some(0x6F + n);
        }
    })
}

fn by_char(c: Option<char>) -> i32 {
    let Some(c) = c else { return 0 };
    match c {
        'a'..='z' => c.to_ascii_uppercase() as i32,
        'A'..='Z' | '0'..='9' => c as i32,
        ')' => '0' as i32,
        '!' => '1' as i32,
        '@' => '2' as i32,
        '#' => '3' as i32,
        '$' => '4' as i32,
        '%' => '5' as i32,
        '^' => '6' as i32,
        '&' => '7' as i32,
        '*' => '8' as i32,
        '(' => '9' as i32,
        ';' | ':' => 0xBA,
        '=' | '+' => 0xBB,
        ',' | '<' => 0xBC,
        '-' | '_' => 0xBD,
        '.' | '>' => 0xBE,
        '/' | '?' => 0xBF,
        '`' | '~' => 0xC0,
        '[' | '{' => 0xDB,
        '\\' | '|' => 0xDC,
        ']' | '}' => 0xDD,
        '\'' | '"' => 0xDE,
        _ => 0,
    }
}

#[test]
fn vk_table() {
    assert_eq!(by_name("Return"), Some(0x0D));
    assert_eq!(by_name("F1"), Some(0x70));
    assert_eq!(by_name("F24"), Some(0x87));
    assert_eq!(by_name("F25"), None);
    assert_eq!(by_name("a"), None);
    assert_eq!(by_char(Some('a')), 0x41);
    assert_eq!(by_char(Some('Z')), 0x5A);
    assert_eq!(by_char(Some('?')), 0xBF);
    assert_eq!(by_char(Some('!')), 0x31);
    assert_eq!(by_char(Some('é')), 0);
}
