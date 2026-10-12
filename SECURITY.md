# Security policy

## Reporting a vulnerability

Please report security problems privately through GitHub's
**Security → Report a vulnerability** on this repository, not in a public
issue. Include steps to reproduce and the version (`pacman -Q novabrowser-git`
or the git commit).

Relevant areas include: the password integration (fill and save), the
control socket and `$QUTE_FIFO`, the internal pages (`:settings`,
`:bookmarks`, `:history`), the renderer bridge (`window.nova`), and the
aria2 RPC connection.

## Chromium security updates

novabrowser bundles CEF, so Chromium security fixes arrive only when
novabrowser updates its CEF version. It currently uses CEF 154.0.34
(Chromium 154.0.8037.98). Be aware of this lag when browsing untrusted
sites. Safe Browsing is turned off (see
[privacy](docs/privacy.md#calls-to-google)).

## Supported versions

Only the latest commit on `main` is supported.
