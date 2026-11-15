#!/bin/sh
# Installs Rust 1.99.1 with the rustup-init release pinned here, checked against its SHA-256.
set -e
[ -x "$HOME/.cargo/bin/cargo" ] && exit 0
case $(uname -m) in
  x86_64) target=x86_64-unknown-linux-gnu sha=b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55 ;;
  aarch64) target=aarch64-unknown-linux-gnu sha=9f608c142e0e38a78957d0661ba1dfaf0b6fb37184cfaa4b9a0aa57af6184282 ;;
  *) echo "no pinned rustup-init for $(uname -m)" >&2; exit 1 ;;
esac
dir=$(mktemp -d)
init="$dir/rustup-init"
curl --proto '=https' --tlsv1.2 -sSfL -o "$init" "https://static.rust-lang.org/rustup/archive/1.29.3/$target/rustup-init"
echo "$sha  $init" | sha256sum -c - >/dev/null
chmod +x "$init"
"$init" -y --profile minimal --default-toolchain 1.99.1 >/dev/null
rm -r "$dir"
