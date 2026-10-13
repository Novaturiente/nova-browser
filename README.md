<p align="center"><img src="packaging/novabrowser.svg" width="96" alt=""></p>

<h1 align="center">novabrowser</h1>

<p align="center">
A keyboard-driven web browser for Linux, in the spirit of qutebrowser,<br>
built on current Chromium (CEF) with a Rust core.
</p>

---

novabrowser browses the way qutebrowser does: Vim-style modes, link hints,
a command line and a plain-text config. It runs on Chromium 154 through CEF,
so sites behave as they do in Chrome, the default build plays H.264/AAC
video, and Widevine (DRM) works. qutebrowser's QtWebEngine is also Chromium,
but usually several versions older.

The browser itself is one Rust binary with no Python or Qt. Tabs, the GPU
process and the browser run as separate processes, and a crashed tab shows
a reload page while the rest keeps running.

It needs no configuration. qutebrowser users will recognise the keys,
commands and setting names, and anyone who prefers the mouse gets tab menus
and a settings page.

License: GPL-3.0-or-later. Platform: Linux (Wayland and X11), x86-64.

## Features

### Browsing

- Normal, insert, passthrough, command and hint modes; counts (`5j`, `3J`);
  `;;` command chaining; aliases.
- Link hints (`f`, `F`, `;y`, `gi`, …) with groups and targets, e.g.
  `:hint links spawn mpv {hint-url}`.
- A floating palette for `o`, `:` and `/`: URL, history, bookmark,
  quickmark and search-engine completion; readline keys; command history.
- Find in page, zoom remembered per site, fullscreen, DevTools.
- A key popup after prefix keys, and `F1` for every binding.

### Tabs and sessions

- Horizontal or vertical tab bar, pinned tabs, move a tab to a new window,
  undo close (across restarts).
- Session restore with back/forward history and scroll position; named
  sessions.
- Idle tab hibernation to save memory, with automatic exemptions for media
  and typed forms.
- Multiple windows and fully separate profiles (work / personal).

### Privacy and content

- Ad and tracker blocking with Brave's engine: network rules, cosmetic
  filtering and uBlock scriptlets, which remove YouTube ads. Lists update
  in the background.
- HTTPS-only by default, third-party cookie blocking, permission prompts
  with per-site memory, Do Not Track.
- Chromium's background calls to Google removed or turned off
  ([details](docs/privacy.md#calls-to-google)).
- Per-site forced dark mode.

### Integration

- Passwords from your existing manager: Bitwarden (`rbw` or `bw`) or `pass`.
  Suggestions appear under login fields, plus TOTP codes, generated
  passwords and a save/update prompt. No built-in vault.
- Downloads without dialogs; optional aria2 backend for 8-connection
  downloads, resume after restart, magnet links and torrents.
- Import history and bookmarks from Chrome, Firefox, Brave, Zen, qutebrowser
  and others.
- Control from outside (`novabrowser --cmd`, `--query`), qutebrowser-style
  userscripts with `QUTE_*` variables and `$QUTE_FIFO`.

### Configuration

- A `:settings` page, `:set` (saved), and an optional
  `~/.config/novabrowser/config.ts` with `bind()`, `pattern()` per-site
  rules, aliases and full TypeScript.
- Theme presets: dark, light, Tokyo Night, Catppuccin; every UI colour and
  font can be changed.

## Install

### Arch Linux (AUR)

Needs a CPU with AVX2 (x86-64-v3: Intel Haswell 2013+, AMD Excavator 2015+).

```sh
yay -S novabrowser-bin    # prebuilt
yay -S novabrowser        # or build from source
```

Both use novabrowser's own CEF build with H.264/AAC, PartitionAlloc and the
privacy patches, compiled for AVX2. Use `paru` or any other AUR helper the
same way.

### Other distributions

The prebuilt binary needs a very new glibc, so build from source. You need
Rust (install it with [rustup](https://rustup.rs); distro versions are often
too old), a C compiler, `cmake`, `ninja`, `pkg-config` and SQLite.

Debian / Ubuntu:

```sh
sudo apt install build-essential cmake ninja-build pkg-config libsqlite3-dev \
  libgtk-3-0 libnss3 libasound2 libxss1 libxkbcommon0 libdrm2 libgbm1 libcups2 libatspi2.0-0
```

Fedora:

```sh
sudo dnf install gcc cmake ninja-build pkgconf sqlite-devel \
  gtk3 nss alsa-lib libXScrnSaver libxkbcommon libdrm mesa-libgbm cups-libs at-spi2-core
```

Then build. On an AVX2 CPU, use novabrowser's CEF (H.264/AAC, privacy patches):

```sh
git clone https://github.com/Novaturiente/nova-browser && cd nova-browser
curl -L https://github.com/Novaturiente/nova-browser/releases/download/cef-154.0.34-nova1/cef-154.0.34-novabrowser-x86_64-v3.tar.zst \
  | tar --zstd -x               # unpacks ./cef-own
CEF_PATH=$PWD/cef-own cargo build --release
```

On an older CPU, run plain `cargo build --release` instead. It downloads the
official CEF (about 1.5 GB, no H.264/AAC).

Install it system-wide:

```sh
r=target/release
sudo install -d /opt/novabrowser
sudo cp -r $r/novabrowser $r/libcef.so $r/libvk_swiftshader.so $r/libvulkan.so.1 \
  $r/vk_swiftshader_icd.json $r/icudtl.dat $r/v8_context_snapshot.bin $r/*.pak $r/locales \
  /opt/novabrowser/
sudo ln -sf /opt/novabrowser/novabrowser /usr/local/bin/novabrowser
sudo install -Dm644 packaging/novabrowser.desktop /usr/local/share/applications/novabrowser.desktop
sudo install -Dm644 packaging/novabrowser.svg /usr/local/share/icons/hicolor/scalable/apps/novabrowser.svg
```

Or skip installing and run `target/release/novabrowser` straight from the
build folder. More detail, including tests, is in
[building](docs/building.md#from-source-any-distribution).

### After installing

Make it the default browser (optional):

```sh
xdg-settings set default-web-browser novabrowser.desktop
```

Optional extras: `aria2` (faster downloads, torrents), `rbw` or `pass`
(passwords), `wl-clipboard` / `xclip` (primary selection).

## Quick start

| Key | Action |
|---|---|
| `o` / `O` | Open a URL or search, in this tab / a new tab |
| `f` / `F` | Click a link by its label / open it in a new tab |
| `j` `k` `gg` `G` | Scroll down, up, to the top, to the bottom |
| `H` / `L` | Back / forward |
| `J` / `K`, `d` / `u` | Next / previous tab, close / reopen tab |
| `/` `n` `N` | Find in page |
| `yy` / `pp` | Copy URL / open the clipboard |
| `i`, `Escape` | Type into the page, back to commands |
| `:` | Command line (`Tab` completes) |
| `Ss`, `Sb`, `Sd`, `Sh` | Settings, bookmarks, downloads, history |
| `F1` | Every key binding |

Bring your data along (with the browser closed):

```sh
novabrowser --import            # list what was found
novabrowser --import firefox    # or chrome, brave, zen, qutebrowser, …
```

A minimal `~/.config/novabrowser/config.ts`:

```ts
import { c, bind, pattern } from "nova";

c.theme = "tokyo-night";
c.tabs.position = "left";
c.auto_save.session = true;
c.url.searchengines = { DEFAULT: "https://duckduckgo.com/?q={}", gh: "https://github.com/search?q={}" };
bind("<Alt-l>", "tab-next");
pattern("*://*.example.com/*", (p) => { p.content.blocking.enabled = false; });
```

## Documentation

| Guide | |
|---|---|
| [Getting started](docs/getting-started.md) | Install, first start, importing, the first five minutes |
| [Key bindings](docs/keybindings.md) | Modes, every default key, hints, key notation, rebinding |
| [Commands](docs/commands.md) | Every command and its flags |
| [Configuration](docs/configuration.md) | `config.ts`, `:set`, per-site rules, themes, files |
| [Settings reference](docs/settings.md) | Every setting with its type and default |
| [Tabs, sessions and profiles](docs/tabs-sessions-profiles.md) | Pinning, hibernation, sessions, windows, profiles |
| [Passwords](docs/passwords.md) | Bitwarden / pass integration |
| [Downloads](docs/downloads.md) | Downloads page, aria2, torrents |
| [Privacy](docs/privacy.md) | Ad blocking, cookies, HTTPS-only, permissions, Google calls |
| [Scripting](docs/scripting.md) | `--cmd`, userscripts, spawn, fake-key |
| [Migrating from qutebrowser](docs/migrating-from-qutebrowser.md) | What carries over, porting `config.py` |
| [Building](docs/building.md) | Packaging, building from source, the custom CEF |
| [Troubleshooting](docs/troubleshooting.md) | Common problems, known limits, bug reports |

## Not supported

Chrome extensions, sync, printing, private windows, macOS and Windows. Some
qutebrowser features are missing too: caret mode, marks, macros and
Greasemonkey scripts.

## Contributing and license

See [CONTRIBUTING.md](CONTRIBUTING.md) and [SECURITY.md](SECURITY.md).
novabrowser is free software under the
[GNU General Public License v3.0 or later](LICENSE). It uses ideas and some
ported code from [qutebrowser](https://qutebrowser.org) (GPL-3.0-or-later).
CEF and Chromium are under their own BSD-style licenses; see
`CEF-CREDITS.html` in the package.
