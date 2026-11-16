#!/usr/bin/env bash
# Fails unless every client's version file carries the version of TAG (v1.2.3 -> 1.2.3).
set -euo pipefail
cd "$(dirname "$0")/.."
tag="${1:?usage: check-version.sh TAG}"
want="${tag#v}"
status=0
check() {
  local value
  value="$(sed -n "$2" "$1" | head -n1)"
  [[ "$value" == "$want" ]] || { echo "$1 has version '${value}', the tag $tag needs $want" >&2; status=1; }
}
check Cargo.toml '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p'
check linux/Cargo.toml 's/^version = "\(.*\)"$/\1/p'
check mobile/Cargo.toml '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p'
check mobile/app/nsh.json 's/^  "version": "\(.*\)",$/\1/p'
check mobile/android/host/build.gradle.kts 's/^ *versionName = "\(.*\)"$/\1/p'
check linux/packaging/rpm/tarn.spec 's/^Version: *\(.*\)$/\1/p'
check linux/packaging/arch/PKGBUILD 's/^pkgver=\(.*\)$/\1/p'
awk -v head="## [$want]" 'index($0, head) == 1 { found = 1; next } /^## \[/ { found = 0 } found && !/^\[[^]]+\]: /' CHANGELOG.md | grep -q '[^[:space:]]' \
  || { echo "CHANGELOG.md has no '## [$want]' section with entries; the release notes come from it" >&2; status=1; }
exit "$status"
