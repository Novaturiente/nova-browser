# AUR packages

| Package | What it does |
|---|---|
| `novabrowser` | Builds the tagged release from source against our CEF build. |
| `novabrowser-bin` | Installs the prebuilt `/usr` tree from the GitHub release. |

Both need an x86-64-v3 (AVX2) CPU and stop in `prepare()` without one.
`packaging/PKGBUILD` (`novabrowser-git`) is the development build and is not published.

## Releasing a new version

1. Set `version` in `Cargo.toml`, rename the changelog's `Unreleased` heading
   to `## X.Y.Z (date)`, commit.
2. Tag and push: `git tag vX.Y.Z && git push origin main vX.Y.Z`.
3. `packaging/release.sh X.Y.Z` builds and tests the source package, uploads
   `novabrowser-X.Y.Z-x86_64.tar.zst` to the GitHub release, and writes the
   sha256 sums and `.SRCINFO` into `packaging/aur/*`. Commit that.
4. Push each directory to its AUR repo:

```sh
for p in novabrowser novabrowser-bin; do
  git clone ssh://aur@aur.archlinux.org/$p.git ~/aur/$p   # first time only
  cp packaging/aur/$p/{PKGBUILD,.SRCINFO} ~/aur/$p/
  git -C ~/aur/$p add PKGBUILD .SRCINFO
  git -C ~/aur/$p commit -m "Update to X.Y.Z"
  git -C ~/aur/$p push origin master
done
```

A packaging-only fix keeps `pkgver`, bumps `pkgrel`, regenerates `.SRCINFO`
(`makepkg --printsrcinfo > .SRCINFO`) and pushes again.

A cef crate bump also needs a new CEF build and release: see
`packaging/cef-patches/README.md`.
