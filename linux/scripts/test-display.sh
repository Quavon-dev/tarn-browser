#!/bin/sh
set -e
root=$(cd "$(dirname "$0")/../.." && pwd)
exec docker run --rm -v tarn-cargo:/root/.cargo/registry -v tarn-rustup:/root/.rustup \
  -v tarn-linux-p2-target:/target -e CARGO_TARGET_DIR=/target -v "$root":/work -w /work/linux tarn-linux-fedora \
  scripts/display-tests.sh "$@"
