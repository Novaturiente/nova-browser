//! novabrowser: one binary; re-executes itself as the CEF subprocesses.
mod bindings;
mod client;
mod commands;
mod completion;
mod devtools;
mod history;
mod import;
mod keys;
mod modes;
mod renderer;
mod window;

use cef::*;
use std::path::PathBuf;

wrap_browser_process_handler! {
    struct NovaBrowserProcess { start_url: String }
    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            modes::init();
            // here, not in main(): its first post_task needs CEF running
            history::start(xdg_dir("XDG_DATA_HOME", ".local/share").join("history.sqlite"));
            window::create(self.start_url.clone());
        }
    }
}

wrap_app! {
    struct NovaApp { start_url: String }
    impl App {
        fn on_before_command_line_processing(&self, process_type: Option<&CefString>, command_line: Option<&mut CommandLine>) {
            let (Some(cmd), true) = (command_line, process_type.is_none_or(|t| t.to_string().is_empty())) else { return };
            let ozone = CefString::from("ozone-platform");
            if std::env::var_os("WAYLAND_DISPLAY").is_some() && cmd.has_switch(Some(&ozone)) == 0 {
                cmd.append_switch_with_value(Some(&ozone), Some(&CefString::from("wayland")));
            }
        }
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(NovaBrowserProcess::new(self.start_url.clone()))
        }
        fn render_process_handler(&self) -> Option<RenderProcessHandler> {
            Some(renderer::NovaRenderProcess::new())
        }
    }
}

/// `novabrowser [switches] [url]`: first non-switch argument; bare hosts get https://.
fn start_url() -> String {
    let Some(arg) = std::env::args().skip(1).find(|a| !a.starts_with('-')) else {
        return "about:blank".into();
    };
    if arg.contains("://") || arg.starts_with("about:") || arg.starts_with("data:") {
        arg
    } else {
        format!("https://{arg}")
    }
}

fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    let base = std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").expect("HOME")).join(fallback));
    base.join("novabrowser")
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) == Some("--import") {
        let data = xdg_dir("XDG_DATA_HOME", ".local/share");
        std::fs::create_dir_all(&data).expect("create data dir");
        if let Err(e) = import::main(argv.get(2).map(String::as_str), &data.join("history.sqlite")) {
            eprintln!("novabrowser: import: {e}");
            std::process::exit(1);
        }
        return;
    }
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = args::Args::new();
    let mut app = NovaApp::new(start_url());
    // Subprocesses exit here; they need the App too, or the renderer gets no handler.
    let code = execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if code >= 0 {
        std::process::exit(code);
    }

    let data = xdg_dir("XDG_DATA_HOME", ".local/share");
    let state = xdg_dir("XDG_STATE_HOME", ".local/state");
    std::fs::create_dir_all(&state).expect("create state dir");
    let profile = CefString::from(data.to_str().expect("utf-8 data dir"));
    let settings = Settings {
        root_cache_path: profile.clone(),
        cache_path: profile, // empty would mean an in-memory (incognito) profile
        // shortcut: no rotation, rotate when Phase 8 daily driving makes it grow
        log_file: CefString::from(state.join("log").to_str().expect("utf-8 state dir")),
        ..Default::default()
    };
    assert_eq!(
        initialize(
            Some(args.as_main_args()),
            Some(&settings),
            Some(&mut app),
            std::ptr::null_mut()
        ),
        1,
        "cef::initialize failed"
    );
    run_message_loop();
    shutdown();
}
