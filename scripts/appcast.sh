#!/usr/bin/env bash
# Usage: scripts/appcast.sh <notarised Tarn-<tag>.zip> <tag> [--critical]
# Writes appcast.xml next to the archive: one item for it, the archive's EdDSA signature and the
# feed signature (generate_appcast signs the feed because Info.plist requires it), made with the private key in the login keychain (account dev.tarn.browser).
# Upload both files to the GitHub release <tag>; the feed URL in packaging/updates.conf follows the
# latest published release.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
zip="${1:?usage: appcast.sh <zip> <tag> [--critical]}"
tag="${2:?usage: appcast.sh <zip> <tag> [--critical]}"
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "tag must look like v1.2.3" >&2; exit 1; }
[[ -f "$zip" && "$zip" == *.zip ]] || { echo "$zip is not a zip archive" >&2; exit 1; }
critical=()
case "${3:-}" in
  --critical) critical=(--critical-update-version "") ;;
  "") ;;
  *) echo "unknown option $3" >&2; exit 1 ;;
esac

bin="$("$root/scripts/fetch-sparkle.sh")/bin"
account=dev.tarn.browser
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
[[ "$(plutil -extract CFBundleIdentifier raw "$app/Contents/Info.plist")" == dev.tarn.browser ]] || { echo "not dev.tarn.browser" >&2; exit 1; }
for key in CFBundleShortVersionString CFBundleVersion; do
  [[ "v$(plutil -extract "$key" raw "$app/Contents/Info.plist")" == "$tag" ]] || { echo "the app's $key is not $tag" >&2; exit 1; }
done
"$bin/generate_appcast" --account "$account" ${critical[@]+"${critical[@]}"} \
  --download-url-prefix "https://github.com/Quavon-dev/tarn-browser/releases/download/$tag/" \
  -o "$work/appcast.xml" "$work/feed"
"$bin/sign_update" --account "$account" --verify "$work/appcast.xml"
grep -q "sparkle:edSignature" "$work/appcast.xml" || { echo "the appcast item carries no archive signature" >&2; exit 1; }
cp "$work/appcast.xml" "$(dirname "$zip")/appcast.xml"
echo "$(cd "$(dirname "$zip")" && pwd)/appcast.xml"
