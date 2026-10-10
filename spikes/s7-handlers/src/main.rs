//! S7 spike: permission + certificate handlers in an Alloy-style BrowserView.
//!
//! Env:
//! - `S7_ALLOW` = `1` accept / `0` deny (default) / `hold` keep the callback
//!   pending / `default` return 0 (CEF default handling).
//!   Held prompts are answered when the page sets `document.title` to
//!   `s7:release:1` or `s7:release:0`.
//! - `S7_CERT` = `1` continue past certificate errors; `cancel` deny via
//!   `callback.cancel()`; otherwise deny by returning 0.
//! - `S7_JS` = `1` answer JS dialogs ourselves; otherwise default handling.
//! - `S7_URL` start URL (default https://example.com/).
//!
//! Every handler logs a line starting with `s7:`.
use cef::*;
use std::cell::{Cell, RefCell};

thread_local! {
    static OPEN: Cell<i32> = const { Cell::new(0) };
    static HELD_PROMPTS: RefCell<Vec<(u64, PermissionPromptCallback)>> = const { RefCell::new(Vec::new()) };
    static HELD_MEDIA: RefCell<Vec<(u32, MediaAccessCallback)>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Answer {
    Allow,
    Deny,
    Hold,
    Default,
}

fn answer() -> Answer {
    match std::env::var("S7_ALLOW").as_deref() {
        Ok("1") => Answer::Allow,
        Ok("hold") => Answer::Hold,
        Ok("default") => Answer::Default,
        _ => Answer::Deny,
    }
}

fn env_on(name: &str) -> bool {
    std::env::var(name).as_deref() == Ok("1")
}

fn text(s: Option<&CefString>) -> String {
    s.map(|s| s.to_string()).unwrap_or_default()
}

const PERMISSION_NAMES: [(u32, &str); 29] = [
    (1, "AR_SESSION"),
    (2, "CAMERA_PAN_TILT_ZOOM"),
    (4, "CAMERA_STREAM"),
    (8, "CAPTURED_SURFACE_CONTROL"),
    (16, "CLIPBOARD"),
    (32, "TOP_LEVEL_STORAGE_ACCESS"),
    (64, "DISK_QUOTA"),
    (128, "LOCAL_FONTS"),
    (256, "GEOLOCATION"),
    (512, "HAND_TRACKING"),
    (1024, "IDENTITY_PROVIDER"),
    (2048, "IDLE_DETECTION"),
    (4096, "MIC_STREAM"),
    (8192, "MIDI_SYSEX"),
    (16384, "MULTIPLE_DOWNLOADS"),
    (32768, "NOTIFICATIONS"),
    (65536, "KEYBOARD_LOCK"),
    (131072, "POINTER_LOCK"),
    (262144, "PROTECTED_MEDIA_IDENTIFIER"),
    (524288, "REGISTER_PROTOCOL_HANDLER"),
    (1048576, "STORAGE_ACCESS"),
    (2097152, "VR_SESSION"),
    (4194304, "WEB_APP_INSTALLATION"),
    (8388608, "WINDOW_MANAGEMENT"),
    (16777216, "FILE_SYSTEM_ACCESS"),
    (33554432, "LOCAL_NETWORK_ACCESS_DEPRECATED"),
    (67108864, "LOCAL_NETWORK"),
    (134217728, "LOOPBACK_NETWORK"),
    (268435456, "SENSORS"),
];

const MEDIA_NAMES: [(u32, &str); 4] = [
    (1, "DEVICE_AUDIO_CAPTURE"),
    (2, "DEVICE_VIDEO_CAPTURE"),
    (4, "DESKTOP_AUDIO_CAPTURE"),
    (8, "DESKTOP_VIDEO_CAPTURE"),
];

fn bit_names(bits: u32, names: &[(u32, &str)]) -> String {
    let v: Vec<&str> = names
        .iter()
        .filter(|(b, _)| bits & b != 0)
        .map(|(_, n)| *n)
        .collect();
    format!("{bits:#x}[{}]", v.join("|"))
}

fn release_held(allow: bool) {
    let result = if allow {
        PermissionRequestResult::ACCEPT
    } else {
        PermissionRequestResult::DENY
    };
    for (id, cb) in HELD_PROMPTS.take() {
        eprintln!("s7: release prompt {id} -> {result:?}");
        cb.cont(result);
    }
    for (bits, cb) in HELD_MEDIA.take() {
        eprintln!(
            "s7: release media {} -> allow={allow}",
            bit_names(bits, &MEDIA_NAMES)
        );
        if allow {
            cb.cont(bits)
        } else {
            cb.cancel()
        }
    }
}

wrap_permission_handler! {
    struct NovaPermission;
    impl PermissionHandler {
        fn on_request_media_access_permission(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut MediaAccessCallback>,
        ) -> ::std::os::raw::c_int {
            let a = answer();
            let frame_url = frame.map(|f| CefString::from(&f.url()).to_string()).unwrap_or_default();
            eprintln!(
                "s7: on_request_media_access_permission origin={:?} perms={} frame={frame_url:?} answer={a:?}",
                text(requesting_origin),
                bit_names(requested_permissions, &MEDIA_NAMES)
            );
            let Some(cb) = callback else { return 0 };
            match a {
                Answer::Allow => cb.cont(requested_permissions),
                Answer::Deny => cb.cancel(),
                Answer::Hold => HELD_MEDIA.with_borrow_mut(|h| h.push((requested_permissions, cb.clone()))),
                Answer::Default => return 0,
            }
            1
        }

        fn on_show_permission_prompt(
            &self,
            _browser: Option<&mut Browser>,
            prompt_id: u64,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut PermissionPromptCallback>,
        ) -> ::std::os::raw::c_int {
            let a = answer();
            eprintln!(
                "s7: on_show_permission_prompt id={prompt_id} origin={:?} perms={} answer={a:?}",
                text(requesting_origin),
                bit_names(requested_permissions, &PERMISSION_NAMES)
            );
            let Some(cb) = callback else { return 0 };
            match a {
                Answer::Allow => cb.cont(PermissionRequestResult::ACCEPT),
                Answer::Deny => cb.cont(PermissionRequestResult::DENY),
                Answer::Hold => HELD_PROMPTS.with_borrow_mut(|h| h.push((prompt_id, cb.clone()))),
                Answer::Default => return 0,
            }
            1
        }

        fn on_dismiss_permission_prompt(
            &self,
            _browser: Option<&mut Browser>,
            prompt_id: u64,
            result: PermissionRequestResult,
        ) {
            eprintln!("s7: on_dismiss_permission_prompt id={prompt_id} result={result:?}");
        }
    }
}

wrap_request_handler! {
    struct NovaRequest;
    impl RequestHandler {
        fn on_certificate_error(
            &self,
            _browser: Option<&mut Browser>,
            cert_error: Errorcode,
            request_url: Option<&CefString>,
            ssl_info: Option<&mut Sslinfo>,
            callback: Option<&mut Callback>,
        ) -> ::std::os::raw::c_int {
            let mode = std::env::var("S7_CERT").unwrap_or_default();
            let allow = mode == "1";
            let (status, subject) = ssl_info
                .map(|i| {
                    let subject = i
                        .x509_certificate()
                        .and_then(|c| c.subject())
                        .map(|p| CefString::from(&p.display_name()).to_string())
                        .unwrap_or_default();
                    (i.cert_status().get_raw(), subject)
                })
                .unwrap_or_default();
            eprintln!(
                "s7: on_certificate_error code={} ({cert_error:?}) url={:?} cert_status={status:#x} subject={subject:?} allow={allow}",
                cert_error.get_raw(),
                text(request_url)
            );
            // Returning 0 shows Chrome's SSL interstitial, whose "Proceed" link bypasses this handler.
            match (allow, mode.as_str(), callback) {
                (true, _, Some(cb)) => { cb.cont(); 1 }
                (false, "cancel", Some(cb)) => { cb.cancel(); 1 }
                _ => 0,
            }
        }
    }
}

wrap_jsdialog_handler! {
    struct NovaJsDialog;
    impl JsdialogHandler {
        fn on_jsdialog(
            &self,
            _browser: Option<&mut Browser>,
            origin_url: Option<&CefString>,
            dialog_type: JsdialogType,
            message_text: Option<&CefString>,
            _default_prompt_text: Option<&CefString>,
            callback: Option<&mut JsdialogCallback>,
            _suppress_message: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let handle = env_on("S7_JS");
            eprintln!(
                "s7: on_jsdialog type={dialog_type:?} origin={:?} msg={:?} handle={handle}",
                text(origin_url),
                text(message_text)
            );
            match (handle, callback) {
                (true, Some(cb)) => { cb.cont(1, Some(&CefString::from("s7-input"))); 1 }
                _ => 0,
            }
        }
    }
}

wrap_download_handler! {
    struct NovaDownload;
    impl DownloadHandler {
        fn can_download(
            &self,
            _browser: Option<&mut Browser>,
            url: Option<&CefString>,
            request_method: Option<&CefString>,
        ) -> ::std::os::raw::c_int {
            eprintln!("s7: can_download url={:?} method={:?}", text(url), text(request_method));
            1
        }
        fn on_before_download(
            &self,
            _browser: Option<&mut Browser>,
            _download_item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            _callback: Option<&mut BeforeDownloadCallback>,
        ) -> ::std::os::raw::c_int {
            eprintln!("s7: on_before_download suggested_name={:?} (default handling)", text(suggested_name));
            0
        }
    }
}

wrap_display_handler! {
    struct NovaDisplay;
    impl DisplayHandler {
        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            match text(title).as_str() {
                "s7:release:1" => release_held(true),
                "s7:release:0" => release_held(false),
                _ => {}
            }
        }
    }
}

wrap_client! {
    struct NovaClient;
    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(NovaLifeSpan::new()) }
        fn permission_handler(&self) -> Option<PermissionHandler> { Some(NovaPermission::new()) }
        fn request_handler(&self) -> Option<RequestHandler> { Some(NovaRequest::new()) }
        fn jsdialog_handler(&self) -> Option<JsdialogHandler> { Some(NovaJsDialog::new()) }
        fn download_handler(&self) -> Option<DownloadHandler> { Some(NovaDownload::new()) }
        fn display_handler(&self) -> Option<DisplayHandler> { Some(NovaDisplay::new()) }
    }
}

wrap_life_span_handler! {
    struct NovaLifeSpan;
    impl LifeSpanHandler {
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            eprintln!(
                "s7: on_before_popup id={popup_id} url={:?} disposition={target_disposition:?} gesture={user_gesture} -> blocked",
                text(target_url)
            );
            1
        }
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
            let url = std::env::var("S7_URL").unwrap_or_else(|_| "https://example.com/".into());
            let mut client = NovaClient::new();
            let mut delegate = NovaBrowserViewDelegate::new();
            let view = browser_view_create(
                Some(&mut client),
                Some(&CefString::from(url.as_str())),
                Some(&BrowserSettings::default()),
                None,
                None,
                Some(&mut delegate),
            )
            .expect("browser_view_create");
            window.add_child_view(Some(&mut View::from(&view)));
            window.show();
            view.request_focus();
            eprintln!("s7: window shown pid={} answer={:?} cert={:?} js={}", std::process::id(), answer(), std::env::var("S7_CERT").unwrap_or_default(), env_on("S7_JS"));
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
        root_cache_path: CefString::from("/tmp/novabrowser-s7"),
        ..Default::default()
    };
    let mut app = NovaApp::new();
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
