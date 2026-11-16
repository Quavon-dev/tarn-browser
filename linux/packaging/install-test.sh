#!/bin/sh
# Installs a built package in a clean container of its distribution, starts Tarn under Xvfb and
# records the dependencies, the WebKitGTK it links and a screenshot in evidence/install-<distro>.*
# usage: install-test.sh debian|ubuntu|fedora|arch <package file in linux/dist>
set -e
root=$(cd "$(dirname "$0")/../.." && pwd)
distro=$1; pkg=$2
case $distro in
  debian) image=debian:trixie; platform=linux/arm64
    setup='export DEBIAN_FRONTEND=noninteractive; apt-get update -qq; apt-get install -y -qq /dist/'$pkg' xvfb imagemagick dbus >/dev/null'
    info='dpkg -s tarn | grep -E "^(Version|Depends)"; dpkg -l "libwebkitgtk-6.0-4*" | tail -1' ;;
  ubuntu) image=ubuntu:26.04; platform=linux/arm64
    setup='export DEBIAN_FRONTEND=noninteractive; apt-get update -qq; apt-get install -y -qq /dist/'$pkg' xvfb imagemagick dbus >/dev/null'
    info='dpkg -s tarn | grep -E "^(Version|Depends)"; dpkg -l "libwebkitgtk-6.0-4*" | tail -1' ;;
  fedora) image=fedora:43; platform=linux/arm64
    setup='dnf -y -q install /dist/'$pkg' xorg-x11-server-Xvfb ImageMagick dbus-daemon mesa-dri-drivers procps-ng >/dev/null'
    info='rpm -q tarn; rpm -qR tarn | grep -v "^rpmlib"; rpm -q webkitgtk6.0' ;;
  arch) image=archlinux:latest; platform=linux/amd64
    setup='sed -i "s/^#DisableSandboxSyscalls/DisableSandboxSyscalls/" /etc/pacman.conf; pacman --disable-sandbox -Sy --noconfirm >/dev/null; pacman --disable-sandbox -U --noconfirm /dist/'$pkg' >/dev/null; pacman --disable-sandbox -S --noconfirm --needed xorg-server-xvfb imagemagick mesa >/dev/null'
    info='pacman -Qi tarn | grep -E "^(Version|Depends On)"; pacman -Q webkitgtk-6.0' ;;
  *) echo "unknown distro $distro" >&2; exit 2 ;;
esac
mkdir -p "$root/linux/evidence"
docker run --rm --platform "$platform" -v "$root/linux/dist":/dist:ro -v "$root/linux/evidence":/evidence "$image" sh -c "
set -e
$setup
echo '## package'; $info
echo '## linked WebKitGTK'; ldd /usr/bin/tarn | grep -E 'webkitgtk|gtk-4|javascriptcore'
echo '## desktop entry'; ls /usr/share/applications/de.quavon.tarn.desktop /usr/share/icons/hicolor/scalable/apps/de.quavon.tarn.svg
Xvfb :99 -screen 0 1280x800x24 >/dev/null 2>&1 &
mkdir -p /run/dbus; dbus-daemon --session --address=unix:path=/tmp/bus --fork >/dev/null
sleep 2
DISPLAY=:99 DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/bus LIBGL_ALWAYS_SOFTWARE=1 WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1 TARN_LOG_EVENTS=1 tarn >/tmp/tarn.log 2>&1 &
sleep 25
echo '## running'; pgrep -a tarn || echo 'tarn is not running'; pgrep -af WebKitWebProcess | head -1 || true
echo '## app log'; grep -E '^.tarn.|^tarn:|panicked' /tmp/tarn.log | head || true
DISPLAY=:99 import -window root /evidence/install-$distro.png
" > "$root/linux/evidence/install-$distro.txt" 2>&1
cat "$root/linux/evidence/install-$distro.txt"
