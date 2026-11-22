#!/bin/bash
# Patch, build and package CEF 158.0.3 (branch 8127) for novabrowser on novahome.
# Patches are in ~/cef-build/patches (packaging/cef-patches from the repo).
set -euo pipefail
B=$HOME/cef-build
P=$B/patches
export PATH=$B/depot_tools:$PATH TMPDIR=$B/tmp GN_OUT_CONFIGS=Release_GN_x64
export GN_DEFINES="is_official_build=true use_sysroot=true proprietary_codecs=true ffmpeg_branding=Chrome symbol_level=0 blink_symbol_level=0 v8_symbol_level=0 chrome_pgo_phase=2"

cd "$B/chromium/src"
if [ ! -e .nova-patched ]; then
  # CEF PRs first (in cef/), then ours; per-site-dark touches cef/ after them
  for f in osr-hold-texture osr-reshow; do (cd cef && patch -p1 --forward < "$P/$f.patch"); done
  for f in allocator privacy x86-64-v3 ua-brand blink-dark-reader per-site-dark capture-aux-planes; do
    patch -p1 --forward < "$P/$f.patch"
  done
  cp cef/tools/gn_args.py "$B/cef/tools/gn_args.py"
  touch .nova-patched
fi

(cd cef && ./cef_create_projects.sh)
autoninja -j16 -C out/Release_GN_x64 cefsimple chrome_sandbox
(cd cef/tools && ./make_distrib.sh --ninja-build --minimal --x64-build --no-docs --no-symbols --no-archive)
echo BUILD-OK
