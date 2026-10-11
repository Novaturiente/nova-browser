# novabrowser — progress

Single source of truth for where the work stands. Read at the start of every
session; update before ending one (rules in `AGENTS.md`).

## Now

- **Phase:** 5 (config.ts)
- **Next action:** Phase 5 per PLAN.md §5.5/§6: decide TS stripping (oxc vs alternatives) and setting scope with user, then `config.ts` loader in QuickJS before `cef::initialize`, `c` proxy with typed errors, bind/unbind/alias, URL patterns (urlmatch.py port), theming via CSS variables (user moved all UI customisation here). Deferred, not blocking: HiDPI (S1), IME (S3b), S9b YouTube jank, own CEF build (allocator + codecs).
- **Blockers:** none

## Checklist

Status: `[ ]` todo · `[~]` in progress / partial · `[x]` done · `[-]` dropped.
Each done item links its evidence (file, commit, or session-log date).

### Phase 0 — spikes (PLAN.md §6)
- [~] S1 native Wayland — renders natively (2026-10-10). `<select>` + clipboard pass automated (2026-10-10, `S1_SENDSHORTCUT=1 spikes/s1-wayland/run.sh`, `/tmp/s1/final/out.txt`): native popup placed under the select, 20 rows, ArrowDown×3+Enter → `value=o3` + input/change; `wl-copy` → Ctrl+V pastes; copy-out reaches `wl-paste` after one real key press (compositor `send_shortcut`), CDP-only input never sets the Wayland selection (no input serial). eDP-1 scale 1, DPR 1. Hands-on by user (2026-10-10, `spikes/s3-keys` page + CDP readback): real drag + Ctrl+C → `wl-paste` = `COPY-ME nova-hands-on`, Ctrl+V into textarea works, mouse click on popup option → `sel.value=cherry`, drag/double-click sets primary (`wl-paste -p` = `keypress`). Left: HiDPI on a scale>1 monitor (no such monitor now).
- [x] S2 Views layout — tabbar + content panel + statusbar + overlay; tab swap keeps state (2026-10-10, `spikes/s2-layout`: bars 24 px, content flex 752 px, overlay 600×120 docked above statusbar and toggles; after swap 1→0 Wikipedia `scrollY` 1500 and JS var kept, hidden tab reports `visibilityState=hidden`).
- [x] S3 key interception — `on_pre_key_event` consumes keys in an Alloy BrowserView on Wayland (2026-10-10, `spikes/s3-keys/run.sh`, real Wayland keys via `wtype`: `j`/`k`/Esc consumed → page gets no keydown/keypress/keyup, input value `"abx"` from `ajkb<Esc>xq`; works with input or body focused; repeats consumed too).
- [~] S3b real keyboard (2026-10-10, user at keyboard, log of s3-keys): Shift+B → vk 0x42 mods 0x2 char 'B', page `key=B code=KeyB`; Shift+j passes (only unmodified j consumed) → `J`; `;` → vk 0xBA; Ctrl+j → char '\n'(10), vk 0x4A (match on vk confirmed); Escape tap + hold consumed incl. repeats (mods 0x2000) and KEYUPs (page saw nothing); `a` hold → repeat=true events, input `aaaaaaaaa`; Super = vk 0x5B mods 0x400. Left: IME/compose (none installed).
- [x] S4 VA-API — hw decode H.264/VP8/VP9/AV1 listed, AV1 YouTube smooth (2026-10-10).
- [x] S5 per-site dark mode — `Emulation.setAutoDarkModeOverride` per tab (2026-10-10, `spikes/s5-s6/s5-darkmode.mjs`, two targets in one cefsimple: override on A → A luma 241→43, B stays 241; survives reload + navigation; with `--enable-features=WebContentsForceDark`, `{enabled:false}` lights only A).
- [x] S5b override lifetime via `execute_dev_tools_method` (2026-10-10, `spikes/s2-layout` title `nova:dark`: one call, no observer → content luma 249 → 37, still dark after 10 s, after navigation, after tab swap away/back; CEF's in-process session persists per browser).
- [x] S6 synthetic input — `Input.dispatchMouseEvent` / `insertText` (2026-10-10, `spikes/s5-s6/s6-input.mjs` over remote port: the-internet.herokuapp.com/login → "You logged into a secure area!", all events `isTrusted=true`). `insertText` sends no key events. Via `execute_dev_tools_method` not yet run (same CDP method; covered by S5b).
- [x] S7 Alloy style enough — Google sign-in passes; permission + certificate handlers fire, no Chrome UI (2026-10-10, `spikes/s7-handlers/drive.sh`, logs `/tmp/s7-*.log`: notifications/geolocation/clipboard → `on_show_permission_prompt`, mic/cam → `on_request_media_access_permission`, page sees allow/deny; expired/self-signed badssl → `on_certificate_error` -201/-202; held prompt answered later works).
- [x] S7b decide cert-deny UX (2026-10-10, user: keep Chrome interstitial) — on deny CEF shows Chrome's interstitial (`chrome-error://chromewebdata/`) whose "Proceed" link bypasses our handler. Options: own error page on load error, or accept it. Needs user decision before Phase 7.
- [ ] S9b YouTube scroll jank in CEF (22% frames >20 ms vs Chrome 3%) — YouTube is DOM/layout-heavy, so likely the same allocator gap [guess]; re-measure on the own build before chasing codecs.
- [x] S8 renderer hooks — `on_context_created` + V8 fn + `on_focused_node_changed` round-trip (2026-10-10, `bash spikes/s8-renderer/drive.sh`, re-run by main session: `nova.send` → browser → `nova.reply` → page JS (`novaReplies=["ack:hello","ack:from-devtools"]`); focus msgs for input/password/textarea/contenteditable=editable, button/A not, blur → empty; works after navigation, in about:blank iframe and cross-site OOPIF).
- [x] S9 baseline vs Chrome 155 — measured; CEF behind on Speedometer (−16%), memory (+12%), YouTube scroll; fallback written in Decisions. Run 2 (AC, EPP balance_power, identical anti-throttling flags, `results/speedometer2.json`, `results/scroll.json`): Speedometer 3.1 all 6 valid, CEF 14.3/14.1/14.3 vs Chrome 16.9/17.1/16.9 (median 14.3 vs 16.9); scroll avg FPS / % frames >20 ms, CEF vs Chrome: YouTube 44.6/22% vs 54.9/3%, GitHub 60/0.2% vs 60/0.3%, Maps 59.6/0.7% vs 60/0.2%, Reddit 56.3/4.4% vs 50.7/4.3%, Wikipedia 59.9/0.5% vs 60/0.3%. Run 1, 2026-10-10, `spikes/s9-baseline/results/*.json`, same GPU path both (ANGLE GL, Intel Arc MTL, gpu_compositing enabled): startup to window median cefsimple 625 ms / s2 639 ms / Chrome 920 ms (n=5); idle PSS 5 heavy sites 60 s: cefsimple 1450 MB (separate windows) vs Chrome 1300 MB windows / 1100 MB tabs (+12–32%, n=3). Speedometer 3.1: CEF 10.3 vs Chrome 12.9 but both runs flagged invalid, on battery, `low-power` profile → not usable. (run 1 Speedometer superseded by run 2).

### Phases 1–9 (PLAN.md §6; tick when the phase's exit check passes)
- [x] 1 Skeleton (2026-10-11; `src/{main,window,client,renderer}.rs`, `ui/{tabbar,statusbar}.html`). Exit check: CDP-driven trusted mouse + user by hand — links, `target=_blank` and `window.open` → new tab (opener kept), middle-click link → background tab, click/middle-click tab bar → switch/close, status bar URL + progress, window title; web pages get no `window.nova`. `kill -9` of the tab's renderer (found by CPU-time delta via `SystemInfo.getProcessInfo`) → crash page with URL, status `[crashed] …`; real `r` (compositor `send_shortcut`) reloads; `r` types normally elsewhere (user). Mod+Q closes; exits 0 with a detached tab open, no leftover processes. `cargo test`: js_str escaping. Scripts: `/tmp/nova-p1/{drive,crash,bg}.mjs` (not kept). Not done here: hang watchdog, atomic writes (nothing writes yet), log rotation — later phases.
- [x] 2 Modal core (2026-10-11; `src/{keys,bindings,commands,modes,devtools}.rs`, focus msg in `renderer.rs`, cmdline in `ui/statusbar.html`). `cargo test` 27 green (keys port of qutebrowser `test_keyutils`/`test_basekeyparser` rows by `worker`, command tokenizer/`;;`/aliases, zoom steps, js_str). Real keys via compositor `send_shortcut` + CDP readback: `o`/`O` + URL, `j`×3+`5j` = 320 px, `G`/`gg`, `J`/`K`, `d`/`u`, `H`/`L`, `+`/`-`/`=` zoom 110/125/100%, pending `g` shown + Escape clears, `:foobar` → error msg, `i`/Escape, click into input → auto insert (`jkr` typed), Escape → `j` scrolls, Ctrl-v passthrough / Shift-Escape. User drove HN without mouse (o/O, j k d u J K H L gg G, Ctrl-d/u, T+n, i/Escape, click into text box): all work. Not here: hints `f`, find `/`, `yy`/clipboard, completion/history, user config bindings (later phases); ~58 default commands, unimplemented ones report "not implemented yet".
- [x] 3 Command line, completion, history (2026-10-11; `src/{history,completion}.rs`, `ui/completion.html`, readline keys in `ui/statusbar.html`). Exit check: `cargo test` `search_100k_rows_under_16ms` — 100k rows, queries `""`/site42/`topic 7`/`slug article`/nomatch/`site4999 99999`/`site 7`/`w`/`zq`/`x y` all 0.01–2.2 ms; live browser on a 100k-row profile logged no query > 16 ms. 30 tests green. Real keys (compositor): `o`+text → history list, Tab/Shift-Tab fill cmdline, Enter opens; `:tab-` → command list; Up/Down prefix history, persists across restart; Ctrl-u/a/k/w/y, Alt-b; Escape hides overlay. User checked both completion and history: working. Not here: settings/bookmark/quickmark models (Phase 5+), match highlighting, Ctrl-d delete-item, redirect filtering, qutebrowser history import.
- [x] 4 Hints and search (2026-10-11; `src/{hints.rs,hints.js,clipboard.rs}`, find/yank/spawn in `commands.rs`, CDP request/response + isolated world in `devtools.rs`). Exit check, real keys (compositor) + CDP readback: Wikipedia `f` follows, `F` background tab, `;y` → `wl-paste` = link, `;o` fills `:open <url>`, Escape removes the label host; GitHub repo page labelled, `f` follows; Jira `f` opens the Dashboards menu, `gi` focuses quick search → insert mode; YouTube `hint links spawn … {hint-url}` got the `watch?v=` URL; `/borrow` incremental highlight, `n`/`N`; `yy`/`Pp`/`yank selection`/`yank inline [{title}]({url:yank})`. Labels = qutebrowser `_hint_scattered` (test values taken from the Python code). 34 tests green. User ran the 20-step manual list incl. real `mpv`: all work. Fixed on the way: `hint inputs --first` flag after the group; placeholders expanded per token; closed tabs/window kept running pages with unload handlers (YouTube audio) — see Decisions. Not here: cross-origin iframe hints (same as qutebrowser), hint styling (Phase 5), `;d` download, `-w` window target, beforeunload prompts on close (Phase 8).
- [ ] 5 config.ts
- [ ] 6 External tools
- [ ] 7 Privacy, blocking, prompts
- [ ] 8 Sessions and daily driving (revisit codecs option A first)
- [ ] 9 Packaging and upkeep
- [ ] 10 Password manager integration — Bitwarden via `rbw`, no own store (PLAN.md Phase 10, rev 2026-10-11)
- [x] History import `novabrowser --import [profile]` (2026-10-11; `src/import.rs`, `history::import`). Chromium `History`, Firefox `places.sqlite`, qutebrowser `history.sqlite`; works on a copy (db + WAL). Run into `/tmp/nova-imp`: no-arg lists 7 profiles here; Zen 11711 + Helium 6584 + Chrome 562 + qutebrowser 1778 → 14245 URLs, re-import unchanged, Zen import 2.0 s; tests `reads_chromium_firefox_qutebrowser`, `import_merges_by_recency_and_is_idempotent` (32 green). Not run on the real profile (`~/.local/share/novabrowser`) — user's call.

## Decisions

Newest first. Settled — don't reopen without new evidence.

- 2026-10-11 — Closing tabs (Phase 4 bug, user found YouTube audio after close): for a page with (before)unload handlers, `close_browser(1)` runs them first and reaches `DoClose` later; if the removed BrowserView was already dropped the browser never finishes (no `OnBeforeClose`, page + audio keep running; with the window closed the process never exits). Fix: `window.rs` keeps removed tab views in `closing` until `do_close`, which posts `release_closing` to drop the view; window destroy moves detached tabs there too. Re-calling `close_browser` after DoClose does NOT finish it (tried).
- 2026-10-11 — Hints/find/yank shape (Phase 4): hint JS runs in a CDP isolated world `novabrowser` (page can't see it), labels in a closed shadow root; following = trusted `Input.dispatchMouseEvent` at the element centre after `scrollIntoView`; tab targets open the href. Find = CEF `BrowserHost::find` (smart case), incremental from the cmdline. Clipboard = `wl-copy`/`wl-paste` subprocesses; reads happen on a thread, then the command runs on the UI thread. `devtools::call` = `send_dev_tools_message` + one observer per browser, forgotten in `on_before_close`.

- 2026-10-11 — Passwords (user, supersedes the built-in manager below): integrate external managers instead of storing secrets. Main backend Bitwarden via `rbw` (logged in on this laptop); features: field detection, suggestion overlay, user-triggered fill, TOTP, generate, save/update prompt. Updates/saves go through an `$VISUAL` helper (rbw `add`/`edit` only take secrets via an editor), not `bw`. `pass`/KeePassXC via Phase 6 userscripts (qute-pass etc. unchanged).

- 2026-10-11 — Import (user): history via CLI `--import` with auto-detect, now; bookmarks imported when the bookmark store is built (Phase 8); passwords → a built-in password manager, added to the plan as Phase 10 (assistant recommended CSV → `pass` instead; risk: security-critical scope). Import merge = per URL newest time, newest non-empty title, max visits (idempotent), then renumber ids by recency.

- 2026-10-11 — History store (Phase 3): `history.sqlite` in the data dir, rusqlite (system sqlite 3.53, FTS5) on its own thread; UI thread only sends messages, results return by `post_task` (thread must start after CEF init, in `on_context_initialized`, else the first post is lost). Plain `LIKE` over 100k rows = ~25 ms on no-match → too slow. Instead: `id` order = recency (a visit deletes + re-inserts the row), FTS5 `trigram` external-content index joined and ordered by `rowid DESC` streams newest matches and stops at LIMIT (<2 ms). Words <3 chars can't use trigram: LIKE filter on top, and short-only queries scan the newest 20k rows only. Superseded queries are skipped via a latest-id atomic. Completion overlay = `add_overlay_view` CUSTOM, bounds = bottom half of the content panel, re-placed in the window's `on_layout_changed`.

- 2026-10-11 — CEF Views crash rules (Phase 2): (1) `request_focus` on a BrowserView that gets detached before CEF's async focus task runs → SIGSEGV in `RequestFocusSync`; focus only via a posted task that focuses the then-active, attached view (`window.rs focus_active`). (2) `LifeSpanHandler::do_close` must return 1: the default (0) makes CEF close the host window, and for a detached tab `CloseHostWindow` hits a CHECK (SIGTRAP); we finish the close by dropping the removed view.
- 2026-10-11 — Modal core shape: `bindings.rs` generated from qutebrowser `configdata.yml` (regenerate, don't hand-edit); keys decided on RAWKEYDOWN in `on_pre_key_event` → `modes::on_key`; unbound plain chars eaten in normal mode, other unbound keys pass (forward_unbound_keys=auto); auto insert mode from renderer `nova.focus` (active tab only); scroll = trusted `Input.dispatchMouseEvent` wheel via `execute_dev_tools_method` (40 px steps); search engine hard-coded DuckDuckGo until Phase 5. Keys port deviations from qutebrowser (worker): `<Ctrl-<` is a literal sequence, Shift dropped from printable keys, Ctrl+Shift+punct uses US base char (shortcut).

- 2026-10-11 — cef-rs gotcha: every default trait method returning `c_int` returns 0, even where CEF's C++ default is true. `WindowDelegate::can_close/can_resize/can_maximize/can_minimize` must be overridden to 1 (else Mod+Q does nothing). Check this for any new handler with a bool return.
- 2026-10-11 — Phase 1 shape: popups are allowed in `on_before_popup` and adopted as tabs in `on_popup_browser_view_created` (keeps `window.opener`); middle/ctrl-click comes via `on_open_urlfrom_tab` (return 1 + `post_task` open). UI ↔ core: UI pages are `data:` URLs from `ui/*.html` (`include_str!`); `window.nova` is installed only in `data:` frames and only `UiClient` handles `nova.send`; core → UI via `nova.apply(json)` with every string through `js_str` (\uXXXX-escapes all but alphanumerics/space). Profile = `$XDG_DATA_HOME/novabrowser` (root + cache path), log = `$XDG_STATE_HOME/novabrowser/log`. `--ozone-platform=wayland` added automatically when `WAYLAND_DISPLAY` is set.

- 2026-10-10 — Cert deny (S7b, user): keep CEF/Chrome's interstitial. Known risk: its Proceed link bypasses `on_certificate_error`, and it is Chrome UI.

- 2026-10-10 — S9 verdict: CEF (Chromium 154) starts faster than Chrome 155 (625 vs 920 ms) but scores 16% lower on Speedometer 3.1, uses ~12% more memory (windows vs windows) and janks on YouTube scroll. Not a stop: the Electron fallback runs the same engine, so it would not fix an engine-version/config gap. Cause isolated 2026-10-10 (`results/speedometer3.json`, `speedometer-cefcs.json`, `spikes/s9-baseline/s9-micro.mjs`): NOT version or field trials — Chrome-for-Testing 154.0.8037.92 with `--disable-variations-seed-fetch --disable-field-trial-config` = 16.9, Chrome 155 = 17.1, CEF 154 = 13.9 (n=3 each, all valid); NOT Alloy style (Chrome-style cefsimple 14.4/13.5). Micro-bench CEF vs CfT 154: pure JS equal (fib/alloc/string), DOM build +25%, layout +30–35% → slow part is Blink C++, not V8. Prebuilt `libcef.so` allocator shim dispatches only to `GlibcMalloc` (no PartitionAlloc-as-malloc symbols; imports `__libc_malloc`), whereas Chrome exports `malloc`/`free` (PartitionAlloc-Everywhere). LD_PRELOAD jemalloc/tcmalloc crashes CEF (`free(): invalid size`). Fix lives in the own CEF build (option A): `use_partition_alloc_as_malloc=true`, confirm PGO; verify with the same micro-bench + Speedometer. Expected to help memory too [Likely]. PGO status of the prebuilt is unverified.

- 2026-10-10 — Tests that copy to the system clipboard need one real input event first (Wayland `set_selection` needs an input serial); CDP-only input can't. Use compositor `send_shortcut` in automated tests. `<select>` popup options can't be clicked via CDP mouse (events go to the page) — use keys.

- 2026-10-10 — Prompts (S7): always answer permission callbacks (allow/deny/hold-then-answer); returning 0 makes prompt-type requests hang. Prompts queue one at a time per tab. Media grants aren't persisted by CEF → own per-origin store (Phase 7). Always implement JsDialogHandler (worker reported a Chrome-style alert when unhandled; screenshot didn't confirm — handle it anyway). Permission bit constants live in `sys`.

- 2026-10-10 — Renderer bridge (S8): one binary; pass the App to `execute_process` too (else renderer has no handler). `window.nova.send` via V8 handler → `frame.send_process_message`; insert mode from `on_focused_node_changed` + `node.is_editable()`. Treat new context/navigation as focus reset (unloading frame's messages are lost). Identify renderers by frame, not pid (sandbox pid ns). Bindings: V8 exception out-param needs `e.try_set(..)` (assigning is silently lost); enter `frame.v8_context()` by hand outside `on_context_created`; Views delegate macros need `struct X {}` not `struct X;`.

- 2026-10-10 — Key handling (S3): consume by returning 1 on RAWKEYDOWN in `on_pre_key_event` (CHAR/KEYUP then dropped). Don't expect a KEYUP per keydown. Mask `mods` bits 0x2000 (repeat), caps/num-lock. Ctrl+letter has `character`=control code → match on `windows_key_code`. `os_event` is null on Wayland. Don't trust `focus_on_editable_field` for insert mode (wrong on autofocus) → track focus from renderer (S8). `is_keyboard_shortcut=1` gives "page first, browser if unhandled" via `on_key_event`.

- 2026-10-10 — Dark mode design: start with `--enable-features=WebContentsForceDark`, per-site exclusion = `Emulation.setAutoDarkModeOverride {enabled:false}` on that tab. Override is per tab (survives reload/navigation, so re-apply per site on each navigation); over remote CDP it lasts only as long as its session, but via `execute_dev_tools_method` it persists (S5b). Pages can't detect it; verify with screenshots.

- 2026-10-10 — Views sizing: `ViewDelegate::preferred_size` must return non-zero width AND height (0 → `gfx::Size` empty → CEF uses the default, i.e. the whole window). Docked overlays (`add_overlay_view`) size from the contents view's preferred size; `set_size` is ignored. Window `child_view_count` includes the overlay host, which `child_view_at` returns as None.
- 2026-10-10 — Tabs: one BrowserView per tab, swapped in/out of a fill-layout content Panel + `was_hidden`; a detached tab's browser is created only when first attached and must be closed by hand on window destroy.

- 2026-10-10 — Codecs: option C (prebuilt CEF, no H.264/AAC); revisit A (own build on novahome) before Phase 8. PLAN.md §0.2. The S9 allocator finding adds a second reason for A: one own build for codecs + PartitionAlloc-as-malloc.
- 2026-10-10 — Parallel work via subagents per AGENTS.md › Subagents; only the main session writes PROGRESS.md/PLAN.md and commits.
- 2026-10-10 — Progress tracked in this file + git; project `AGENTS.md` holds the session rules.
- 2026-10-10 — qutebrowser is a UX/code reference only; not fixing or keeping it.
- 2026-10-10 — VPN-instance (`vpn-run`) setup is outdated; ignore it.
- 2026-10-10 — DRM/Widevine: not now; possible later after codecs.
- 2026-10-10 — Core in Rust on CEF (`cef` crate), Alloy style, config.ts, external-tool hooks instead of extensions. GPL-3.0-or-later.

## Session log

Newest first. One entry per session: date, what changed, evidence, what's next.

### 2026-10-11 (session 2, Phase 4)
- Phase 4 built and passed (see checklist); user manual test of 20 steps passed. Found and fixed the tab/window close leak for pages with unload handlers.
- Next: Phase 5 (config.ts).

### 2026-10-11 (session 2, import)
- Phase 10 rewritten: password manager integration (Bitwarden/rbw + editor helper) instead of a built-in store.
- Added `novabrowser --import` (history from Chromium/Firefox/qutebrowser profiles); PLAN.md: Phase 8 bookmark import, new Phase 10 password manager.
- Next: Phase 4.

### 2026-10-11 (session 2, Phase 3)
- Phase 3 built and passed (see checklist). Found: LIKE scan too slow at 100k → FTS5 trigram with recency ids; `post_task` before CEF init silently dropped.
- Next: Phase 4.

### 2026-10-11 (session 2, Phase 2)
- Phase 2 built and passed (see checklist). Keys/trie port by `worker` (spike crate removed after copy to `src/keys.rs`). Fixed two CEF crashes (async focus on detached view; `do_close` default). Phase 1's PROGRESS.md update missed commit e87778d (raced the commit) — included here.
- Next: Phase 3.

### 2026-10-11 (session 2, after midnight)
- S9 follow-up: Chrome 154 for Testing with field trials off = 16.9 = Chrome 155; CEF 13.9 → gap is libcef build (glibc malloc, not PartitionAlloc), not version/flags (commit 9ec5119).
- Phase 1 built and passed (see checklist). Found and fixed the cef-rs `can_close` default (Mod+Q did nothing).
- Next: Phase 2.

### 2026-10-10 (session 2)
- Phase 0 batch: S2, S3, S5, S5b, S6, S8 pass (S3, S5+S6, S8 by `worker` subagents, evidence re-checked here; S8 `drive.sh` re-run). S2 fixes: non-empty `preferred_size`, overlay sizing. S5b showed `execute_dev_tools_method` override persists.
- Added S3b (real keyboard, needs user). Found: `<iframe srcdoc>` in a `data:` page hangs that page (not our hooks) — note only.
- Subagents had no `process` tool; they used `timeout`/`systemd-run --user` wrappers instead.
- S7 pass (handlers fire, no Chrome UI); S7b cert-deny UX needs user decision. S1: `<select>` + clipboard pass automated; real-input checks left for user.
- S9 partial: startup CEF faster than Chrome; idle memory 12–32% above Chrome; Speedometer runs invalid (battery, low-power profile). Worker timed out at 30 min; orphan cefsimple killed.
- S9 run 2 (AC): Speedometer CEF 14.3 vs Chrome 16.9 (all valid; run-1 invalidity fixed by anti-throttling flags + focus), scrolling equal except YouTube. S9 closed with written verdict.
- Hands-on session: real copy/paste, primary selection, mouse `<select>`, Shift/Ctrl/repeat/Escape all pass. S7b: user chose Chrome interstitial. Phase 0 done except HiDPI, IME (no hardware/IME here) and S9b.
- Next: Phase 1.

### 2026-10-10
- Wrote PLAN.md (rev 1–3). Diagnosed qutebrowser slowness (ananicy nice 16 / idle I/O inherited from `ly-dm`).
- Cloned `cef-rs` to `../cef-rs` (commit 2861f69), `export-cef-dir` → `~/.local/share/cef` (CEF 154.0.34), built `cefsimple` (41 s).
- Spikes: S4 pass, S7 Google sign-in pass, S1 basic pass. Found no H.264/AAC in prebuilt CEF → decision C.
- Added PROGRESS.md, AGENTS.md, `spikes/cdp.mjs`; `git init`.
- Added Subagents section to AGENTS.md; `cdp.mjs` takes `CDP_PORT`.
- Next: parallel batches S2+S3+S8 and S5+S6 (AGENTS.md › Subagents).
