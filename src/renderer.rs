//! Renderer process: `window.nova.send(str)` for our UI pages (S8), and the
//! login-field helper (passwords.js) in web frames.
use cef::*;
use std::cell::RefCell;
use std::collections::HashMap;

const PASSWORDS_JS: &str = include_str!("passwords.js");

thread_local! {
    /// frame id → the object passwords.js returned for that frame's context
    static HELPERS: RefCell<HashMap<String, V8Value>> = RefCell::new(HashMap::new());
}

fn frame_id(f: &Frame) -> String {
    CefString::from(&f.identifier()).to_string()
}

/// Call `helper[name](args…)` in the frame's context; string result.
fn call_helper(frame: &Frame, name: &str, args: &[&str]) -> Option<String> {
    let mut obj = HELPERS.with_borrow(|h| h.get(&frame_id(frame)).cloned())?;
    let ctx = frame.v8_context()?;
    // outside on_context_created the context must be entered by hand
    ctx.enter();
    let r = obj.value_bykey(Some(&s(name))).and_then(|f| {
        let args: Vec<Option<V8Value>> = args
            .iter()
            .map(|a| v8_value_create_string(Some(&s(a))))
            .collect();
        f.execute_function(Some(&mut obj), Some(&args))
    });
    ctx.exit();
    let r = r?;
    (r.is_string() != 0).then(|| CefString::from(&r.string_value()).to_string())
}

fn s(v: &str) -> CefString {
    CefString::from(v)
}

wrap_v8_handler! {
    struct NovaV8Handler {}
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
            if name != "send" && name != "report" && name != "scroll" {
                return 0;
            }
            let arg = arguments.and_then(|a| a.first()).and_then(|a| a.as_ref()).filter(|a| a.is_string() != 0);
            let frame = v8_context_get_current_context().and_then(|c| c.frame());
            let (Some(arg), Some(frame)) = (arg, frame) else {
                // Assigning to the out-param is silently lost; try_set works.
                if let Some(e) = exception { e.try_set("nova.send(str): expected a string"); }
                return 1;
            };
            let kind = match name.as_str() { "report" => "nova.creds", "scroll" => "nova.scroll", _ => "nova.send" };
            let msg = process_message_create(Some(&s(kind))).expect("process_message_create");
            msg.argument_list().expect("argument_list").set_string(0, Some(&CefString::from(&arg.string_value())));
            frame.send_process_message(ProcessId::BROWSER, Some(&mut msg.clone()));
            if let Some(r) = retval { *r = v8_value_create_bool(1); }
            1
        }
    }
}

wrap_render_process_handler! {
    pub struct NovaRenderProcess {}
    impl RenderProcessHandler {
        fn on_context_created(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, context: Option<&mut V8Context>) {
            let (Some(frame), Some(context)) = (frame, context) else { return };
            // Our UI pages are data: URLs. Web pages don't get `nova`; the browser
            // process also only accepts messages from UI browsers (client.rs).
            let url = CefString::from(&frame.url()).to_string();
            if url.starts_with("http://") || url.starts_with("https://") {
                return install_helper(frame, context);
            }
            if !url.starts_with("data:") {
                return;
            }
            let global = context.global().expect("global");
            let mut nova = v8_value_create_object(None, None).expect("object");
            let mut send = v8_value_create_function(Some(&s("send")), Some(&mut NovaV8Handler::new())).expect("function");
            nova.set_value_bykey(Some(&s("send")), Some(&mut send), V8Propertyattribute::default());
            global.set_value_bykey(Some(&s("nova")), Some(&mut nova), V8Propertyattribute::default());
        }

        fn on_context_released(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, _context: Option<&mut V8Context>) {
            if let Some(f) = frame {
                HELPERS.with_borrow_mut(|h| h.remove(&frame_id(f)));
            }
        }

        /// "nova.fill" (user, pass, what) from passwords.rs: fill this frame's form.
        fn on_process_message_received(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, _source: ProcessId, message: Option<&mut ProcessMessage>) -> ::std::os::raw::c_int {
            let (Some(frame), Some(m)) = (frame, message) else { return 0 };
            if CefString::from(&m.name()).to_string() != "nova.fill" { return 0; }
            let Some(l) = m.argument_list() else { return 1 };
            let a: Vec<String> = (0..3).map(|i| CefString::from(&l.string(i)).to_string()).collect();
            call_helper(frame, "fill", &[&a[0], &a[1], &a[2]]);
            1
        }

        /// Auto insert mode (S8): report whether the newly focused node is editable.
        fn on_focused_node_changed(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, node: Option<&mut Domnode>) {
            let editable = node.is_some_and(|n| n.is_editable() != 0);
            // frame is None when focus leaves every focusable node
            let Some(frame) = frame.map(|f| f.clone()).or_else(|| browser.and_then(|b| b.main_frame())) else { return };
            // "kind x y w h" for login fields (passwords.js), else ""
            let field = if editable { call_helper(&frame, "focus", &[]).unwrap_or_default() } else { String::new() };
            let msg = process_message_create(Some(&s("nova.focus"))).expect("process_message_create");
            let args = msg.argument_list().expect("argument_list");
            args.set_bool(0, editable as i32);
            args.set_string(1, Some(&s(&field)));
            frame.send_process_message(ProcessId::BROWSER, Some(&mut msg.clone()));
        }
    }
}

/// Evaluate passwords.js in a fresh web-frame context (before page scripts) and
/// keep the object it returns; its `report` callback is our native function.
fn install_helper(frame: &Frame, context: &mut V8Context) {
    let mut ret = None;
    let mut exc = None;
    if context.eval(
        Some(&s(PASSWORDS_JS)),
        Some(&s("novabrowser://passwords.js")),
        1,
        Some(&mut ret),
        Some(&mut exc),
    ) == 0
    {
        return;
    }
    let Some(f) = ret else { return };
    let report = v8_value_create_function(Some(&s("report")), Some(&mut NovaV8Handler::new()));
    if let Some(obj) = f.execute_function_with_context(Some(context), None, Some(&[report])) {
        if obj.is_object() != 0 {
            HELPERS.with_borrow_mut(|h| h.insert(frame_id(frame), obj));
        }
    }
    if frame.is_main() != 0 {
        install_scroll(context);
    }
}

/// Report the page's scroll position 800 ms after scrolling stops (session
/// restore). Passive, no layout reads while scrolling.
const SCROLL_JS: &str = "(function(r){let t;addEventListener('scroll',()=>{clearTimeout(t);t=setTimeout(()=>r(String(Math.round(scrollY))),800)},{passive:true})})";

fn install_scroll(context: &mut V8Context) {
    let (mut ret, mut exc) = (None, None);
    context.eval(
        Some(&s(SCROLL_JS)),
        Some(&s("novabrowser://scroll.js")),
        1,
        Some(&mut ret),
        Some(&mut exc),
    );
    if let Some(f) = ret {
        let r = v8_value_create_function(Some(&s("scroll")), Some(&mut NovaV8Handler::new()));
        f.execute_function_with_context(Some(context), None, Some(&[r]));
    }
}
