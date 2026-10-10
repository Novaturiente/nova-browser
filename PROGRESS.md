# novabrowser — progress

Single source of truth for where the work stands. Read at the start of every
session; update before ending one (rules in `AGENTS.md`).

## Now

- **Phase:** 0 (spikes)
- **Next action:** launch spike batches in parallel — S2 + S3 + S8 (Rust) and S5 + S6 (DevTools); see AGENTS.md › Subagents.
- **Blockers:** none

## Checklist

Status: `[ ]` todo · `[~]` in progress / partial · `[x]` done · `[-]` dropped.
Each done item links its evidence (file, commit, or session-log date).

### Phase 0 — spikes (PLAN.md §6)
- [~] S1 native Wayland — renders natively (2026-10-10). Left: HiDPI, clipboard, `<select>`.
- [x] S2 Views layout — tabbar + content panel + statusbar + overlay; tab swap keeps state (2026-10-10, `spikes/s2-layout`: bars 24 px, content flex 752 px, overlay 600×120 docked above statusbar and toggles; after swap 1→0 Wikipedia `scrollY` 1500 and JS var kept, hidden tab reports `visibilityState=hidden`).
- [x] S3 key interception — `on_pre_key_event` consumes keys in an Alloy BrowserView on Wayland (2026-10-10, `spikes/s3-keys/run.sh`, real Wayland keys via `wtype`: `j`/`k`/Esc consumed → page gets no keydown/keypress/keyup, input value `"abx"` from `ajkb<Esc>xq`; works with input or body focused; repeats consumed too).
- [ ] S3b needs user: real keyboard — Shift+letter chars, `e.code`/`native_key_code`, Escape-KEYUP asymmetry, key-repeat feel, IME/compose (no fcitx/ibus here).
- [x] S4 VA-API — hw decode H.264/VP8/VP9/AV1 listed, AV1 YouTube smooth (2026-10-10).
- [x] S5 per-site dark mode — `Emulation.setAutoDarkModeOverride` per tab (2026-10-10, `spikes/s5-s6/s5-darkmode.mjs`, two targets in one cefsimple: override on A → A luma 241→43, B stays 241; survives reload + navigation; with `--enable-features=WebContentsForceDark`, `{enabled:false}` lights only A).
- [x] S5b override lifetime via `execute_dev_tools_method` (2026-10-10, `spikes/s2-layout` title `nova:dark`: one call, no observer → content luma 249 → 37, still dark after 10 s, after navigation, after tab swap away/back; CEF's in-process session persists per browser).
- [x] S6 synthetic input — `Input.dispatchMouseEvent` / `insertText` (2026-10-10, `spikes/s5-s6/s6-input.mjs` over remote port: the-internet.herokuapp.com/login → "You logged into a secure area!", all events `isTrusted=true`). `insertText` sends no key events. Via `execute_dev_tools_method` not yet run (same CDP method; covered by S5b).
- [~] S7 Alloy style enough — Google sign-in passes (2026-10-10). Left: permission + certificate handlers fire.
- [ ] S8 renderer hooks — `on_context_created` + V8 fn + `on_focused_node_changed` round-trip.
- [ ] S9 baseline vs Chrome 155 — startup, idle RSS, Speedometer 3.1, scrolling.

### Phases 1–9 (PLAN.md §6; tick when the phase's exit check passes)
- [ ] 1 Skeleton
- [ ] 2 Modal core
- [ ] 3 Command line, completion, history
- [ ] 4 Hints and search
- [ ] 5 config.ts
- [ ] 6 External tools
- [ ] 7 Privacy, blocking, prompts
- [ ] 8 Sessions and daily driving (revisit codecs option A first)
- [ ] 9 Packaging and upkeep

## Decisions

Newest first. Settled — don't reopen without new evidence.

- 2026-10-10 — Key handling (S3): consume by returning 1 on RAWKEYDOWN in `on_pre_key_event` (CHAR/KEYUP then dropped). Don't expect a KEYUP per keydown. Mask `mods` bits 0x2000 (repeat), caps/num-lock. Ctrl+letter has `character`=control code → match on `windows_key_code`. `os_event` is null on Wayland. Don't trust `focus_on_editable_field` for insert mode (wrong on autofocus) → track focus from renderer (S8). `is_keyboard_shortcut=1` gives "page first, browser if unhandled" via `on_key_event`.

- 2026-10-10 — Dark mode design: start with `--enable-features=WebContentsForceDark`, per-site exclusion = `Emulation.setAutoDarkModeOverride {enabled:false}` on that tab. Override is per tab (survives reload/navigation, so re-apply per site on each navigation); over remote CDP it lasts only as long as its session, but via `execute_dev_tools_method` it persists (S5b). Pages can't detect it; verify with screenshots.

- 2026-10-10 — Views sizing: `ViewDelegate::preferred_size` must return non-zero width AND height (0 → `gfx::Size` empty → CEF uses the default, i.e. the whole window). Docked overlays (`add_overlay_view`) size from the contents view's preferred size; `set_size` is ignored. Window `child_view_count` includes the overlay host, which `child_view_at` returns as None.
- 2026-10-10 — Tabs: one BrowserView per tab, swapped in/out of a fill-layout content Panel + `was_hidden`; a detached tab's browser is created only when first attached and must be closed by hand on window destroy.

- 2026-10-10 — Codecs: option C (prebuilt CEF, no H.264/AAC); revisit A (own build on novahome) before Phase 8. PLAN.md §0.2.
- 2026-10-10 — Parallel work via subagents per AGENTS.md › Subagents; only the main session writes PROGRESS.md/PLAN.md and commits.
- 2026-10-10 — Progress tracked in this file + git; project `AGENTS.md` holds the session rules.
- 2026-10-10 — qutebrowser is a UX/code reference only; not fixing or keeping it.
- 2026-10-10 — VPN-instance (`vpn-run`) setup is outdated; ignore it.
- 2026-10-10 — DRM/Widevine: not now; possible later after codecs.
- 2026-10-10 — Core in Rust on CEF (`cef` crate), Alloy style, config.ts, external-tool hooks instead of extensions. GPL-3.0-or-later.

## Session log

Newest first. One entry per session: date, what changed, evidence, what's next.

### 2026-10-10
- Wrote PLAN.md (rev 1–3). Diagnosed qutebrowser slowness (ananicy nice 16 / idle I/O inherited from `ly-dm`).
- Cloned `cef-rs` to `../cef-rs` (commit 2861f69), `export-cef-dir` → `~/.local/share/cef` (CEF 154.0.34), built `cefsimple` (41 s).
- Spikes: S4 pass, S7 Google sign-in pass, S1 basic pass. Found no H.264/AAC in prebuilt CEF → decision C.
- Added PROGRESS.md, AGENTS.md, `spikes/cdp.mjs`; `git init`.
- Added Subagents section to AGENTS.md; `cdp.mjs` takes `CDP_PORT`.
- Next: parallel batches S2+S3+S8 and S5+S6 (AGENTS.md › Subagents).
