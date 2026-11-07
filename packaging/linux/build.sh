#!/usr/bin/env bash
# Builds Oxbow for Linux and packages it into $OUT (dist/ by default): an AppImage, a .deb and a .tar.gz.
# Meant to run in the image of packaging/linux/Dockerfile (see there), which has the tools it needs.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
out=${OUT:-$root/dist}
target=${CARGO_TARGET_DIR:-$root/target}
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/crates/app/Cargo.toml" | head -n1)
mkdir -p "$out"
cd "$root"

cargo build --release --locked -p oxbow
bin="$target/release/oxbow"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# .tar.gz: the binary, its menu entry and icon, to put anywhere.
name="oxbow-$version-linux-x64"
mkdir -p "$work/$name"
install -m755 "$bin" "$work/$name/oxbow"
install -m644 packaging/linux/oxbow.desktop README.md "$work/$name/"
install -m644 crates/app/assets/icon.png "$work/$name/oxbow.png"
install -m644 LICENSE "$work/$name/LICENSE.txt"
tar -C "$work" -czf "$out/$name.tar.gz" "$name"

# .deb (Debian, Ubuntu and their relatives), which pulls in git. Its contents are in crates/app/Cargo.toml.
cargo deb -p oxbow --no-build --output "$out/oxbow_${version}_amd64.deb"

# AppImage: one file that runs on most distributions.
app="$work/Oxbow.AppDir"
install -Dm755 "$bin" "$app/usr/bin/oxbow"
install -Dm644 packaging/linux/oxbow.desktop "$app/usr/share/applications/oxbow.desktop"
install -Dm644 crates/app/assets/icon.png "$app/usr/share/icons/hicolor/256x256/apps/oxbow.png"
cp packaging/linux/oxbow.desktop "$app/oxbow.desktop"
cp crates/app/assets/icon.png "$app/oxbow.png"
ln -s usr/bin/oxbow "$app/AppRun"
ARCH=x86_64 appimagetool --no-appstream --runtime-file "${APPIMAGE_RUNTIME:?set APPIMAGE_RUNTIME to the AppImage runtime}" \
    "$app" "$out/oxbow-$version-x86_64.AppImage"

ls -l "$out"/oxbow*"$version"*
