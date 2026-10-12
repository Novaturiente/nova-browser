# Downloads

Downloads start immediately, without a dialog, into
`downloads.location.directory` (default `~/Downloads`). A file that already
exists is never overwritten: the new one becomes `name (1).ext` (or
`name.1.ext` with aria2).

The right end of the tab bar shows `↓ 2 · 45%` while downloads run, then
`✓` or `✗` until you look at them. Click it, press `Sd` or run `:downloads`
for the downloads page: name, size, folder, times, source URL and state.

| Command | Does |
|---|---|
| `gd`, `:download [url]` | Download the current page or *url* |
| `:download-open [N]` | Open finished download N (newest = 1) with `xdg-open` or `downloads.open_dispatcher` |
| `:download-open 1 mpv {}` | Open it with a specific program |
| `:download-cancel [N]` | Cancel a running download |
| `:download-clear` | Forget finished entries (the files stay) |

## aria2 (faster downloads, torrents)

If [aria2](https://aria2.github.io/) is installed (`pacman -S aria2`),
novabrowser uses it for downloads automatically:

- **8 connections per file**, which is much faster on servers that limit
  each connection.
- **Resume after restart**: quitting the browser stops aria2 and keeps a list
  of unfinished downloads; they continue at the next start.
- **Torrents**: clicking a `magnet:` link, opening a `.torrent` file, or
  `:download magnet:?xt=…` downloads the torrent. Seeding stops as soon as it
  finishes.
- The page's cookies, user agent and referrer are passed on, so downloads
  that need you to be logged in still work.

Chromium's own downloader is still used for form (POST) downloads,
page-generated files (`blob:`, `data:`), and anything aria2 refuses.

```ts
c.downloads.backend = "auto";      // default: aria2 if installed
// "aria2": always use it (warning if missing), "chromium": never
```

Privacy notes:

- While a download is unfinished, its cookies are stored in
  `~/.local/share/novabrowser/aria2.session` so it can resume. They are gone
  once it finishes or is cancelled.
- aria2 listens only on `127.0.0.1`, with a random port and a random secret.
- Torrenting shows your IP address to other peers, as in any torrent client.

Cancelling a torrent leaves its folder in place, in case it contains files
you already had; delete it yourself.
