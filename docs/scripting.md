# Scripting and external tools

novabrowser has no extension system. Instead, other programs can control it,
and it can start other programs.

## Controlling a running browser

```sh
novabrowser --cmd 'open -t https://example.com'    # run any command
novabrowser --cmd 'tab-next ;; reload'
novabrowser --query state                          # JSON: windows, tabs, active tab, mode
novabrowser https://example.com                    # opens as a tab in the running instance
novabrowser                                        # opens a new window
```

`--cmd` prints `ok`, or the error. These talk to the instance of the same
profile over a Unix socket (`~/.local/share/novabrowser/ipc`); add
`--profile=name` for another profile. If nothing is running, they exit
with status 1.

Examples: bind a desktop shortcut to `novabrowser --cmd 'open -t {clipboard}'`,
or open links from a terminal or chat client into your current window.

## Starting programs

```
:spawn mpv {url}                 play the current page in mpv
:spawn -o ls -la ~/Downloads     show a program's output in a new tab
:hint links spawn mpv {hint-url} pick a link, play it
```

`spawn` never blocks the browser. Placeholders like `{url}`, `{title}` and
`{clipboard}` are replaced first.

## Userscripts

A userscript is any executable in `~/.config/novabrowser/userscripts/` (or
`~/.local/share/novabrowser/userscripts/`). Run it with `:spawn -u name
[args]` or bind it:

```ts
bind(",r", "spawn -u readability");
```

It gets the same environment as a qutebrowser userscript, so many
qutebrowser userscripts work unchanged:

| Variable | Contents |
|---|---|
| `QUTE_MODE` | `command` (or `hints` from a hint target) |
| `QUTE_URL`, `QUTE_TITLE` | current page |
| `QUTE_SELECTED_TEXT` | selected text |
| `QUTE_HTML`, `QUTE_TEXT` | paths of files with the page's HTML and text |
| `QUTE_FIFO` | a pipe: each line you write to it runs as a command |
| `QUTE_COUNT` | the count, if one was given |
| `QUTE_COMMANDLINE_TEXT` | text in the command line |
| `QUTE_CONFIG_DIR`, `QUTE_DATA_DIR`, `QUTE_DOWNLOAD_DIR` | directories |
| `QUTE_USERSCRIPT`, `QUTE_VERSION` | script path, browser version |

Commands written to `$QUTE_FIFO` run in the tab the script was started from
(dropped if that tab was closed meanwhile).

This one saves the page's URL to a reading list:

```sh
#!/bin/sh
echo "$QUTE_URL $QUTE_TITLE" >> ~/reading-list.txt
echo "message-info 'Saved to reading list'" >> "$QUTE_FIFO"
```

## Typing into pages

- `:insert-text hello` types into the focused field.
- `:fake-key <Ctrl-a>` or `:fake-key hello<Return>` sends key presses in
  [key notation](keybindings.md#key-notation); `\<` is a literal `<`.

Both produce real (trusted) input events, so sites react as if you typed.
Together with `$QUTE_FIFO` this is enough for password-manager scripts, form
fillers and similar tools.

## DevTools

`F12` opens Chromium DevTools for the current tab. For automation, start with
`--remote-debugging-port=9222` and use any Chrome DevTools Protocol client.
