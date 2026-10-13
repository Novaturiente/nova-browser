# Building and packaging

## Arch Linux package

From the AUR (needs an AVX2 / x86-64-v3 CPU):

```sh
yay -S novabrowser-bin    # prebuilt
yay -S novabrowser        # or build the release from source
```

Both install into `/usr/lib/novabrowser` with `/usr/bin/novabrowser`, a
desktop entry and icons, and use novabrowser's own CEF build. They refuse to
install on CPUs without AVX2. Check yours:
`/lib/ld-linux-x86-64.so.2 --help | grep x86-64-v3`.

To build the latest commit instead (`novabrowser-git`, not on the AUR):

```sh
git clone https://github.com/Novaturiente/nova-browser
cd nova-browser/packaging
makepkg -si
```

Building runs the test suite. Which Chromium engine (CEF) this one uses:

| Your CPU | CEF used | H.264/AAC video | Google background calls |
|---|---|---|---|
| x86-64-v3 (AVX2: Intel Haswell 2013+, AMD Excavator 2015+) | novabrowser's own build, downloaded from this repo's Releases (~50 MB) | yes | removed (see [privacy](privacy.md#calls-to-google)) |
| older x86-64 | the official prebuilt CEF (~1.5 GB download, a warning is printed) | no | not removed |

Set `NOVA_CEF=/path/to/unpacked/cef` to use a CEF you already have.

The prebuilt CEF download is large: build on a real disk, not a small
`/tmp` tmpfs (`TMPDIR=~/.cache/makepkg-tmp makepkg -si`).

Make it the default browser:

```sh
xdg-settings set default-web-browser novabrowser.desktop
```

## From source (any distribution)

The [README](../README.md#other-distributions) has the package lists for
Debian/Ubuntu and Fedora and the system-wide install commands.

Requirements: Rust (stable, via rustup), `cmake`, `ninja`, `pkg-config`, a C
compiler, SQLite (with headers), and Chromium's runtime libraries (GTK 3, NSS, ALSA, libxkbcommon,
libdrm, Mesa, CUPS, at-spi2). Optional: `wl-clipboard` (Wayland) or `xclip`
(X11) for primary selection and paste, `xdg-utils`, `aria2`.

```sh
cargo build --release
target/release/novabrowser
```

The first build downloads the matching CEF (the `cef` crate pins the exact
version) and copies it next to the binary; the binary finds it there, so no
`LD_LIBRARY_PATH` is needed. To use an already unpacked CEF:

```sh
CEF_PATH=$HOME/.local/share/cef-own cargo build --release
```

The CEF version must match the `cef` crate exactly (currently CEF 154.0.34).

Tests:

```sh
cargo test --release
tests/smoke.sh target/release/novabrowser   # needs a running desktop session
```

`smoke.sh` starts the browser with throwaway profile directories, drives it
over the control socket (tabs, `:set`, quit) and needs no network.

## novabrowser's own CEF build

The published CEF is Chromium 154 built with:

- proprietary codecs (H.264, AAC, HEVC playback),
- PartitionAlloc as the system allocator (faster than the prebuilt's glibc
  malloc),
- Chromium's official, PGO and ThinLTO build settings, compiled for x86-64-v3,
- a privacy patch: only four background components (CRLSet, Certificate
  Transparency, Widevine, hyphenation) and no Google account polling.

The full recipe, patches and publishing steps are in
[`packaging/cef-patches/`](../packaging/cef-patches/README.md). A build takes
about 5 hours on an 18-thread laptop with 16 GB RAM, plus a ~30 GB source
download.

H.264 and AAC are patented in some countries. If that matters where you live,
use the official prebuilt CEF (`NOVA_CEF`), which has no proprietary codecs.

## Runtime layout

```
/usr/lib/novabrowser/novabrowser        the binary
/usr/lib/novabrowser/libcef.so …        CEF runtime, locales, resources
/usr/bin/novabrowser                    symlink
/usr/share/applications/novabrowser.desktop
/usr/share/icons/hicolor/scalable/apps/novabrowser.svg   (+ PNGs, 16–512 px)
```
