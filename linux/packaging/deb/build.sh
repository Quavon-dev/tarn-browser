#!/bin/sh
# Builds tarn_<version>_<arch>.deb from the repository at /work inside a Debian or Ubuntu image;
# dpkg-shlibdeps fills Depends from the binary, so it names the distribution's WebKitGTK 6.0.
set -e
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends ca-certificates curl build-essential pkg-config dpkg-dev \
  libgtk-4-dev libwebkitgtk-6.0-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev >/dev/null
sh /work/linux/packaging/rustup.sh
export PATH="$HOME/.cargo/bin:$PATH"
cd /work/linux
cargo build --release --locked
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
arch=$(dpkg --print-architecture)
root=$(mktemp -d)/tarn
TARN_BIN=${CARGO_TARGET_DIR:-target}/release/tarn sh packaging/install.sh "$root"
work=$(mktemp -d)
mkdir -p "$work/debian" "$root/DEBIAN"
printf 'Source: tarn\n\nPackage: tarn\nArchitecture: any\n' > "$work/debian/control"
depends=$(cd "$work" && dpkg-shlibdeps -O "$root/usr/bin/tarn" | sed -n 's/^shlibs:Depends=//p')
cat > "$root/DEBIAN/control" <<CONTROL
Package: tarn
Version: $version
Architecture: $arch
Maintainer: Tarn <accounts@quavon.de>
Depends: $depends
Section: web
Priority: optional
Homepage: https://github.com/Quavon-dev/tarn-browser
Description: Private web browser on the system WebKitGTK 6.0
 Tarn with Shield levels, profiles and a private space, built on Native Shell and the
 distribution's WebKitGTK 6.0.
CONTROL
out=${OUT:-/work/linux/dist}
mkdir -p "$out"
. /etc/os-release
dpkg-deb --root-owner-group --build "$root" "$out/tarn_${version}_${ID}${VERSION_ID}_${arch}.deb"
