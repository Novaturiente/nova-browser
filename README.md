# novabrowser

A keyboard-driven web browser for Linux: Vim-style modes, link hints, a
floating command palette, built-in ad blocking. Rust on top of CEF
(Chromium). Works with no config at all; everything can be changed later.

License: GPL-3.0-or-later.

## Install (Arch Linux)

```sh
git clone https://github.com/Novaturiente/nova-browser
cd nova-browser/packaging && makepkg -si
```

This builds `novabrowser-git`. The build downloads the matching CEF
(about 1.5 GB unpacked, so build on a real disk, not a small `/tmp`). It
installs into `/usr/lib/novabrowser` (about 360 MB) with `/usr/bin/novabrowser`,
a desktop entry and an icon.

To make it the default browser:

```sh
xdg-settings set default-web-browser novabrowser.desktop
```

## Build and run from source

Needs Rust, cmake and ninja. `cargo build` downloads the CEF version that
matches the `cef` crate and copies it next to the binary, so this is all:

```sh
cargo build --release
target/release/novabrowser [url]
tests/smoke.sh            # start, tabs over IPC, :set, quit (needs a desktop session)
```

To reuse an unpacked CEF instead, set `CEF_PATH` to it (the version must match).

Starting it again while it runs opens a new window (or the URL as a tab) in
the running instance.

## Keys

Press **F1** for every key binding (type to filter). After a prefix key such
as `g` or `;`, a popup at the bottom right shows what can follow.

| Key | Does |
|---|---|
| `o` / `O` | open URL or search (this tab / new tab) |
| `go` / `gO` | edit the current URL (this tab / new tab) |
| `f` / `F` | follow a link by hint (this tab / new tab) |
| `j` `k` `h` `l`, `gg` `G` | scroll; top / bottom |
| `Ctrl-d` / `Ctrl-u` | half page down / up |
| `H` / `L` | back / forward |
| `J` / `K` | next / previous tab |
| `d` / `u` | close tab / undo close |
| `/` `?` `n` `N` | find in page |
| `yy` / `pp` | copy URL / open the clipboard |
| `i`, `Escape` | insert mode (type into the page), back to normal |
| `Ctrl-n` | new window |
| `:` | command line (Tab completes) |

Mouse works too: click and middle-click tabs, hover a tab for its full URL,
right-click a tab for its menu.

## Commands worth knowing

- `:settings` — the common settings as a form (theme, search engine, start
  page, tab bar, dark mode, ad blocking, cookies, downloads…).
- `:bookmarks` — add, edit and delete quickmarks and bookmarks.
- `:downloads` — finished and running downloads.
- `:set <setting> <value>` — any setting; saved. `:set -t` only until restart.
- `:profile-new` — create a profile (asks for a name) and open it in its own
  window; `:profile-open <name>` opens one. Each profile has its own history,
  cookies, logins, bookmarks and sessions. `:set` and `:settings` changes apply
  to the current profile only (a new profile starts from the default profile's
  choices); config.ts is shared and can branch with
  `import { profile } from "nova"; if (profile === "work") …`. The name shows
  at the start of the tab bar.
- **Passwords** — no built-in store; uses your password manager's CLI, set by
  `passwords.backend`: `rbw` (Bitwarden, default), `bw` (official Bitwarden CLI,
  needs `BW_SESSION` in the browser's environment), `pass` (`<site>/<user>`
  entries, OTP via pass-otp) or `none`. Focusing a login, one-time-code or
  sign-up field lists matching logins (same registrable domain, like
  Bitwarden's default) under it: Down/Up pick, Enter fills, Escape hides.
  `<Space>pp` fills without focusing a field, `<Space>po` fills the TOTP code.
  Sign-up fields offer a generated password. After you submit a login form
  novabrowser asks to save it or update the stored password. Nothing is filled
  on page load, and frames from another site get nothing.
- `:dark-toggle` / `:https-toggle` (`<Space>td` / `<Space>th`) — per-site dark
  mode and HTTPS-only exceptions.
- `--import [browser]` — import history and bookmarks from Chrome, Chromium,
  Brave, Helium, Vivaldi, Edge, Firefox, Zen, LibreWolf, Floorp or qutebrowser
  (`novabrowser --import` lists what it finds).

## Configuration

Optional. `~/.config/novabrowser/config.ts` (TypeScript) is loaded at start
and by `:config-source`; values set there win over `:set` / `:settings`.

```ts
import { c, bind, pattern } from "nova";

c.theme = "catppuccin";              // dark, light, tokyo-night, catppuccin
c.url.searchengines = { DEFAULT: "https://duckduckgo.com/?q={}" };
c.tabs.position = "left";
bind("<Space>?", "bindings");
pattern("*://*.example.com/*", (p) => { p.colors.webpage.darkmode.enabled = false; });
```

Setting names follow qutebrowser's, so most qutebrowser configs port over.
`novabrowser --check-config` checks the file without starting the browser.

Other tools can drive the browser: `novabrowser --cmd ':open -t example.com'`,
`novabrowser --query state`, and `:spawn --userscript` with qutebrowser's
`QUTE_*` environment.

## Not supported

Chrome extensions, Widevine DRM (Netflix etc.), macOS/Windows. The prebuilt
CEF has no H.264/AAC, so some sites' video won't play until novabrowser ships
its own CEF build.
