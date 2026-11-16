#!/usr/bin/env bash
# Checks every signed binary of the bundle against its own entitlement list: no App Sandbox,
# allow-jit only in the GPU and renderer helpers, camera and microphone only in the app and the
# plain helper. Entries are "<key> <value>", one line per array element.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
app="${1:-$root/target/release/Tarn.app}"
frameworks="$app/Contents/Frameworks"

jit="com.apple.security.cs.allow-jit true"
media="com.apple.security.device.audio-input true
com.apple.security.device.camera true"

plist="$(mktemp)"
trap 'rm -f "$plist"' EXIT
status=0

check() {
  local target="$1" expected="$2"
  codesign -d --entitlements - --xml "$target" 2>/dev/null > "$plist" || true
  local actual=""
  [[ -s "$plist" ]] && actual="$(plutil -p "$plist" | awk '
    /=> \[$/ { key = $1; gsub(/"/, "", key); items = 0; next }
    /^ *\]$/ { if (!items) print key " []"; key = ""; next }
    key != "" { value = $3; gsub(/"/, "", value); print key " " value; items++; next }
    /=>/ { name = $1; gsub(/"/, "", name); print name " " $3 }' | sort)"
  local wanted=""
  [[ -n "$expected" ]] && wanted="$(printf '%s\n' "$expected" | sort)"
  if [[ "$actual" != "$wanted" ]]; then
    echo "entitlements of ${target#"$app/"} differ from the expected set" >&2
    diff <(echo "$wanted") <(echo "$actual") >&2 || true
    status=1
  fi
  [[ "$(codesign -d --verbose=2 "$target" 2>&1)" =~ flags=[^$'\n']*runtime ]] \
    || { echo "hardened runtime not enabled on ${target#"$app/"}" >&2; status=1; }
}

sparkle="$frameworks/Sparkle.framework"
if [[ -d "$sparkle" ]]; then
  check "$sparkle" ""
  for part in Autoupdate Updater.app XPCServices/Installer.xpc; do check "$sparkle/Versions/B/$part" ""; done
  [[ "$(ls -A "$sparkle/Versions/B/XPCServices")" == Installer.xpc ]] \
    || { echo "Sparkle must ship only its installer service" >&2; status=1; }
else
  echo "the bundle has no Sparkle.framework" >&2
  status=1
fi
[[ "$(plutil -extract SURequireSignedFeed raw "$app/Contents/Info.plist" 2>/dev/null)" == true ]] \
  || { echo "Info.plist must set SURequireSignedFeed" >&2; status=1; }

framework="$frameworks/Chromium Embedded Framework.framework"
[[ -d "$framework" ]] || { echo "the bundle has no CEF framework" >&2; exit 1; }
check "$app" "$media"
check "$framework" ""
for lib in "$framework/Libraries/"*.dylib; do check "$lib" ""; done
for flavor in "" " (GPU)" " (Renderer)" " (Plugin)" " (Alerts)"; do
  helper="$frameworks/tarn Helper$flavor.app"
  [[ -d "$helper" ]] || { echo "missing ${helper#"$app/"}" >&2; status=1; continue; }
  if [[ "$flavor" == " (GPU)" || "$flavor" == " (Renderer)" ]]; then check "$helper" "$jit"
  elif [[ -z "$flavor" ]]; then check "$helper" "$media"
  else check "$helper" ""; fi
done
expected_entries="Chromium Embedded Framework.framework
Sparkle.framework
tarn Helper (Alerts).app
tarn Helper (GPU).app
tarn Helper (Plugin).app
tarn Helper (Renderer).app
tarn Helper.app"
[[ "$(ls -A "$frameworks" | LC_ALL=C sort)" == "$(LC_ALL=C sort <<<"$expected_entries")" ]] \
  || { echo "Contents/Frameworks holds other entries than the two frameworks and five helpers" >&2; status=1; }
[[ "$(ls -A "$app/Contents/MacOS")" == tarn ]] || { echo "Contents/MacOS must hold only tarn" >&2; status=1; }

[[ "$status" == 0 ]] && echo "entitlements ok"
exit "$status"
