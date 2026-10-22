# Tabs, windows, sessions and profiles

## Tabs

- `J` / `K` switch tabs, `Alt-1`…`Alt-8` jump to a tab, `Alt-9` to the last
  one, `Ctrl-Tab` to the one you used before.
- `d` closes, `u` reopens (the closed-tab list survives restarts, last 100).
- `gJ` / `gK` move a tab (or drag it in the tab bar; pinned tabs stay in front), `gC` duplicates it, `co` closes all others.
- Mouse: click a tab to switch, middle-click to close, hover for its full URL,
  right-click for a menu (reload, clone, copy URL, mute, bookmark, pin, move
  to new window, sleep, keep awake, close, close others).
- `tabs.position` puts the tab bar `top`, `bottom`, `left` or `right`
  (vertical). `tabs.show` hides it: `never`, or `multiple` (only with 2+ tabs).
- Tab titles are set with `tabs.title.format`, e.g. `"{audio}{index}: {current_title}"`.

The right end of the tab bar shows the mode, typed keys, messages and the
download status. A thin line under the tabs shows loading progress.

### Pinned tabs

`Ctrl-p` (`:tab-pin`) pins a tab: it moves to the left, shows a short title
(`tabs.title.format_pinned`) until you focus it, and `d`, `co` and middle-click won't close it.
`:tab-close --force` does. Pins are saved with the session.

### Tab hibernation

A tab you haven't looked at for `tabs.hibernate_after` minutes (default 30)
is unloaded to free memory. It stays in the tab bar, dimmed, and loads again
when you switch to it, at the same scroll position and with its back/forward
history.

A tab is never unloaded while it is active, loading or crashed. Just before
unloading, novabrowser also checks whether it is playing audio or video or
has text typed into a form field, and leaves it alone if so. Limits: media inside a frame (e.g. an embedded YouTube player)
and text typed into rich editors (`contenteditable`, like webmail compose)
aren't detected.

- `:tab-keep-awake` (tab menu: *keep*) exempts a tab permanently.
- `:tab-hibernate` (tab menu: *sleep*) unloads one now.
- Per site: `pattern("*://music.example.com/*", (p) => { p.tabs.hibernate_after = 0; })`.
- `c.tabs.hibernate_after = 0` turns it off.

Before that, after `tabs.freeze_after` minutes (default 5), a background tab
is frozen: its page stays loaded but runs no scripts, timers or network until
you switch to it. Skipped: pinned and kept-awake tabs, pages playing media or
with typed form text, and sites you allowed notifications (chat and mail keep
getting messages). `pattern("*://chat.example.com/*", (p) => { p.tabs.freeze_after = 0; })`
exempts a site; `c.tabs.freeze_after = 0` turns it off.

### Crashed and frozen tabs

If a tab's renderer crashes, the tab shows a crash page; press `r` to reload.
If a page stops responding for a while, the tab bar asks *"This page isn't
responding. Kill it?"* Press `y` to end it (then `r` reloads) or `n` to keep waiting.

## Windows

`Ctrl-n` or `:open -w url` opens a window, `gD` (`:tab-give`) moves a tab to
a new window. `:close` closes one window, `:quit` (`Ctrl-q`) all of them.
Starting `novabrowser` while it is already running opens a new window in the
running instance; `novabrowser <url>` opens the URL as a tab.

## Sessions

With `c.auto_save.session = true`, every window and tab is saved as you
browse (and on quit) and restored on the next start. Restored tabs load only
when you switch to them. Back/forward history (up to 50 entries each way)
and the scroll position are restored too.

Named sessions:

```
:session-save work          save everything as "work"
:session-load work          open it in addition to what's open
:session-load -c work       replace the current tabs with it
:session-delete work
:quit -s / :wq              save the default session and quit
```

Sessions are JSON files in `~/.local/share/novabrowser/sessions/`.
`novabrowser --import qutebrowser` turns qutebrowser's `default.yml` into a
session called `qutebrowser`.

## Profiles

A profile is a separate browser identity: its own history, cookies and
logins, site data, bookmarks, quickmarks, sessions, downloads and
permission answers. Use one for work and one for personal browsing, or one
per client.

```
:profile-new                 asks for a name, creates it, opens a window
:profile-new work            same, without asking
:profile-open work           open it
novabrowser --profile=work   start it from a shell or a desktop shortcut
novabrowser --profile work   same (also -P work)
novabrowser --list-profiles  list your profiles
```

A name that doesn't exist yet creates that profile. If the profile is
already running, the URL (or a new window) goes to that instance. For a
menu entry per profile, copy `novabrowser.desktop` to
`~/.local/share/applications/novabrowser-work.desktop` and change it to
`Name=novabrowser (work)` and `Exec=novabrowser --profile=work %U`.

Names may contain letters, digits, spaces, `-` and `_` (up to 40 characters). The profile's name appears at the start of the tab bar
and in the window title; the default profile shows nothing. Each profile
runs as its own process, so profiles never share cookies.

Settings:

- `config.ts` is shared by all profiles. Branch on the profile name if
  needed: `import { profile } from "nova"; if (profile === "work") { … }`.
- `:set` and `:settings` changes apply to the current profile only. A new
  profile starts from the default profile's choices.

Where profile data lives: `~/.local/share/novabrowser/profiles/<name>/`
(including its quickmarks and bookmarks) and
`~/.config/novabrowser/profiles/<name>/autoconfig.json`. To delete a profile,
quit it and remove both directories.
