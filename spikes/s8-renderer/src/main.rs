//! S8 spike: renderer hooks round-trip to the browser process.
//!
//! Renderer: `on_context_created` installs `window.nova.send(str)` (a V8
//! function) which posts `nova.send` to the browser; `on_focused_node_changed`
//! posts `nova.focus` (editable, tag, type). Browser: logs every message and
//! answers `nova.send` with `nova.reply`; the renderer then calls the page's
//! `window.__novaReply(str)` through V8 (full round trip).
use cef::*;
use std::cell::Cell;

const PAGE: &str = r#"<!doctype html><title>s8</title>
<body style="font:14px sans-serif">
<p>S8 renderer hooks</p>
<input id="i" type="text" placeholder="input">
<input id="p" type="password" placeholder="password">
<textarea id="t">textarea</textarea>
<div id="ce" contenteditable="true" style="border:1px solid #888">contenteditable</div>
<button id="b">button</button>
<p id="log"></p>
<iframe id="f" src="about:blank"></iframe>
<script>
window.novaReplies = [];
window.__novaReply = r => { novaReplies.push(r); document.getElementById('log').textContent = novaReplies.join(' | '); };
// <iframe srcdoc> inside a data: page hangs the renderer (even without our hooks), so fill about:blank instead.
const fd = document.getElementById('f').contentDocument;
fd.body.innerHTML = '<input id=fi placeholder=same-origin-iframe>';
addEventListener('load', () => { window.nova.send('hello'); document.getElementById('f').contentWindow.nova.send('hello-from-iframe'); });
</script>
</body>"#;

thread_local! {
    static OPEN: Cell<i32> = const { Cell::new(0) };
}

fn data_url(html: &str) -> CefString {
    let b64 = CefString::from(&base64_encode(Some(html.as_bytes()))).to_string();
    CefString::from(format!("data:text/html;base64,{b64}").as_str())
}

fn s(v: &str) -> CefString {
    CefString::from(v)
}

fn frame_desc(frame: &Frame) -> String {
    let url = CefString::from(&frame.url()).to_string();
    let url: String = url.chars().take(48).collect();
    format!("main={} url={url}", frame.is_main())
}

/// Build a process message whose argument list holds `args` as strings/bools.
enum Arg<'a> {
    Str(&'a str),
    Bool(bool),
}

fn message(name: &str, args: &[Arg]) -> ProcessMessage {
    let msg = process_message_create(Some(&s(name))).expect("process_message_create");
    let list = msg.argument_list().expect("argument_list");
    for (i, a) in args.iter().enumerate() {
        match a {
            Arg::Str(v) => list.set_string(i, Some(&s(v))),
            Arg::Bool(v) => list.set_bool(i, *v as i32),
        };
    }
    msg
}

fn dump_args(msg: &ProcessMessage) -> Vec<String> {
    let Some(list) = msg.argument_list() else {
        return vec![];
    };
    (0..list.size())
        .map(|i| match list.get_type(i) {
            ValueType::STRING => format!("{:?}", CefString::from(&list.string(i)).to_string()),
            ValueType::BOOL => format!("{}", list.bool(i) != 0),
            ValueType::INT => format!("{}", list.int(i)),
            t => format!("<{t:?}>"),
        })
        .collect()
}

// ---------------------------------------------------------------- renderer

wrap_v8_handler! {
    struct NovaV8Handler;
    impl V8Handler {
        fn execute(
            &self,
            name: Option<&CefString>,
            _object: Option<&mut V8Value>,
            arguments: Option<&[Option<V8Value>]>,
            retval: Option<&mut Option<V8Value>>,
            exception: Option<&mut CefString>,
        ) -> ::std::os::raw::c_int {
            let name = name.map(|n| n.to_string()).unwrap_or_default();
            if name != "send" {
                return 0;
            }
            let arg = arguments
                .and_then(|a| a.first())
                .and_then(|a| a.as_ref())
                .filter(|a| a.is_string() != 0);
            let Some(arg) = arg else {
                // Out-param strings are BorrowedMut: assigning `*e = ...` is silently lost; use try_set.
                if let Some(e) = exception { e.try_set("nova.send(str): expected a string"); }
                return 1;
            };
            let text = CefString::from(&arg.string_value()).to_string();
            // The calling frame: the current V8 context is the one executing the call.
            let frame = v8_context_get_current_context().and_then(|c| c.frame());
            let Some(frame) = frame else {
                if let Some(e) = exception { e.try_set("nova.send: no frame"); }
                return 1;
            };
            eprintln!("s8[renderer {}] nova.send({text:?}) {}", std::process::id(), frame_desc(&frame));
            let mut msg = message("nova.send", &[Arg::Str(&text)]);
            frame.send_process_message(ProcessId::BROWSER, Some(&mut msg));
            if let Some(r) = retval { *r = v8_value_create_bool(1); }
            1
        }
    }
}

wrap_render_process_handler! {
    struct NovaRenderProcess;
    impl RenderProcessHandler {
        fn on_context_created(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            context: Option<&mut V8Context>,
        ) {
            let (Some(frame), Some(context)) = (frame, context) else { return };
            // The context is already entered during this callback.
            let global = context.global().expect("global");
            let mut nova = v8_value_create_object(None, None).expect("object");
            let mut handler = NovaV8Handler::new();
            let mut send = v8_value_create_function(Some(&s("send")), Some(&mut handler)).expect("function");
            nova.set_value_bykey(Some(&s("send")), Some(&mut send), V8Propertyattribute::default());
            global.set_value_bykey(Some(&s("nova")), Some(&mut nova), V8Propertyattribute::default());
            eprintln!("s8[renderer {}] on_context_created {}", std::process::id(), frame_desc(frame));
            let url = CefString::from(&frame.url()).to_string();
            let mut msg = message("nova.context", &[Arg::Str(&url), Arg::Bool(frame.is_main() != 0)]);
            frame.send_process_message(ProcessId::BROWSER, Some(&mut msg));
        }

        fn on_focused_node_changed(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            node: Option<&mut Domnode>,
        ) {
            // node (and frame) are None when focus leaves all editable/focusable nodes.
            let (editable, tag, ty) = match node {
                Some(n) => (
                    n.is_editable() != 0,
                    CefString::from(&n.element_tag_name()).to_string(),
                    CefString::from(&n.element_attribute(Some(&s("type")))).to_string(),
                ),
                None => (false, String::new(), String::new()),
            };
            let frame = frame.map(|f| f.clone()).or_else(|| browser.and_then(|b| b.main_frame()));
            let Some(frame) = frame else { return };
            eprintln!(
                "s8[renderer {}] on_focused_node_changed editable={editable} tag={tag:?} type={ty:?} {}",
                std::process::id(),
                frame_desc(&frame)
            );
            let mut msg = message("nova.focus", &[Arg::Bool(editable), Arg::Str(&tag), Arg::Str(&ty)]);
            frame.send_process_message(ProcessId::BROWSER, Some(&mut msg));
        }

        fn on_process_message_received(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> ::std::os::raw::c_int {
            let (Some(frame), Some(message)) = (frame, message) else { return 0 };
            if CefString::from(&message.name()).to_string() != "nova.reply" {
                return 0;
            }
            let text = message
                .argument_list()
                .map(|l| CefString::from(&l.string(0)).to_string())
                .unwrap_or_default();
            let Some(context) = frame.v8_context() else { return 1 };
            // Outside V8 callbacks the context must be entered explicitly.
            if context.enter() == 0 {
                return 1;
            }
            let called = context
                .global()
                .and_then(|g| g.value_bykey(Some(&s("__novaReply"))))
                .filter(|f| f.is_function() != 0)
                .map(|f| {
                    let arg = v8_value_create_string(Some(&s(&text)));
                    f.execute_function(None, Some(&[arg])).is_some()
                })
                .unwrap_or(false);
            context.exit();
            eprintln!(
                "s8[renderer {}] nova.reply({text:?}) -> window.__novaReply called={called} {}",
                std::process::id(),
                frame_desc(frame)
            );
            1
        }
    }
}

// ----------------------------------------------------------------- browser

wrap_client! {
    struct NovaClient;
    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(NovaLifeSpan::new()) }

        fn on_process_message_received(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> ::std::os::raw::c_int {
            let Some(message) = message else { return 0 };
            let name = CefString::from(&message.name()).to_string();
            let args = dump_args(message);
            let where_ = frame.as_deref().map(frame_desc).unwrap_or_default();
            eprintln!(
                "s8[browser {}] recv {name} {args:?} from={} {where_}",
                std::process::id(),
                source_process.get_raw()
            );
            if name == "nova.send" {
                if let Some(frame) = frame {
                    let text = message
                        .argument_list()
                        .map(|l| CefString::from(&l.string(0)).to_string())
                        .unwrap_or_default();
                    let mut reply = self::message("nova.reply", &[Arg::Str(&format!("ack:{text}"))]);
                    frame.send_process_message(ProcessId::RENDERER, Some(&mut reply));
                }
            }
            1
        }
    }
}

wrap_life_span_handler! {
    struct NovaLifeSpan;
    impl LifeSpanHandler {
        fn on_after_created(&self, _browser: Option<&mut Browser>) {
            OPEN.set(OPEN.get() + 1);
        }
        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            OPEN.set(OPEN.get() - 1);
            if OPEN.get() == 0 { quit_message_loop(); }
        }
    }
}

wrap_browser_view_delegate! {
    struct NovaBrowserViewDelegate {}
    impl ViewDelegate {}
    impl BrowserViewDelegate {
        fn browser_runtime_style(&self) -> RuntimeStyle { RuntimeStyle::ALLOY }
    }
}

wrap_window_delegate! {
    struct NovaWindowDelegate {}
    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size { Size { width: 900, height: 600 } }
    }
    impl PanelDelegate {}
    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let window = window.unwrap();
            let mut client = NovaClient::new();
            let mut delegate = NovaBrowserViewDelegate::new();
            let view = browser_view_create(
                Some(&mut client),
                Some(&data_url(PAGE)),
                Some(&BrowserSettings::default()),
                None,
                None,
                Some(&mut delegate),
            )
            .expect("browser_view_create");
            window.add_child_view(Some(&mut View::from(&view)));
            window.show();
            view.request_focus();
        }
        fn window_runtime_style(&self) -> RuntimeStyle { RuntimeStyle::ALLOY }
    }
}

wrap_browser_process_handler! {
    struct NovaBrowserProcess;
    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            window_create_top_level(Some(&mut NovaWindowDelegate::new()));
        }
    }
}

wrap_app! {
    struct NovaApp;
    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(NovaBrowserProcess::new())
        }
        fn render_process_handler(&self) -> Option<RenderProcessHandler> {
            Some(NovaRenderProcess::new())
        }
    }
}

fn main() {
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = args::Args::new();
    let mut app = NovaApp::new();
    // Subprocesses (renderer included) exit here; the App is required so the
    // renderer gets our RenderProcessHandler.
    let code = execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if code >= 0 {
        std::process::exit(code);
    }
    let settings = Settings {
        root_cache_path: CefString::from("/tmp/novabrowser-s8"),
        ..Default::default()
    };
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
