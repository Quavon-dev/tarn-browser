#!/usr/bin/env bash
# Usage: scripts/windows-feed.sh <installer.exe> <version> <download URL> [--critical]
# Writes windows-update.json and windows-update.json.sig next to the installer: the installer's and
# the feed's Ed25519 signatures with the update key in TARN_ED_KEY_FILE (Sparkle's private key
# file). release.yml runs it in its signing job for the installer that run built.
set -euo pipefail

readonly DOWNLOADS=https://github.com/Quavon-dev/tarn-browser/releases/download/

refuse() { echo "windows-feed.sh: $*" >&2; exit 1; }

# A release feed has Get-FeedJson's format in scripts/sign-windows.ps1, which -Check enforces. A
# nightly (four-part version) also carries "channel":"nightly" inside the signed bytes, which the
# Release channel refuses, and its URL in the nightly release; -Check does not read nightly feeds.
feed_json() {
  local version="$1" critical="$2" url="$3" sha256="$4" length="$5" signature="$6" channel=""
  if [[ "$version" =~ ^[0-9]+(\.[0-9]+){3}$ ]]; then channel='"channel":"nightly",'; fi
  printf '{"version":"%s",%s"critical":%s,"installer":{"url":"%s","sha256":"%s","length":%s,"signature":"%s"}}' \
    "$version" "$channel" "$critical" "$url" "$sha256" "$length" "$signature"
}

main() {
  local installer="${1:-}" version="${2:-}" url="${3:-}" critical=false
  [[ -f "$installer" && "$installer" == *.exe ]] || refuse "usage: windows-feed.sh <installer.exe> <version> <download URL> [--critical]"
  [[ "$version" =~ ^[0-9]+(\.[0-9]+){2,3}$ ]] || refuse "version must look like 1.2.3 or 1.2.3.4"
  [[ "$url" == "$DOWNLOADS"* && "$url" == *"/${installer##*/}" && "$url" =~ ^[A-Za-z0-9:/._-]+$ ]] \
    || refuse "the URL must be this repository's release download of ${installer##*/}"
  case "${4:-}" in
    --critical) critical=true ;;
    "") ;;
    *) refuse "unknown option $4" ;;
  esac
  local key="${TARN_ED_KEY_FILE:-}"
  [[ -f "$key" ]] || refuse "set TARN_ED_KEY_FILE to the update key file"

  local root bin dir sha length sig feedsig
  root="$(cd "$(dirname "$0")/.." && pwd)"
  bin="$("$root/scripts/fetch-sparkle.sh")/bin"
  dir="$(dirname "$installer")"
  edsig() { "$bin/sign_update" --ed-key-file "$key" "$1" | sed -n 's/.*edSignature="\([^"]*\)".*/\1/p'; }
  sha=$(shasum -a 256 "$installer" | cut -d' ' -f1)
  length=$(wc -c <"$installer" | tr -d ' ')
  sig=$(edsig "$installer")
  [[ "$sig" =~ ^[A-Za-z0-9+/]{86}==$ ]] || refuse "sign_update gave no signature for $installer"
  feed_json "$version" "$critical" "$url" "$sha" "$length" "$sig" >"$dir/windows-update.json"
  feedsig=$(edsig "$dir/windows-update.json")
  [[ "$feedsig" =~ ^[A-Za-z0-9+/]{86}==$ ]] || refuse "sign_update gave no signature for windows-update.json"
  printf '%s' "$feedsig" >"$dir/windows-update.json.sig"
  "$bin/sign_update" --ed-key-file "$key" --verify "$installer" "$sig"
  "$bin/sign_update" --ed-key-file "$key" --verify "$dir/windows-update.json" "$feedsig"
}

[[ "${BASH_SOURCE[0]}" != "$0" ]] || main "$@"
