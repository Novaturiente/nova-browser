#!/bin/bash
# Release vX.Y.Z: build the source package from the pushed tag, upload the -bin tarball,
# fill in sha256sums and .SRCINFO for packaging/aur/*. Run from anywhere; needs gh + makepkg.
# Usage: packaging/release.sh 0.1.0   (tag v0.1.0 must already be pushed)
set -euo pipefail
v=${1:?version}
repo=$(cd "$(dirname "$0")/.." && pwd)
aur=$repo/packaging/aur
work=${WORK:-$HOME/.cache/nova-release}/$v
url=https://github.com/Novaturiente/nova-browser
mkdir -p "$work/tmp"
export TMPDIR=$work/tmp  # /tmp is too small for the CEF build tree

git -C "$repo" ls-remote --exit-code --tags origin "v$v" >/dev/null || { echo "push tag v$v first"; exit 1; }

# 1. source package: pin the tag tarball's sha256, build + test it
src_sha=$(curl -fsSL "$url/archive/refs/tags/v$v.tar.gz" | sha256sum | cut -d' ' -f1)
sed -i -e "s/^pkgver=.*/pkgver=$v/" -e "s/^pkgrel=.*/pkgrel=1/" \
  -e "0,/^sha256sums=('[^']*'/s//sha256sums=('$src_sha'/" "$aur/novabrowser/PKGBUILD"
mkdir -p "$work/novabrowser" && cp "$aur/novabrowser/PKGBUILD" "$work/novabrowser/"
(cd "$work/novabrowser" && makepkg -f --cleanbuild)
pkg=$(ls "$work"/novabrowser/novabrowser-"$v"-1-x86_64.pkg.tar.zst)

# 2. -bin tarball = the package's /usr tree
rm -rf "$work/stage" && mkdir -p "$work/stage"
bsdtar -xf "$pkg" -C "$work/stage" usr
tarball=$work/novabrowser-$v-x86_64.tar.zst
tar -C "$work/stage" -cf - usr | zstd -19 -T0 -f -o "$tarball"
awk -v v="## $v" '$0 ~ "^## " {p = index($0, v) == 1} p' "$repo/CHANGELOG.md" > "$work/notes.md"
gh release create "v$v" "$tarball" -R Novaturiente/nova-browser --title "novabrowser $v" --notes-file "$work/notes.md"

bin_sha=$(sha256sum "$tarball" | cut -d' ' -f1)
sed -i -e "s/^pkgver=.*/pkgver=$v/" -e "s/^pkgrel=.*/pkgrel=1/" \
  -e "s/^sha256sums=.*/sha256sums=('$bin_sha')/" "$aur/novabrowser-bin/PKGBUILD"

# 3. .SRCINFO for both
for p in novabrowser novabrowser-bin; do (cd "$aur/$p" && makepkg --printsrcinfo > .SRCINFO); done
echo "done: commit packaging/aur, then push each dir to its AUR repo (packaging/aur/README.md)"
