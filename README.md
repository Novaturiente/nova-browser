# novabrowser

A keyboard-driven web browser for Linux: Vim-style modes, link hints, a
floating command palette, built-in ad blocking. Rust on top of CEF
(Chromium). Works with no config at all; everything can be changed later.

License: GPL-3.0-or-later.

## Build and run

Needs Rust and the CEF binary distribution matching the `cef` crate
(currently CEF 154.0.34, unpacked with `export-cef-dir`).

```sh
export CEF_PATH=$HOME/.local/share/cef
cargo build --release
LD_LIBRARY_PATH=$CEF_PATH target/release/novabrowser [url]
```

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
  cookies, logins, bookmarks and sessions; the config is shared. The name shows
  at the start of the tab bar.
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
