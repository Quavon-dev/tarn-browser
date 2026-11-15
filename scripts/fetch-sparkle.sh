#!/usr/bin/env bash
# Downloads the Sparkle release pinned in packaging/sparkle.lock, checks its SHA-256 and unpacks it
# into target/sparkle/<version>. The lock is read as data, never sourced.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
lock="${SPARKLE_LOCK:-$root/packaging/sparkle.lock}"
dir="${SPARKLE_DIR:-$root/target/sparkle}"
field() {
  local value
  value="$(grep -E "^$1=" "$lock" | head -n1 | cut -d= -f2-)"
  [[ -n "$value" ]] || { echo "sparkle.lock has no $1" >&2; exit 1; }
  printf '%s' "$value"
}
version="$(field sparkle_version)"
sha256="$(field sha256)"
[[ "$(field host)" == "github.com" ]] || { echo "sparkle.lock names another download host" >&2; exit 1; }
[[ "$version" =~ ^2\.[0-9]+\.[0-9]+$ ]] || { echo "bad sparkle_version" >&2; exit 1; }
[[ "$sha256" =~ ^[0-9a-f]{64}$ ]] || { echo "bad sha256" >&2; exit 1; }

tmp=""
part=""
trap 'rm -rf "$tmp" "$part"' EXIT
name="Sparkle-$version.tar.xz"
cache="$dir/downloads"
archive="$cache/$name"
dest="$dir/$version"
mkdir -p "$cache"
if [[ ! -f "$archive" ]] || [[ "$(shasum -a 256 "$archive" | cut -d' ' -f1)" != "$sha256" ]]; then
  part="$(mktemp "$archive.XXXXXX")"
  curl --proto '=https' --tlsv1.2 -sSfL -o "$part" "https://github.com/sparkle-project/Sparkle/releases/download/$version/$name"
  mv "$part" "$archive"
  part=""
fi
actual="$(shasum -a 256 "$archive" | cut -d' ' -f1)"
if [[ "$actual" != "$sha256" ]]; then
  rm -f "$archive"
  echo "SHA-256 mismatch for $name: expected $sha256, got $actual" >&2
  exit 1
fi
if [[ "$(cat "$dest/.archive-sha256" 2>/dev/null)" != "$sha256" ]]; then
  tmp="$(mktemp -d "$dest.XXXXXX")"
  tar -xJf "$archive" -C "$tmp" --no-same-owner Sparkle.framework bin LICENSE
  framework="$tmp/Sparkle.framework"
  [[ -d "$framework" && ! -L "$framework" && -x "$tmp/bin/sign_update" && -f "$tmp/LICENSE" ]] || { echo "unexpected archive layout" >&2; exit 1; }
  # Task 60's seven-day rule needs a feed Sparkle verifies; that arrived in Sparkle 2.9.
  grep -q SURequireSignedFeed "$framework/Versions/B/Sparkle" || { echo "Sparkle $version cannot verify a signed feed" >&2; exit 1; }
  printf '%s' "$sha256" > "$tmp/.archive-sha256"
  rm -rf "$dest"
  mv "$tmp" "$dest"
  tmp=""
fi
echo "$dest"
