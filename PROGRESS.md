# novabrowser — progress

Single source of truth for where the work stands. Read at the start of every
session; update before ending one (rules in `AGENTS.md`).

## Now

- **Phase:** 0 (spikes)
- **Next action:** S2 — Views layout spike in `spikes/s2-layout/`.
- **Blockers:** none

## Checklist

Status: `[ ]` todo · `[~]` in progress / partial · `[x]` done · `[-]` dropped.
Each done item links its evidence (file, commit, or session-log date).

### Phase 0 — spikes (PLAN.md §6)
- [~] S1 native Wayland — renders natively (2026-10-10). Left: HiDPI, clipboard, `<select>`.
- [ ] S2 Views layout — tabbar + content panel + statusbar + overlay; tab swap keeps state.
- [ ] S3 key interception — `on_pre_key_event` consumes keys in an Alloy BrowserView on Wayland.
- [x] S4 VA-API — hw decode H.264/VP8/VP9/AV1 listed, AV1 YouTube smooth (2026-10-10).
- [ ] S5 per-site dark mode — `Emulation.setAutoDarkModeOverride` per tab.
- [ ] S6 synthetic input — `Input.dispatchMouseEvent` / `insertText` via `execute_dev_tools_method`.
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

- 2026-10-10 — Codecs: option C (prebuilt CEF, no H.264/AAC); revisit A (own build on novahome) before Phase 8. PLAN.md §0.2.
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
- Next: S2.
