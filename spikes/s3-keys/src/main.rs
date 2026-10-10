//! S3 spike: KeyboardHandler::on_pre_key_event consuming keys for an Alloy
//! BrowserView (Views) on Wayland.
//! Consumed: j, k, Escape (no modifiers). `q` is flagged is_keyboard_shortcut
//! and passed through, to see whether on_key_event fires for it.
//! S3_MODE=raw → consume only RAWKEYDOWN (see whether CHAR/KEYUP leak);
//! default `all` → consume every event type for the consumed keys.
use cef::*;

const PAGE: &str = r#"<!doctype html><title>s3</title>
<body style="font:16px monospace">
<input id=i autofocus style="font:20px monospace;width:400px">
<p id=copy>COPY-ME nova-hands-on</p>
<select id=sel onchange="console.log('select '+this.value)">
  <option>apple</option><option>banana</option><option>cherry</option><option>date</option>
</select>
<textarea id=ta rows=3 cols=40 placeholder="paste here"></textarea>
<pre id=out></pre>
<script>
window.L = [];
for (const t of ["keydown", "keypress", "keyup"])
  addEventListener(t, e => {
    const s = `${t} key=${e.key} code=${e.code} repeat=${e.repeat} target=${e.target.tagName}`;
    L.push(s); console.log(s);
    out.textContent = L.slice(-20).join("\n");
  }, true);
</script></body>"#;

fn data_url(html: &str) -> CefString {
    let b64 = CefString::from(&base64_encode(Some(html.as_bytes()))).to_string();
    CefString::from(format!("data:text/html;base64,{b64}").as_str())
}

const VK_ESCAPE: i32 = 0x1B;
const VK_J: i32 = 0x4A;
const VK_K: i32 = 0x4B;
const VK_Q: i32 = 0x51;
const MOD_MASK: u32 = 0x2 | 0x4 | 0x8 | 0x80; // shift | ctrl | alt | command

fn type_name(t: KeyEventType) -> &'static str {
    match t {
        KeyEventType::RAWKEYDOWN => "RAWKEYDOWN",
        KeyEventType::KEYDOWN => "KEYDOWN",
        KeyEventType::KEYUP => "KEYUP",
        KeyEventType::CHAR => "CHAR",
        _ => "?",
    }
}

fn describe(e: &KeyEvent) -> String {
    let ch = char::from_u32(e.character as u32).unwrap_or('?');
    format!(
        "{} vk=0x{:02X} native={} mods=0x{:X} char={:?}({}) unmod={} sys={} editable={}",
        type_name(e.type_),
        e.windows_key_code,
        e.native_key_code,
        e.modifiers,
        ch,
        e.character,
        e.unmodified_character,
        e.is_system_key,
        e.focus_on_editable_field
    )
}

wrap_client! {
    struct S3Client;
    impl Client {
        fn keyboard_handler(&self) -> Option<KeyboardHandler> { Some(S3Keys::new()) }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(S3LifeSpan::new()) }
    }
}

wrap_keyboard_handler! {
    struct S3Keys;
    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            _browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            os_event: Option<&mut sys::XEvent>,
            is_keyboard_shortcut: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let Some(e) = event else { return 0 };
            let ours = e.modifiers & MOD_MASK == 0
                && matches!(e.windows_key_code, VK_J | VK_K | VK_ESCAPE);
            let raw_only = std::env::var("S3_MODE").as_deref() == Ok("raw");
            let consume = ours && (!raw_only || e.type_ == KeyEventType::RAWKEYDOWN);
            if e.windows_key_code == VK_Q && e.modifiers & MOD_MASK == 0 {
                if let Some(s) = is_keyboard_shortcut { *s = 1; }
            }
            eprintln!(
                "s3: pre  {} os_event={} -> {}",
                describe(e),
                os_event.is_some(),
                if consume { "CONSUME" } else { "pass" }
            );
            consume as i32
        }
        fn on_key_event(
            &self,
            _browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: Option<&mut sys::XEvent>,
        ) -> ::std::os::raw::c_int {
            if let Some(e) = event { eprintln!("s3: post {}", describe(e)); }
            0
        }
    }
}

wrap_life_span_handler! {
    struct S3LifeSpan;
    impl LifeSpanHandler {
        fn on_before_close(&self, _browser: Option<&mut Browser>) { quit_message_loop(); }
    }
}

wrap_browser_view_delegate! {
    struct S3BrowserViewDelegate {}
    impl ViewDelegate {}
    impl BrowserViewDelegate {
        fn browser_runtime_style(&self) -> RuntimeStyle { RuntimeStyle::ALLOY }
    }
}

wrap_window_delegate! {
    struct S3WindowDelegate { view: BrowserView }
    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size { Size { width: 800, height: 600 } }
    }
    impl PanelDelegate {}
    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let window = window.unwrap();
            window.add_child_view(Some(&mut View::from(&self.view)));
            window.set_title(Some(&CefString::from("novabrowser-s3")));
            window.show();
            self.view.request_focus();
        }
        fn window_runtime_style(&self) -> RuntimeStyle { RuntimeStyle::ALLOY }
    }
}

wrap_browser_process_handler! {
    struct S3BrowserProcess;
    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            let view = browser_view_create(
                Some(&mut S3Client::new()),
                Some(&data_url(PAGE)),
                Some(&BrowserSettings::default()),
                None,
                None,
                Some(&mut S3BrowserViewDelegate::new()),
            )
            .expect("browser_view_create");
            window_create_top_level(Some(&mut S3WindowDelegate::new(view)));
        }
    }
}

wrap_app! {
    struct S3App;
    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(S3BrowserProcess::new())
        }
    }
}

fn main() {
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = args::Args::new();
    let code = execute_process(Some(args.as_main_args()), None, std::ptr::null_mut());
    if code >= 0 {
        std::process::exit(code);
    }
    let settings = Settings {
        root_cache_path: CefString::from("/tmp/novabrowser-s3"),
        ..Default::default()
    };
    let mut app = S3App::new();
    assert_eq!(
        initialize(
            Some(args.as_main_args()),
            Some(&settings),
            Some(&mut app),
            std::ptr::null_mut()
        ),
        1
    );
    run_message_loop();
    shutdown();
}
