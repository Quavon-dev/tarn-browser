#!/bin/sh
# Builds the RPM from the repository at /work inside a Fedora image.
set -e
dnf -y -q install rpm-build gcc pkgconf-pkg-config gtk4-devel webkitgtk6.0-devel gstreamer1-devel \
  gstreamer1-plugins-base-devel curl tar >/dev/null
command -v cargo >/dev/null || sh /work/linux/packaging/rustup.sh
export PATH="$HOME/.cargo/bin:$PATH"
top=$(mktemp -d)
mkdir -p "$top/SOURCES"
cp /work/linux/dist/src/tarn-*.tar.gz "$top/SOURCES/"
rpmbuild -bb --define "_topdir $top" /work/linux/packaging/rpm/tarn.spec
mkdir -p /work/linux/dist
cp "$top"/RPMS/*/*.rpm /work/linux/dist/
