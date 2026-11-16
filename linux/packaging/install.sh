#!/bin/sh
# Installs a built release binary, the desktop entry and the icon under $1 (a package root).
set -e
dest=$1
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
bin=${TARN_BIN:-$repo/linux/target/release/tarn}
install -Dm755 "$bin" "$dest/usr/bin/tarn"
install -Dm644 "$here/de.quavon.tarn.desktop" "$dest/usr/share/applications/de.quavon.tarn.desktop"
install -Dm644 "$repo/crates/ui/brand/mark.svg" "$dest/usr/share/icons/hicolor/scalable/apps/de.quavon.tarn.svg"
install -Dm644 "$repo/LICENSE" "$dest/usr/share/licenses/tarn/LICENSE"
install -Dm644 "$repo/packaging/third-party-licenses-linux.txt" "$dest/usr/share/licenses/tarn/third-party-licenses.txt"
