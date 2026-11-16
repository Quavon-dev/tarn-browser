#!/bin/sh
# Regenerates cargo-sources.json from linux/Cargo.lock with flatpak-cargo-generator (pinned commit and
# sha256), in a clean Fedora container with the generator's Python modules from Fedora.
set -e
root=$(cd "$(dirname "$0")/../../.." && pwd)
commit=9d86d336a39b9af38b86f114b6ce5820fe260f91
sha256=0a2db6be87d75910facef28ab46d4d6460802e8419ab850d0caa6a364d26b380
docker run --rm -v "$root":/work:ro -v "$root/linux/packaging/flatpak":/out \
  fedora:43@sha256:f01112b9a1f1d0504ad962d9adfa401425033f8ab067e0a821904ebdde5b8ca5 sh -ec "
    dnf -y -q install python3-aiohttp python3-pyyaml python3-tomlkit >/dev/null
    mkdir /gen && cd /gen
    curl -fsSLo generator.py https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/$commit/cargo/flatpak-cargo-generator.py
    echo '$sha256  generator.py' | sha256sum -c -
    python3 -I generator.py /work/linux/Cargo.lock -o /out/cargo-sources.json"
