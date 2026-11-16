#!/usr/bin/env bash
# Signs a Tarn.app that scripts/bundle.sh built, inside out, with TARN_SIGN_IDENTITY: hardened
# runtime and a secure timestamp on every binary, the JIT entitlements on the GPU and renderer
# helpers, camera and microphone on the plain helper and the app (scripts/check-entitlements.sh).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
app="${1:-}"
identity="${TARN_SIGN_IDENTITY:-}"
[[ -d "$app" && "$(basename "$app")" == Tarn.app ]] || { echo "usage: scripts/codesign-app.sh <Tarn.app>" >&2; exit 1; }
[[ -n "$identity" && "$identity" != - && "$identity" != none ]] || { echo "set TARN_SIGN_IDENTITY to the Developer ID Application identity" >&2; exit 1; }

sign() { codesign --force --options runtime --timestamp -s "$identity" "$@"; }

frameworks="$app/Contents/Frameworks"
updater="$frameworks/Sparkle.framework/Versions/B"
sign "$updater/XPCServices/Installer.xpc" "$updater/Autoupdate" "$updater/Updater.app"
sign "$frameworks/Sparkle.framework"
sign "$frameworks/Chromium Embedded Framework.framework/Libraries/"*.dylib
sign "$frameworks/Chromium Embedded Framework.framework"
for flavor in "" " (GPU)" " (Renderer)" " (Plugin)" " (Alerts)"; do
  helper="$frameworks/tarn Helper$flavor.app"
  case "$flavor" in
    " (GPU)" | " (Renderer)") sign --entitlements "$root/packaging/Helper-JIT.entitlements" "$helper" ;;
    # Chromium captures audio and video in utility processes of the plain helper.
    "") sign --entitlements "$root/packaging/Tarn.entitlements" "$helper" ;;
    *) sign "$helper" ;;
  esac
done
# Sites the user allows may use the camera and microphone (WebKit captures in the app's process).
sign --entitlements "$root/packaging/Tarn.entitlements" "$app"
codesign --verify --deep --strict "$app"
