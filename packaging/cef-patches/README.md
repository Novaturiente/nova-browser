# Own CEF build (CEF 158.0.3, branch 8127 @ 8c29be2)

Built on novahome (16 threads, 31 GB RAM; ~115 GB disk for source + output).

Download (once, ~31 GB in ~/cef-build):

    python3 automate-git.py --download-dir=$HOME/cef-build --branch=8127 \
      --checkout=8c29be240971eebee960ac92114d3efccb5780ff --with-pgo-profiles \
      --no-chromium-history --x64-build --no-debug-build --no-build --no-distrib

Then copy this folder's `*.patch` to `~/cef-build/patches/` and run `build.sh`
(patches, `cef_create_projects.sh`, `autoninja -j10`, `make_distrib.sh`).

Patches, in the order `build.sh` applies them:

- `osr-hold-texture.patch` (in `cef/`) — CEF PR #4358: `OnAcceleratedPaint` with a
  release callback (`on_accelerated_paint2`, experimental API), so the browser shows
  CEF's dmabuf without copying it.
- `osr-reshow.patch` (in `cef/`) — CEF PR #4316: a tab shown again while 5+ off-screen
  views are visible is not left blank (fixes #3427).
- `allocator.patch` — PartitionAlloc-as-malloc + BackupRefPtr on; ThinLTO link jobs 8.
- `privacy.patch` — component allowlist (Widevine, CRLSet, PKI metadata, hyphenation).
  Gaia ListAccounts is off without a patch since CEF 158 (CEF's `signin_allowed_4276`
  plus `signin.allowed = false` in main.rs).
- `x86-64-v3.patch` — AVX2 CPUs only (SIGILL elsewhere). The published build uses it;
  `packaging/PKGBUILD` falls back to the official prebuilt CEF on CPUs without x86-64-v3.
- `ua-brand.patch` — Helium's: "Google Chrome" in the `Sec-CH-UA` brand list.
- `blink-dark-reader.patch` — forced dark mode uses Dark Reader's colour maps per role
  (background, border, text/SVG), the same maths as novabrowser-webkit's WebKit patch;
  every colour is mapped (no brightness thresholds).
- `per-site-dark.patch` — force dark per page from the AUTO_DARK_WEB_CONTENT content
  setting (registered on Linux; http(s) pages only), recomputed after each navigation
  and for all open pages when the setting changes. novabrowser sets it in
  `src/window.rs` (`apply_dark`, `apply_dark_default`).

CEF's own patcher runs again in `cef_create_projects.sh`: a patch of ours that touches
a file CEF also patches must apply on top of CEF's version (the old Gaia hunk did not).

`concurrent_links` can't be set with ThinLTO (one link at a time already).

The bindings must use the experimental API (`CEF_API_VERSION` 999999) to see
`on_accelerated_paint2`: libcef's generated `CefRenderHandler_999999_CToCpp` is the only
one that calls it.

Publish (the PKGBUILD downloads this):

    D=cef/binary_distrib/cef_binary_*_minimal; mkdir cef-own
    cp -a $D/Release/. $D/Resources/. cef-own/; cp -a $D/{include,cmake,libcef_dll,CMakeLists.txt,CREDITS.html} cef-own/
    cp archive.json cef-own/   # the official 158.0.3 minimal entry from cef-builds index.json; cef-dll-sys checks it
    tar -cf - cef-own | zstd -19 -T0 -o cef-<ver>-novabrowser-x86_64-v3.tar.zst
    gh release create cef-<ver>-novaN cef-<ver>-novabrowser-x86_64-v3.tar.zst

Then update `_cef`, the release tag in `source` and the sha256 in `packaging/PKGBUILD`.
Current: release `cef-158.0.3-nova1` (previous: `cef-154.0.34-nova1`).
