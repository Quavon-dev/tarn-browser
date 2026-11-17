#!/usr/bin/env bash
# Every refusal of dmg.sh before it builds an image, and the styled image dmg-layout.sh builds from a
# stub app (drives Finder); no network, keychain or signing identity.
set -euo pipefail
script="$(cd "$(dirname "$0")" && pwd)/dmg.sh"
dir="$(mktemp -d)"
trap 'hdiutil detach -quiet -force "$dir/mnt" 2>/dev/null || true; rm -rf "$dir"' EXIT
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

layout="$(dirname "$script")/dmg-layout.sh"
refused layout-other-app bash "$layout" "$dir/Other.app" "$dir/out.dmg"
refused layout-not-dmg bash "$layout" "$dir/Tarn.app" "$dir/out.zip"

cp "$(dirname "$script")/../packaging/Tarn.icns" "$dir/Tarn.app/Contents/"
bash "$layout" "$dir/Tarn.app" "$dir/styled.dmg"
hdiutil attach -quiet -nobrowse -readonly -mountpoint "$dir/mnt" "$dir/styled.dmg"
items=("$dir/mnt"/*)
[[ "${items[*]##*/}" == "Applications Tarn.app" ]] || { echo "FAIL layout: holds ${items[*]##*/}" >&2; failed=1; }
cmp -s "$dir/mnt/.background/background.tiff" "$(dirname "$script")/../packaging/dmg-background.tiff" \
  || { echo "FAIL layout: background missing" >&2; failed=1; }
[[ -f "$dir/mnt/.VolumeIcon.icns" && "$(GetFileInfo -ac "$dir/mnt")" == 1 ]] || { echo "FAIL layout: no volume icon" >&2; failed=1; }
if ! grep -q Iloc "$dir/mnt/.DS_Store" || ! grep -q icvp "$dir/mnt/.DS_Store"; then echo "FAIL layout: no window layout" >&2; failed=1; fi
hdiutil detach -quiet "$dir/mnt"

[[ "$failed" == 0 ]] && echo "dmg: all cases pass"
exit "$failed"
