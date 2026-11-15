#!/usr/bin/env bash
# Downloads the CEF build pinned in packaging/cef.lock, checks its SHA-256 and unpacks it into
# target/cef/<platform>, the layout CEF_PATH expects. The lock is read as data, never sourced.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
lock="${CEF_LOCK:-$root/packaging/cef.lock}"
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) host_arch=windows64 ;; *) host_arch="$(uname -m)" ;; esac
arch="${1:-$host_arch}"
case "$arch" in
  arm64|aarch64|macosarm64) platform=macosarm64 ;;
  x86_64|macosx64) platform=macosx64 ;;
  windows64) platform=windows64 ;;
  *) echo "unknown architecture: $arch" >&2; exit 1 ;;
esac
field() {
  local value
  value="$(grep -E "^$1=" "$lock" | head -n1 | cut -d= -f2-)"
  [[ -n "$value" ]] || { echo "cef.lock has no $1" >&2; exit 1; }
  printf '%s' "$value"
}
version="$(field cef_version)"
sha256="$(field "sha256_$platform")"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+\+g[0-9a-f]{7}\+chromium-[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "bad cef_version" >&2; exit 1; }
crate="$(grep -A1 '^name = "cef-dll-sys"$' "$root/Cargo.lock" | sed -n 's/^version = ".*+\(.*\)"$/\1/p' || true)"
cef="${version%%+*}"
# cef-dll-sys's build script accepts an archive of its own major.minor no newer than its own CEF.
[[ "$crate" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ && "${cef%.*}" == "${crate%.*}" && "${cef##*.}" -le "${crate##*.}" ]] \
  || { echo "cef.lock $version is not a CEF ${crate:-?} build or an older one of its branch" >&2; exit 1; }
[[ "$version" == *"+chromium-$(field chromium_version)" ]] || { echo "cef.lock chromium_version does not match cef_version" >&2; exit 1; }
[[ "$(field host)" == "cef-builds.spotifycdn.com" ]] || { echo "cef.lock names another download host" >&2; exit 1; }
[[ "$sha256" =~ ^[0-9a-f]{64}$ ]] || { echo "bad sha256_$platform" >&2; exit 1; }

sha() { if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }
tmp=""
part=""
trap 'rm -rf "$tmp" "$part"' EXIT
name="cef_binary_${version}_${platform}_minimal"
cache="$root/target/cef/downloads"
archive="$cache/$name.tar.bz2"
dest="$root/target/cef/$platform"
mkdir -p "$cache"
if [[ ! -f "$archive" ]] || [[ "$(sha "$archive")" != "$sha256" ]]; then
  part="$(mktemp "$archive.XXXXXX")"
  curl --proto '=https' --tlsv1.2 -sSfL -o "$part" "https://cef-builds.spotifycdn.com/${name//+/%2B}.tar.bz2"
  mv "$part" "$archive"
  part=""
fi
actual="$(sha "$archive")"
if [[ "$actual" != "$sha256" ]]; then
  rm -f "$archive"
  echo "SHA-256 mismatch for $name: expected $sha256, got $actual" >&2
  exit 1
fi
if [[ "$(cat "$dest/.archive-sha256" 2>/dev/null)" != "$sha256" || ! -f "$dest/LICENSE.txt" ]]; then
  tmp="$(mktemp -d "$dest.XXXXXX")"
  tar -xjf "$archive" -C "$tmp" --no-same-owner
  src="$tmp/$name"
  [[ -d "$src/Release" && ! -L "$src/Release" ]] || { echo "unexpected archive layout" >&2; exit 1; }
  rm -rf "$dest"
  mv "$src/Release" "$dest"
  # Outside macOS the resources sit beside the libraries, where cef-dll-sys copies them from.
  if [[ "$platform" != macos* ]]; then
    [[ -d "$src/Resources" && ! -L "$src/Resources" ]] || { echo "unexpected archive layout" >&2; exit 1; }
    mv "$src/Resources"/* "$dest/"
  fi
  mv "$src/CMakeLists.txt" "$src/cmake" "$src/include" "$src/libcef_dll" "$src/CREDITS.html" "$src/LICENSE.txt" "$dest/"
  printf '{"type":"minimal","name":"%s.tar.bz2","sha1":""}\n' "$name" > "$dest/archive.json"
  printf '%s' "$sha256" > "$dest/.archive-sha256"
fi
echo "$dest"
