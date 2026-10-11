# novabrowser — plan (rev 3)

Keyboard-driven, minimal browser in the qutebrowser style, built on **CEF**
(Chromium Embedded Framework) with a **Rust** core.
Rev 1–3: 2026-10-10 (IST). Rev 3 adds the first spike results (section 0).

Tags: [Certain] = verified on novarch or in source this session,
[Likely] = strong inference, [Guessing] = needs a spike.

---

## 0. Verified on novarch (2026-10-10)

qutebrowser is only a reference for the UX; it is not being fixed or kept.
Chrome 155 runs fast and stable on this machine, so the GPU, Mesa and driver
stack is fine, and a Chromium-based browser can be too.

### 0.1 Spike results so far

Built the `cef` crate's `examples/cefsimple` (reference clone at
`../cef-rs`, CEF 154.0.34 = Chromium 154.0.8037.98, "minimal" distribution
from `export-cef-dir`). `cargo build --release` took 41 s once CEF was
downloaded. Run: `--use-alloy-style --ozone-platform=wayland`.

| Spike | Result | Evidence |
|---|---|---|
| S1 Native Wayland | **Pass (basic).** The window opens and renders natively on Hyprland. HiDPI, clipboard and `<select>` not checked yet. Harmless log line: `'--ozone-platform=wayland' is not compatible with Vulkan` (Vulkan is off anyway). | ran it [Certain] |
| S4 VA-API | **Pass.** With today's `qt.args` flags, `chrome://gpu` lists hardware decode for H.264, VP8, VP9 and AV1. The GPU process loads `/usr/lib/dri/iHD_drv_video.so`. YouTube AV1 1080p played with 4 of 106 frames dropped during startup. | `chrome://gpu` over DevTools, `/proc/<gpu>/maps` [Certain] |
| S7 Google sign-in | **Pass.** A real account signed in to Gmail and myaccount in an Alloy-style browser with the stock CEF user agent (`Chrome/154.0.0.0`). | user signed in; DevTools target list showed `mail.google.com/mail/u/0/#inbox` [Certain] |
| Codecs | **Problem.** VP9, AV1, Opus and MP3 work. **H.264, AAC and HEVC do not:** `MediaSource.isTypeSupported('video/mp4; codecs="avc1.42E01E"')` is `false`. | DevTools `Runtime.evaluate` [Certain] |
| Widevine | Not available (`requestMediaKeySystemAccess` rejected). Not needed now. | same [Certain] |

Notes:
- **Google sign-in fallback:** if Google starts blocking it, do what
  qutebrowser does (`ua-google` quirk, issue #5182): a Firefox user agent on
  `accounts.google.com/*`. In CEF, use `Emulation.setUserAgentOverride` over
  DevTools for that tab. It also changes `navigator.userAgent` and client
  hints, not just the HTTP header.
- Set `CefSettings.root_cache_path` (CEF warns about singleton behaviour
  without it).

### 0.2 Decision: H.264 / AAC — option C for now (2026-10-10)

Prebuilt CEF leaves out H.264/AAC on purpose because of patent licensing
(CEF #3559, still open; the maintainer in #3820: "We don't provide builds with
proprietary codecs"). qutebrowser plays them because Arch builds QtWebEngine
with `webengine_proprietary_codecs=ON`. The VA-API hardware decoder for H.264
exists, but Chromium never reaches it: the MP4/H.264 code path is compiled
out.

What breaks without them [Likely]: Twitch (H.264 only), many X/Reddit/news
`.mp4` videos, some streaming sites. YouTube works (VP9/AV1).

| Option | Cost | Risk |
|---|---|---|
| **A. Build CEF ourselves with `proprietary_codecs=true ffmpeg_branding="Chrome"` on novahome** (16 cores, 31 GB RAM, 138 GB free), using the AUR `cef-vaapi` PKGBUILD as the starting point | Several hours of build time per CEF bump (monthly), about 100 GB of disk during the build. Chromium patches can break the build. | Build upkeep. The `cef` crate version must match the built CEF version exactly. |
| B. AUR `cef-vaapi-bin` (prebuilt with codecs) | none | Third-party binary from one maintainer (`bnema`, 0 votes, since 2026-05). At 152.0.6 now while the crate is at 154: we would pin a matching older `cef` crate. |
| C. Ship without H.264/AAC | none | The sites above don't play video. Workaround: `hint links spawn mpv {hint-url}`. |

**Decided: C** (user, 2026-10-10). Revisit A before daily driving (Phase 8). No phase
before 8 depends on codecs, and swapping the CEF build only changes `CEF_PATH`.

### 0.3 Process priority (needed for packaging)

ananicy-cpp gives `ly-dm` (the display manager) `nice 16` and idle I/O, and
every process in the session inherits that (`ionice -p <Hyprland pid>` →
`idle`) [Certain]. Chrome and Zen escape it through their own `Doc-View`
rules. A binary called `novabrowser` with no rule would run at nice 16. Phase
9 ships `/etc/ananicy.d/99-local/novabrowser.rules`:
`{ "name": "novabrowser", "type": "Doc-View", "ioclass": "best-effort" }`.
Fixing `ly-dm` itself is optional and outside this project.

---

## 1. Goals and non-goals

### Audience (rev 2026-10-11, user)
A general browser for **keyboard-heavy users** (Vimium / Tridactyl /
qutebrowser / Zen people), not tuned to one person: it must be installable and
usable with **zero config**, learnable from inside the browser, with the mouse
as a fallback. Personal setup lives only in the user's own `config.ts`. Not
aimed at mouse-only users (modal stays the default).

### Goals (v1)
- Modal, keyboard-first: normal / insert / command / hint / passthrough /
  prompt modes. qutebrowser default bindings and command names, so muscle
  memory and existing scripts carry over.
- Minimal UI: tab bar, status bar, `:` command line with completion. Nothing
  else on screen.
- Fast: never block the CEF UI thread. History, adblock list loading, file IO
  and spawned scripts run elsewhere.
- **Stable:** a renderer or GPU crash never loses the session (see section 7).
- Configured with one `config.ts`, with per-domain overrides like
  `config.pattern()`.
- **External tool integration instead of extensions and greasemonkey** (user
  decision): qutebrowser-compatible `spawn` / userscripts (env vars +
  `QUTE_FIFO`), plus a control socket so mpv, `pass`, rofi, waybar and
  Hyprland binds can drive the browser.
- Built-in adblock (Brave's `adblock` crate), dark mode with per-site
  excludes, HTTPS-only, third-party cookie blocking, permission prompts.
- Native Wayland on Hyprland, VA-API video decode on Meteor Lake (Intel Arc).

### Non-goals (v1)
- Chrome extensions, greasemonkey, Widevine DRM.
- macOS / Windows builds.
- Full qutebrowser settings parity (~380 options). Only the ~60 settings the
  current `config.py` uses, plus what they depend on.
- Compiling Chromium. CEF ships prebuilt binaries.

---

## 2. Stack decisions

| Area | Choice | Why |
|---|---|---|
| Engine | CEF via the `cef` crate (tauri-apps/cef-rs) | Version `154.5.0+154.0.34` = Chromium 154, released 2026-10-07, ~weekly releases, x86_64 Linux supported [Certain]. |
| Bootstrap / style | Chrome bootstrap (the only one since M128) with **Alloy-style** windows and browsers | Alloy style has no Chrome UI, no built-in Chrome shortcuts or permission bubbles that fight modal keys. Lightweight Alloy-style windows landed (CEF #3681, closed 2025-12) [Certain]. Google sign-in works in Alloy style (section 0.1) [Certain]. S7 still checks our permission/certificate handlers. Chrome style is the fallback. |
| Windowing | CEF Views: `window_create_top_level`, `browser_view_create`, `panel_create`, `add_overlay_view` | All present in the crate bindings [Certain]. Views on Wayland is supported with `--ozone-platform=wayland`. The `<select>` crash on Wayland (CEF #3937) was fixed 2025-05 [Certain]. CEF #2804 (embedding into non-Views native windows on Wayland) is still open but does not affect us. |
| Core language | Rust | User decision. Native crates for adblock, sqlite and JS. |
| Concurrency | `std::thread` + `std::sync::mpsc`; hop back to the UI thread with CEF `post_task(TID_UI)` | No tokio. A few long-lived worker threads cover everything; an async runtime adds weight and a second scheduler. |
| Browser UI | HTML + TS in `BrowserView`s on a private `nova://` scheme | CSS theming (current `theme.py` colours carry over). Vanilla TS, `bun build`, embedded with `include_bytes!`. |
| Page automation | **DevTools protocol through `execute_dev_tools_method`** (in-process, no debug port) | One mechanism for trusted clicks (`Input.dispatchMouseEvent`), `fake-key` (`Input.dispatchKeyEvent`), `insert-text` (`Input.insertText`) and per-tab dark mode (`Emulation.setAutoDarkModeOverride`) [Likely]. Replaces `SendMouseClickEvent`, which is mainly meant for off-screen rendering. |
| Config | `config.ts` → types stripped by `oxc` → run in QuickJS (`rquickjs`) | User decision. Must run before `cef::initialize` because it sets Chromium switches. |
| Storage | SQLite (`rusqlite`) on its own thread; JSON/text for sessions, quickmarks, bookmarks | |
| Adblock | `adblock` crate 0.13.3 | Same engine and list format as qutebrowser's `adblock` method. |
| Sandbox | Chromium's user-namespace sandbox | `kernel.unprivileged_userns_clone = 1` on novarch [Certain], so no setuid `chrome-sandbox` is needed. |
| Codecs | Prebuilt CEF first; our own CEF build with H.264/AAC before Phase 8 (decision open, section 0.2) | Prebuilt CEF has no H.264/AAC [Certain]. |
| Packaging | `PKGBUILD`: binary + CEF libs in `/usr/lib/novabrowser/`, ananicy rule, `.desktop` | |

**Licence:** porting qutebrowser code (JS helpers, URL pattern rules, key
parsing, tests) makes novabrowser **GPL-3.0-or-later**. CEF (BSD) and
`adblock` (MPL-2.0) are compatible.

---

## 3. Architecture

```
novabrowser (one binary; re-executes itself as CEF subprocesses)
│
├─ Browser process
│   ├─ CEF UI thread (= main thread)      never blocks
│   │   window, tabs, keys/modes, command dispatch, DevTools calls
│   ├─ CEF IO thread                      adblock check, headers, HTTPS upgrade
│   ├─ history thread                     rusqlite: writes + completion queries
│   ├─ io thread (ours)                   session/autoconfig writes (atomic), list downloads
│   └─ ipc thread                         unix socket: single instance + `novabrowser --cmd`
│
├─ GPU process        (separate, unlike QtWebEngine; restarts on its own after a crash)
├─ Network service    (separate)
└─ Renderer processes
    └─ renderer.rs (RenderProcessHandler)
         - focused editable changed → browser process → auto insert mode
         - page scripts: hints, scroll, cosmetic CSS (closed shadow root)
         - nova:// frames only: window.nova.send(json) bridge
```

**Messaging:** UI → core through `window.nova.send()` (a V8 function from the
render process handler, delivered as a process message). Core → UI through
`frame.execute_java_script("nova.apply(<json>)")`. One JSON message shape.

**Keys:** `KeyboardHandler::on_pre_key_event` runs in the browser process and
decides on the spot. The mode and the keymap trie live in Rust: no JS round
trip per key.
- Normal / hint: consume the key, walk the trie, run the command.
- Insert / passthrough: pass keys through; only the `mode-leave` bindings are
  checked, and only single keys (avoids the `jk` problem noted in `config.py`).
- Command: focus moves to the status bar `<input>`, which edits text natively
  and sends `{text, cursor}` to the core for completion.

**Window layout:**
```
CefWindow
└─ BoxLayout (vertical)
   ├─ tabbar BrowserView        nova://ui/tabbar     fixed height
   ├─ content Panel (flex 1)    holds the active tab's BrowserView only
   └─ statusbar BrowserView     nova://ui/statusbar  fixed height
overlays: completion, prompt, keyhint, messages (add_overlay_view, docked bottom)
```
Switching tabs swaps the child of the content panel and calls
`was_hidden(true)` on the old tab, so Chromium throttles it like a background
tab in Chrome.

---

## 4. Repository layout

One crate. Split into a workspace only if build times demand it.

```
novabrowser/
├─ Cargo.toml
├─ PLAN.md
├─ src/
│  ├─ main.rs          subprocess dispatch, CLI, cef::initialize
│  ├─ app.rs           App: switches from config (GPU, VA-API, ozone)
│  ├─ window.rs        layout, overlays
│  ├─ tabs.rs          tab list, lazy restore, pin, undo-close, was_hidden
│  ├─ client.rs        Client + handlers: display, load, keyboard, request,
│  │                   permission, download, life-span, find, crash
│  ├─ devtools.rs      execute_dev_tools_method helpers (input, dark mode)
│  ├─ modes.rs
│  ├─ keys.rs          key notation, trie, count prefix
│  ├─ commands/        registry + arg parser; tab, nav, hint, spawn, config, misc
│  ├─ completion.rs
│  ├─ history.rs       sqlite thread
│  ├─ config/          eval.rs (oxc + QuickJS), settings.rs, urlmatch.rs, apply.rs
│  ├─ adblock.rs
│  ├─ privacy.rs       HTTPS-only, headers, cookies
│  ├─ spawn.rs         userscript env + FIFO
│  ├─ ipc.rs           $XDG_RUNTIME_DIR/novabrowser/ipc
│  └─ renderer.rs
├─ ui/                 tabbar.ts, statusbar.ts, completion.ts, prompt.ts, theme.css
├─ js/                 page scripts ported from qutebrowser: webelem.js, hints.js, scroll.js
├─ nova.d.ts           config API types for the editor
├─ tests/              ported qutebrowser test tables (urlmatch, keyutils), smoke.sh
├─ spikes/             Phase 0 throwaway code
└─ pkg/                PKGBUILD, novabrowser.desktop, ananicy rule
```

### Dev setup
```sh
cargo install export-cef-dir                 # on crates.io [Certain]
export-cef-dir --force ~/.local/share/cef    # repeat on every cef crate bump
export CEF_PATH=~/.local/share/cef
export LD_LIBRARY_PATH=$LD_LIBRARY_PATH:$CEF_PATH
cargo run -- --ozone-platform=wayland
```
Reference: the crate's `examples/cefsimple` (~1,100 lines) already shows a
Views window + BrowserView with the `wrap_*!` handler macros. Phase 0 starts
by copying it.

---

## 5. Component design

### 5.1 Modes, keys, commands
- Key notation identical to qutebrowser (`<Ctrl-[>`, `<Space>tg`, `gT`). Port
  the rules and test cases from `qutebrowser/keyinput/keyutils.py`.
- Default bindings: port `bindings.default` from
  `qutebrowser/config/configdata.yml` (line 3676).
- v1 commands: `open [-t|-b|-w]`, `tab-close`, `tab-next`, `tab-prev`,
  `tab-focus`, `tab-move`, `tab-pin`, `tab-mute`, `undo`, `back`, `forward`,
  `reload`, `stop`, `scroll`, `scroll-page`, `scroll-to-perc`, `zoom`,
  `zoom-in`, `zoom-out`, `search`, `search-next`, `search-prev`, `hint`,
  `yank`, `mode-enter`, `mode-leave`, `cmd-set-text`, `set`, `config-cycle`,
  `config-source`, `bind`, `unbind`, `spawn`, `fake-key`, `insert-text`,
  `jseval`, `devtools`, `download-open`, `adblock-update`, `session-save`,
  `session-load`, `quickmark-*`, `bookmark-*`, `message-info`,
  `message-error`, `view-source`, `quit`. `;;` chaining and aliases.
- Placeholders: `{url}`, `{url:host}`, `{title}`, `{clipboard}`, `{primary}`.

### 5.2 Command line and completion
- Completion models: commands, settings, quickmarks, bookmarks, history,
  search engines. History queries run on the history thread.
- History: `LIKE` on url/title, index, `LIMIT`, newest first. Each new query
  supersedes the pending one, so fast typing never queues work.
- Target: under 16 ms over 100k rows. `cmd-history` with up/down.

### 5.3 Hints
- Element selection from `qutebrowser/javascript/webelem.js`; labels from
  `qutebrowser/browser/hints.py` (home-row chars, uppercase).
- Labels in a closed shadow root so page CSS can't restyle them.
- Follow a hint with `Input.dispatchMouseEvent` over DevTools at the element
  centre (a trusted click). Fallback: JS `click()`.
- Targets: `normal`, `tab`, `tab-bg`, `yank`, `spawn`
  (`hint links spawn mpv {hint-url}`), `userscript`.

### 5.4 Tabs, sessions, history
- One `BrowserView` per tab; restored tabs are lazy (created on first focus).
- **Known limit [Certain]:** the CEF API can read a tab's back/forward list
  (`navigation_entries`) but has no call to restore one. Restored tabs keep
  their URL and scroll position, not their back history. qutebrowser does
  restore back history. Workaround only if this hurts in daily use.
- Session autosave: debounced, atomic (write temp file, `fsync`, rename).
- Downloads: `DownloadHandler`, no prompt, `~/Downloads/`; `download-open`
  uses `xdg-open` (zathura gets PDFs).

### 5.5 Config (`config.ts`)
Same shape as today's `config.py`:

```ts
import { c, bind, unbind, alias, pattern, configDir, readLines } from "nova";

c.url.start_pages = ["https://search.novarch.site"];
c.tabs.position = "top";
c.hints.chars = "asdfghjkl";
c.hints.uppercase = true;
c.qt.args = ["enable-gpu-rasterization", "enable-features=VaapiVideoDecoder,..."];

c.content.https_only.enabled = true;
c.content.https_only.excludes = readLines(`${configDir}/https_excludes`);

for (const d of readLines(`${configDir}/darkmode_excludes`))
  pattern(`*://${d}/*`, (p) => { p.colors.webpage.darkmode.enabled = false; });

alias("toggle-dark-mode", "spawn --userscript toggle_darkmode.py");
bind("<Space>td", "toggle-dark-mode");
bind("<Space>pp", "spawn --userscript qute-pass");
unbind("<Ctrl-v>");
bind("<Ctrl-p>", "mode-enter passthrough");
```

- `c` is a typed proxy. Unknown setting or wrong type → error with the line
  number; the old config stays active.
- `:config-source` re-runs the file. Settings needing a restart
  (`c.qt.args`) say so.
- `:set` writes `autoconfig.json`; `config.ts` wins, as today.
- `autoconfig.yml`'s per-site answers (clipboard for gemini/github/pi.dev,
  notifications for proton/whatsapp, mailto handlers) become the
  first entries of `autoconfig.json`.
- URL patterns ported from `qutebrowser/utils/urlmatch.py` with its test
  table.

### 5.6 How today's settings map to CEF

| qutebrowser setting (in use today) | novabrowser mechanism |
|---|---|
| `qt.args` (GPU raster, zero-copy, VA-API features) | `on_before_command_line_processing` switches; the same flags give hardware decode in CEF 154 (section 0.1) [Certain] |
| `content.blocking.*` (9 lists) | `adblock` engine in `on_before_resource_load` (IO thread); cosmetic CSS per URL |
| `content.cookies.accept = no-3rdparty` + Google exception | request-context preference + per-site content setting (`set_content_setting` exists) [Likely] |
| `content.headers.custom` (`Sec-GPC`), `accept_language` | header edit in `on_before_resource_load`; `accept_language_list` |
| HTTPS-only (`https_only.py`) | built in: upgrade in `on_before_browse`, same local/Tailscale/RFC1918 exemptions + exclude list |
| `colors.webpage.darkmode.*` + per-domain excludes | `--enable-features=WebContentsForceDark` globally; per tab on navigation: `Emulation.setAutoDarkModeOverride` off for excluded hosts [Likely, S5] |
| `colors.webpage.preferred_color_scheme = dark` | `request_context.set_chrome_color_scheme` (exists) or Blink switch |
| permissions = `ask` | permission handler (`on_show_permission_prompt`, `on_request_media_access_permission`) → prompt (`y`/`n`/`A`/`N`), answers saved per site |
| `content.tls.certificate_errors = ask-block-thirdparty` | `on_certificate_error` → prompt for main frame, block subresources |
| `content.autoplay = false` | `--autoplay-policy=document-user-activation-required` |
| `content.webrtc_ip_handling_policy` | request-context preference (`set_preference`) |
| `content.fullscreen.window = true` | `on_fullscreen_mode_change` → fill the window only |
| `content.pdfjs = false` | `plugins.always_open_pdf_externally` preference |
| `content.site_specific_quirks` (Google login) | not needed today: sign-in works with the stock UA [Certain]. Fallback: Firefox UA on `accounts.google.com/*` via `Emulation.setUserAgentOverride` |
| `spellcheck.languages` | spellcheck preferences |
| `downloads.*`, `new_instance_open_target = tab` | download handler; IPC socket opens a tab in the running instance |
| fonts, tab/statusbar/completion/hint/keyhint colours | CSS variables from `c.colors.*` / `c.fonts.*`; port `theme.py` + `theme_colors.py` |

### 5.7 External tools (replaces extensions and greasemonkey)
One command dispatcher, three ways in:

1. **`spawn [--userscript] [--detach] [--output] <cmd> [args]`** on a worker
   thread. With `--userscript`: the same env as qutebrowser (`QUTE_URL`,
   `QUTE_TITLE`, `QUTE_FIFO`, `QUTE_MODE`, `QUTE_SELECTED_TEXT`, `QUTE_HTML`,
   `QUTE_TEXT`, `QUTE_COUNT`, `QUTE_CONFIG_DIR`, `QUTE_DATA_DIR`,
   `QUTE_DOWNLOAD_DIR`, `QUTE_COMMANDLINE_TEXT`, `QUTE_USER_AGENT`,
   `QUTE_VERSION`), searching `<config>/userscripts/` then `<data>/userscripts/`.
   No qutebrowser scripts are shipped or imported (user, 2026-10-11): common
   needs are built in (`dark-toggle`, HTTPS-only in Phase 7, passwords in
   Phase 10); userscripts are the user's own extension point.
2. **`QUTE_FIFO`**: each line runs as a command in the spawning tab.
   `qute-pass` needs `fake-key`, `insert-text`, `mode-enter insert` (all via
   DevTools input, so the page sees trusted events).
3. **Control socket** `$XDG_RUNTIME_DIR/novabrowser/ipc`: single instance,
   `novabrowser --cmd ':open -t x'` from rofi/waybar/Hyprland binds, and
   `--query state` (JSON: tabs, mode, url) for scripts and the smoke test.

---

## 6. Phases

Each phase ends with something runnable and an exit check.

### Phase 0 — Spikes (throwaway, `spikes/`)
| # | Question | Pass condition | Already known |
|---|---|---|---|
| S1 | `cefsimple` runs natively on Hyprland | `--ozone-platform=wayland`, HiDPI right, clipboard, `<select>` works | **renders natively [Certain]**; HiDPI/clipboard/`<select>` left |
| S2 | Views layout | tabbar + content panel + statusbar + overlay; swapping tabs keeps page state | all APIs in bindings |
| S3 | Key interception | `on_pre_key_event` can consume keys for an Alloy-style BrowserView on Wayland | |
| S4 | Video | `chrome://gpu` shows hardware decode with the current VA-API flags | **done, pass [Certain]** |
| S5 | Per-site dark mode | `Emulation.setAutoDarkModeOverride` toggles force-dark for one tab only | method exists in CDP [Likely] |
| S6 | Synthetic input | `Input.dispatchMouseEvent` / `insertText` over `execute_dev_tools_method` work on a login form | method in bindings [Certain] |
| S7 | Alloy style is enough | Google login works, permission + certificate handlers fire, no Chrome UI shows | **Google login passes [Certain]**; handlers left |
| S8 | Renderer hooks | `on_context_created` + V8 function + `on_focused_node_changed` round-trip to the browser process | all in bindings [Certain] |
| S9 | Baseline | startup, idle RSS, Speedometer 3.1, scrolling on 5 heavy sites: spike vs Chrome 155 (the bar to match) | |

Exit: all pass or each failure has a written fallback; otherwise stop and
re-evaluate (fallback stack: Electron + TS, same UI design).

### Phase 1 — Skeleton
Window, tabs, HTML status bar with URL and progress, `novabrowser <url>`,
crash handling (section 7). **Exit:** browse with the mouse; killing a
renderer shows a crashed-tab page that `r` reloads.

### Phase 2 — Modal core
Modes, keys (ported tests), command registry, default bindings, auto insert
mode, passthrough. **Exit:** daily navigation without the mouse;
`cargo test` green.

### Phase 3 — Command line, completion, history
**Exit:** `:open` completion stays under 16 ms with 100k fake history rows.

### Phase 4 — Hints and search
Hints (all targets), `/ ? n N` via `find`, yank. **Exit:** `f`, `F`, `;y`,
`hint links spawn mpv {hint-url}` work on GitHub, Jira, YouTube.

### Phase 5 — config.ts
**Exit:** a translated `config.py` loads; a broken config shows an error and
keeps the old one.

### Phase 6 — External tools
`spawn`, userscript env, `QUTE_FIFO`, `fake-key`, `insert-text`, IPC socket,
`--cmd`, `--query`, built-in `dark-toggle`. **Exit:** a userscript's env and
`QUTE_FIFO` commands work; `fake-key` fills a login form; `dark-toggle` flips
and persists; `novabrowser --cmd` opens a tab in the running instance.
(Rev 2026-10-11: the `<Space>pp`/qute-pass check moved to Phase 10.)

### Phase 7 — Privacy, blocking, prompts
Adblock uses all three parts of Brave's `adblock` engine (rev 2026-10-11,
user): network blocking, cosmetic CSS, and **scriptlets** (uBlock `+js(...)`
rules, needed for YouTube ads) injected into the page's main world before
page scripts run. Engine loaded with uBlock's scriptlet resources. Filter
lists auto-update daily in the background (plus `:adblock-update`), engine
cached on disk so start-up never waits for the network.
**Exit:** blocking matches qutebrowser on a test page set; no ads on 3
YouTube videos; per-site prompt answers survive a restart.

### Phase 8 — Sessions and daily driving
Lazy restore, undo-close, quickmarks/bookmarks, devtools,
fullscreen-in-window, spellcheck. One-shot import with the bookmark store:
bookmarks from Chromium-based (`Bookmarks` JSON) and Firefox-based
(`places.sqlite` `moz_bookmarks`) browsers, plus qutebrowser sessions,
quickmarks, bookmarks. History import is done (`novabrowser --import`,
Phase 3). **Exit:** one week as the only
browser; every problem logged.

### Phase 8b — General defaults (rev 2026-10-11, user)
Make it work for a stranger with no config:
- Clipboard: yank through Chromium's own clipboard (works on X11 and Wayland);
  `{clipboard}`/`{primary}` read via `wl-paste`, falling back to `xclip`.
- Discoverability: which-key popup after a pending prefix (`g`, `;`, `<Space>`…,
  qutebrowser's keyhint + `colors.keyhint.*`); first-run page with ~10 keys,
  F1, and an `--import` offer. F1 / `:bindings` overlay already done.
- Settings without code: `:set` persisted (autoconfig.json, config.ts wins) and
  a `:settings` page (like `:bookmarks`) for the common ones (search engine,
  start page, theme, dark mode, tabs position, downloads dir, cookies).
- A polished default theme + a few presets (light, dark, Tokyo Night).
- Mouse fallback: URL visible somewhere besides the window title; right-click
  menu checked (open in new tab, copy link, copy image, inspect).
- Schema cleanup: drop or clearly mark qutebrowser settings with no meaning
  here (`colors.statusbar.*`, `statusbar.*`, …); defaults say novabrowser.
- README with install, keys, config.
**Exit:** a fresh profile with no config.ts (clean-room tester given only the
README) can browse, search, use hints, change the search engine and theme,
and find a binding, on Wayland and on X11 (Xwayland session counts).

### Phase 9 — Packaging and upkeep (rev 2026-10-11: general)
AUR PKGBUILD only (user): binary + CEF libs under `/usr/lib/novabrowser`,
launcher script setting the library path, `.desktop` + icon, `xdg-settings`
default browser instructions, no machine-specific bits (no ananicy rule,
compositor binds belong in the user's dotfiles). Monthly: bump the `cef`
crate (it follows Chrome stable, so this is the security update), re-run
`export-cef-dir`, run `tests/smoke.sh`. Flatpak / AppImage: not now.

### Phase 10 — Password manager integration (rev 2026-10-11, user)
No built-in store: novabrowser never keeps secrets at rest. Backend-neutral
(rev, user): one small contract (list entries for an origin, get secret, TOTP,
generate, save/update) with per-manager adapters. `rbw` (Bitwarden) first;
`pass`, 1Password CLI (`op`), KeePassXC (`keepassxc-cli`) after, each only as
much as its CLI allows. The rest of this section describes the rbw adapter. Backend ops: `rbw list` (metadata only), `rbw get`,
`rbw code` (TOTP), `rbw generate`, `rbw add --uri`, `rbw edit`.
- **Field detection:** renderer script (S8 bridge) finds username / password /
  OTP fields on load and focus, including split login pages and iframes.
- **Suggestions:** focusing a login field opens an overlay with the vault
  entries whose URI matches the frame's origin; OTP fields offer `rbw code`;
  sign-up / change-password forms offer `rbw generate`.
- **Fill:** only after the user picks (or a key binding); the secret is fetched
  then and set in that frame only (never via the page's JS, never on load).
- **Save / update:** on submit of a form with a password, a prompt offers
  "Save to Bitwarden?" (`rbw add`) or "Update password?" (`rbw edit`) when the
  site has an entry with a different password. The captured password lives in
  browser-process memory only until answered.
- **Editor helper:** `rbw add`/`rbw edit` take the password through
  `$VISUAL`/`$EDITOR` (first line = password, rest = notes). We run them with
  `VISUAL=<helper>`: a tiny program that replaces only the first line of the
  temp file, keeping notes; the secret reaches it over a pipe/fd, never argv.
- **Locked vault:** prompt to run `rbw unlock` (its own pinentry).
- Rules: exact origin match (registrable domain at most, decide in design),
  cross-site frames get nothing, no autofill without a user action.
- To verify first: `rbw list --raw` exposes entry URIs; how `rbw` spawns the
  editor (stdin inherited?) for the helper's secret channel.
**Decisions (2026-10-11, user):** backends via setting `passwords.backend`
= `rbw` (default) | `bw` | `pass` | `none`; adapters in one module, contract:
list entries for a site, secret, TOTP, save, update. Password generation is
ours (urandom), the same for every backend. Match = registrable domain (like
Bitwarden's default "base domain"); `rbw list --raw` has `uris` (checked: 113 of
126 entries). Fill = focus a login/OTP field → suggestion list under it
(non-focusing overlay); Down/Up select, Enter fills, Escape hides; `<Space>pp`
fills the best match without the list. Never on load. The editor helper gets
the secret through its environment (same-uid only, like an fd; never argv).
**Exit:** on a test page set (single-page login, split login, iframe login,
TOTP, sign-up) suggestions appear, fill works, save and update land in the
vault (checked with `rbw get`), and a cross-site iframe gets no suggestion.

---

## 7. Stability rules (the reason for the rewrite)

- **Renderer crash** (`on_render_process_terminated`): the tab shows a local
  "crashed" page with the URL; `r` reloads. The session is untouched.
- **GPU process crash:** Chromium restarts it on its own; we only log it.
- **Hang watchdog:** a debug-build thread checks the UI thread answers a
  posted task within 100 ms and logs a backtrace if not.
- **Atomic writes** for session, autoconfig, quickmarks, bookmarks: temp file,
  `fsync`, rename. A crash mid-write never corrupts them.
- **No blocking on the UI thread:** sqlite, file IO and process waits are
  debug-asserted off it.
- **Logs:** `$XDG_STATE_HOME/novabrowser/log` (CEF log + ours), rotated.
  Coredumps already go to systemd-coredump.

## 8. Performance budget (measured)

| Metric | Target | How |
|---|---|---|
| Key → command dispatched | < 2 ms | timing log in `keys.rs` |
| Completion update | < 16 ms at 100k rows | history-thread timing |
| Cold start → first window | < 500 ms with 30 lazy tabs | `hyperfine` + log timestamp |
| Tab switch | no reload, < 1 frame | log |
| Idle RSS | ≤ Chrome 155 with the same tabs | `smem` |

## 9. Checks
- Unit tests: key notation and trie, URL patterns (ported table), command
  argument parser, config evaluation errors.
- `tests/smoke.sh` over the IPC socket: start, `:open` a local page,
  `--query state`, run a hint, spawn a userscript that writes to `QUTE_FIFO`,
  check the result. Run on every `cef` bump.

## 10. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| No H.264/AAC in prebuilt CEF | Twitch and many `.mp4` videos fail | own CEF build on novahome (section 0.2); `spawn mpv` meanwhile |
| Own CEF build breaks on a bump | stuck on an old Chromium | stay on the last good build; option B as a stopgap |
| `cef` crate missing an API | blocks a feature | S2/S6/S8 checked the main ones already [Certain]; upstream PR or a C shim |
| Per-site dark mode via DevTools fails | `<Space>td` breaks | S5; fallback: injected CSS filter for excluded sites |
| No back-history restore | restored tabs lose back history | accept, or rebuild through redirect stubs later |
| Wayland quirks (IME, HiDPI, decorations #4273) | daily annoyance | S1; Hyprland draws no decorations anyway; XWayland switch as fallback |
| CEF bumps change behaviour | monthly churn | pin version, upgrade on purpose, smoke test |
| Scope creep toward qutebrowser parity | never finishes | settings fixed to what `config.py` uses |

## 11. Plan shape summary

- **Rung:** nothing existing gives qutebrowser UX on a current Chromium with a
  compiled core (Vieb is Electron; Nyxt and luakit use WebKit). So we build,
  reusing CEF, `adblock`, the DevTools protocol, and qutebrowser's JS helpers,
  URL pattern rules, key notation, default bindings and tests.
- **Diff:** a new crate at `novabrowser/` (section 4). `../qutebrowser` stays
  a read-only reference, `../cef-rs` is the `cef` crate source used for spikes.
- **Check:** remaining spikes (S1 rest, S2, S3, S5, S6, S7 rest, S8, S9), then each phase's exit.
- **Skipped (add when needed):**
  - Chrome extensions and greasemonkey; `auto-cookie-consent.js` is mostly
    covered by fanboy-cookiemonster. Add "inject JS on pattern" if a site needs it.
  - Widevine DRM — CEF on Linux can fetch the Widevine CDM through Chrome's component updater (works in stock CEF on the Shaka demo, CEF #3820) [Likely]. Real services (Netflix, Spotify) also need H.264/AAC, so it comes after the codec build. Netflix on Linux is capped at 720p even in Chrome [Likely].
  - Caret mode (`caret.js`) — Phase 8+ if keyboard selection is missed.
  - Script event hooks (`on-load`, `on-url-change`) — when a script needs one.
  - Tab discarding (unloading idle tabs) — when memory with many tabs hurts.
  - Back-history restore — if its absence hurts in daily use.
  - Multi-profile / private windows — second request context when needed.
  - Non-Linux builds.
