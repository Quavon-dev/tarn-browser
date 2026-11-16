#!/usr/bin/env bash
# Usage: scripts/windows-feed.sh <tag> [--critical]
# Signs an update feed for release <tag>'s unsigned Windows installer and uploads
# windows-update.json and windows-update.json.sig to that release (draft or published), for
# releases without the signed installer (publish-release.sh --no-windows). It signs only the
# installer that release.yml built on a GitHub-hosted runner from the maintainer-signed tag.
# Needs the update key in the login keychain (account dev.tarn.browser), as scripts/appcast.sh, and
# the maintainers' SSH signing keys in TARN_ALLOWED_SIGNERS (default: the signing Mac's file).
set -euo pipefail

# refuse, maintainer_signed and PREDICATE_TYPE
# shellcheck source=sign-release.sh
source "$(dirname "${BASH_SOURCE[0]}")/sign-release.sh"
readonly R=Quavon-dev/tarn-browser

# Same format as Get-FeedJson in scripts/sign-windows.ps1, which -Check enforces.
feed_json() {
  local version="$1" critical="$2" sha256="$3" length="$4" signature="$5"
  printf '{"version":"%s","critical":%s,"installer":{"url":"https://github.com/%s/releases/download/v%s/Tarn-v%s-windows-x64-setup-unsigned.exe","sha256":"%s","length":%s,"signature":"%s"}}' \
    "$version" "$critical" "$R" "$version" "$version" "$sha256" "$length" "$signature"
}

# The certificate, not the self-reported predicate, ties the build to the tag: release.yml at the
# tag's own commit, pushed as that tag, on a GitHub-hosted runner.
check_attestation() {
  local json="$1" digest="$2" commit="$3" tag="$4" repo="https://github.com/$R"
  jq -e --arg digest "$digest" --arg commit "$commit" --arg repo "$repo" --arg tag "$tag" --arg type "$PREDICATE_TYPE" '
    any(.[]; .verificationResult as $r | $r.signature.certificate as $c
      | ($r.statement.subject | any(.digest.sha256 == $digest))
        and $r.statement.predicateType == $type
        and $r.statement.predicate.commit == $commit
        and $r.statement.predicate.tag == $tag
        and $c.runnerEnvironment == "github-hosted"
        and $c.sourceRepositoryURI == $repo
        and $c.buildSignerURI == "\($repo)/.github/workflows/release.yml@refs/tags/\($tag)"
        and $c.buildConfigURI == $c.buildSignerURI
        and $c.buildSignerDigest == $commit
        and $c.sourceRepositoryRef == "refs/tags/\($tag)"
        and $c.sourceRepositoryDigest == $commit)' "$json" >/dev/null \
    || refuse "no attestation of $digest by release.yml at $tag ($commit) on a GitHub-hosted runner"
}

main() {
  local root tag version critical=false
  root="$(cd "$(dirname "$0")/.." && pwd)"
  tag="${1:?usage: windows-feed.sh <tag> [--critical]}"
  [[ "$tag" =~ ^v([0-9]+\.[0-9]+\.[0-9]+)$ ]] || refuse "tag must look like v1.2.3"
  version=${BASH_REMATCH[1]}
  case "${2:-}" in
    --critical) critical=true ;;
    "") ;;
    *) refuse "unknown option $2" ;;
  esac

  local signers=${TARN_ALLOWED_SIGNERS:-/usr/local/libexec/tarn-sign/allowed_signers}
  [[ -f "$signers" ]] || refuse "no allowed signers file $signers (set TARN_ALLOWED_SIGNERS)"
  git -C "$root" fetch -q origin '+refs/heads/main:refs/remotes/origin/main' "refs/tags/$tag:refs/tags/$tag" \
    || refuse "could not fetch $tag; if a local $tag differs from origin's, delete it (git tag -d $tag) and run this again"
  maintainer_signed "$root" "$tag" "$signers" || refuse "$tag is not signed by a key in $signers"
  local commit
  commit=$(git -C "$root" rev-parse --verify "refs/tags/$tag^{commit}")
  git -C "$root" merge-base --is-ancestor "$commit" refs/remotes/origin/main || refuse "$tag ($commit) is not on main"

  local setup=Tarn-$tag-windows-x64-setup-unsigned.exe id listing want
  id=$(gh release view "$tag" -R "$R" --json databaseId --jq .databaseId)
  listing=$(gh api "repos/$R/releases/$id/assets" --paginate --jq '.[] | "\(.digest) \(.name)"')
  awk -v n="Tarn-$tag-windows-x64-setup.exe" '$2==n {f=1} END {exit !f}' <<<"$listing" \
    && refuse "$tag has the signed installer; its feed comes from scripts/sign-windows.ps1"
  want=$(awk -v n="$setup" '$2==n {print $1}' <<<"$listing")
  [[ "$want" =~ ^sha256:[0-9a-f]{64}$ ]] || refuse "$tag has no $setup with a SHA-256 digest"

  local bin account=dev.tarn.browser sha
  bin="$("$root/scripts/fetch-sparkle.sh")/bin"
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  gh release download "$tag" -R "$R" -p "$setup" -D "$work"
  sha=$(shasum -a 256 "$work/$setup" | cut -d' ' -f1)
  [[ "sha256:$sha" == "$want" ]] || refuse "$setup changed while it was downloaded"
  gh attestation verify "$work/$setup" --repo "$R" --predicate-type "$PREDICATE_TYPE" --format json >"$work/attestation.json"
  check_attestation "$work/attestation.json" "$sha" "$commit" "$tag"

  edsig() { "$bin/sign_update" --account "$account" "$1" | sed -n 's/.*edSignature="\([^"]*\)".*/\1/p'; }
  local sig feedsig length
  sig=$(edsig "$work/$setup")
  [[ -n "$sig" ]] || refuse "sign_update gave no signature for $setup"
  length=$(wc -c <"$work/$setup" | tr -d ' ')
  feed_json "$version" "$critical" "$sha" "$length" "$sig" >"$work/windows-update.json"
  feedsig=$(edsig "$work/windows-update.json")
  [[ -n "$feedsig" ]] || refuse "sign_update gave no signature for windows-update.json"
  printf '%s' "$feedsig" >"$work/windows-update.json.sig"
  "$bin/sign_update" --account "$account" --verify "$work/$setup" "$sig"
  "$bin/sign_update" --account "$account" --verify "$work/windows-update.json" "$feedsig"

  cat "$work/windows-update.json"; echo
  local a
  read -r -p "Upload windows-update.json and windows-update.json.sig to $tag? [y/N] " a
  [[ $a == [yY]* ]] || refuse "not uploaded"
  gh release upload "$tag" -R "$R" --clobber "$work/windows-update.json" "$work/windows-update.json.sig"
  echo "Uploaded. Windows installs of $tag verify their update check once it is the latest release."
}

[[ "${BASH_SOURCE[0]}" != "$0" ]] || main "$@"
