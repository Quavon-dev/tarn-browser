#!/usr/bin/env bash
# Every refusal of dmg.sh before it builds an image; no network, keychain or signing identity.
set -euo pipefail
script="$(cd "$(dirname "$0")" && pwd)/dmg.sh"
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
mkdir -p "$dir/Tarn.app/Contents" "$dir/Other.app"
failed=0

refused() {
  local name="$1"
  shift
  if env "$@" >"$dir/out" 2>&1; then echo "FAIL $name: expected a refusal" >&2; failed=1; fi
  [[ ! -e "$dir/out.dmg" ]] || { echo "FAIL $name: wrote an image" >&2; failed=1; }
}

ok=(TARN_SIGN_IDENTITY="Developer ID Application: Test" TARN_NOTARY_PROFILE=tarn-notary)
refused no-args "${ok[@]}" bash "$script"
refused other-app "${ok[@]}" bash "$script" "$dir/Other.app" "$dir/out.dmg"
refused not-dmg "${ok[@]}" bash "$script" "$dir/Tarn.app" "$dir/out.zip"
refused no-identity TARN_NOTARY_PROFILE=tarn-notary bash "$script" "$dir/Tarn.app" "$dir/out.dmg"
refused adhoc-identity TARN_SIGN_IDENTITY=- TARN_NOTARY_PROFILE=tarn-notary bash "$script" "$dir/Tarn.app" "$dir/out.dmg"
refused no-profile TARN_SIGN_IDENTITY="Developer ID Application: Test" bash "$script" "$dir/Tarn.app" "$dir/out.dmg"
refused bad-profile TARN_SIGN_IDENTITY="Developer ID Application: Test" "TARN_NOTARY_PROFILE=a b" bash "$script" "$dir/Tarn.app" "$dir/out.dmg"
refused not-notarised "${ok[@]}" bash "$script" "$dir/Tarn.app" "$dir/out.dmg"
grep -q "not notarised" "$dir/out" || { echo "FAIL not-notarised: $(cat "$dir/out")" >&2; failed=1; }

[[ "$failed" == 0 ]] && echo "dmg: all cases pass"
exit "$failed"
