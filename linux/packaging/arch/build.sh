#!/bin/sh
# Builds the Arch package with makepkg from the repository at /work, using a local source tarball
# of HEAD in place of the release download.
set -e
sed -i "s/^#DisableSandboxSyscalls/DisableSandboxSyscalls/" /etc/pacman.conf
pacman --disable-sandbox -Syu --noconfirm --needed base-devel rust webkitgtk-6.0 gtk4 gstreamer gst-plugins-base-libs >/dev/null
useradd -m builder 2>/dev/null || true
dir=$(mktemp -d)
cp /work/linux/packaging/arch/PKGBUILD "$dir/"
cp /work/linux/dist/src/tarn-*.tar.gz "$dir/"
chown -R builder "$dir"
mkdir -p /target && chown -R builder /target
su builder -c "cd $dir && CARGO_TARGET_DIR=/target makepkg --noconfirm"
mkdir -p /work/linux/dist
cp "$dir"/*.pkg.tar.zst /work/linux/dist/
