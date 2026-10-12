# Contributing

Bug reports, fixes and documentation improvements are welcome.

## Reporting bugs

Open a GitHub issue with what you did, what happened, your distribution,
Wayland or X11, and the relevant part of `~/.local/state/novabrowser/log`.
See [troubleshooting](docs/troubleshooting.md#reporting-a-bug). Security
problems: see [SECURITY.md](SECURITY.md).

## Development setup

```sh
cargo build --release                        # downloads the matching CEF on first build
cargo test --release
tests/smoke.sh target/release/novabrowser    # needs a desktop session
```

Use a throwaway profile while testing so your real one stays untouched:

```sh
export XDG_CONFIG_HOME=/tmp/nb/cfg XDG_DATA_HOME=/tmp/nb/data XDG_STATE_HOME=/tmp/nb/state
target/release/novabrowser --remote-debugging-port=9222
```

## Code layout

| Path | Contents |
|---|---|
| `src/main.rs` | startup, CLI flags, Chromium switches |
| `src/window.rs` | windows, tabs, tab bar, overlays |
| `src/client.rs` | CEF handlers (load, display, downloads, permissions…) |
| `src/modes.rs`, `src/keys.rs`, `src/bindings.rs` | modes, key parsing, default bindings |
| `src/commands.rs` | the command parser and every command |
| `src/config.rs`, `src/config.js`, `src/settings.rs` | config.ts loader and the settings schema |
| `src/renderer.rs`, `src/*.js` | code injected into pages (hints, passwords, scroll) |
| `src/adblock.rs`, `src/net.rs`, `src/permissions.rs` | blocking, cookies/HTTPS, prompts |
| `ui/*.html` | tab bar, palette, popups |
| `packaging/` | PKGBUILD, desktop file, CEF build patches |

## Guidelines

- Keep changes small and focused; one feature or fix per pull request.
- `cargo fmt` and `cargo clippy`; `cargo test` must pass.
- New setting: add it to `src/settings.rs` and to `docs/settings.md`.
  New command: add it to `NAMES` in `src/commands.rs` and to
  `docs/commands.md`. New default key: `src/bindings.rs` and
  `docs/keybindings.md`.
- Code injected into every page (`src/*.js` run by `renderer.rs`) must not
  add page-wide event listeners or force layout on load: an earlier version
  made Speedometer 40% slower that way. Benchmark such changes.
- Code ported from qutebrowser is fine (both are GPL-3.0-or-later); say so
  in a comment.

By contributing you agree that your contribution is licensed under
GPL-3.0-or-later.
