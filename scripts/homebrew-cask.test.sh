#!/usr/bin/env bash
# The rendered cask and homebrew-cask.sh's refusals of bad arguments; no network.
set -euo pipefail
script="$(cd "$(dirname "$0")" && pwd)/homebrew-cask.sh"
failed=0
digest=$(printf 'a%.0s' {1..64})
cask="$("$script" v1.2.3 "$digest" Quavon-dev/tarn-browser)"
grep -qx '  version "1.2.3"' <<<"$cask" || { echo "FAIL version" >&2; failed=1; }
grep -qx "  sha256 \"$digest\"" <<<"$cask" || { echo "FAIL sha256" >&2; failed=1; }
grep -qF 'url "https://github.com/Quavon-dev/tarn-browser/releases/download/v#{version}/Tarn-v#{version}.dmg",' <<<"$cask" || { echo "FAIL url" >&2; failed=1; }
! grep -qE '@[A-Z0-9_]+@' <<<"$cask" || { echo "FAIL placeholder left" >&2; failed=1; }

refused() {
  local name="$1"
  shift
  if "$script" "$@" >/dev/null 2>&1; then
    echo "FAIL $name: not refused" >&2
    failed=1
  fi
}
refused nightly nightly "$digest" Quavon-dev/tarn-browser
refused no-v 1.2.3 "$digest" Quavon-dev/tarn-browser
refused short-sha v1.2.3 abc Quavon-dev/tarn-browser
refused upper-sha v1.2.3 "$(tr a A <<<"$digest")" Quavon-dev/tarn-browser
refused bad-repo v1.2.3 "$digest" 'Quavon-dev/tarn|browser'
refused missing v1.2.3 "$digest"

[[ "$failed" == 0 ]] || exit 1
echo "homebrew-cask.test.sh: all passed"
