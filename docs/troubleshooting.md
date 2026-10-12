# Troubleshooting

Logs: `~/.local/state/novabrowser/log` (the browser) and
`~/.local/state/novabrowser/aria2.log` (downloads via aria2). Starting
`novabrowser` from a terminal also prints errors.

## Typing does nothing / letters trigger commands

You're in normal mode. Click the field or press `i` (or `gi` for the first
field) to type; `Escape` returns to normal mode. The mode is shown at the
right end of the tab bar.

## A site doesn't work right

1. Turn off ad blocking for it:
   `:set -u '*://*.example.com/*' content.blocking.enabled false`, reload.
   If that fixes it, make it permanent with `pattern()` in config.ts.
2. If it needs third-party cookies (some logins): set
   `content.cookies.accept` to `all` for it.
3. If it only works over plain HTTP: `<Space>th`.
4. If forced dark mode makes it unreadable: `<Space>td`.

## Video doesn't play (Twitter/X, some news sites)

Those sites use H.264/AAC. The official prebuilt CEF (used on CPUs without
AVX2, or when you set `NOVA_CEF` to it) can't play them. The default
novabrowser CEF on AVX2 CPUs can. See [building](building.md).

## Paste doesn't work in a web terminal

Web terminals often turn `Ctrl-V` into a control character. Use
`Ctrl-Shift-V` (`:paste`), `Shift-Insert`, or right-click → Paste.

## A page asks to read the clipboard

That's the `content.javascript.clipboard = "ask"` default. Answer `A` to
always allow that site.

## Google shows a captcha / "unusual traffic"

Usually your IP address or network is flagged, not the browser: a fresh
Chrome on the same network gets the same page. It goes away by itself.

## "no running instance"

`--cmd` and `--query` need a running novabrowser with the same profile. Pass
`--profile=name` for named profiles.

## The browser won't start or exits at once

- Another novabrowser on the same profile is still shutting down: wait a
  second and try again.
- Run it from a terminal to see the error.
- Check the config: `novabrowser --check-config`. A broken `config.ts` at
  startup is reported, and the defaults are used.

## Password suggestions don't appear

- Check the backend works in a terminal: `rbw list`, `bw list items`, or
  `pass ls`.
- `bw` needs `BW_SESSION` in novabrowser's environment.
- Suggestions only show for logins whose saved URL shares the site's domain.

## A download goes through Chromium instead of aria2

Expected for form (POST) downloads, `blob:` and `data:` files, and when
`aria2c` isn't installed or `downloads.backend = "chromium"`. Restart after
installing aria2.

## Known limits

- Linux only. X11 support exists but is less tested than Wayland.
- HiDPI (scaling above 100%) and input methods (IME, compose) are untested.
- Hint labels don't appear inside frames from another site.
- No printing, private windows, Chrome extensions, or sync.

## Reporting a bug

Open an issue on GitHub with: what you did, what happened, your
distribution, Wayland or X11, the output of
`novabrowser --check-config`, and the relevant part of the log. Never paste
cookies, passwords or `aria2.session`.
