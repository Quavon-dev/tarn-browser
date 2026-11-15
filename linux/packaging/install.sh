#!/bin/sh
# Installs a built release binary, the desktop entry and the icon under $1 (a package root).
set -e
dest=$1
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
bin=${TARN_BIN:-$repo/linux/target/release/tarn}
install -Dm755 "$bin" "$dest/usr/bin/tarn"
install -Dm644 "$here/dev.tarn.Tarn.desktop" "$dest/usr/share/applications/dev.tarn.Tarn.desktop"
install -Dm644 "$repo/crates/ui/brand/mark.svg" "$dest/usr/share/icons/hicolor/scalable/apps/dev.tarn.Tarn.svg"
install -Dm644 "$repo/LICENSE" "$dest/usr/share/licenses/tarn/LICENSE"
