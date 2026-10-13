# Commands

Run commands from the command line (`:`), from key bindings, from
`novabrowser --cmd '…'`, or from a userscript's `$QUTE_FIFO`.

```
:command [flags] [args]          one command
:cmd1 ;; cmd2                    chain several
5:scroll down                    a count (also from a key: 5j)
```

- Quote arguments with spaces: `:message-info 'hello world'`. A backslash
  escapes the next character.
- Placeholders are replaced before running: `{url}`, `{url:host}`,
  `{url:pretty}`, `{title}`, `{clipboard}`, `{primary}` (primary selection).
- Aliases from the `aliases` setting expand first. Defaults: `:w` =
  `session-save`, `:q` = `close`, `:qa` = `quit`, `:wq` / `:wqa` = `quit --save`.
- `Tab` completes command names, then URLs (history, bookmarks, quickmarks,
  search engines) for `:open`.

Arguments in `[brackets]` are optional. "Tab index" arguments accept a count
or a 1-based number; without one they act on the current tab.

## Opening pages

| Command | Description |
|---|---|
| `open [-t\|-b\|-w] [url or search]` | Open in this tab, a new tab (`-t`), a background tab (`-b`) or a new window (`-w`). Text that isn't a URL is searched with `url.searchengines`; a first word matching an engine keyword picks that engine. No argument opens `url.default_page`. |
| `back`, `forward` | History navigation (count = steps). Works across restarts for restored tabs. |
| `reload [-f]` | Reload; `-f` bypasses the cache. On a crashed tab, reloads it. |
| `stop` | Stop loading. |
| `history [words]` | History page: search, browse by day, delete entries. |
| `help` | The welcome page with the basics. |

## Tabs and windows

| Command | Description |
|---|---|
| `tab-next`, `tab-prev` | Switch tabs (count = how many). |
| `tab-focus [index\|last]` | Go to tab *index* (negative counts from the end), or the previous tab. |
| `tab-move [+\|-\|index]` | Move the tab right/left, or to a position. |
| `tab-close [-f]` | Close the tab. Pinned tabs need `-f`/`--force`. |
| `tab-only` | Close all other tabs except pinned ones. |
| `tab-clone [-b]` | Duplicate the tab (`-b`: in the background). |
| `tab-mute` | Toggle mute. |
| `tab-pin [index]` | Pin / unpin. Pinned tabs stay at the left and `d` won't close them. |
| `tab-give [index]` | Move the tab to a new window (the page reloads). |
| `tab-hibernate [index]` | Unload a background tab now. |
| `tab-keep-awake [index]` | Toggle the never-hibernate flag (saved with the session). |
| `undo` | Reopen the last closed tab (kept across restarts). |
| `close [-s]` | Close this window (`-s`: save the session first). |
| `quit [-s]` | Close every window and exit (`-s`: save the session first). |
| `fullscreen [--leave]` | Toggle window fullscreen; `--leave` only leaves page fullscreen. |
| `devtools` | Toggle Chromium DevTools for this tab. |

## Scrolling and zoom

| Command | Description |
|---|---|
| `scroll up\|down\|left\|right\|top\|bottom` | Scroll one step (40 px × count). |
| `scroll-page x y` | Scroll by a fraction of the page: `scroll-page 0 0.5` = half a page down. |
| `scroll-to-perc [-x] [percent]` | Jump to a percentage (no argument = bottom; `-x` = horizontal). |
| `zoom-in`, `zoom-out` | Next / previous step from `zoom.levels`. Zoom is remembered per site. |
| `zoom [percent]` | Set zoom (default 100%). |

## Finding, hints and copying

| Command | Description |
|---|---|
| `search [-r] text` | Find in page (`-r` backwards). `search` alone clears the highlight. Typing `/text` does the same. |
| `search-next`, `search-prev` | Next / previous match. |
| `hint [--rapid] [--first] [group] [target] [args]` | Label elements and act on one. See [hints](keybindings.md#hints). |
| `yank [url\|title\|domain\|pretty-url\|selection\|inline TEXT] [-s]` | Copy to the clipboard (`-s`: primary selection). `inline` copies TEXT after placeholders are replaced. |
| `paste` | Paste the clipboard into the focused field (like right-click → Paste). |

## Modes and the command line

| Command | Description |
|---|---|
| `mode-enter insert\|passthrough\|normal` | Switch mode. |
| `mode-leave` | Back to normal mode. |
| `cmd-set-text [-s] text` | Open the command line with *text* (`-s` adds a trailing space). |
| `clear-keychain` | Forget partially typed keys. |
| `bindings [filter]` | List the active key bindings (same as F1). |
| `message-info\|message-warning\|message-error text` | Show a message in the tab bar. |
| `nop` | Do nothing (useful to disable a key). |

## Bookmarks and quickmarks

Quickmarks are named URLs (`b name`); bookmarks are URLs with titles. Both are
plain text files in qutebrowser's format (see [configuration](configuration.md#files)).

| Command | Description |
|---|---|
| `bookmarks` | Manager page: add, edit, delete, filter, open. |
| `quickmark-save` | Save this page: asks for a name. |
| `quickmark-add url name` | Add a quickmark. |
| `quickmark-load [-t\|-b\|-w] name` | Open a quickmark. |
| `quickmark-del [name]` | Delete a quickmark (default: the one for this page). |
| `bookmark-add [url [title]] [--toggle]` | Bookmark this page or *url*; `--toggle` removes it if it exists. |
| `bookmark-load [-t\|-b\|-w] url` | Open a bookmark. |
| `bookmark-del [url]` | Delete a bookmark (default: this page). |
| `bookmark-list [-j]` | Same as `bookmarks` (`-j` opens it in this tab). |

## Downloads

| Command | Description |
|---|---|
| `downloads` | Downloads page (running and finished). |
| `download [url\|magnet:…]` | Download *url* or the current page. Magnet links need aria2. |
| `download-open [N] [program args…]` | Open finished download N (newest = 1, default newest) with `downloads.open_dispatcher` or the given program; `{}` stands for the file. |
| `download-cancel [N]` | Cancel download N, or the newest running one. |
| `download-clear` | Forget finished downloads (files stay). |

See [downloads](downloads.md).

## Sessions and profiles

| Command | Description |
|---|---|
| `session-save [name]` | Save all windows and tabs. |
| `session-load [-c] name` | Open a saved session (`-c`: close the current tabs first). |
| `session-delete name` | Delete a saved session. |
| `profile-new [name]` | Create a profile and open it in a new window (asks for a name if none given). |
| `profile-open [name]` | Open a profile (a new window, or a new instance if it isn't running). With no name, lists your profiles to pick from. |

See [tabs, sessions and profiles](tabs-sessions-profiles.md).

## Settings and site toggles

| Command | Description |
|---|---|
| `settings` | Settings page for the common settings. |
| `set name` | Show a setting's value. |
| `set [-t] name value` | Change a setting; saved unless `-t`/`--temp`. Lists and dicts as JSON. |
| `set -u pattern name value` | Change a setting for matching URLs until restart. |
| `config-cycle [-t] name v1 v2 …` | Switch to the value after the current one. |
| `config-source` | Reload `config.ts`. On an error the old config stays and the error is shown. |
| `dark-toggle` | Toggle forced dark mode for this site (saved in `darkmode_excludes`). |
| `https-toggle` | Allow / stop allowing plain HTTP for this site (saved in `https_excludes`). |
| `adblock-update` | Download the filter lists now. |

## Passwords

| Command | Description |
|---|---|
| `password-fill [-o]` | Fill the best matching login on this page; `-o`/`--otp` fills the TOTP code. |

See [passwords](passwords.md).

## Scripting

| Command | Description |
|---|---|
| `spawn [-u] [-o] program [args]` | Start a program (never blocks the browser). `-u`: a userscript from `userscripts/`; `-o`: show its output in a new tab. |
| `fake-key keys` | Send keys to the page, in key notation: `fake-key <Ctrl-a>`, `fake-key hello`. Use `\<` for a literal `<`. |
| `insert-text text` | Type text into the focused field. |

See [scripting](scripting.md).
