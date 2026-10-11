# Own CEF build (CEF 154.0.34, branch 8037 @ 14c5a08)

Download (once, ~29 GB in ~/cef-build):

    python3 automate-git.py --download-dir=$HOME/cef-build --branch=8037 \
      --checkout=14c5a089e8452874cb1a556dc3fde26ed2d87723 --with-pgo-profiles \
      --no-chromium-history --x64-build --no-debug-build --no-build --no-distrib

Patches (`patch -p1` in `chromium/src`; copy the patched `cef/tools/gn_args.py`
back to `~/cef-build/cef/tools/` so a CEF re-copy keeps it):

- `allocator.patch` — PartitionAlloc-as-malloc + BackupRefPtr on; ThinLTO link jobs 8.
- `privacy.patch` — component allowlist (Widevine, CRLSet, PKI metadata, hyphenation); no Gaia ListAccounts.
- `x86-64-v3.patch` — personal builds only (SIGILL on pre-AVX2 CPUs). Skip for public builds.

Configure and build:

    export PATH=$HOME/cef-build/depot_tools:$PATH TMPDIR=$HOME/cef-build/tmp GN_OUT_CONFIGS=Release_GN_x64
    export GN_DEFINES="is_official_build=true use_sysroot=true proprietary_codecs=true ffmpeg_branding=Chrome symbol_level=0 blink_symbol_level=0 v8_symbol_level=0 chrome_pgo_phase=2"
    cd chromium/src/cef && ./cef_create_projects.sh && cd ..
    autoninja -j6 -C out/Release_GN_x64 cefsimple
    cd cef/tools && ./make_distrib.sh --ninja-build --minimal --x64-build --no-docs --no-symbols --no-archive

`concurrent_links` can't be set with ThinLTO (one link at a time already).
