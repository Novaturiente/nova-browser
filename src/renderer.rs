//! Renderer process: `window.nova.send(str)` for our UI pages (S8).
use cef::*;

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
            if name.map(|n| n.to_string()).as_deref() != Some("send") {
                return 0;
            }
            let arg = arguments.and_then(|a| a.first()).and_then(|a| a.as_ref()).filter(|a| a.is_string() != 0);
            let frame = v8_context_get_current_context().and_then(|c| c.frame());
            let (Some(arg), Some(frame)) = (arg, frame) else {
                // Assigning to the out-param is silently lost; try_set works.
                if let Some(e) = exception { e.try_set("nova.send(str): expected a string"); }
                return 1;
            };
            let msg = process_message_create(Some(&s("nova.send"))).expect("process_message_create");
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
            if !CefString::from(&frame.url()).to_string().starts_with("data:") {
                return;
            }
            let global = context.global().expect("global");
            let mut nova = v8_value_create_object(None, None).expect("object");
            let mut send = v8_value_create_function(Some(&s("send")), Some(&mut NovaV8Handler::new())).expect("function");
            nova.set_value_bykey(Some(&s("send")), Some(&mut send), V8Propertyattribute::default());
            global.set_value_bykey(Some(&s("nova")), Some(&mut nova), V8Propertyattribute::default());
        }
    }
}
