#!/usr/bin/env bash
# The feed format and windows-feed.sh's refusals of bad arguments; no network or keys.
set -euo pipefail
script="$(cd "$(dirname "$0")" && pwd)/windows-feed.sh"
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
failed=0
digest=$(printf 'a%.0s' {1..64})
url=https://github.com/Quavon-dev/tarn-browser/releases/download/v0.4.0/Tarn-v0.4.0-windows-x64-setup-unsigned.exe
touch "$dir/Tarn-v0.4.0-windows-x64-setup-unsigned.exe"

feed=$(bash -c 'source "$1"; feed_json 0.4.0 false "$2" "$3" 123 SIG' windows-feed-test "$script" "$url" "$digest")
[[ "$feed" == '{"version":"0.4.0","critical":false,"installer":{"url":"'"$url"'","sha256":"'"$digest"'","length":123,"signature":"SIG"}}' ]] \
  || { echo "FAIL feed format: $feed" >&2; failed=1; }
nightly=$(bash -c 'source "$1"; feed_json 0.4.0.7 false "$2" "$3" 123 SIG' windows-feed-test "$script" "$url" "$digest")
[[ "$nightly" == '{"version":"0.4.0.7","channel":"nightly","critical":false,"installer":{"url":"'"$url"'","sha256":"'"$digest"'","length":123,"signature":"SIG"}}' ]] \
  || { echo "FAIL nightly feed format: $nightly" >&2; failed=1; }

refused() {
  local name="$1"
  shift
  if env TARN_ED_KEY_FILE="$dir/missing" bash "$script" "$@" >"$dir/out" 2>&1; then
    echo "FAIL $name: not refused" >&2
    failed=1
  fi
}
installer="$dir/Tarn-v0.4.0-windows-x64-setup-unsigned.exe"
refused no-installer "$dir/none.exe" 0.4.0 "$url"
refused bad-version "$installer" 0.4 "$url"
refused other-host "$installer" 0.4.0 "https://example.com/Tarn-v0.4.0-windows-x64-setup-unsigned.exe"
refused other-file "$installer" 0.4.0 "${url%/*}/other.exe"
refused quote-in-url "$installer" 0.4.0 "${url%/*}/\"/Tarn-v0.4.0-windows-x64-setup-unsigned.exe"
refused bad-option "$installer" 0.4.0 "$url" --later
refused no-key "$installer" 0.4.0 "$url"
grep -q "TARN_ED_KEY_FILE" "$dir/out" || { echo "FAIL no-key: $(cat "$dir/out")" >&2; failed=1; }

[[ "$failed" == 0 ]] || exit 1
echo "windows-feed.test.sh: all passed"
