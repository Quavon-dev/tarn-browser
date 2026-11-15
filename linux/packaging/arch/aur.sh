#!/bin/sh
# Pushes the PKGBUILD of a release tag to the AUR package tarn: aur.sh <tag>, with the AUR account's
# SSH key in AUR_SSH_PRIVATE_KEY. The source tarball's SHA-256 replaces SKIP.
set -eu
tag=$1
root=$(cd "$(dirname "$0")/../../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
printf '%s\n' "${AUR_SSH_PRIVATE_KEY:?AUR_SSH_PRIVATE_KEY is not set}" > "$work/key"
chmod 600 "$work/key"
echo 'aur.archlinux.org ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEuBKrPzbawxA/k2g6NcyV5jmqwJ2s+zpgZGZ7tpLIcN' > "$work/known_hosts"
export GIT_SSH_COMMAND="ssh -i $work/key -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=$work/known_hosts"
git clone -q ssh://aur@aur.archlinux.org/tarn.git "$work/aur"
git -C "$root" show "refs/tags/$tag:linux/packaging/arch/PKGBUILD" > "$work/PKGBUILD"
grep -qx "pkgver=${tag#v}" "$work/PKGBUILD" || { echo "the PKGBUILD of $tag is not version ${tag#v}" >&2; exit 1; }
curl -fsSL -o "$work/source.tar.gz" "https://github.com/Quavon-dev/tarn-browser/archive/refs/tags/$tag.tar.gz"
sum=$(sha256sum "$work/source.tar.gz" | cut -d' ' -f1)
sed "s/^sha256sums=.*/sha256sums=('$sum')/" "$work/PKGBUILD" > "$work/aur/PKGBUILD"
docker run --rm --platform linux/amd64 -v "$work/aur":/pkg archlinux:latest@sha256:d4594bb2300cba11f41c2f27923eb56f63d9988909c402f9c95517d1e362c899 \
  sh -c 'useradd -m builder && cd /pkg && su builder -c "makepkg --printsrcinfo" > .SRCINFO'
cd "$work/aur"
git add PKGBUILD .SRCINFO
if git diff --cached --quiet; then
  echo "the AUR package is already at $tag"
  exit 0
fi
git -c user.name=Tarn -c user.email=accounts@quavon.de commit -q -m "Update to ${tag#v}"
git push -q origin HEAD:master
