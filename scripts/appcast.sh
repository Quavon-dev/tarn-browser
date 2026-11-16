#!/usr/bin/env bash
# Usage: scripts/appcast.sh <notarised Tarn-<tag>.zip> <tag> [--critical]
# Writes appcast.xml next to the archive: one item for it, the archive's EdDSA signature and the
# feed signature (generate_appcast signs the feed because Info.plist requires it), made with the
# update key in TARN_ED_KEY_FILE (Sparkle's private key file). <tag> is v1.2.3, whose app must be
# 1.2.3, or nightly, whose app must be TARN_BUILD_VERSION. release.yml uploads both files to the
# release <tag>; the feed URL in packaging/updates.conf follows the latest published release, and
# the Nightly channel reads the nightly release's.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
zip="${1:?usage: appcast.sh <zip> <tag> [--critical]}"
tag="${2:?usage: appcast.sh <zip> <tag> [--critical]}"
if [[ "$tag" =~ ^v([0-9]+\.[0-9]+\.[0-9]+)$ ]]; then
  version="${BASH_REMATCH[1]}"
elif [[ "$tag" == nightly && "${TARN_BUILD_VERSION:-}" =~ ^[0-9]+(\.[0-9]+){3}$ ]]; then
  version="$TARN_BUILD_VERSION"
else
  echo "tag must look like v1.2.3, or be nightly with TARN_BUILD_VERSION like 1.2.3.4" >&2
  exit 1
fi
key="${TARN_ED_KEY_FILE:-}"
[[ -f "$key" ]] || { echo "set TARN_ED_KEY_FILE to the update key file" >&2; exit 1; }
[[ -f "$zip" && "$zip" == *.zip ]] || { echo "$zip is not a zip archive" >&2; exit 1; }
flags=()
[[ "$tag" != nightly ]] || flags=(--channel nightly)
case "${3:-}" in
  --critical) flags+=(--critical-update-version "") ;;
  "") ;;
  *) echo "unknown option $3" >&2; exit 1 ;;
esac

bin="$("$root/scripts/fetch-sparkle.sh")/bin"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir "$work/feed"
cp "$zip" "$work/feed/"
ditto -x -k "$work/feed/$(basename "$zip")" "$work/unpacked"
app="$work/unpacked/Tarn.app"
[[ "$(ls -A "$work/unpacked")" == Tarn.app && -d "$app" && ! -L "$app" ]] || { echo "$zip must hold only the Tarn.app folder" >&2; exit 1; }
[[ "$(spctl -a -t execute -vv "$app" 2>&1 || true)" == *": accepted"$'\n'"source=Notarized Developer ID"* ]] \
  || { echo "Tarn.app in $zip is not notarised (run scripts/notarize.sh first)" >&2; exit 1; }
[[ "$(codesign -dv "$app" 2>&1 || true)" == *$'\nTeamIdentifier=CFV35FGSHF'* ]] || { echo "Tarn.app is not signed by team CFV35FGSHF" >&2; exit 1; }
[[ "$(plutil -extract CFBundleIdentifier raw "$app/Contents/Info.plist")" == de.quavon.tarn ]] || { echo "not de.quavon.tarn" >&2; exit 1; }
for field in CFBundleShortVersionString CFBundleVersion; do
  [[ "$(plutil -extract "$field" raw "$app/Contents/Info.plist")" == "$version" ]] || { echo "the app's $field is not $version" >&2; exit 1; }
done
"$bin/generate_appcast" --ed-key-file "$key" ${flags[@]+"${flags[@]}"} \
  --download-url-prefix "https://github.com/Quavon-dev/tarn-browser/releases/download/$tag/" \
  -o "$work/appcast.xml" "$work/feed"
"$bin/sign_update" --ed-key-file "$key" --verify "$work/appcast.xml"
grep -q "sparkle:edSignature" "$work/appcast.xml" || { echo "the appcast item carries no archive signature" >&2; exit 1; }
cp "$work/appcast.xml" "$(dirname "$zip")/appcast.xml"
echo "$(cd "$(dirname "$zip")" && pwd)/appcast.xml"
