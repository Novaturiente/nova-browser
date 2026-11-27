# Changelog

## 0.1.22 (2026-11-27)

- Keyboard scrolling (`j`/`k`, `h`/`l`, `Ctrl-d`/`Ctrl-u`, `Ctrl-f`/`Ctrl-b`)
  is as smooth as the mouse wheel instead of jumping.

## 0.1.21 (2026-11-24)

- Pages with a "Sign in with Google" prompt (FedCM) no longer crash the
  whole browser. FedCM is off; the normal Google sign-in button still works.

## 0.1.20 (2026-11-22)

- Runs on our own CEF 158.0.3 build (Chromium 158), up from 154.
- Forced dark mode uses Dark Reader's colours: white backgrounds become
  `#181a1b`, black text becomes `#e8e6e3`, and colours keep their hue.
  Pages that are already dark stay as they are.
- `<Space>td` (`dark-toggle`) switches a site dark or light at once,
  without a reload.
- `colors.webpage.darkmode.algorithm` is ignored now: there is one
  algorithm. `colors.webpage.darkmode.policy.images = never` leaves all
  images alone.
- A tab shown again no longer stays blank when many pages are visible.
- Sites see "Google Chrome" in the browser brand list (`Sec-CH-UA`).
- Page animations follow your monitor's refresh rate instead of 60 fps.

## 0.1.19 (2026-11-21)

- Mobile view shows devices: `<Space>m` (`:mobile-toggle`) draws the page in
  an iPhone 16 and an iPad Air (landscape) frame side by side, at their real
  sizes, zoomed out to fit and centred. Each scrolls on its own; following a
  link or reloading in either updates both.
- Pick devices with `:mobile-toggle <device> [device]` (iPhone SE/16/16 Pro
  Max, Pixel 9, Galaxy S24, iPad mini/Air/Pro 13, Galaxy Tab S9); one device
  shows it alone.
- Drag a device's bottom-right corner to resize its width and height, like
  Chrome's responsive mode.

## 0.1.18 (2026-11-21)

- Mobile view per tab: `:mobile-toggle` gives the current tab a 390 px wide
  viewport, touch and an iPhone user agent, like Chrome's device mode. Other
  tabs and the window stay as they are. Run it again to turn it off.
- `content.user_agent` works in this edition: set a custom user agent per
  site with `pattern()` or `:set -u`. Sites see it in requests and in
  `navigator.userAgent`.
- DevTools (`F12`) open in their own working window again, instead of a
  blank tab. `F12` again closes them.

## 0.1.17 (2026-11-20)

- Touchpad scrolling coasts after your fingers lift, like Chrome and GTK apps,
  instead of stopping dead. Touching the touchpad again stops it.

## 0.1.16 (2026-11-20)

- Copy and paste work again with other apps. Text copied in a page (Ctrl+C,
  Ctrl+X, the right-click menu, a site's copy button) reaches the system
  clipboard, and Ctrl+V, Shift+Insert, the menu's Paste and `:paste` paste
  from it. Pasting is text only for now.

## 0.1.15 (2026-11-20)

- Tabs look like Safari again (as in 0.1.13): rounded squares, plain
  inactive tabs, the active one raised with a soft shadow. The tab bar is
  never thinner than 38 px.
- The URL box and its result cards are see-through: the blurred page shows
  behind them. Menus, tooltips, which-key, questions and messages are
  slightly see-through too.

## 0.1.14 (2026-11-20)

- New interface: the window, tab bar, URL box, search pill, which-key, tab
  menu, password list, questions and messages are native GTK, the same as
  novabrowser-webkit. Pages are drawn off-screen by Chromium and shown in
  the GTK window. The URL box floats over the page, which shows blurred
  behind it. No more popup flash.
- Right-click menus on pages are GTK menus.
- DevTools open in a tab next to the page; `devtools` again closes it.
- Needs gtk4 and libadwaita.
- Not yet: dragging files or links into pages, or out of them.

## 0.1.13 (2026-11-19)

- Popups rebuilt as solid cards: the URL box is a drop-down card under the
  tab bar, messages, questions, which-key and tab menus are cards too. No
  more white flash, no box round them and no blurred page behind the URL
  box (it showed old colours). The URL box takes keys at once, also when
  reopened quickly, and closes when you click the page.
- `dark-toggle` works in a fresh profile (it failed when the config folder
  did not exist yet).

## 0.1.12 (2026-11-19)

- The URL box takes keys again as soon as it opens, and Escape closes it.
  This undoes 0.1.11's popup change, so popups can flash again; a rebuilt
  popup layer follows.

## 0.1.11 (2026-11-19)

- Popups (URL bar, palette, toast, which-key, menus) no longer flash
  white or show the old page on their first show or after the page
  changed (for example the dark-mode toggle message).
- Small palette cards (e.g. two bookmarks) are only as tall as their rows.

## 0.1.10 (2026-11-19)

- Popups (toast, search pill, which-key, palette, menus) no longer flash
  white or show a dark box around them before their content appears.
- A tab restored with an empty URL opens `about:blank` instead of a dead
  tab that ignored every key.

## 0.1.9 (2026-11-19)

- New Safari-style look. The tab bar has rounded tabs with a close
  button on hover, a spinner and a progress line while loading, and
  tabs that grow in and shrink out. The command palette is a floating
  search field with one card per completion category and a sliding
  selection. Search (`/`, `?`) is a pill that shows the match count.
  Messages appear as a toast at the top, and questions (permissions,
  dialogs, saving a password) as a sheet with buttons you can click.
  Small popups are frosted glass over the page and fade in and out.
- Find shows the match position (`3 / 17`).
- Tabs opened from a page open next to it; closing the active tab
  focuses the tab to its left.
- Hints: `F` on an element inside a link opens that link; other
  elements get a Ctrl-click.
- The mouse back and forward buttons navigate.
- aria2 downloads that fail before the first byte are retried by
  Chromium.
- The saved-password list hides when you switch tabs.
- `dark-toggle` with dark mode off by default saves the site in
  `darkmode_sites` (read it in `config.ts` to keep it after a restart).
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
