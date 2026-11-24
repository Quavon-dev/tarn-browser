#!/usr/bin/env bash
# Writes the Homebrew cask for release TAG to stdout from packaging/homebrew/tarn.rb.in, with the
# SHA-256 of that release's Tarn-<tag>.dmg. REPO is owner/name, as GITHUB_REPOSITORY gives it.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
usage="usage: homebrew-cask.sh TAG DMG_SHA256 REPO"
tag="${1:?$usage}" sha="${2:?$usage}" repo="${3:?$usage}"
[[ "$tag" =~ ^v[0-9]+(\.[0-9]+){1,3}$ ]] || { echo "homebrew-cask.sh: '$tag' is not a release tag like v1.2.3" >&2; exit 1; }
[[ "$sha" =~ ^[0-9a-f]{64}$ ]] || { echo "homebrew-cask.sh: '$sha' is not a lowercase SHA-256" >&2; exit 1; }
[[ "$repo" =~ ^[A-Za-z0-9-]+/[A-Za-z0-9._-]+$ ]] || { echo "homebrew-cask.sh: '$repo' is not owner/name" >&2; exit 1; }
sed -e "s|@VERSION@|${tag#v}|g" -e "s|@SHA256@|$sha|g" -e "s|@REPO@|$repo|g" "$root/packaging/homebrew/tarn.rb.in"
