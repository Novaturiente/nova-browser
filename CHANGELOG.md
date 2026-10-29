# Changelog

## Unreleased

- Downloads through aria2: the RPC secret is now passed in a private
  (0600) config file instead of on aria2c's command line, where other
  local users could read it.
- Most of the browser logic now lives in the shared `nova-core` crate,
  also used by the WebKit edition. No change in behaviour expected.
- `:set` refuses settings that only apply to the WebKit edition
  (`webkit.*`).

## 0.1.8 (2026-10-27)

- Submitting a payment card form (for example "Update payment method" on
  claude.ai) no longer crashes the browser. Chromium's autofill tried to
  show its "Save card?" bubble, which needs Chrome UI that novabrowser
  doesn't have. Autofill for cards and addresses is now off.

## 0.1.7 (2026-10-22)

- Drag a tab in the tab bar to move it.
- A focused pinned tab shows its full title; others keep the short
  `tabs.title.format_pinned`.
- Moving tabs (`gJ`/`gK`, `:tab-move`, dragging) keeps pinned tabs in front
  of the others.

## 0.1.6 (2026-10-21)

- Hint labels keep their configured colours with
  `colors.webpage.darkmode` on: force-dark recoloured them on some
  pages only (white text on yellow or dark).

## 0.1.5 (2026-10-21)

- Hints work in modal dialogs (ChatGPT's login box): labels are drawn
  above the dialog, and only the dialog's own elements get labels while
  it is open (the page behind can't be clicked).
- Permission prompts (microphone, camera, …) and messages no longer
  vanish when many tabs are open: the tabs used to push the message
  area off the right edge of the bar.

## 0.1.4 (2026-10-20)

- "Sign in with Google" (and other logins that hop across sites) works
  with `content.cookies.accept = "no-3rdparty"`: a page you navigate to
  was judged a third party of the page you came from and lost its
  cookies (ChatGPT showed "Something went wrong").

## 0.1.3 (2026-10-20)

- `--profile work` and `-P work` start a profile (as `--profile=work`
  did); `--list-profiles` lists them.
- Address-box suggestions: bookmarks first, then pages you opened most
  from the address box, then the most visited, then the most recent.
  Bookmarked pages are no longer listed twice. `--import` brings typed
  counts from Chromium and Firefox.
- The first page of a new tab is now recorded in history when its URL
  was exactly the one asked for (it was skipped).
- Up/Down move through the completion list when one is shown; Ctrl-p /
  Ctrl-n always browse command history.
- Background tabs freeze after `tabs.freeze_after` minutes (default 5;
  not pinned, kept-awake, playing, typed-in or notification-allowed tabs).
- Passwords: nothing is filled or offered on `http://` pages; stricter
  login-field detection; `Sp` fills the focused field (replaces
  `<Space>pp`).
- HTTPS-only: a failed upgrade shows a hint naming `:https-toggle`.
- A second launch hands its URL over as a URL only (never run as
  commands).
- Data and state directories are private (0700), files we write 0600.
- aria2 downloads send the Referer the way Chromium does; save/update
  password prompts expire after 5 minutes; clipboard reads are capped;
  renderer processes limited to 12.

## 0.1.2 (2026-10-14)

- Tabs show the site's icon; icons are saved with the session, so restored
  tabs show them before they load.
- Default search engine, start page and new-tab page are now Google
  (were DuckDuckGo).

## 0.1.1 (2026-10-14)

- `:profile-open` lists your profiles as soon as you type it; no space
  or Enter needed.
- README: demo GIF and screenshot; install steps for other distros.

## 0.1.0 (2026-10-13)

First public version.

- Modal browsing: normal, insert, passthrough, command, hint and prompt
  modes; counts; command chaining and aliases; qutebrowser key notation.
- Link hints with groups and targets; find in page; yank and paste.
- Floating command palette with history, bookmark, quickmark and
  search-engine completion; key popup after prefix keys; `F1` binding list.
- Tabs: horizontal or vertical, pinning, move to new window, undo close,
  idle hibernation; multiple windows; separate profiles.
- Session restore including back/forward history and scroll position.
- config.ts (TypeScript) configuration, `:set` saved to autoconfig.json,
  `:settings` page, theme presets.
- Ad blocking (network, cosmetic, scriptlets), HTTPS-only, cookie policy,
  permission prompts with per-site memory.
- Password integration with rbw, bw and pass.
- Downloads page; aria2 backend with resume and torrents.
- History, bookmarks and session import from Chromium-based browsers,
  Firefox-based browsers and qutebrowser.
- Control socket (`--cmd`, `--query`) and qutebrowser-style userscripts.
- Own CEF 154 build with H.264/AAC, PartitionAlloc and reduced Google
  background calls; Arch Linux PKGBUILD.
