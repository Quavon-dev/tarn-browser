#!/usr/bin/env bash
# Every refusal of sign-release.sh against fixtures; no network, keychain or signing identity.
set -euo pipefail
script="$(cd "$(dirname "$0")" && pwd)/sign-release.sh"
template="$(cd "$(dirname "$0")/.." && pwd)/packaging/Info.plist"
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
failed=0
repo=Quavon-dev/tarn-browser
url="https://github.com/$repo"

expect() {
  local want="$1" name="$2" got
  shift 2
  if bash -c 'source "$1"; shift; "$@"' sign-release-test "$script" "$@" >"$dir/out" 2>&1; then got=pass; else got=refused; fi
  [[ "$got" == "$want" ]] || { echo "FAIL $name: expected $want, got $got: $(cat "$dir/out")" >&2; failed=1; }
  [[ -z "${VERBOSE:-}" ]] || echo "$name: $got $(tail -n1 "$dir/out")"
}

expect refused no-inputs main
expect refused no-config main --config "$dir/missing.conf" v0.1.1

lock() {
  printf '%s\n' "cef_version=156.0.27+gabcdef0+chromium-156.0.8078.99" "chromium_version=156.0.8078.99" \
    "release_date=${1:-2026-11-10}" "host=${2:-cef-builds.spotifycdn.com}" \
    "sha256_macosarm64=$(printf 'a%.0s' {1..64})" "sha256_macosx64=$(printf 'b%.0s' {1..64})" \
    "sha256_windows64=$(printf 'c%.0s' {1..64})" "${@:3}" > "$dir/cef.lock"
  echo "$dir/cef.lock"
}
expect pass lock check_lock "$(lock)" 2026-11-14
expect refused lock-host check_lock "$(lock 2026-11-10 evil.example)" 2026-11-14
expect refused lock-future-date check_lock "$(lock 2026-11-15)" 2026-11-14
expect refused lock-extra-key check_lock "$(lock 2026-11-10 cef-builds.spotifycdn.com extra=1)" 2026-11-14

sparkle_lock() {
  printf '%s\n' "sparkle_version=2.10.1" "release_date=${2:-2026-10-25}" "host=${1:-github.com}" "sha256=$(printf 'a%.0s' {1..64})" "${@:3}" > "$dir/sparkle.lock"
  echo "$dir/sparkle.lock"
}
expect pass sparkle-lock check_sparkle_lock "$(sparkle_lock)"
expect refused sparkle-lock-host check_sparkle_lock "$(sparkle_lock evil.example)"
expect refused sparkle-lock-bad-date check_sparkle_lock "$(sparkle_lock github.com soon)"
expect refused sparkle-lock-extra-key check_sparkle_lock "$(sparkle_lock github.com 2026-10-25 extra=1)"

google() {
  jq -n --arg d "$1" '{releases: [
    {name: "chrome/platforms/mac/channels/extended/versions/156.0.8078.99/releases/2", version: "156.0.8078.99", serving: {startTime: ($d + "T18:00:00Z")}},
    {name: "chrome/platforms/mac/channels/stable/versions/156.0.8078.99/releases/3", version: "156.0.8078.99", serving: {startTime: "2026-11-12T18:00:00Z"}},
    {name: "chrome/platforms/mac/channels/beta/versions/156.0.8078.99/releases/1", version: "156.0.8078.99", serving: {startTime: "2026-11-01T18:00:00Z"}}]}' > "$dir/google.json"
  echo "$dir/google.json"
}
expect pass google check_google "$(google 2026-11-10)" 156.0.8078.99 2026-11-10
expect refused google-other-date check_google "$(google 2026-11-11)" 156.0.8078.99 2026-11-10
expect refused google-not-listed check_google "$(google 2026-11-10)" 156.0.8078.100 2026-11-10

expect pass versions check_versions 0.1.1 0.1.2 156.0.8078.88 156.0.8078.99
expect pass versions-same-chromium check_versions 0.1.1 0.1.2 156.0.8078.88 156.0.8078.88
expect refused versions-older-chromium check_versions 0.1.1 0.1.2 156.0.8078.99 156.0.8078.88
expect refused versions-same-app check_versions 0.1.2 0.1.2 156.0.8078.88 156.0.8078.99
expect refused versions-older-app check_versions 0.1.10 0.1.9 156.0.8078.88 156.0.8078.99

git_repo="$dir/repo"
g() { git -C "$git_repo" -c user.name=t -c user.email=t@example.invalid -c commit.gpgsign=false -c tag.gpgsign=false "$@"; }
mkdir -p "$git_repo/packaging" "$git_repo/src" "$git_repo/.github/workflows"
g init -q -b main
cargo_lock() { printf '# @generated\nversion = 4\n\n[[package]]\nname = "app"\nversion = "%s"\n\n[[package]]\nname = "dep"\nversion = "%s"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n' "$1" "$2" > "$git_repo/Cargo.lock"; }
printf '[workspace.package]\nversion = "0.1.1"\n' > "$git_repo/Cargo.toml"
cargo_lock 0.1.1 0.1.1
echo "fn main() {}" > "$git_repo/src/main.rs"
echo "name: Release" > "$git_repo/.github/workflows/release.yml"
cp "$(lock 2026-11-07)" "$git_repo/packaging/cef.lock"
g add -A && g commit -qm signed && signed_commit="$(g rev-parse HEAD)"
cp "$(lock)" "$git_repo/packaging/cef.lock"
sed -i '' 's/0.1.1/0.1.2/' "$git_repo/Cargo.toml"
cargo_lock 0.1.2 0.1.1
g commit -qam engine && engine="$(g rev-parse HEAD)"
g tag -a v0.1.2 -m v0.1.2
echo "fn main() { evil() }" > "$git_repo/src/main.rs"
g commit -qam code && code="$(g rev-parse HEAD)"
g reset -q --hard "$engine"
cargo_lock 0.1.2 0.1.2
g commit -qam dep && dep="$(g rev-parse HEAD)"
echo "name: Release, patched" > "$git_repo/.github/workflows/release.yml"
g commit -qam hostile && hostile="$(g rev-parse HEAD)"
g update-ref refs/remotes/origin/main "$hostile"
g switch -q -c side "$signed_commit"
echo x > "$git_repo/packaging/other"
g add -A && g commit -qm side && side="$(g rev-parse HEAD)"
: > "$dir/allowed_signers"

expect pass diff-engine-only check_diff "$git_repo" "$signed_commit" "$engine" 0.1.1 0.1.2
expect refused diff-code check_diff "$git_repo" "$signed_commit" "$code" 0.1.1 0.1.2
expect refused diff-dependency check_diff "$git_repo" "$signed_commit" "$dep" 0.1.1 0.1.2
expect refused diff-other-file check_diff "$git_repo" "$signed_commit" "$side" 0.1.1 0.1.2
expect pass on-main check_ancestry "$git_repo" "$engine"
expect refused off-main check_ancestry "$git_repo" "$side"
expect refused unsigned-tag maintainer_signed "$git_repo" v0.1.2 "$dir/allowed_signers"

digest="$(printf 'c%.0s' {1..64})"
attestation() {
  local signer="$url/.github/workflows/release.yml@refs/heads/main" config="$url/.github/workflows/chromium-update.yml@refs/heads/main"
  local ref=refs/heads/main runner=github-hosted commit="$engine" type=https://github.com/Quavon-dev/tarn-browser/attestations/release/v1
  local signer_digest="$dep" config_digest="$dep" source_digest="$dep"
  [[ $# -eq 0 ]] || local "$@"
  jq -n --arg d "$digest" --arg signer "$signer" --arg config "$config" --arg ref "$ref" --arg runner "$runner" \
    --arg commit "$commit" --arg type "$type" --arg url "$url" \
    --arg signer_digest "$signer_digest" --arg config_digest "$config_digest" --arg source_digest "$source_digest" \
    '[{verificationResult: {signature: {certificate: {buildSignerURI: $signer, buildConfigURI: $config,
      sourceRepositoryRef: $ref, sourceRepositoryURI: $url, runnerEnvironment: $runner,
      buildSignerDigest: $signer_digest, buildConfigDigest: $config_digest, sourceRepositoryDigest: $source_digest}},
      statement: {subject: [{name: "Tarn-v0.1.2-unsigned.zip", digest: {sha256: $d}}], predicateType: $type,
      predicate: {commit: $commit, tag: "v0.1.2"}}}}]' > "$dir/attestation.json"
  echo "$dir/attestation.json"
}
tagged="$url/.github/workflows/release.yml@refs/tags/v0.1.2"
expect pass attestation check_attestation "$(attestation)" "$digest" "$engine" "$repo" v0.1.2 false "$git_repo"
expect refused attestation-other-workflow check_attestation "$(attestation signer="$url/.github/workflows/ci.yml@refs/heads/main")" "$digest" "$engine" "$repo" v0.1.2 false "$git_repo"
expect refused attestation-other-caller check_attestation "$(attestation config="$url/.github/workflows/release.yml@refs/heads/main")" "$digest" "$engine" "$repo" v0.1.2 false "$git_repo"
expect refused attestation-other-branch check_attestation "$(attestation signer="$url/.github/workflows/release.yml@refs/heads/x" ref=refs/heads/x)" "$digest" "$engine" "$repo" v0.1.2 false "$git_repo"
expect refused attestation-self-hosted check_attestation "$(attestation runner=self-hosted)" "$digest" "$engine" "$repo" v0.1.2 false "$git_repo"
expect refused attestation-other-commit check_attestation "$(attestation commit="$code")" "$digest" "$engine" "$repo" v0.1.2 false "$git_repo"
expect refused attestation-other-digest check_attestation "$(attestation)" "$(printf 'd%.0s' {1..64})" "$engine" "$repo" v0.1.2 false "$git_repo"
expect refused attestation-other-type check_attestation "$(attestation type=https://slsa.dev/provenance/v1)" "$digest" "$engine" "$repo" v0.1.2 false "$git_repo"
expect refused attestation-later-workflow check_attestation "$(attestation signer_digest="$hostile" config_digest="$hostile")" "$digest" "$engine" "$repo" v0.1.2 false "$git_repo"
expect refused attestation-later-caller check_attestation "$(attestation config_digest="$hostile")" "$digest" "$engine" "$repo" v0.1.2 false "$git_repo"
expect refused attestation-unknown-workflow-commit check_attestation "$(attestation signer_digest="$(printf 'f%.0s' {1..40})")" "$digest" "$engine" "$repo" v0.1.2 false "$git_repo"
expect refused attestation-tag-build-unsigned check_attestation "$(attestation signer="$tagged" config="$tagged" ref=refs/tags/v0.1.2)" "$digest" "$engine" "$repo" v0.1.2 false "$git_repo"
expect refused attestation-tag-build-other-source check_attestation "$(attestation signer="$tagged" config="$tagged" ref=refs/tags/v0.1.2)" "$digest" "$engine" "$repo" v0.1.2 true "$git_repo"
expect pass attestation-tag-build-signed check_attestation "$(attestation signer="$tagged" config="$tagged" ref=refs/tags/v0.1.2 source_digest="$engine")" "$digest" "$engine" "$repo" v0.1.2 true "$git_repo"

route() { printf '%s\n' "repo=$repo" "$1" > "$dir/route.conf"; echo "$dir/route.conf"; }
expect refused route-unset second_route "$(route token_service=x)" 0
expect pass route-unset-acknowledged second_route "$(route token_service=x)" 1
expect pass route-proxy second_route "$(route "second_route=proxy https://proxy.example:8443")" 0
expect pass route-mirror second_route "$(route "second_route=mirror https://mirror.example/cef/")" 0
expect refused route-mirror-same-cdn second_route "$(route "second_route=mirror https://CEF-builds.spotifycdn.com.")" 0
expect refused route-http second_route "$(route "second_route=proxy http://proxy.example")" 0
expect refused route-credentials second_route "$(route "second_route=proxy https://user:pw@proxy.example")" 0
expect refused route-mirror-same-cdn-port second_route "$(route "second_route=mirror https://CEF-builds.spotifycdn.com.:443/x")" 0
expect pass host-of-userinfo eval '[[ "$(host_of "https://x@CEF-builds.spotifycdn.com.:443/f?a=b")" == cef-builds.spotifycdn.com ]]'
expect refused host-of-other eval '[[ "$(host_of "https://cef-builds.spotifycdn.com@mirror.example/f")" == cef-builds.spotifycdn.com ]]'
expect refused route-kind second_route "$(route "second_route=https://mirror.example")" 1
expect refused route-empty-acknowledged main --config "$(route token_service=x)" --acknowledge-single-source
echo archive > "$dir/first.tar.bz2"
echo archive > "$dir/second.tar.bz2"
echo other > "$dir/other.tar.bz2"
expect pass downloads-same same_download "$dir/first.tar.bz2" "$dir/second.tar.bz2"
expect refused downloads-differ same_download "$dir/first.tar.bz2" "$dir/other.tar.bz2"
cef=156.0.27+gabcdef0+chromium-156.0.8078.99
expect pass acknowledged acknowledge_single_source "$dir/ack" v0.1.2 "$cef" <<<"$cef"
grep -q "v0.1.2 CEF $cef: single-source signing acknowledged by" "$dir/ack/single-source.log" || { echo "FAIL acknowledged: not logged" >&2; failed=1; }
expect refused acknowledged-other-version acknowledge_single_source "$dir/ack" v0.1.2 "$cef" <<<"156.0.27"
expect refused acknowledged-nothing acknowledge_single_source "$dir/ack" v0.1.2 "$cef" < /dev/null

soak="$(printf 'e%.0s' {1..64})"
expect refused soak-first-seen check_soak "$dir/state" "$soak" 1000000
expect refused soak-under-24h check_soak "$dir/state" "$soak" $((1000000 + 86399))
expect pass soak-24h check_soak "$dir/state" "$soak" $((1000000 + 86400))

plist() {
  local file="$1"
  shift
  { echo '<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict>'; printf '<key>%s</key><string>%s</string>' "$@"; echo '</dict></plist>'; } > "$file"
}
feed=https://example.invalid/appcast.xml
key=pinned-key
dist="$dir/sparkle-dist"
b="$dist/Sparkle.framework/Versions/B"
mkdir -p "$b/XPCServices/Installer.xpc/Contents/MacOS" "$b/XPCServices/Downloader.xpc/Contents/MacOS" \
  "$b/Updater.app/Contents/MacOS" "$b/Resources" "$b/_CodeSignature"
echo sparkle > "$b/Sparkle"
echo autoupdate > "$b/Autoupdate"
echo installer > "$b/XPCServices/Installer.xpc/Contents/MacOS/Installer"
echo downloader > "$b/XPCServices/Downloader.xpc/Contents/MacOS/Downloader"
echo updater > "$b/Updater.app/Contents/MacOS/Updater"
echo resources > "$b/_CodeSignature/CodeResources"
echo plist > "$b/Resources/Info.plist"
chmod 755 "$b/Sparkle" "$b/Autoupdate" "$b/XPCServices/Installer.xpc/Contents/MacOS/Installer" \
  "$b/XPCServices/Downloader.xpc/Contents/MacOS/Downloader" "$b/Updater.app/Contents/MacOS/Updater"
ln -s B "$dist/Sparkle.framework/Versions/Current"
for item in Sparkle Resources Autoupdate Updater.app XPCServices; do ln -s "Versions/Current/$item" "$dist/Sparkle.framework/$item"; done
echo license > "$dist/LICENSE"

# The steps of scripts/bundle.sh with TARN_SIGN_IDENTITY=none.
bundle() {
  local app="$dir/Tarn.app" flavor h
  rm -rf "$app" "$dir/reference" "$dir/sparkle" "$dir/notices"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$dir/reference/Libraries" "$dir/reference/Resources"
  echo bin > "$app/Contents/MacOS/tarn"
  echo icon > "$app/Contents/Resources/Tarn.icns"
  cp "$dist/LICENSE" "$app/Contents/Resources/Sparkle-LICENSE.txt"
  mkdir "$dir/notices"
  for notice in LICENSE.txt Chromium-CREDITS.html CEF-LICENSE.txt Third-Party-Licenses.txt; do
    echo "$notice" > "$dir/notices/$notice"
    cp "$dir/notices/$notice" "$app/Contents/Resources/$notice"
  done
  sed "s/@VERSION@/0.1.2/g" "$template" > "$app/Contents/Info.plist"
  plutil -insert SUFeedURL -string "$feed" "$app/Contents/Info.plist"
  plutil -insert SUPublicEDKey -string "$key" "$app/Contents/Info.plist"
  mkdir -p "$app/Contents/Frameworks"
  ditto "$dist/Sparkle.framework" "$app/Contents/Frameworks/Sparkle.framework"
  rm -rf "$app/Contents/Frameworks/Sparkle.framework/Versions/B/XPCServices/Downloader.xpc"
  echo framework > "$dir/reference/Chromium Embedded Framework"
  echo lib > "$dir/reference/Libraries/libEGL.dylib"
  echo res > "$dir/reference/Resources/resources.pak"
  cp -R "$dir/reference" "$app/Contents/Frameworks/Chromium Embedded Framework.framework"
  for flavor in "" " (GPU)" " (Renderer)" " (Plugin)" " (Alerts)"; do
    h="$app/Contents/Frameworks/tarn Helper$flavor.app/Contents"
    mkdir -p "$h/MacOS"
    echo helper > "$h/MacOS/tarn Helper$flavor"
    plist "$h/Info.plist" CFBundleIdentifier dev.tarn.browser.helper
  done
  expect pass sparkle-reference sparkle_reference "$dist" "$dir/sparkle"
}
tree() { expect "$1" "$2" check_tree "$dir/Tarn.app" "$dir/reference" "$dir/sparkle" "$template" 0.1.2 "$feed" "$key" "$dir/notices"; }
fw="$dir/Tarn.app/Contents/Frameworks/Chromium Embedded Framework.framework"
sp="$dir/Tarn.app/Contents/Frameworks/Sparkle.framework"
info="$dir/Tarn.app/Contents/Info.plist"
bundle; tree pass tree
bundle; echo changed > "$fw/Libraries/libEGL.dylib"; tree refused tree-framework-file-differs
bundle; chmod 755 "$fw/Resources/resources.pak"; tree refused tree-framework-mode-differs
bundle; echo extra > "$fw/Libraries/extra.dylib"; tree refused tree-framework-extra-file
bundle; rm "$fw/Resources/resources.pak"; tree refused tree-framework-missing-file
bundle; echo changed > "$sp/Versions/B/Autoupdate"; tree refused tree-sparkle-file-differs
bundle; chmod 777 "$sp/Versions/B/Updater.app/Contents/MacOS/Updater"; tree refused tree-sparkle-mode-differs
bundle; ditto "$dist/Sparkle.framework/Versions/B/XPCServices/Downloader.xpc" "$sp/Versions/B/XPCServices/Downloader.xpc"; tree refused tree-sparkle-downloader-kept
bundle; rm "$sp/Versions/Current"; ln -s /tmp "$sp/Versions/Current"; tree refused tree-sparkle-symlink-retargeted
bundle; ln -s /etc/hosts "$sp/Versions/B/Resources/link"; tree refused tree-sparkle-extra-symlink
bundle; rm "$sp/Sparkle" "$sp/Updater.app"; ln -s $'Versions/Current/Sparkle ./Sparkle\nl Versions/Current/Updater.app' "$sp/Updater.app"; tree refused tree-sparkle-symlink-forged-line
bundle; rm -rf "$sp"; tree refused tree-sparkle-missing
bundle; echo changed > "$dir/Tarn.app/Contents/Resources/Sparkle-LICENSE.txt"; tree refused tree-sparkle-license-differs
bundle; echo changed > "$dir/Tarn.app/Contents/Resources/Chromium-CREDITS.html"; tree refused tree-credits-differ
bundle; echo changed > "$dir/Tarn.app/Contents/Resources/Third-Party-Licenses.txt"; tree refused tree-third-party-licenses-differ
bundle; rm "$dir/Tarn.app/Contents/Resources/CEF-LICENSE.txt"; tree refused tree-cef-license-missing
bundle; echo extra > "$dir/Tarn.app/Contents/Resources/extra"; tree refused tree-extra-file
bundle; ln -s /etc/hosts "$dir/Tarn.app/Contents/Resources/link"; tree refused tree-symlink
bundle; rm "$dir/Tarn.app/Contents/Resources/Tarn.icns"; ln -s /etc/hosts "$dir/Tarn.app/Contents/Resources/Tarn.icns"; tree refused tree-symlink-in-place
bundle; chmod 4755 "$dir/Tarn.app/Contents/MacOS/tarn"; tree refused tree-setuid
bundle; plutil -insert SUAllowsAutomaticUpdates -bool NO "$info"; tree refused tree-extra-update-key
bundle; plutil -replace SURequireSignedFeed -bool NO "$info"; tree refused tree-pinned-update-value-changed
bundle; plutil -remove SUVerifyUpdateBeforeExtraction "$info"; tree refused tree-pinned-update-key-missing
bundle; plutil -replace SUFeedURL -string https://evil.example/appcast.xml "$info"; tree refused tree-other-feed
bundle; plutil -remove SUPublicEDKey "$info"; tree refused tree-missing-key
bundle; plutil -replace CFBundleShortVersionString -string 0.1.1 "$info"; tree refused tree-other-version
bundle; plutil -replace CFBundleIdentifier -string dev.tarn.other "$info"; tree refused tree-other-bundle-id
bundle; plist "$dir/Tarn.app/Contents/Frameworks/tarn Helper (GPU).app/Contents/Info.plist" CFBundleIdentifier dev.tarn.browser.helper SUFeedURL "$feed"; tree refused tree-helper-update-key

[[ "$failed" == 0 ]] && echo "sign-release: all cases pass"
exit "$failed"
