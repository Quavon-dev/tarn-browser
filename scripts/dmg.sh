#!/usr/bin/env bash
# Packs a notarised Tarn.app into a drag-to-install disk image (dmg-layout.sh), then signs,
# notarises and staples the image. Same credentials as notarize.sh:
#
#   TARN_SIGN_IDENTITY=<Developer ID Application> TARN_NOTARY_PROFILE=<profile> scripts/dmg.sh APP OUT.dmg
set -euo pipefail

app="${1:-}"
out="${2:-}"
identity="${TARN_SIGN_IDENTITY:-}"
profile="${TARN_NOTARY_PROFILE:-}"
[[ -d "$app" && "$(basename "$app")" == Tarn.app && "$out" == *.dmg ]] || { echo "usage: scripts/dmg.sh <Tarn.app> <out.dmg>" >&2; exit 1; }
[[ -n "$identity" && "$identity" != - ]] || { echo "set TARN_SIGN_IDENTITY to the Developer ID Application identity" >&2; exit 1; }
[[ "$profile" =~ ^[A-Za-z0-9._-]+$ ]] || { echo "set TARN_NOTARY_PROFILE to a notarytool keychain profile name" >&2; exit 1; }
assessment="$(spctl -a -vv -t execute "$app" 2>&1)" && [[ "$assessment" == *"source=Notarized Developer ID"* ]] \
  || { echo "$app is not notarised: run scripts/notarize.sh first" >&2; exit 1; }
codesign --verify --deep --strict -R '=anchor apple generic and certificate leaf[subject.OU] = "CFV35FGSHF"' "$app" \
  || { echo "$app is not signed by team CFV35FGSHF" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'hdiutil detach -quiet "$tmp/mnt" 2>/dev/null || true; rm -rf "$tmp"' EXIT
"$(dirname "$0")/dmg-layout.sh" "$app" "$out"
codesign --force --timestamp -s "$identity" "$out"

result="$(xcrun notarytool submit "$out" --keychain-profile "$profile" --wait --output-format json)"
[[ "$(plutil -extract status raw -o - - <<<"$result")" == Accepted ]] || { echo "notarisation of $out: $result" >&2; exit 1; }
xcrun stapler staple "$out"
assessment="$(spctl -a -vv -t open --context context:primary-signature "$out" 2>&1)" && [[ "$assessment" == *"source=Notarized Developer ID"* ]] \
  || { echo "Gatekeeper does not accept $out as notarised" >&2; exit 1; }

hdiutil attach -quiet -nobrowse -readonly -mountpoint "$tmp/mnt" "$out"
items=("$tmp/mnt"/* "$tmp/mnt"/.[!.]*)
[[ "${items[*]##*/}" == "Applications Tarn.app .DS_Store .VolumeIcon.icns .background" \
  && "$(readlink "$tmp/mnt/Applications")" == /Applications ]] \
  || { echo "$out does not hold exactly Tarn.app, the Applications link and the window layout" >&2; exit 1; }
codesign --verify --deep --strict "$tmp/mnt/Tarn.app"
echo "signed, notarised and stapled: $out"
