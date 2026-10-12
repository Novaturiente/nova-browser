# Privacy and content blocking

## Ad and tracker blocking

Built in and on by default, using Brave's
[adblock-rust](https://github.com/brave/adblock-rust) engine with
EasyList/uBlock Origin filter syntax:

- **Network blocking** of ads and trackers.
- **Cosmetic filtering**: hides ad containers, including generic class/id
  rules.
- **Scriptlets** (uBlock Origin's `+js(...)` rules), injected before page
  scripts run. This is what removes YouTube ads.

Default lists: EasyList and EasyPrivacy. Add more:

```ts
c.content.blocking.adblock.lists = [
  "https://easylist.to/easylist/easylist.txt",
  "https://easylist.to/easylist/easyprivacy.txt",
  "https://secure.fanboy.co.nz/fanboy-annoyance.txt",
  "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/filters.txt",
  `file://${configDir}/my-filters.txt`,   // your own rules
];
```

Lists are compiled once and cached, so startup never waits for them. They
update daily in the background; `:adblock-update` updates now.

Turn blocking off for one site:

```ts
pattern("*://*.example.com/*", (p) => { p.content.blocking.enabled = false; });
```

Limits: no element picker, no procedural cosmetic filters (`:has-text` and
similar), and cosmetic rules don't reach inside frames. Hiding can flash
briefly on slow pages.

## Cookies

`content.cookies.accept`:

| Value | Effect |
|---|---|
| `all` (default) | accept everything |
| `no-3rdparty` | block third-party cookies **and** third-party JavaScript storage |
| `never` | block all cookies and site storage |

Per site, e.g. allow everything for a site that needs third-party login
cookies:

```ts
c.content.cookies.accept = "no-3rdparty";
pattern("*://*.google.com/*", (p) => { p.content.cookies.accept = "all"; });
```

## HTTPS-only

Every `http://` page load is upgraded to `https://`, including redirects.
Exempt automatically: `localhost`, hostnames without a dot, `.local`, `.lan`,
`.internal`, `.home`, `.arpa`, `.ts.net`, private IPv4/IPv6 ranges and
Tailscale (`100.64.0.0/10`). For other sites that only work over HTTP press
`<Space>th` (`:https-toggle`); the host is saved in
`~/.config/novabrowser/https_excludes`.

## Permissions

Camera, microphone, screen sharing, location, notifications, clipboard
reading, persistent storage, pointer lock and protocol handlers all **ask**
by default. Answer in the tab bar with `y`, `n`, `A` (always for this site)
or `N` (never). Always/Never answers are stored in
`~/.local/share/novabrowser/permissions.json`; delete a line there to be
asked again. Set a policy instead of being asked:

```ts
c.content.notifications.enabled = false;              // never ask, deny
pattern("*://app.slack.com/*", (p) => {
  p.content.notifications.enabled = true;
  p.content.media.audio_capture = true;
});
```

## Headers and fingerprinting

- `DNT: 1` is sent by default (`content.headers.do_not_track`).
- `content.headers.custom` adds headers, e.g. `{"Sec-GPC": "1"}`.
- `content.canvas_reading = false` blocks canvas read-back.
- `content.webrtc_ip_handling_policy = "default-public-interface-only"` keeps
  WebRTC from exposing your local network addresses.

## Calls to Google

Chromium contacts Google in the background even with no page open. With
novabrowser's own CEF build (the default on AVX2 CPUs, see
[building](building.md)) these are removed or reduced:

| Background call | novabrowser |
|---|---|
| Google account check (`accounts.google.com` ListAccounts) | removed (build patch) |
| Component downloads (optimization models, privacy sandbox data, on-device models, …) | removed, except CRLSet (revoked certificates), Certificate Transparency, Widevine (DRM video) and hyphenation |
| Network time queries | off |
| Search preconnect, AI-mode eligibility, Topics API | off |
| Browser sign-in | off |
| Safe Browsing | off (it needs a Google API key that CEF doesn't have) |
| Hyperlink auditing (`<a ping>`) | off |

What remains: `update.googleapis.com` is asked about updates for the four
kept components. Spell-check dictionaries are downloaded when you enable
`spellcheck.languages`, and filter lists when ad blocking updates.

With the official prebuilt CEF (CPUs without AVX2) the runtime switches still
apply, but the account check and the component downloads remain.

**Trade-off:** with Safe Browsing off, novabrowser does not warn about known
phishing or malware sites. Ad-blocking lists catch some of them.
