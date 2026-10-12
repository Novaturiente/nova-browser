# Configuration

novabrowser works without any configuration. There are three ways to change
it, and you can mix them:

| Way | Good for | Stored in |
|---|---|---|
| `:settings` page (`Ss`) | the ~16 most common settings, by mouse or keyboard | `autoconfig.json` |
| `:set name value` | any setting, quickly | `autoconfig.json` |
| `config.ts` | everything: bindings, per-site rules, aliases, logic | your file |

If both set the same setting, **`config.ts` wins**. A `:set` on a setting
from `config.ts` lasts only until restart, and you get a warning saying so.

The full list of settings is in the [settings reference](settings.md).

## config.ts

`~/.config/novabrowser/config.ts` is TypeScript (types are stripped, then it
runs in a small JavaScript engine). It is loaded at start and again with
`:config-source`. If it has an error, the browser keeps the previous config
and shows `config unchanged: config.ts:<line>: <message>`.

Check it without starting the browser:

```sh
novabrowser --check-config
```

### A complete example

```ts
import { c, bind, unbind, alias, pattern, configDir, readLines, profile } from "nova";

// look
c.theme = "tokyo-night";                 // dark, light, tokyo-night, catppuccin
c.fonts.default_family = "JetBrains Mono";
c.fonts.default_size = "11pt";
c.tabs.position = "left";
c.tabs.width = "12%";

// start page and search
c.url.start_pages = ["https://start.duckduckgo.com"];
c.url.searchengines = {
  DEFAULT: "https://duckduckgo.com/?q={}",
  g: "https://www.google.com/search?q={}",
  gh: "https://github.com/search?q={}",
  aw: "https://wiki.archlinux.org/?search={}",
};

// behaviour
c.auto_save.session = true;
c.downloads.location.directory = "~/Downloads";
c.tabs.hibernate_after = 45;
c.content.cookies.accept = "no-3rdparty";
c.spellcheck.languages = ["en-US"];

// keys
bind("<Alt-l>", "tab-next");
bind("<Alt-h>", "tab-prev");
bind(";v", "hint links spawn mpv {hint-url}");
bind("<Space>t", "config-cycle tabs.position left top");
unbind("<Ctrl-q>");

// aliases: :mpv plays the current page
alias("mpv", "spawn mpv {url}");

// per site
pattern("*://*.youtube.com/*", (p) => {
  p.tabs.hibernate_after = 0;            // never unload YouTube tabs
});
pattern("*://mail.google.com/*", (p) => {
  p.content.notifications.enabled = true;
});

// a second profile with its own start page
if (profile === "work") {
  c.url.start_pages = ["https://intranet.example.com"];
}
```

### The `nova` module

| Export | What it does |
|---|---|
| `c` | All settings. Assign to set: `c.tabs.position = "left"`. Reading gives the current value; lists and dicts are copies, so change them by assigning again. |
| `bind(key, command, mode?)` | Add or replace a binding. `mode` is `"normal"` (default), `"insert"` or `"passthrough"`. |
| `unbind(key, mode?)` | Remove a binding, including a default one. |
| `alias(name, command)` | Add a command alias. |
| `pattern(urlPattern, fn)` | Settings for matching URLs only (see below). |
| `configDir` | The config directory path, e.g. for `readLines`. |
| `readLines(path)` | A text file as an array of trimmed, non-empty lines (`[]` if missing). |
| `profile` | Name of the running [profile](tabs-sessions-profiles.md#profiles), `"default"` for the unnamed one. |

You can split your config: `import { setup } from "./theme.ts";` works for
files next to `config.ts`.

Mistakes are reported with the line number: an unknown setting name, a wrong
type (`c.tabs.position = 3`), an invalid enum value, a malformed key or URL
pattern.

## Key bindings

```ts
bind("<Ctrl-Shift-Right>", "open -t {url}");
bind("gh", "open https://github.com");
bind("<Space>r", "config-source ;; message-info 'reloaded'");
unbind("<Ctrl-v>");
```

The [key bindings page](keybindings.md) lists the defaults and the key
notation. Commands are listed in [commands](commands.md).

## Per-site settings

`pattern()` takes a qutebrowser URL pattern:

| Pattern | Matches |
|---|---|
| `*://example.com/*` | example.com over any scheme |
| `*://*.example.com/*` | example.com and every subdomain |
| `https://example.com/app/*` | only paths under `/app/` |
| `*://*/*` | everything |

Inside the callback, set settings on its argument:

```ts
pattern("*://*.example.com/*", (p) => {
  p.content.blocking.enabled = false;
  p.colors.webpage.darkmode.enabled = false;
  p.content.cookies.accept = "all";
});
```

Only some settings can differ per site: forced dark mode, ad blocking,
cookies, custom headers, Do Not Track, tab hibernation and all permission
settings. The [settings reference](settings.md) marks them "Per site". The
last matching pattern wins.

`:set -u '*://example.com/*' content.blocking.enabled false` does the same
until restart.

### Dark-mode and HTTPS exceptions

`<Space>td` (`:dark-toggle`) and `<Space>th` (`:https-toggle`) flip a setting
for the current site and remember it in a plain text file, one host per line:

- `~/.config/novabrowser/https_excludes` — hosts (or `host:port`) that may use
  plain HTTP. Read automatically.
- `~/.config/novabrowser/darkmode_excludes` — hosts where forced dark mode is
  off. To keep these across restarts, read the file in `config.ts`:

```ts
c.colors.webpage.darkmode.enabled = true;
for (const host of readLines(`${configDir}/darkmode_excludes`)) {
  pattern(`*://${host}/*`, (p) => { p.colors.webpage.darkmode.enabled = false; });
}
```

`*.example.com` lines work too.

## Themes

`c.theme` (or the `:settings` page) picks a preset: `dark` (default),
`light`, `tokyo-night`, `catppuccin`. It colours the tab bar, palette, popups
and the built-in pages. Any `c.colors.*` you set overrides the preset, so a
preset plus a few tweaks works:

```ts
c.theme = "catppuccin";
c.colors.hints.bg = "#1a2a6c";
c.colors.hints.fg = "white";
```

Forced dark mode for *web pages* is separate: `colors.webpage.darkmode.enabled`.
It never applies to novabrowser's own pages.

## Files

Paths below are for the default profile. Named profiles keep their data in
`~/.local/share/novabrowser/profiles/<name>/` and their saved settings in
`~/.config/novabrowser/profiles/<name>/` (see [profiles](tabs-sessions-profiles.md#profiles)).
`$XDG_CONFIG_HOME`, `$XDG_DATA_HOME` and `$XDG_STATE_HOME` are respected.

**`~/.config/novabrowser/`** — yours to edit or back up:

| File | Contents |
|---|---|
| `config.ts` | Your configuration (optional). |
| `autoconfig.json` | Settings changed with `:set` / `:settings`. |
| `quickmarks` | `name url` per line (qutebrowser format). |
| `bookmarks/urls` | `url title` per line (qutebrowser format). |
| `https_excludes`, `darkmode_excludes` | Site exceptions, one host per line. |
| `userscripts/` | Your [userscripts](scripting.md#userscripts). |

**`~/.local/share/novabrowser/`** — browser data:

| File | Contents |
|---|---|
| `history.sqlite` | History and command history. |
| `sessions/<name>.json` | Saved sessions; `default.json` is the autosave. |
| `permissions.json` | Your Always/Never answers to permission prompts. |
| `downloads.json` | Download history (last 500). |
| `aria2.session` | Unfinished aria2 downloads. |
| `cookie_exceptions.json` | Cookie exceptions created by per-site `content.cookies.accept`. |
| `adblock/` | Downloaded filter lists and the compiled engine (shared by all profiles). |
| `Default/`, other Chromium files | Cookies, cache, site storage, zoom levels. |
| `ipc` | Control socket of the running instance. |

**`~/.local/state/novabrowser/`** — `log` (browser log) and `aria2.log`.

All files novabrowser writes itself are replaced atomically, so a crash never
leaves half a file.
