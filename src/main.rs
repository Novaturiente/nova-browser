//! novabrowser: one binary; re-executes itself as the CEF subprocesses.
mod adblock;
mod client;
mod clipboard;
mod commands;
mod completion;
mod config;
mod devtools;
mod hints;
mod history;
mod import;
mod ipc;
mod keys;
mod modes;
mod net;
mod permissions;
mod renderer;
mod session;
mod settings;
mod spawn;
mod theme;
mod urlmatch;
mod window;

use cef::*;
use std::path::PathBuf;

wrap_browser_process_handler! {
    struct NovaBrowserProcess { start_url: Option<String> }
    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            modes::init();
            net::apply_cookie_policy();
            history::start(xdg_dir("XDG_DATA_HOME", ".local/share").join("history.sqlite"));
            adblock::start();
            ipc::start(); // after CEF init: its requests post_task to the UI thread
            window::create(self.start_url.clone());
        }
    }
}

wrap_app! {
    struct NovaApp { start_url: Option<String> }
    impl App {
        fn on_before_command_line_processing(&self, process_type: Option<&CefString>, command_line: Option<&mut CommandLine>) {
            let (Some(cmd), true) = (command_line, process_type.is_none_or(|t| t.to_string().is_empty())) else { return };
            let ozone = CefString::from("ozone-platform");
            if std::env::var_os("WAYLAND_DISPLAY").is_some() && cmd.has_switch(Some(&ozone)) == 0 {
                cmd.append_switch_with_value(Some(&ozone), Some(&CefString::from("wayland")));
            }
            for (name, value) in switches(&config::get()) {
                let name = CefString::from(name.as_str());
                if cmd.has_switch(Some(&name)) == 1 { continue } // command line wins
                match value {
                    Some(v) => cmd.append_switch_with_value(Some(&name), Some(&CefString::from(v.as_str()))),
                    None => cmd.append_switch(Some(&name)),
                }
            }
            // Our HTTPS-only (net.rs) replaces Chromium's, which would re-upgrade https_excludes hosts.
            let df = CefString::from("disable-features");
            let old = CefString::from(&cmd.switch_value(Some(&df))).to_string();
            let merged = if old.is_empty() { "HttpsUpgrades".to_string() } else { format!("{old},HttpsUpgrades") };
            cmd.append_switch_with_value(Some(&df), Some(&CefString::from(merged.as_str())));
        }
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(NovaBrowserProcess::new(self.start_url.clone()))
        }
        fn render_process_handler(&self) -> Option<RenderProcessHandler> {
            Some(renderer::NovaRenderProcess::new())
        }
    }
}

/// Chromium switches from config (qutebrowser qtargs.py + darkmode.py): `qt.args`
/// verbatim, then the settings that map to a switch.
fn switches(cfg: &config::Config) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = cfg
        .list("qt.args")
        .iter()
        .map(|a| {
            let a = a.trim_start_matches('-');
            match a.split_once('=') {
                Some((k, v)) => (k.to_string(), Some(v.to_string())),
                None => (a.to_string(), None),
            }
        })
        .collect();
    let mut add = |k: &str, v: Option<&str>| out.push((k.into(), v.map(Into::into)));
    add(
        if cfg.bool("scrolling.smooth") {
            "enable-smooth-scrolling"
        } else {
            "disable-smooth-scrolling"
        },
        None,
    );
    if !cfg.bool("content.autoplay") {
        add("autoplay-policy", Some("user-gesture-required"));
    }
    if !cfg.bool("content.canvas_reading") {
        add("disable-reading-from-canvas", None);
    }
    match cfg.str("content.webrtc_ip_handling_policy").as_str() {
        "all-interfaces" => {}
        p => add(
            "force-webrtc-ip-handling-policy",
            Some(&p.replace('-', "_")),
        ),
    }
    match cfg.str("qt.chromium.low_end_device_mode").as_str() {
        "always" => add("enable-low-end-device-mode", None),
        "never" => add("disable-low-end-device-mode", None),
        _ => {}
    }
    let mut blink = Vec::new();
    match cfg.str("colors.webpage.preferred_color_scheme").as_str() {
        "dark" => blink.push("preferredColorScheme=0".to_string()),
        "light" => blink.push("preferredColorScheme=1".to_string()),
        _ => {}
    }
    // Dark mode is on for every page; per-site exclusions switch it off per tab (window.rs).
    if cfg.bool("colors.webpage.darkmode.enabled") || cfg_any_dark_site(cfg) {
        blink.push("forceDarkModeEnabled=true".into());
        let alg = match cfg.str("colors.webpage.darkmode.algorithm").as_str() {
            "brightness-rgb" => 1,
            "lightness-hsl" => 2,
            _ => 3,
        };
        let (policy, classifier) = match cfg.str("colors.webpage.darkmode.policy.images").as_str() {
            "always" => (0, None),
            "never" => (1, None),
            "smart-simple" => (2, Some(1)),
            _ => (2, Some(0)),
        };
        let mut dm = format!("InversionAlgorithm={alg},ImagePolicy={policy}");
        if let Some(c) = classifier {
            dm.push_str(&format!(",ImageClassifierPolicy={c}"));
        }
        out.push(("dark-mode-settings".into(), Some(dm)));
    }
    if !blink.is_empty() {
        out.push(("blink-settings".into(), Some(blink.join(","))));
    }
    out
}

/// Dark mode off globally but on for some pattern: still start Chromium with it.
fn cfg_any_dark_site(cfg: &config::Config) -> bool {
    cfg.any_pattern_value(
        "colors.webpage.darkmode.enabled",
        &serde_json::Value::Bool(true),
    )
}

/// `novabrowser [switches] [url]`: first non-switch argument; bare hosts get https://.
fn start_url() -> Option<String> {
    let arg = std::env::args().skip(1).find(|a| !a.starts_with('-'))?;
    Some(if arg.contains("://") || arg.starts_with("about:") || arg.starts_with("data:") {
        arg
    } else {
        format!("https://{arg}")
    })
}

/// Temp file, fsync, rename: a crash leaves the old or the new file, never half of one.
pub(crate) fn write_atomic(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    std::fs::rename(tmp, path)
}

pub(crate) fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
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
        if let Err(e) = import::main(
            argv.get(2).map(String::as_str),
            &data.join("history.sqlite"),
        ) {
            eprintln!("novabrowser: import: {e}");
            std::process::exit(1);
        }
        return;
    }
    if argv.get(1).map(String::as_str) == Some("--check-config") {
        // load config.ts without starting the browser: errors and unbuilt settings
        match config::load() {
            Ok(()) => println!(
                "{}: ok; not built yet: {}",
                config::path().display(),
                config::get().pending().join(", ")
            ),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    // Control socket: `--cmd` / `--query` talk to the running instance; a plain
    // start while one runs hands the URL over (new_instance_open_target = tab).
    let is_subprocess = argv.iter().any(|a| a.starts_with("--type="));
    if !is_subprocess {
        let req = match argv.get(1).map(String::as_str) {
            Some("--cmd") => Some(format!("cmd {}", argv[2..].join(" "))),
            Some("--query") => Some(format!(
                "query {}",
                argv.get(2).map_or("state", String::as_str)
            )),
            _ => None,
        };
        if let Some(req) = req {
            match ipc::request(&req) {
                Some(reply) => println!("{reply}"),
                None => {
                    eprintln!(
                        "novabrowser: no running instance ({})",
                        ipc::socket_path().display()
                    );
                    std::process::exit(1);
                }
            }
            return;
        }
        if let Some(url) = std::env::args().skip(1).find(|a| !a.starts_with('-')) {
            if ipc::request(&format!("cmd open -t {url}")).is_some() {
                return;
            }
        } else if ipc::request("cmd open -t").is_some() {
            return;
        }
    }
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = args::Args::new();
    // Before CEF: config sets Chromium switches. Subprocesses don't need it (they
    // get switches on their command line); loading there would just cost startup.
    let config_error = if is_subprocess {
        None
    } else {
        config::load().err()
    };
    if !is_subprocess {
        net::load_https_excludes();
    }
    let mut app = NovaApp::new(if is_subprocess { None } else { start_url() });
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
        // navigator.languages; the header itself is set per request in net.rs
        accept_language_list: CefString::from(
            net::language_list(&config::get().str("content.headers.accept_language")).as_str(),
        ),
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
    if let Some(e) = config_error {
        window::startup_message("error", &format!("config: {e}"));
    } else {
        let pending = config::get().pending();
        if !pending.is_empty() {
            eprintln!(
                "novabrowser: config settings not applied yet: {}",
                pending.join(", ")
            );
            window::startup_message(
                "info",
                &format!(
                    "config: {} settings not built yet: {}",
                    pending.len(),
                    pending.join(", ")
                ),
            );
        }
    }
    run_message_loop();
    shutdown();
}
