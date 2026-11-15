#!/bin/sh
# Builds the signed apt and dnf repositories from release packages inside a Debian image:
# build.sh <packages-dir> <site-dir>. The key is TARN_REPO_GPG_KEY (armored secret key) with
# TARN_REPO_GPG_PASSPHRASE and must match tarn.asc next to this script.
set -eu
in=$1
mkdir -p "$2"
out=$(cd "$2" && pwd)
public=$(cd "$(dirname "$0")" && pwd)/tarn.asc
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends apt-utils createrepo-c rpm gnupg >/dev/null

GNUPGHOME=$(mktemp -d)
export GNUPGHOME
printf '%s' "${TARN_REPO_GPG_PASSPHRASE:-}" > "$GNUPGHOME/passphrase"
sign_args="--batch --pinentry-mode loopback --passphrase-file $GNUPGHOME/passphrase"
printf '%s\n' "${TARN_REPO_GPG_KEY:?TARN_REPO_GPG_KEY is not set}" | gpg --batch --quiet --import
key=$(gpg --with-colons --list-secret-keys | awk -F: '$1 == "fpr" { print $10; exit }')
expected=$(gpg --with-colons --show-keys "$public" 2>/dev/null | awk -F: '$1 == "fpr" { print $10; exit }')
[ -n "$expected" ] || { echo "$public holds no public key" >&2; exit 1; }
[ "$key" = "$expected" ] || { echo "the signing key is not the key in $public" >&2; exit 1; }
sign() { gpg --batch --pinentry-mode loopback --passphrase-file "$GNUPGHOME/passphrase" --local-user "$key" "$@"; }

cp "$public" "$out/tarn.asc"
count=0
for file in "$in"/*.deb "$in"/*.rpm; do
  [ -e "$file" ] || continue
  name=${file##*/}
  case $name in
    tarn_*_*_*.deb)
      suite=$(printf '%s' "$name" | cut -d_ -f3)
      printf '%s' "$suite" | grep -Eqx '[a-z]+[0-9.]+' || { echo "unexpected package name $name" >&2; exit 1; }
      mkdir -p "$out/apt/pool/$suite"
      cp "$file" "$out/apt/pool/$suite/" ;;
    tarn-*.fc[0-9]*.*.rpm)
      release=$(printf '%s' "$name" | sed -n 's/^tarn-.*\.fc\([0-9][0-9]*\)\.\([a-z0-9_]*\)\.rpm$/\1/p')
      arch=$(rpm -qp --qf '%{ARCH}' "$file")
      printf '%s' "$arch" | grep -Eqx '[a-z0-9_]+' || { echo "unexpected architecture in $name" >&2; exit 1; }
      if [ -z "$release" ] || [ "$name" != "$(rpm -qp --qf '%{NAME}-%{VERSION}-%{RELEASE}.%{ARCH}' "$file").rpm" ]; then
        echo "unexpected package name $name" >&2
        exit 1
      fi
      mkdir -p "$out/rpm/fedora/$release/$arch"
      cp "$file" "$out/rpm/fedora/$release/$arch/"
      rpmsign --addsign --define "_gpg_name $key" --define "_gpg_sign_cmd_extra_args $sign_args" \
        "$out/rpm/fedora/$release/$arch/$name" >/dev/null ;;
    *) echo "unexpected package name $name" >&2; exit 1 ;;
  esac
  count=$((count + 1))
done
[ "$count" -gt 0 ] || { echo "no packages in $in" >&2; exit 1; }

if [ -d "$out/apt/pool" ]; then
  cd "$out/apt"
  for pool in pool/*; do
    suite=${pool#pool/}
    archs=$(find "$pool" -name '*.deb' | sed 's/.*_\([^_]*\)\.deb$/\1/' | sort -u | tr '\n' ' ')
    archs=${archs% }
    for arch in $archs; do
      printf '%s' "$arch" | grep -Eqx '[a-z0-9]+' || { echo "unexpected architecture $arch" >&2; exit 1; }
      mkdir -p "dists/$suite/main/binary-$arch"
      apt-ftparchive --arch "$arch" packages "$pool" > "dists/$suite/main/binary-$arch/Packages"
      gzip -9kn "dists/$suite/main/binary-$arch/Packages"
    done
    apt-ftparchive -o APT::FTPArchive::Release::Origin=Tarn -o APT::FTPArchive::Release::Label=Tarn \
      -o "APT::FTPArchive::Release::Suite=$suite" -o "APT::FTPArchive::Release::Codename=$suite" \
      -o "APT::FTPArchive::Release::Architectures=$archs" \
      -o APT::FTPArchive::Release::Components=main release "dists/$suite" > "$GNUPGHOME/Release"
    mv "$GNUPGHOME/Release" "dists/$suite/Release"
    sign --clearsign -o "dists/$suite/InRelease" "dists/$suite/Release"
    sign --detach-sign --armor -o "dists/$suite/Release.gpg" "dists/$suite/Release"
  done
fi

for dir in "$out"/rpm/fedora/*/*; do
  [ -d "$dir" ] || continue
  createrepo_c -q "$dir"
  sign --detach-sign --armor -o "$dir/repodata/repomd.xml.asc" "$dir/repodata/repomd.xml"
done
