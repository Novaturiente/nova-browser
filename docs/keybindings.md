# Key bindings

novabrowser is modal, like Vim and qutebrowser: in normal mode keys run
commands, in insert mode they type into the page. Press F1 at any
time for a filterable list of every binding that is active right now,
including yours.

After a prefix key such as `g`, `;` or `<Space>`, a popup in the bottom-right
corner shows which keys can follow.

## Modes

| Mode | How you get there | Keys do | Leave with |
|---|---|---|---|
| Normal | default | run commands | |
| Insert | `i`, or focusing a text field | type into the page | `Escape` (or clicking outside the field) |
| Passthrough | `Ctrl-v` | everything goes to the page, even `Escape` | `Shift-Escape` |
| Command | `:`, `o`, `O`, `/`, `?` | edit the command line | `Escape`, `Ctrl-[`, or Backspace on an empty line |
| Hint | `f`, `F`, `;…`, `gi` | type a label to pick an element | `Escape` |
| Prompt | a site asks for a permission or shows a dialog | answer the question in the tab bar | `Escape` (= no) |

Insert mode starts by itself when a text field gets focus and ends when focus
leaves it (`input.insert_mode.auto_enter` / `auto_leave`). Unbound keys in
normal mode: plain letters are swallowed so they never reach the page by
accident; keys with modifiers (like `Ctrl-c`) and special keys go to the page.

## Counts

Type a number before a key to repeat it or give it an argument: `5j` scrolls
five steps, `3J` goes three tabs right, `50G` jumps to 50% of the page, `2d`…
does what you'd expect, `150=` sets zoom to 150%, `3<Ctrl-p>` pins tab 3.

## Default bindings

These are the built-in bindings (`bindings.default`). Each action has exactly
one key. Change any of them with [`bind()`](configuration.md#key-bindings).

### Open / edit address

| Key | Action | Command |
|---|---|---|
| `o` | Open a URL or search in this tab (empty address box) | `cmd-set-text -s :open` |
| `O` | Same, in a new tab | `cmd-set-text -s :open -t` |
| `go` | Edit the current URL, open in this tab | `cmd-set-text :open {url}` |
| `gO` | Edit the current URL, open in a new tab | `cmd-set-text :open -t {url}` |
| `:` | Open the command line | `cmd-set-text :` |
| `pp` | Open the URL (or search) in the clipboard | `open -- {clipboard}` |
| `Pp` | Same, in a new tab | `open -t -- {clipboard}` |

### Page navigation

| Key | Action | Command |
|---|---|---|
| `H` | Back | `back` |
| `L` | Forward | `forward` |
| `r` | Reload | `reload` |
| `R` | Reload, bypassing the cache | `reload -f` |
| `<Ctrl-s>` | Stop loading | `stop` |

### Scrolling

| Key | Action | Command |
|---|---|---|
| `h` | Scroll left | `scroll left` |
| `j` | Scroll down | `scroll down` |
| `k` | Scroll up | `scroll up` |
| `l` | Scroll right | `scroll right` |
| `gg` | Top of the page (with a count: that percentage) | `scroll-to-perc 0` |
| `G` | Bottom of the page (with a count: that percentage) | `scroll-to-perc` |
| `<Ctrl-d>` | Half a page down | `scroll-page 0 0.5` |
| `<Ctrl-u>` | Half a page up | `scroll-page 0 -0.5` |
| `<Ctrl-f>` | A full page down | `scroll-page 0 1` |
| `<Ctrl-b>` | A full page up | `scroll-page 0 -1` |

### Find in page

| Key | Action | Command |
|---|---|---|
| `/` | Find in page (forward) | `cmd-set-text /` |
| `?` | Find in page (backward) | `cmd-set-text ?` |
| `n` | Next match | `search-next` |
| `N` | Previous match | `search-prev` |

### Tabs

| Key | Action | Command |
|---|---|---|
| `J` | Next tab | `tab-next` |
| `K` | Previous tab | `tab-prev` |
| `d` | Close tab (pinned tabs need `:tab-close --force`) | `tab-close` |
| `u` | Reopen the last closed tab | `undo` |
| `co` | Close all other tabs (pinned tabs stay) | `tab-only` |
| `gC` | Duplicate the tab | `tab-clone` |
| `gJ` | Move the tab right | `tab-move +` |
| `gK` | Move the tab left | `tab-move -` |
| `<Ctrl-Tab>` | Switch to the previously used tab | `tab-focus last` |
| `<Alt-m>` | Mute / unmute the tab | `tab-mute` |
| `<Alt-1>` | Go to tab 1 | `tab-focus 1` |
| `<Alt-2>` | Go to tab 2 | `tab-focus 2` |
| `<Alt-3>` | Go to tab 3 | `tab-focus 3` |
| `<Alt-4>` | Go to tab 4 | `tab-focus 4` |
| `<Alt-5>` | Go to tab 5 | `tab-focus 5` |
| `<Alt-6>` | Go to tab 6 | `tab-focus 6` |
| `<Alt-7>` | Go to tab 7 | `tab-focus 7` |
| `<Alt-8>` | Go to tab 8 | `tab-focus 8` |
| `<Alt-9>` | Go to the last tab | `tab-focus -1` |

### Hints

| Key | Action | Command |
|---|---|---|
| `f` | Follow a link or click an element by hint | `hint` |
| `F` | Open a link in a new tab by hint (background) | `hint all tab` |
| `;b` | Open a link in a background tab by hint | `hint all tab-bg` |
| `;r` | Rapid hints: open several links in background tabs | `hint --rapid links tab-bg` |
| `;y` | Copy a link's URL by hint | `hint links yank` |
| `;o` | Put a link's URL in the command line by hint | `hint links fill :open {hint-url}` |
| `gi` | Focus the first text field | `hint inputs --first` |

### Copy

| Key | Action | Command |
|---|---|---|
| `yy` | Copy the URL | `yank` |
| `yt` | Copy the page title | `yank title` |
| `yd` | Copy the scheme and host (`https://example.com`) | `yank domain` |
| `ym` | Copy a Markdown link: `[title](url)` | `yank inline [{title}]({url})` |

### Quickmarks / bookmarks

| Key | Action | Command |
|---|---|---|
| `m` | Save the page as a quickmark (asks for a name) | `quickmark-save` |
| `b` | Open a quickmark | `cmd-set-text -s :quickmark-load` |
| `B` | Open a quickmark in a new tab | `cmd-set-text -s :quickmark-load -t` |
| `M` | Bookmark the page | `bookmark-add` |
| `gb` | Open a bookmark | `cmd-set-text -s :bookmark-load` |
| `gB` | Open a bookmark in a new tab | `cmd-set-text -s :bookmark-load -t` |
| `Sb` | Bookmarks and quickmarks manager | `bookmarks` |
| `Ss` | Settings page | `settings` |
| `Sd` | Downloads page | `downloads` |
| `Sh` | History page | `history` |
| `<Ctrl-p>` | Pin / unpin the tab | `tab-pin` |
| `gD` | Move the tab to a new window | `tab-give` |

### Zoom

| Key | Action | Command |
|---|---|---|
| `+` | Zoom in | `zoom-in` |
| `-` | Zoom out | `zoom-out` |
| `=` | Reset zoom (with a count: that percentage) | `zoom` |

### Site toggles

| Key | Action | Command |
|---|---|---|
| `<Space>td` | Toggle dark mode for this site | `dark-toggle` |
| `<Space>th` | Toggle HTTPS-only for this site | `https-toggle` |
| `<Space>pp` | Fill the best matching login from your password manager | `password-fill` |
| `<Space>po` | Fill the one-time (TOTP) code | `password-fill --otp` |

### Modes and the rest

| Key | Action | Command |
|---|---|---|
| `i` | Insert mode: keys go to the page | `mode-enter insert` |
| `<Ctrl-v>` | Passthrough mode: every key goes to the page | `mode-enter passthrough` |
| `<Escape>` | Clear pending keys, end the search, leave fullscreen | `clear-keychain ;; search ;; fullscreen --leave` |
| `gd` | Download the current page | `download` |
| `<F1>` | Show every key binding (type to filter) | `bindings` |
| `<F11>` | Toggle fullscreen | `fullscreen` |
| `<F12>` | Toggle DevTools | `devtools` |
| `<Ctrl-n>` | New window | `open -w` |
| `<Ctrl-q>` | Quit (all windows) | `quit` |
| `<Ctrl-Shift-v>` | Paste, like right-click → Paste (works in web terminals that swallow Ctrl-V) | `paste` |

### Insert mode

| Key | Action | Command |
|---|---|---|
| `<Escape>` | Back to normal mode | `mode-leave` |
| `<Ctrl-Shift-v>` | Paste, like right-click → Paste (works in web terminals that swallow Ctrl-V) | `paste` |

### Passthrough mode

| Key | Action | Command |
|---|---|---|
| `<Shift-Escape>` | Back to normal mode | `mode-leave` |

## Command line keys

The palette (`:`, `o`, `/`) uses readline keys:

| Key | Action |
|---|---|
| `Enter` | Run |
| `Escape`, `Ctrl-[` | Close |
| `Tab` / `Shift-Tab` | Next / previous completion |
| `Up` / `Down`, `Ctrl-p` / `Ctrl-n` | Command history (filtered by what you typed) |
| `Ctrl-a` / `Ctrl-e` | Start / end of line |
| `Ctrl-b` / `Ctrl-f`, `Alt-b` / `Alt-f` | Back / forward one character, one word |
| `Ctrl-h` | Delete the character before the cursor |
| `Ctrl-w` / `Alt-d` | Delete the word before / after the cursor |
| `Ctrl-u` / `Ctrl-k` | Delete to the start / end of line |
| `Ctrl-y` | Paste the last deleted text |

`/text` and `?text` search as you type. In `o`/`O` the box starts empty; what
you type is opened as a URL, or searched if it doesn't look like one (see
`url.auto_search`). Prefix a search engine keyword to use that engine:
`o gh rust-lang`.

## Prompts

Permission requests (camera, notifications, clipboard…) and page dialogs
(`alert`, `confirm`, leave-page warnings) appear in the tab bar:

| Key | Answer |
|---|---|
| `y` | Yes, this time |
| `n`, `Escape` | No, this time |
| `A` | Always for this site (saved) |
| `N` | Never for this site (saved) |

## Hints

`f` puts a short label on every clickable element; type the label to click it.
Labels use `hints.chars` (home row by default). The `:hint` command takes a
group and a target:

```
:hint [--rapid] [--first] [group] [target] [args…]
```

| Group | Elements |
|---|---|
| `all` (default) | anything clickable |
| `links` | links |
| `images`, `media` | images; audio/video/images |
| `url` | anything with `src` or `href` |
| `inputs` | text fields |
| your own | any key you add to `hints.selectors` |

| Target | What happens |
|---|---|
| `normal` (default) | click it |
| `tab` / `tab-bg` | open the link in a background tab |
| `tab-fg`, `window` | open the link in a foreground tab |
| `hover` | move the mouse over it |
| `yank` / `yank-primary` | copy the link to the clipboard / primary selection |
| `fill <cmd>` | put `<cmd>` in the command line, `{hint-url}` replaced |
| `run <cmd>` | run `<cmd>` with `{hint-url}` replaced |
| `spawn <program> [args]` | start a program with `{hint-url}` replaced |

`--rapid` keeps hint mode open after each pick; `--first` picks the first
match without asking (used by `gi`). To play a video link in mpv:

```ts
bind(";v", "hint links spawn mpv {hint-url}");
```

## Key notation

Bindings use qutebrowser's notation:

- Plain characters as typed: `j`, `G`, `?`, `gg`.
- Modifiers in angle brackets: `<Ctrl-x>`, `<Alt-Shift-j>`, `<Meta-k>`
  (`<Mod1-…>` = Alt, `<Mod4-…>` = Meta).
- Named keys: `<Escape>`, `<Return>` (`<Enter>`), `<Tab>`, `<Space>`,
  `<Backspace>`, `<Delete>`, `<Up>`, `<Left>`, `<PgDown>`, `<Home>`, `<F1>`…
- Sequences combine: `<Space>td`, `;y`, `<Ctrl-x>k`.
- Shifted printable keys are written as the character (`J`, not `<Shift-j>`);
  with Ctrl or Alt write the shift explicitly (`<Ctrl-Shift-v>`).

## Changing bindings

In `~/.config/novabrowser/config.ts`:

```ts
import { bind, unbind } from "nova";

bind("<Alt-l>", "tab-next");
bind("<Alt-h>", "tab-prev");
bind("<Ctrl-Shift-Right>", "open -t {url}");
bind(",m", "spawn mpv {url}");
bind("<Ctrl-e>", "mode-leave", "insert");   // modes: normal, insert, passthrough
unbind("<Ctrl-q>");
```

Then run `:config-source` (or restart). Several commands can be chained with
`;;`. See [configuration](configuration.md) and [commands](commands.md).
