# novabrowser — agent instructions

Keyboard-driven browser: Rust core on CEF. Design lives in `PLAN.md`; state
lives in `PROGRESS.md`.

## Every session

1. **Start:** read `PROGRESS.md` (Now, Checklist, Decisions), then the
   `PLAN.md` sections for the next action. Don't re-derive anything listed
   under Decisions.
2. **During:** work on the item under "Now". New scope found → add a
   checklist item; don't absorb it silently.
3. **End (before the final message):** update `PROGRESS.md` —
   - tick/annotate checklist items, with evidence (file, commit, test output summary);
   - add new decisions (dated, newest first);
   - add a session-log entry (date IST, what changed, what's next);
   - set "Now" to the next concrete action.
   Then commit: `git add -A && git commit -m "<phase/spike>: <what>"`.
4. A checklist item is `[x]` only when its pass condition (PLAN.md §6) was
   checked this session. Partial → `[~]` with what's left.

## Layout

- `PLAN.md` — design, phases, spike pass conditions. Change only when a
  decision changes it; note the change in PROGRESS.md.
- `PROGRESS.md` — status, decisions, session log.
- `spikes/` — throwaway Phase 0 code, one dir per spike (`s2-layout/`, …).
- `spikes/cdp.mjs` — DevTools probe for a running CEF with
  `--remote-debugging-port=9333`: `node spikes/cdp.mjs '<js expr>'`,
  `node spikes/cdp.mjs @screenshot out.png`.
- `../cef-rs` — `cef` crate source (tauri-apps/cef-rs); `examples/cefsimple`
  is the starting point for spikes. `../qutebrowser` — read-only reference
  (GPL-3.0; porting code is fine, project is GPL-3.0-or-later).

## Build / run

```sh
export CEF_PATH=$HOME/.local/share/cef     # CEF 154.0.34, from export-cef-dir
export LD_LIBRARY_PATH=$CEF_PATH
cd ../cef-rs && cargo build --release -p cefsimple
target/release/cefsimple --use-alloy-style --ozone-platform=wayland \
  --remote-debugging-port=9333 --url=https://example.com
```

On a `cef` crate bump: re-run `export-cef-dir --force $CEF_PATH` (versions
must match exactly).

## Conventions

- Run GUI tests with the `process` tool (never `&`); stop them when done.
- Codecs: prebuilt CEF has no H.264/AAC — expected, not a bug (PLAN.md §0.2).
