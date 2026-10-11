//! CEF clients: `TabClient` for web pages, `UiClient` for our bars.
//! Only `UiClient` handles `nova.send`, so web pages can't drive the browser.
use crate::{adblock, commands, completion, modes, net, window};
use cef::*;
use std::cell::Cell;

thread_local! {
    static OPEN: Cell<i32> = const { Cell::new(0) };
    /// Disposition of the popup being created (on_before_popup → on_popup_browser_view_created).
    static BACKGROUND_POPUP: Cell<bool> = const { Cell::new(false) };
}

pub fn take_background_popup() -> bool {
    BACKGROUND_POPUP.replace(false)
}

fn new_tab_disposition(d: WindowOpenDisposition) -> Option<bool> {
    // Some(foreground)
    if d == WindowOpenDisposition::NEW_BACKGROUND_TAB {
        Some(false)
    } else if d == WindowOpenDisposition::NEW_FOREGROUND_TAB
        || d == WindowOpenDisposition::NEW_WINDOW
        || d == WindowOpenDisposition::NEW_POPUP
    {
        Some(true)
    } else {
        None
    }
}

wrap_task! {
    struct OpenTab { url: String, foreground: bool }
    impl Task {
        fn execute(&self) { window::open(&self.url, self.foreground); }
    }
}

wrap_task! {
    struct ReleaseClosing { browser: Browser }
    impl Task {
        fn execute(&self) { window::release_closing(&self.browser); }
    }
}

wrap_client! {
    pub struct TabClient {}
    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> { Some(TabDisplay::new()) }
        fn load_handler(&self) -> Option<LoadHandler> { Some(TabLoad::new()) }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(NovaLifeSpan::new()) }
        fn permission_handler(&self) -> Option<PermissionHandler> { Some(TabPermission::new()) }
        fn jsdialog_handler(&self) -> Option<JsdialogHandler> { Some(TabDialogs::new()) }
        fn request_handler(&self) -> Option<RequestHandler> { Some(TabRequest::new()) }
        fn keyboard_handler(&self) -> Option<KeyboardHandler> { Some(TabKeys::new()) }
        fn on_process_message_received(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _source: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> ::std::os::raw::c_int {
            // Only focus reports from renderer.rs; worst a page can do is toggle insert mode.
            let (Some(b), Some(m)) = (browser, message) else { return 0 };
            if CefString::from(&m.name()).to_string() != "nova.focus" { return 0; }
            let active = window::active_browser().is_some_and(|a| a.identifier() == b.identifier());
            if active {
                modes::on_focus(m.argument_list().is_some_and(|l| l.bool(0) != 0));
            }
            1
        }
    }
}

wrap_client! {
    pub struct UiClient {}
    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(NovaLifeSpan::new()) }
        fn keyboard_handler(&self) -> Option<KeyboardHandler> { Some(UiKeys::new()) }
        fn on_process_message_received(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _source: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> ::std::os::raw::c_int {
            let Some(message) = message else { return 0 };
            if CefString::from(&message.name()).to_string() != "nova.send" { return 0; }
            let text = message.argument_list().map(|l| CefString::from(&l.string(0)).to_string()).unwrap_or_default();
            let (cmd, arg) = text.split_once(' ').unwrap_or((&text, ""));
            match (cmd, arg.parse::<i64>()) {
                ("ready", _) => window::ui_ready(),
                ("tab", Ok(n)) if n >= 0 => window::show(n as usize),
                ("close", Ok(n)) if n >= 0 => window::close(n as usize),
                ("run", _) => { completion::record(arg); window::close_cmdline(); commands::run(arg, None); }
                ("text", _) => completion::on_text(arg),
                ("complete", Ok(by)) => completion::select(by),
                ("hist", Ok(by)) => completion::history_step(by),
                ("cmdleave", _) => window::close_cmdline(),
                _ => eprintln!("novabrowser: unknown UI message {text:?}"),
            }
            1
        }
    }
}

wrap_display_handler! {
    struct TabDisplay {}
    impl DisplayHandler {
        fn on_address_change(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, url: Option<&CefString>) {
            let (Some(b), Some(f), Some(url)) = (browser, frame, url) else { return };
            if f.is_main() != 0 { window::set_url(b, url.to_string()); }
        }
        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            if let (Some(b), Some(t)) = (browser, title) { window::set_title(b, t.to_string()); }
        }
        fn on_loading_progress_change(&self, browser: Option<&mut Browser>, progress: f64) {
            if let Some(b) = browser { window::set_progress(b, progress); }
        }
    }
}

wrap_load_handler! {
    struct TabLoad {}
    impl LoadHandler {
        fn on_loading_state_change(&self, browser: Option<&mut Browser>, is_loading: ::std::os::raw::c_int, back: ::std::os::raw::c_int, fwd: ::std::os::raw::c_int) {
            if let Some(b) = browser { window::set_loading(b, is_loading != 0, back != 0, fwd != 0); }
        }
    }
}

wrap_request_handler! {
    struct TabRequest {}
    impl RequestHandler {
        fn on_open_urlfrom_tab(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, target_url: Option<&CefString>, target_disposition: WindowOpenDisposition, _user_gesture: ::std::os::raw::c_int) -> ::std::os::raw::c_int {
            // Middle-click / ctrl+click: CEF would otherwise navigate the source tab.
            let (Some(url), Some(foreground)) = (target_url, new_tab_disposition(target_disposition)) else { return 0 };
            // Don't create browsers from inside a navigation callback; do it next on the UI loop.
            post_task(ThreadId::UI, Some(&mut OpenTab::new(url.to_string(), foreground)));
            1
        }
        fn on_before_browse(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, request: Option<&mut Request>, _user_gesture: ::std::os::raw::c_int, _is_redirect: ::std::os::raw::c_int) -> ::std::os::raw::c_int {
            if let (Some(b), Some(f), Some(r)) = (browser, frame, request) {
                if f.is_main() != 0 { adblock::on_navigate(b, &CefString::from(&r.url()).to_string()); }
            }
            0
        }
        fn resource_request_handler(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _request: Option<&mut Request>, _is_navigation: ::std::os::raw::c_int, _is_download: ::std::os::raw::c_int, _request_initiator: Option<&CefString>, _disable_default_handling: Option<&mut ::std::os::raw::c_int>) -> Option<ResourceRequestHandler> {
            Some(TabResource::new())
        }
        fn on_render_process_terminated(&self, browser: Option<&mut Browser>, status: TerminationStatus, error_code: ::std::os::raw::c_int, _error_string: Option<&CefString>) {
            let Some(b) = browser else { return };
            let why = if status == TerminationStatus::PROCESS_WAS_KILLED { "killed".to_string() }
                else if status == TerminationStatus::PROCESS_OOM { "out of memory".to_string() }
                else { format!("exit code {error_code}") };
            window::crashed(b, &why);
        }
    }
}

wrap_resource_request_handler! {
    struct TabResource {}
    impl ResourceRequestHandler {
        // IO thread.
        fn on_before_resource_load(&self, browser: Option<&mut Browser>, _frame: Option<&mut Frame>, request: Option<&mut Request>, _callback: Option<&mut Callback>) -> ReturnValue {
            let (Some(b), Some(r)) = (browser, request) else { return ReturnValue::CONTINUE };
            let url = CefString::from(&r.url()).to_string();
            let source = b.main_frame().map(|f| CefString::from(&f.url()).to_string()).unwrap_or_default();
            if adblock::should_block(&url, &source, r.resource_type()) { return ReturnValue::CANCEL; }
            let method = CefString::from(&r.method()).to_string();
            if method == "GET" || method == "HEAD" { // other methods can't be redirected safely
                if let Some(https) = net::https_upgrade(&url) {
                    r.set_url(Some(&https.as_str().into())); // CEF turns this into an internal redirect
                    return ReturnValue::CONTINUE;
                }
            }
            net::set_headers(r, &url);
            net::note_request(&url, &source);
            ReturnValue::CONTINUE
        }
        fn on_resource_redirect(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _request: Option<&mut Request>, _response: Option<&mut Response>, new_url: Option<&mut CefString>) {
            // Redirects skip on_before_resource_load: without this an https server could
            // send us back to http. A site that only works on http then loops
            // (ERR_TOO_MANY_REDIRECTS) until https-toggle excludes it.
            let Some(u) = new_url else { return };
            if let Some(https) = net::https_upgrade(&u.to_string()) { *u = https.as_str().into(); }
        }
        fn cookie_access_filter(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _request: Option<&mut Request>) -> Option<CookieAccessFilter> {
            Some(TabCookies::new())
        }
    }
}

fn first_party(b: Option<&mut Browser>) -> String {
    b.and_then(|b| b.main_frame()).map(|f| CefString::from(&f.url()).to_string()).unwrap_or_default()
}

wrap_cookie_access_filter! {
    struct TabCookies {}
    impl CookieAccessFilter {
        // IO thread.
        fn can_send_cookie(&self, browser: Option<&mut Browser>, _frame: Option<&mut Frame>, request: Option<&mut Request>, _cookie: Option<&Cookie>) -> ::std::os::raw::c_int {
            let url = request.map(|r| CefString::from(&r.url()).to_string()).unwrap_or_default();
            net::cookie_allowed(&url, &first_party(browser)) as i32
        }
        fn can_save_cookie(&self, browser: Option<&mut Browser>, _frame: Option<&mut Frame>, request: Option<&mut Request>, _response: Option<&mut Response>, _cookie: Option<&Cookie>) -> ::std::os::raw::c_int {
            let url = request.map(|r| CefString::from(&r.url()).to_string()).unwrap_or_default();
            net::cookie_allowed(&url, &first_party(browser)) as i32
        }
    }
}

wrap_permission_handler! {
    struct TabPermission {}
    impl PermissionHandler {
        fn on_show_permission_prompt(&self, browser: Option<&mut Browser>, prompt_id: u64, requesting_origin: Option<&CefString>, requested_permissions: u32, callback: Option<&mut PermissionPromptCallback>) -> ::std::os::raw::c_int {
            let (Some(b), Some(cb)) = (browser, callback) else { return 0 };
            crate::permissions::request(b, prompt_id, &requesting_origin.map(|o| o.to_string()).unwrap_or_default(), requested_permissions, cb);
            1
        }
        fn on_request_media_access_permission(&self, browser: Option<&mut Browser>, _frame: Option<&mut Frame>, requesting_origin: Option<&CefString>, requested_permissions: u32, callback: Option<&mut MediaAccessCallback>) -> ::std::os::raw::c_int {
            let (Some(b), Some(cb)) = (browser, callback) else { return 0 };
            crate::permissions::request_media(b, &requesting_origin.map(|o| o.to_string()).unwrap_or_default(), requested_permissions, cb);
            1
        }
        fn on_dismiss_permission_prompt(&self, _browser: Option<&mut Browser>, prompt_id: u64, _result: PermissionRequestResult) {
            crate::permissions::dismissed(prompt_id);
        }
    }
}

wrap_jsdialog_handler! {
    struct TabDialogs {}
    impl JsdialogHandler {
        fn on_jsdialog(&self, browser: Option<&mut Browser>, origin_url: Option<&CefString>, dialog_type: JsdialogType, message_text: Option<&CefString>, default_prompt_text: Option<&CefString>, callback: Option<&mut JsdialogCallback>, _suppress_message: Option<&mut ::std::os::raw::c_int>) -> ::std::os::raw::c_int {
            let (Some(b), Some(cb)) = (browser, callback) else { return 0 };
            let s = |x: Option<&CefString>| x.map(|x| x.to_string()).unwrap_or_default();
            crate::permissions::dialog(b, &s(origin_url), dialog_type, &s(message_text), &s(default_prompt_text), cb);
            1
        }
        fn on_before_unload_dialog(&self, browser: Option<&mut Browser>, _message_text: Option<&CefString>, is_reload: ::std::os::raw::c_int, callback: Option<&mut JsdialogCallback>) -> ::std::os::raw::c_int {
            let (Some(b), Some(cb)) = (browser, callback) else { return 0 };
            crate::permissions::leave_page(b, is_reload != 0, cb);
            1
        }
        fn on_reset_dialog_state(&self, browser: Option<&mut Browser>) {
            if let Some(b) = browser { crate::permissions::reset_dialogs(b); }
        }
    }
}

wrap_keyboard_handler! {
    struct UiKeys {}
    impl KeyboardHandler {
        fn on_pre_key_event(&self, _browser: Option<&mut Browser>, event: Option<&KeyEvent>, _os_event: Option<&mut sys::_XEvent>, _is_shortcut: Option<&mut ::std::os::raw::c_int>) -> ::std::os::raw::c_int {
            // Only prompts focus the tab bar (window::focus_active); the palette handles its own keys.
            let Some(e) = event else { return 0 };
            if e.type_ != KeyEventType::RAWKEYDOWN || modes::mode() != modes::Mode::Prompt { return 0; }
            modes::on_key(e.windows_key_code, e.character, e.modifiers) as i32
        }
    }
}

wrap_keyboard_handler! {
    struct TabKeys {}
    impl KeyboardHandler {
        fn on_pre_key_event(&self, browser: Option<&mut Browser>, event: Option<&KeyEvent>, _os_event: Option<&mut sys::_XEvent>, _is_shortcut: Option<&mut ::std::os::raw::c_int>) -> ::std::os::raw::c_int {
            // Decide on RAWKEYDOWN only; consuming it makes Chromium drop the CHAR/KEYUP (S3).
            let (Some(_), Some(e)) = (browser, event) else { return 0 };
            if e.type_ != KeyEventType::RAWKEYDOWN { return 0; }
            modes::on_key(e.windows_key_code, e.character, e.modifiers) as i32
        }
    }
}

wrap_life_span_handler! {
    struct NovaLifeSpan {}
    impl LifeSpanHandler {
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: ::std::os::raw::c_int,
            _target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            _user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            // Allow it: CEF then hands us the popup's BrowserView, which becomes a tab
            // (window.rs adopt_popup) and keeps window.opener working.
            BACKGROUND_POPUP.set(target_disposition == WindowOpenDisposition::NEW_BACKGROUND_TAB);
            0
        }
        // A closing tab must never close the window (CEF's default for Views), and a
        // detached tab has no window: CloseHostWindow then hits a CHECK. We finish the
        // close ourselves: window.rs keeps the removed BrowserView until here, then drops
        // it (posted, not while CEF is inside DoClose), and that destroys the browser.
        fn do_close(&self, browser: Option<&mut Browser>) -> ::std::os::raw::c_int {
            if let Some(b) = browser {
                post_task(ThreadId::UI, Some(&mut ReleaseClosing::new(b.clone())));
            }
            1
        }
        fn on_after_created(&self, _browser: Option<&mut Browser>) {
            OPEN.set(OPEN.get() + 1);
        }
        fn on_before_close(&self, browser: Option<&mut Browser>) {
            if let Some(b) = browser {
                crate::devtools::forget(b);
                adblock::forget(b);
                crate::permissions::forget(b);
            }
            OPEN.set(OPEN.get() - 1);
            if OPEN.get() == 0 { quit_message_loop(); }
        }
    }
}
