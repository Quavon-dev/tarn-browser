#!/usr/bin/env bash
# Walks a release through docs/release.md on the maintainer's Mac: signed tag, CI draft, signed and
# notarised macOS zip, disk image and appcast, the Windows files signed on Windows, checks, publish.
# Needs the Developer ID identity, the tarn-notary profile and the update key on this Mac.
# Every step that publishes something asks first; re-running skips what is already done.
# Usage: scripts/publish-release.sh [--skip-ci]
set -euo pipefail

R=Quavon-dev/tarn-browser
NEW="$(cd "$(dirname "$0")/.." && pwd)"
IDENTITY=${TARN_SIGN_IDENTITY:-"Developer ID Application: Quavon UG (haftungsbeschrankt) (CFV35FGSHF)"}
NOTARY=${TARN_NOTARY_PROFILE:-tarn-notary}

# The release page shows these labels; file names stay as the updaters and package repositories need them.
label() {
  local v=${VERSION//./\\.} n=$1
  case "$n" in
    "Tarn-$TAG.dmg") echo "macOS (Apple silicon): installer (.dmg)" ;;
    "Tarn-$TAG.zip") echo "macOS (Apple silicon): app for updates (.zip)" ;;
    "Tarn-$TAG-unsigned.zip") echo "macOS (Apple silicon): unsigned build (.zip)" ;;
    appcast.xml) echo "macOS update feed" ;;
    "Tarn-$TAG-windows-x64-setup.exe") echo "Windows (x64): installer (.exe)" ;;
    "Tarn-$TAG-windows-x64-setup-unsigned.exe") echo "Windows (x64): unsigned installer (.exe)" ;;
    "Tarn-$TAG-windows-x64-unsigned.zip") echo "Windows (x64): unsigned build (.zip)" ;;
    windows-update.json) echo "Windows update feed" ;;
    windows-update.json.sig) echo "Windows update feed signature" ;;
    "Tarn-$TAG-ios-unsigned.zip") echo "iOS: unsigned build (.zip)" ;;
    "Tarn-$TAG-android-unsigned.apk") echo "Android: unsigned build (.apk)" ;;
    "Tarn-$TAG.cdx.json") echo "Software bill of materials (CycloneDX)" ;;
    SHA256SUMS) echo "SHA-256 checksums" ;;
    *) if [[ "$n" =~ ^tarn_${v}_debian([0-9.]+)_amd64\.deb$ ]]; then echo "Linux: Debian ${BASH_REMATCH[1]} (x64, .deb)"
       elif [[ "$n" =~ ^tarn_${v}_ubuntu([0-9.]+)_amd64\.deb$ ]]; then echo "Linux: Ubuntu ${BASH_REMATCH[1]} (x64, .deb)"
       elif [[ "$n" =~ ^tarn-${v}-[0-9]+\.fc([0-9]+)\.x86_64\.rpm$ ]]; then echo "Linux: Fedora ${BASH_REMATCH[1]} (x64, .rpm)"
       elif [[ "$n" =~ ^tarn-${v}-[0-9]+-x86_64\.pkg\.tar\.zst$ ]]; then echo "Linux: Arch (x64, .pkg.tar.zst)"
       fi ;;
  esac
}

say() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
ask() { local a; read -r -p "$1 [y/N] " a; [[ $a == [yY]* ]]; }
assets() { gh release view "$TAG" -R "$R" --json assets --jq '.assets[].name' 2>/dev/null || true; }

git -C "$NEW" fetch -q origin main --tags
VERSION=$(git -C "$NEW" show origin/main:Cargo.toml | sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' | head -n1)
TAG=v$VERSION
WIN_SETUP=Tarn-$TAG-windows-x64-setup.exe

say "Release $TAG"
SKIP_CI=false; [ "${1:-}" = --skip-ci ] && SKIP_CI=true
run=$(gh run list -R "$R" --workflow CI --branch main -L 1 --json databaseId,headSha --jq '.[0] | "\(.databaseId) \(.headSha)"')
read -r run_id run_sha <<<"$run"
[ "$run_sha" = "$(git -C "$NEW" rev-parse origin/main)" ] || { echo "The latest CI run is not for origin/main yet; wait a moment."; exit 1; }
jobs=$(gh api "repos/$R/actions/runs/$run_id/jobs" --jq '.jobs[] | "\(.name) \(.status) \(.conclusion)"')
failed=$(awk '$2=="completed" && $3!="success" && $3!="skipped"' <<<"$jobs")
pending=$(awk '$2!="completed"' <<<"$jobs")
[ -z "$failed" ] || { echo "CI on main failed, not releasing:"; echo "$failed"; exit 1; }
if [ -n "$pending" ]; then
  $SKIP_CI || { echo "CI on main is still running:"; echo "$pending"; echo "Wait, or run with --skip-ci to release without those results."; exit 1; }
  echo "Releasing without waiting for:"; echo "$pending"
fi
echo "CI on main: no failures for $(git -C "$NEW" log -1 --format='%h %s' origin/main)."

say "1/5 Tag and CI draft"
if git -C "$NEW" ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
  echo "$TAG exists."
else
  check=$(mktemp -d)/main
  git -C "$NEW" worktree add -q --detach "$check" origin/main
  if "$check/scripts/check-version.sh" "$TAG"; then ok=true; else ok=false; fi
  git -C "$NEW" worktree remove --force "$check"
  $ok || { echo "Version files disagree with $TAG; fix them first."; exit 1; }
  ask "Tag origin/main ($(git -C "$NEW" log -1 --format='%h %s' origin/main)) as $TAG, signed, and push it?" || exit 0
  git -C "$NEW" tag -s "$TAG" origin/main -m "$TAG"
  git -C "$NEW" push origin "refs/tags/$TAG"
  echo "Tag pushed; release.yml now builds the draft. Run this script again when it has finished."
  exit 0
fi
gh release view "$TAG" -R "$R" >/dev/null 2>&1 || { echo "No draft release $TAG yet; wait for the Release workflow, then run this again."; exit 1; }
[ "$(gh release view "$TAG" -R "$R" --json isDraft --jq .isDraft)" = true ] || { echo "$TAG is already published."; exit 0; }

say "2/5 Signed and notarised macOS build"
MAC_ZIP=Tarn-$TAG.zip
MAC_DMG=Tarn-$TAG.dmg
if assets | grep -qx "$MAC_ZIP" && assets | grep -qx "$MAC_DMG" && assets | grep -qx appcast.xml; then
  echo "$MAC_ZIP, $MAC_DMG and appcast.xml are already on the draft."
else
  work=$(mktemp -d)/tag
  git -C "$NEW" worktree add -q --detach "$work" "$TAG"
  trap 'git -C "$NEW" worktree remove --force "$work" 2>/dev/null || true' EXIT
  (
    cd "$work"
    TARN_RELEASE_BUILD=1 TARN_SIGN_IDENTITY="$IDENTITY" scripts/bundle.sh
    scripts/check-entitlements.sh
    TARN_NOTARY_PROFILE="$NOTARY" scripts/notarize.sh
    ditto -c -k --keepParent target/release/Tarn.app "$MAC_ZIP"
    TARN_SIGN_IDENTITY="$IDENTITY" TARN_NOTARY_PROFILE="$NOTARY" scripts/dmg.sh target/release/Tarn.app "$MAC_DMG"
    scripts/appcast.sh "$MAC_ZIP" "$TAG"
  )
  ask "Upload $MAC_ZIP, $MAC_DMG and appcast.xml to the draft $TAG?" || exit 0
  gh release upload "$TAG" -R "$R" --clobber "$work/$MAC_ZIP" "$work/$MAC_DMG" "$work/appcast.xml"
fi

say "3/5 Windows installer and update feed"
until assets | grep -qx "$WIN_SETUP" && assets | grep -qx windows-update.json && assets | grep -qx windows-update.json.sig; do
  cat <<EOF
Missing on the draft: $WIN_SETUP, windows-update.json, windows-update.json.sig.
On the Windows machine (PowerShell, in a checkout of $TAG):
  gh release download $TAG -R $R -p "Tarn-$TAG-windows-x64-unsigned.zip"
  gh attestation verify Tarn-$TAG-windows-x64-unsigned.zip --repo $R --signer-workflow $R/.github/workflows/release.yml
  scripts\\sign-windows.ps1 -Package Tarn-$TAG-windows-x64-unsigned.zip -Version $VERSION -Thumbprint <SHA-1>
Copy dist\\$WIN_SETUP to this Mac and run the three sign_update commands it printed here.
Bring windows-update.json and windows-update.json.sig back to Windows, then:
  scripts\\sign-windows.ps1 -Check windows-update.json -Installer dist\\$WIN_SETUP
  gh release upload $TAG -R $R dist\\$WIN_SETUP windows-update.json windows-update.json.sig
EOF
  read -r -p "Press Enter once the three files are uploaded (Ctrl-C to stop and resume later)... "
done
echo "Windows files are on the draft."

say "4/5 Check the draft"
gh release view "$TAG" -R "$R" --json assets --jq '.assets[] | "  \(.name)  \(.size) bytes"'
for f in "$MAC_ZIP" "$MAC_DMG" appcast.xml "$WIN_SETUP" windows-update.json windows-update.json.sig SHA256SUMS; do
  assets | grep -qx "$f" || { echo "Missing $f; not publishing."; exit 1; }
done
for p in "tarn_${VERSION}_debian.*_amd64\.deb" "tarn_${VERSION}_ubuntu.*_amd64\.deb" "tarn-${VERSION}-.*\.x86_64\.rpm" "tarn-${VERSION}-.*-x86_64\.pkg\.tar\.zst"; do
  assets | grep -qE "^$p$" || { echo "Missing the Linux package $p; not publishing."; exit 1; }
done

# Anyone with write access can replace a draft's files: check what is there now (docs/release.md).
say "Verify the macOS files on the draft"
chk=$(mktemp -d)
gh release download "$TAG" -R "$R" -p "$MAC_ZIP" -p "$MAC_DMG" -D "$chk"
ditto -x -k "$chk/$MAC_ZIP" "$chk/zip"
mnt="$chk/mnt"; hdiutil attach -quiet -readonly -nobrowse -mountpoint "$mnt" "$chk/$MAC_DMG"
verify_mac() {
  local req='=anchor apple generic and certificate leaf[subject.OU] = "CFV35FGSHF"' app
  codesign --verify --deep --strict -R "$req" "$chk/$MAC_DMG" "$mnt/Tarn.app" "$chk/zip/Tarn.app" || return 1
  spctl -a -vv -t open --context context:primary-signature "$chk/$MAC_DMG" 2>&1 | grep -q 'source=Notarized Developer ID' || return 1
  for app in "$mnt/Tarn.app" "$chk/zip/Tarn.app"; do
    spctl -a -vv -t execute "$app" 2>&1 | grep -q 'source=Notarized Developer ID' || return 1
    [ "$(defaults read "$app/Contents/Info" CFBundleShortVersionString)" = "$VERSION" ] || return 1
  done
}
if verify_mac; then ok=true; else ok=false; fi
hdiutil detach -quiet "$mnt"; rm -rf "$chk"
$ok || { echo "The macOS files on the draft failed the team, notarisation or version check. Not publishing."; exit 1; }
echo "zip and dmg: team CFV35FGSHF, notarised, version $VERSION."

say "Label the files"
gh api "repos/$R/releases/$(gh release view "$TAG" -R "$R" --json databaseId --jq .databaseId)/assets" --paginate \
  --jq '.[] | "\(.id) \(.name)"' | while read -r id name; do
  l=$(label "$name")
  [ -n "$l" ] && gh api -X PATCH "repos/$R/releases/assets/$id" -f label="$l" >/dev/null && echo "  $name -> $l"
done

say "5/5 Publish"
echo "Publishing makes $TAG the latest release: Macs and Windows PCs update to it within a day,"
echo "and the apt/dnf repositories update automatically."
ask "Publish $TAG now?" || exit 0
gh release edit "$TAG" -R "$R" --draft=false --latest
echo "Published: https://github.com/$R/releases/tag/$TAG"
curl -fsSIL "https://github.com/$R/releases/latest/download/appcast.xml" >/dev/null && echo "appcast.xml reachable at the feed URL."
curl -fsSIL "https://github.com/$R/releases/latest/download/windows-update.json" >/dev/null && echo "windows-update.json reachable at the feed URL."
