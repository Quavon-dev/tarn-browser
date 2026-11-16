#!/bin/sh
set -eu

cd "$(dirname "$0")"
VERSION=8.14.3
SHA256=ed1a8d686605fd7c23bdf62c7fc7add1c5b23b2bbc3721e661934ef4a4911d7c
HOME_DIR="$PWD/build/gradle-$VERSION"

if [ ! -x "$HOME_DIR/bin/gradle" ]; then
    mkdir -p build
    tmp="$(mktemp -d "$PWD/build/gradle.XXXXXX")"
    trap 'rm -rf "$tmp"' EXIT
    curl -fsSL --proto '=https' --tlsv1.2 --retry 3 -o "$tmp/gradle.zip" "https://services.gradle.org/distributions/gradle-$VERSION-all.zip"
    echo "$SHA256  $tmp/gradle.zip" | shasum -a 256 -c -
    unzip -q "$tmp/gradle.zip" -d "$tmp"
    mv "$tmp/gradle-$VERSION" "$HOME_DIR" 2>/dev/null || true
    rm -rf "$HOME_DIR/gradle-$VERSION"
    [ -x "$HOME_DIR/bin/gradle" ] || { echo "$HOME_DIR is incomplete; delete it and run again" >&2; exit 1; }
    rm -rf "$tmp"
    trap - EXIT
fi

cd host
exec "$HOME_DIR/bin/gradle" "$@"
