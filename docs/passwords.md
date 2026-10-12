# Passwords

novabrowser has no password store of its own. It talks to the password
manager you already use, through its command-line tool:

| `passwords.backend` | Tool | Notes |
|---|---|---|
| `rbw` (default) | [rbw](https://github.com/doy/rbw), an unofficial Bitwarden / Vaultwarden client | Recommended for Bitwarden. Unlocks with rbw's own pinentry. |
| `bw` | the official Bitwarden CLI | Run `bw unlock` and start novabrowser with `BW_SESSION` set. |
| `pass` | [pass](https://www.passwordstore.org/) | Entries named `<site>/<username>`, password on the first line. TOTP needs pass-otp. Unlocks through gpg-agent. |
| `none` | | Turns the feature off. |

```ts
c.passwords.backend = "pass";
```

## Filling

1. Click or focus a username, password, one-time-code or sign-up field.
2. A list of matching logins appears under the field.
3. `Down` / `Up` choose, `Enter` fills, `Escape` hides the list.

Or press `<Space>pp` anywhere on the page to fill the best match, and
`<Space>po` to fill the current TOTP code.

- Matching uses the site's registrable domain, like Bitwarden's default:
  a login saved for `example.com` is offered on `login.example.com`. Logins
  for the exact host are listed first.
- Nothing is filled on page load, only when you pick an entry.
- Frames from a different site than the page get nothing.
- Sign-up and "new password" fields offer a generated 20-character password
  (made locally from `/dev/urandom`) that is filled into both fields.
- Login forms split across two pages (username first, then password) work:
  the username from the first page is remembered.

### Locked vault

Focusing a field never asks for your master password by itself. If the vault
is locked, the list shows *"Bitwarden is locked"*: choose it (`Down`,
`Enter`) to unlock, and the logins appear. `<Space>pp` unlocks directly.

## Saving and updating

After you submit a login form, the tab bar asks whether to save it (new
login) or update the stored password (changed password). Answer `y` or `n`.
If the password didn't change, nothing is asked. With `rbw`, a locked vault
is unlocked first.

The secret is handed to the password manager through a pipe or the
environment, never on a command line where other programs could see it.

Limits: the save prompt needs you to have focused a login field on that page
first (that's when the form is watched). A login filled and submitted
entirely by a script gets no prompt.

## Other password managers

For KeePassXC, 1Password or anything else with a CLI, write a
[userscript](scripting.md#userscripts) that looks up the login and sends
`insert-text` / `fake-key` commands to `$QUTE_FIFO`.
