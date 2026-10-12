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
    struct OpenTab { url: String, foreground: bool, opener: Browser }
    impl Task {
        fn execute(&self) { window::in_owner(&self.opener, || window::open(&self.url, self.foreground)); }
    }
}

wrap_task! {
    struct ReleaseClosing { browser: Browser }
    impl Task {
        fn execute(&self) { window::did_close(&self.browser); }
    }
}

wrap_download_handler! {
    struct TabDownloads {}
    impl DownloadHandler {
        fn can_download(&self, browser: Option<&mut Browser>, url: Option<&CefString>, method: Option<&CefString>) -> ::std::os::raw::c_int {
            let (Some(b), Some(url)) = (browser, url) else { return 1 };
            let method = method.map(|m| m.to_string()).unwrap_or_default();
            crate::downloads::chromium_should_download(b, &url.to_string(), &method) as i32
        }
        fn on_before_download(&self, _browser: Option<&mut Browser>, item: Option<&mut DownloadItem>, suggested_name: Option<&CefString>, callback: Option<&mut BeforeDownloadCallback>) -> ::std::os::raw::c_int {
            let (Some(item), Some(cb)) = (item, callback) else { return 0 };
            crate::downloads::before(item, &suggested_name.map(|s| s.to_string()).unwrap_or_default(), cb);
            1
        }
        fn on_download_updated(&self, _browser: Option<&mut Browser>, item: Option<&mut DownloadItem>, callback: Option<&mut DownloadItemCallback>) {
            if let Some(item) = item { crate::downloads::updated(item, callback.as_deref()); }
        }
    }
}

/// Our right-click items: (id offset from MENU_ID_USER_FIRST, label), "" = separator.
fn menu_items(p: &ContextMenuParams) -> Vec<(i32, String)> {
    let s = |x: CefStringUserfree| CefString::from(&x).to_string();
    let flags = p.type_flags().as_ref().0;
    let mut v = vec![];
    if flags & 4 != 0 && !s(p.link_url()).is_empty() {
        // CM_TYPEFLAG_LINK
        for (id, l) in [
            (0, "Open link in new tab"),
            (1, "Open link in background tab"),
            (2, "Open link in new window"),
            (3, "Copy link address"),
            (4, "Save link"),
        ] {
            v.push((id, l.to_string()));
        }
    }
    let src = s(p.source_url());
    let noun = match p.media_type() {
        ContextMenuMediaType::IMAGE => "image",
        ContextMenuMediaType::VIDEO => "video",
        ContextMenuMediaType::AUDIO => "audio",
        _ => "",
    };
    if !noun.is_empty() && !src.is_empty() {
        if !v.is_empty() {
            v.push((-1, String::new()));
        }
        v.push((5, format!("Open {noun} in new tab")));
        if !src.starts_with("blob:") {
            v.push((6, format!("Save {noun}")));
        }
        v.push((7, format!("Copy {noun} address")));
    }
    let sel = s(p.selection_text());
    let sel = sel.trim();
    if !sel.is_empty() && p.is_editable() == 0 {
        if !v.is_empty() {
            v.push((-1, String::new()));
        }
        let short: String = sel.chars().take(30).collect();
        let more = if sel.chars().count() > 30 { "…" } else { "" };
        v.push((8, format!("Search for \"{short}{more}\"")));
    }
    v
}

const MENU_ID_USER_FIRST: i32 = 26500;

wrap_context_menu_handler! {
    struct TabMenu {}
    impl ContextMenuHandler {
        fn on_before_context_menu(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, params: Option<&mut ContextMenuParams>, model: Option<&mut MenuModel>) {
            let (Some(p), Some(m)) = (params, model) else { return };
            let items = menu_items(p);
            for (i, (id, label)) in items.iter().enumerate() {
                if *id < 0 {
                    m.insert_separator_at(i);
                } else {
                    m.insert_item_at(i, MENU_ID_USER_FIRST + id, Some(&CefString::from(label.as_str())));
                }
            }
            if !items.is_empty() && m.count() > items.len() {
                m.insert_separator_at(items.len());
            }
        }
        fn on_context_menu_command(&self, browser: Option<&mut Browser>, _frame: Option<&mut Frame>, params: Option<&mut ContextMenuParams>, command_id: ::std::os::raw::c_int, _event_flags: EventFlags) -> ::std::os::raw::c_int {
            let (Some(b), Some(p)) = (browser, params) else { return 0 };
            if !(MENU_ID_USER_FIRST..MENU_ID_USER_FIRST + 9).contains(&command_id) { return 0; }
            let s = |x: CefStringUserfree| CefString::from(&x).to_string();
            let (link, src, sel) = (s(p.link_url()), s(p.source_url()), s(p.selection_text()));
            window::in_owner(b, || match command_id - MENU_ID_USER_FIRST {
                0 => window::open(&link, true),
                1 => window::open(&link, false),
                2 => window::open_window(Some(link)),
                3 => crate::clipboard::write(&link, false),
                4 => crate::downloads::start(b, &link),
                5 => window::open(&src, true),
                6 => crate::downloads::start(b, &src),
                7 => crate::clipboard::write(&src, false),
                _ => window::open(&commands::url_or_search(sel.trim()), true),
            });
            1
        }
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
        fn download_handler(&self) -> Option<DownloadHandler> { Some(TabDownloads::new()) }
        fn context_menu_handler(&self) -> Option<ContextMenuHandler> { Some(TabMenu::new()) }
        fn on_process_message_received(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _source: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> ::std::os::raw::c_int {
            let (Some(b), Some(m)) = (browser, message) else { return 0 };
            let name = CefString::from(&m.name()).to_string();
            if name == "nova.send" {
                // Our manager pages (bookmarks): main-frame data: page holding this run's token.
                let text = m.argument_list().map(|l| CefString::from(&l.string(0)).to_string()).unwrap_or_default();
                let main_data = frame.is_some_and(|f| f.is_main() != 0 && CefString::from(&f.url()).to_string().starts_with("data:"));
                let msg = text.strip_prefix("page ").and_then(|t| t.split_once(' ')).filter(|(tok, _)| main_data && *tok == crate::page_token());
                if let Some(msg) = msg.and_then(|(_, j)| serde_json::from_str::<serde_json::Value>(j).ok()) {
                    let (on_page, page): (fn(&serde_json::Value) -> Result<(), String>, fn() -> String) = match msg["page"].as_str() {
                        Some("settings") => (crate::settings_page::on_page, crate::settings_page::page),
                        Some("history") => (crate::history::on_page, || crate::history::page(None)),
                        _ => (crate::marks::on_page, crate::marks::page),
                    };
                    match on_page(&msg) {
                        Ok(()) => {
                            let page = page();
                            if let Some(f) = b.main_frame() { f.load_url(Some(&window::data_url(&page))); }
                        }
                        Err(e) => window::message("error", &e),
                    }
                }
                return 1;
            }
            if name == "nova.scroll" {
                // main frame only (renderer.rs); a page can only misreport its own scroll
                let y = m.argument_list().map(|l| CefString::from(&l.string(0)).to_string()).unwrap_or_default();
                if let Ok(y) = y.parse() { window::set_scroll(b, y); }
                return 1;
            }
            if name == "nova.creds" {
                // a page can fake this; worst case is a save prompt the user declines
                let text = m.argument_list().map(|l| CefString::from(&l.string(0)).to_string()).unwrap_or_default();
                if let Some(f) = frame { window::in_owner(b, || crate::passwords::on_submit(b, f, &text)); }
                return 1;
            }
            // Focus reports from renderer.rs; worst a page can do is toggle insert mode
            // or list the user's own logins for its site (filling still needs the user).
            if name != "nova.focus" { return 0; }
            let field = m.argument_list().map(|l| CefString::from(&l.string(1)).to_string()).unwrap_or_default();
            window::in_owner(b, || {
                let active = window::active_browser().is_some_and(|a| a.identifier() == b.identifier());
                if active {
                    modes::on_focus(m.argument_list().is_some_and(|l| l.bool(0) != 0));
                    if let Some(f) = frame {
                        let mut p = field.split(' ');
                        let kind = p.next().unwrap_or("");
                        let n: Vec<i32> = p.filter_map(|x| x.parse().ok()).collect();
                        let rect = match n[..] { [x, y, width, height] => Rect { x, y, width, height }, _ => Rect::default() };
                        crate::passwords::on_focus(b, f, kind, rect);
                    }
                }
            });
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
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _source: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> ::std::os::raw::c_int {
            let (Some(b), Some(message)) = (browser, message) else { return 0 };
            if CefString::from(&message.name()).to_string() != "nova.send" { return 0; }
            let text = message.argument_list().map(|l| CefString::from(&l.string(0)).to_string()).unwrap_or_default();
            let (cmd, arg) = text.split_once(' ').unwrap_or((&text, ""));
            window::in_owner(b, || match (cmd, arg.parse::<i64>()) {
                ("ready", _) => window::ui_ready(),
                ("tab", Ok(n)) if n >= 0 => window::show(n as usize),
                ("close", Ok(n)) if n >= 0 => window::close_user(n as usize, false),
                ("run", _) => { completion::record(arg); window::close_cmdline(); commands::run(arg, None); }
                ("text", _) => completion::on_text(arg),
                ("complete", Ok(by)) => completion::select(by),
                ("hist", Ok(by)) => completion::history_step(by),
                ("cmdleave", _) => window::close_cmdline(),
                ("tip", _) => window::tab_tip(arg),
                ("menu", _) => window::tab_menu(arg),
                ("menuact", _) => window::tab_menu_action(arg),
                ("menuclose", _) => window::close_popup(),
                ("pwpick", Ok(i)) if i >= 0 => crate::passwords::on_click(i as usize),
                _ => eprintln!("novabrowser: unknown UI message {text:?}"),
            });
            1
        }
    }
}

wrap_display_handler! {
    struct TabDisplay {}
    impl DisplayHandler {
        fn on_fullscreen_mode_change(&self, browser: Option<&mut Browser>, fullscreen: ::std::os::raw::c_int) {
            let Some(b) = browser else { return };
            window::in_owner(b, || window::page_fullscreen(fullscreen != 0));
        }
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
        fn on_load_start(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, _transition_type: TransitionType) {
            if let (Some(b), Some(f)) = (browser, frame) {
                if f.is_main() != 0 {
                    adblock::on_load_start(b, &CefString::from(&f.url()).to_string());
                    crate::passwords::hide();
                }
            }
        }
        fn on_loading_state_change(&self, browser: Option<&mut Browser>, is_loading: ::std::os::raw::c_int, back: ::std::os::raw::c_int, fwd: ::std::os::raw::c_int) {
            if let Some(b) = browser { window::set_loading(b, is_loading != 0, back != 0, fwd != 0); }
        }
    }
}

wrap_request_handler! {
    struct TabRequest {}
    impl RequestHandler {
        fn on_open_urlfrom_tab(&self, browser: Option<&mut Browser>, _frame: Option<&mut Frame>, target_url: Option<&CefString>, target_disposition: WindowOpenDisposition, _user_gesture: ::std::os::raw::c_int) -> ::std::os::raw::c_int {
            // Middle-click / ctrl+click: CEF would otherwise navigate the source tab.
            let (Some(b), Some(url), Some(foreground)) = (browser, target_url, new_tab_disposition(target_disposition)) else { return 0 };
            // Don't create browsers from inside a navigation callback; do it next on the UI loop.
            post_task(ThreadId::UI, Some(&mut OpenTab::new(url.to_string(), foreground, b.clone())));
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
        fn on_render_process_unresponsive(&self, browser: Option<&mut Browser>, callback: Option<&mut UnresponsiveProcessCallback>) -> ::std::os::raw::c_int {
            let (Some(b), Some(cb)) = (browser, callback) else { return 0 };
            crate::permissions::hung(b, cb);
            1
        }
        fn on_render_process_responsive(&self, browser: Option<&mut Browser>) {
            if let Some(b) = browser { crate::permissions::responsive(b); }
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
        /// magnet: links go to aria2 (if it runs); other unknown schemes stay blocked.
        fn on_protocol_execution(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, request: Option<&mut Request>, _allow: Option<&mut ::std::os::raw::c_int>) {
            let Some(r) = request else { return };
            let url = CefString::from(&r.url()).to_string();
            if url.starts_with("magnet:") && !crate::downloads::magnet(url) {
                eprintln!("novabrowser: magnet link ignored: aria2 isn't running");
            }
        }
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
    b.and_then(|b| b.main_frame())
        .map(|f| CefString::from(&f.url()).to_string())
        .unwrap_or_default()
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
        fn on_pre_key_event(&self, browser: Option<&mut Browser>, event: Option<&KeyEvent>, _os_event: Option<&mut sys::_XEvent>, _is_shortcut: Option<&mut ::std::os::raw::c_int>) -> ::std::os::raw::c_int {
            // Only prompts focus the tab bar (window::focus_active); the palette handles its own keys.
            let (Some(b), Some(e)) = (browser, event) else { return 0 };
            if e.type_ != KeyEventType::RAWKEYDOWN || modes::mode() != modes::Mode::Prompt { return 0; }
            window::in_owner(b, || modes::on_key(e.windows_key_code, e.character, e.modifiers)) as i32
        }
    }
}

wrap_keyboard_handler! {
    struct TabKeys {}
    impl KeyboardHandler {
        fn on_pre_key_event(&self, browser: Option<&mut Browser>, event: Option<&KeyEvent>, _os_event: Option<&mut sys::_XEvent>, _is_shortcut: Option<&mut ::std::os::raw::c_int>) -> ::std::os::raw::c_int {
            // Decide on RAWKEYDOWN only; consuming it makes Chromium drop the CHAR/KEYUP (S3).
            let (Some(b), Some(e)) = (browser, event) else { return 0 };
            if e.type_ != KeyEventType::RAWKEYDOWN { return 0; }
            window::in_owner(b, || modes::on_key(e.windows_key_code, e.character, e.modifiers)) as i32
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
                crate::passwords::forget(b);
            }
            OPEN.set(OPEN.get() - 1);
            if OPEN.get() == 0 { quit_message_loop(); }
        }
    }
}
