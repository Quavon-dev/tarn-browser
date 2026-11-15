#!/usr/bin/env bash
# Downloads the Ninja release pinned here, checks its SHA-256 and unpacks it into DIR.
set -euo pipefail
readonly VERSION=1.13.2
readonly SHA256=c99048673aa765960a99cf10c6ddb9f1fad506099ff0a0e137ad8960a88f321b
dest="${1:?usage: fetch-ninja.sh DIR}"
mkdir -p "$dest"
curl --proto '=https' --tlsv1.2 -sSfL -o "$dest/ninja-mac.zip" "https://github.com/ninja-build/ninja/releases/download/v$VERSION/ninja-mac.zip"
echo "$SHA256  $dest/ninja-mac.zip" | shasum -a 256 -c - >/dev/null
unzip -oq "$dest/ninja-mac.zip" ninja -d "$dest"
rm "$dest/ninja-mac.zip"
