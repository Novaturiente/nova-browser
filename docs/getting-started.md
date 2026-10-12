# Getting started

## Install

On Arch Linux:

```sh
git clone https://github.com/Novaturiente/nova-browser
cd nova-browser/packaging && makepkg -si
xdg-settings set default-web-browser novabrowser.desktop   # optional
```

Other distributions: see [building](building.md).

## First start

Run `novabrowser` (or pick it from your app launcher). The first start opens
a welcome page with the basic keys and the import commands for the browsers
found on your machine. `:help` brings it back.

## Bring your data

Quit novabrowser first, then:

```sh
novabrowser --import            # list the browser profiles found, with sizes
novabrowser --import firefox    # history + bookmarks from the biggest Firefox profile
novabrowser --import /path/to/profile
```

Supported: Chrome, Chromium, Brave, Helium, Vivaldi, Edge, Firefox, Zen,
LibreWolf, Floorp and qutebrowser (history, bookmarks, quickmarks, and the
session as `:session-load qutebrowser`). Importing twice doesn't create
duplicates. Passwords are not imported: novabrowser uses your
[password manager](passwords.md) directly.

## Five minutes of keys

novabrowser starts in **normal mode**: letters are commands, not text.

1. `o`, type `wikipedia.org`, `Enter`. Typing something that isn't a URL
   searches for it.
2. `j` / `k` scroll, `Ctrl-d` / `Ctrl-u` half a page, `gg` / `G` top/bottom.
3. `f` shows labels on every link; type a label to click it. `F` opens it in
   a new tab.
4. `J` / `K` switch tabs, `d` closes one, `u` brings it back.
5. `H` / `L` go back / forward.
6. Click a search box (or press `gi`): you're in **insert mode** and can
   type. `Escape` goes back to normal mode.
7. `/word` finds text, `n` / `N` jump between matches.
8. `yy` copies the URL, `pp` opens the URL in your clipboard.
9. `:` opens the command line; `Tab` completes.
10. `F1` lists every key. After a prefix key like `g`, a popup shows what
    can follow.

The mouse works everywhere too.

## Make it yours

- `Ss` opens the settings page: theme, search engine, start page, tab bar
  position, dark mode, ad blocking, cookies, downloads, password manager.
- `:set <setting> <value>` changes any setting ([reference](settings.md)).
- `~/.config/novabrowser/config.ts` holds key bindings, per-site rules and
  anything you want to keep in git ([configuration](configuration.md)).

Turn on session restore, so your tabs come back after a restart:

```
:set auto_save.session true
```

## Next

- [Key bindings](keybindings.md) and [commands](commands.md)
- [Tabs, sessions and profiles](tabs-sessions-profiles.md)
- [Passwords](passwords.md), [downloads](downloads.md), [privacy](privacy.md)
- Coming from qutebrowser? Read [migrating](migrating-from-qutebrowser.md).
