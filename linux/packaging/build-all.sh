#!/bin/sh
# Builds each package in a clean container of its distribution: build-all.sh deb-debian|deb-ubuntu|rpm|arch
# TARN_PKG_PLATFORM picks the Debian, Ubuntu and Fedora architecture (default linux/arm64).
set -e
root=$(cd "$(dirname "$0")/../.." && pwd)
host=${TARN_PKG_PLATFORM:-linux/arm64}
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/linux/Cargo.toml" | head -n1)
rm -rf "$root/linux/dist/src"
mkdir -p "$root/linux/dist/src"
git -C "$root" archive --format=tar.gz --prefix="tarn-browser-$version/" -o "$root/linux/dist/src/tarn-$version.tar.gz" HEAD
run() { image=$1; platform=$2; script=$3; volume=tarn-pkg-$4-target
  docker run --rm --platform "$platform" -v "$root":/work -v "$volume":/target -e CARGO_TARGET_DIR=/target "$image" sh "/work/linux/packaging/$script"; }
case $1 in
  deb-debian) run debian:trixie@sha256:9c6e381421d0f48d1b8133e573f33c185ee16cbf505eda9c4a8a4791b25ea8a6 "$host" deb/build.sh debian ;;
  deb-ubuntu) run ubuntu:26.04@sha256:065aa8626841f16057b78b027b464cd59c4fac36c204c72dcbf48d66f5fa8a96 "$host" deb/build.sh ubuntu ;;
  rpm) run fedora:43@sha256:f01112b9a1f1d0504ad962d9adfa401425033f8ab067e0a821904ebdde5b8ca5 "$host" rpm/build.sh fedora ;;
  arch) run archlinux:latest@sha256:d4594bb2300cba11f41c2f27923eb56f63d9988909c402f9c95517d1e362c899 linux/amd64 arch/build.sh arch ;;
  *) echo "usage: $0 deb-debian|deb-ubuntu|rpm|arch" >&2; exit 2 ;;
esac
