#!/usr/bin/env bash
# Notarises and staples a Developer ID signed bundle. Credentials come only from the keychain
# profile named by TARN_NOTARY_PROFILE (`xcrun notarytool store-credentials`).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
app="${1:-$root/target/release/Tarn.app}"
profile="${TARN_NOTARY_PROFILE:-}"
[[ "$profile" =~ ^[A-Za-z0-9._-]+$ ]] || { echo "set TARN_NOTARY_PROFILE to a notarytool keychain profile name" >&2; exit 1; }
[[ -d "$app" ]] || { echo "no bundle at $app" >&2; exit 1; }

info="$(codesign -d --verbose=2 "$app" 2>&1)" || { echo "$info" >&2; exit 1; }
[[ ! "$info" =~ flags=[^$'\n']*adhoc ]] || { echo "ad-hoc signature: build with TARN_SIGN_IDENTITY set" >&2; exit 1; }
[[ "$info" == *$'\n'"Authority=Developer ID Application: "* ]] || { echo "not signed with a Developer ID Application certificate" >&2; exit 1; }
[[ "$info" == *$'\n'"Timestamp="* ]] || { echo "the signature has no secure timestamp" >&2; exit 1; }
[[ "$info" =~ flags=[^$'\n']*runtime ]] || { echo "hardened runtime not enabled" >&2; exit 1; }
codesign --verify --deep --strict "$app"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
zip="$tmp/$(basename "$app" .app).zip"
ditto -c -k --keepParent "$app" "$zip"
result="$(xcrun notarytool submit "$zip" --keychain-profile "$profile" --wait --output-format json)"
status="$(plutil -extract status raw -o - - <<<"$result")"
if [[ "$status" != Accepted ]]; then
  id="$(plutil -extract id raw -o - - <<<"$result" 2>/dev/null || echo "<submission-id>")"
  echo "notarisation $status; read the log with: xcrun notarytool log $id --keychain-profile $profile" >&2
  exit 1
fi

xcrun stapler staple "$app"
xcrun stapler validate "$app"
assessment="$(spctl -a -vv -t execute "$app" 2>&1)" || true
echo "$assessment"
[[ "$assessment" == *"source=Notarized Developer ID"* ]] || { echo "Gatekeeper does not accept the bundle as notarised" >&2; exit 1; }
echo "notarised and stapled: $app"
