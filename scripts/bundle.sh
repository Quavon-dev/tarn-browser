#!/usr/bin/env bash
# TARN_PROFILE=debug bundles a debug build (tests: it reads TARN_TEST_FILTER_KEY).
# The bundle embeds the CEF framework and helpers and runs without the App Sandbox.
# TARN_EXTRA_FEATURES adds cargo features, e.g. app/translation (needs the macOS 26 SDK).
# TARN_SIGN_IDENTITY=none leaves every binary unsigned, for the signing Mac (docs/release.md).
# TARN_LOCAL_CHROMIUM=1 lets a bundle without a reachable update feed open Chromium until 28 days
# after its CEF release (browser_core::staleness::LOCAL_BUILD); never for a release build.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

version="$(cargo pkgid -p app)"
version="${version##*[#@]}"
profile="${TARN_PROFILE:-release}"
[[ "$profile" == release || "$profile" == debug ]] || { echo "TARN_PROFILE must be release or debug" >&2; exit 1; }
build=(--locked)
[[ "$profile" == release ]] && build+=(--release)
identity="${TARN_SIGN_IDENTITY:-}"
if [[ -z "$identity" || "$identity" == "-" ]]; then
  echo "TARN_SIGN_IDENTITY must name a signing identity or be none: an ad-hoc signature cannot load the framework" >&2
  exit 1
fi

sparkle="$(scripts/fetch-sparkle.sh)"
CEF_PATH="$(scripts/fetch-cef.sh)"
export CEF_PATH
cargo build "${build[@]}" -p app -p chromium-helper --features "app/chromium,chromium-helper/chromium${TARN_EXTRA_FEATURES:+,$TARN_EXTRA_FEATURES}"

app="target/$profile/Tarn.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "target/$profile/tarn" "$app/Contents/MacOS/tarn"
cp packaging/Tarn.icns "$app/Contents/Resources/Tarn.icns"
cp "$sparkle/LICENSE" "$app/Contents/Resources/Sparkle-LICENSE.txt"
cp "$CEF_PATH/CREDITS.html" "$app/Contents/Resources/Chromium-CREDITS.html"
cp "$CEF_PATH/LICENSE.txt" "$app/Contents/Resources/CEF-LICENSE.txt"
cp packaging/third-party-licenses.txt "$app/Contents/Resources/Third-Party-Licenses.txt"
cp LICENSE "$app/Contents/Resources/LICENSE.txt"
sed "s/@VERSION@/$version/g" packaging/Info.plist > "$app/Contents/Info.plist"
conf() { grep -E "^$1=" packaging/updates.conf | head -n1 | cut -d= -f2- || true; }
feed="$(conf feed_url)"
key="$(conf public_ed_key)"
# Only release bundles update themselves: Sparkle would replace a development bundle with a release.
if [[ "$profile" == release && -n "$feed$key" ]]; then
  feed_re='^https://[A-Za-z0-9.-]+(:[0-9]+)?/[!-~]*$'
  [[ "$feed" =~ $feed_re ]] || { echo "updates.conf: feed_url must be an https URL" >&2; exit 1; }
  [[ "$key" =~ ^[A-Za-z0-9+/]{43}=$ ]] || { echo "updates.conf: public_ed_key must be Sparkle's base64 EdDSA public key" >&2; exit 1; }
  plutil -insert SUFeedURL -string "$feed" "$app/Contents/Info.plist"
  plutil -insert SUPublicEDKey -string "$key" "$app/Contents/Info.plist"
fi
plutil -lint "$app/Contents/Info.plist" >/dev/null

sign() { [[ "$identity" == none ]] || codesign --force --options runtime --timestamp -s "$identity" "$@"; }

frameworks="$app/Contents/Frameworks"
mkdir -p "$frameworks"
ditto "$sparkle/Sparkle.framework" "$frameworks/Sparkle.framework"
updater="$frameworks/Sparkle.framework/Versions/B"
# The unsandboxed app downloads updates itself, so Sparkle's downloader service goes.
rm -rf "$updater/XPCServices/Downloader.xpc"
sign "$updater/XPCServices/Installer.xpc" "$updater/Autoupdate" "$updater/Updater.app"
sign "$frameworks/Sparkle.framework"

cp -R "$CEF_PATH/Chromium Embedded Framework.framework" "$frameworks/"
sign "$frameworks/Chromium Embedded Framework.framework/Libraries/"*.dylib
sign "$frameworks/Chromium Embedded Framework.framework"
for flavor in "" " (GPU)" " (Renderer)" " (Plugin)" " (Alerts)"; do
  name="tarn Helper$flavor"
  helper="$frameworks/$name.app"
  mkdir -p "$helper/Contents/MacOS"
  cp "target/$profile/tarn-chromium-helper" "$helper/Contents/MacOS/$name"
  suffix="$(tr -d ' ()' <<<"$flavor" | tr '[:upper:]' '[:lower:]')"
  cat > "$helper/Contents/Info.plist" <<P
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>dev.tarn.browser.helper${suffix:+.$suffix}</string>
<key>CFBundleExecutable</key><string>$name</string>
<key>CFBundleName</key><string>$name</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>$version</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>LSUIElement</key><true/>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
P
  if [[ "$flavor" == " (GPU)" || "$flavor" == " (Renderer)" ]]; then
    sign --entitlements packaging/Helper-JIT.entitlements "$helper"
  else
    sign "$helper"
  fi
done
sign "$app"
[[ "$identity" == none ]] || codesign --verify --deep --strict "$app"

echo "$root/$app"
