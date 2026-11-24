#!/usr/bin/env bash
# Checks release files against the attestations release.yml makes, for the steps that copy or name
# them (GH_REPO is owner/name):
#   sums FILE TAG  passes when SHA256SUMS FILE is attested for TAG by release.yml run from that tag
#                  or from main (start-release.yml and chromium-update.yml build there), on a
#                  GitHub-hosted runner; a run from any other branch cannot vouch for a release.
#   script FILE    prints the tag an install.ps1 names once that release's attested SHA256SUMS
#                  lists its exact bytes; users run the script with irm | iex.
#   tag FILE       prints the tag an install.ps1 names, unchecked.
set -euo pipefail

script_tag() { sed -n "s/^\$Tag = '\(v[0-9][0-9.]*\)'\$/\1/p" "$1"; }

attested() {
  local repo="${GH_REPO:?GH_REPO must name owner/repo}"
  gh attestation verify "$1" --repo "$repo" --deny-self-hosted-runners \
    --cert-identity-regex "^https://github\.com/${repo//./\\.}/\.github/workflows/release\.yml@refs/(tags/${2//./\\.}|heads/main)\$" \
    --predicate-type https://github.com/Quavon-dev/tarn-browser/attestations/release/v1 \
    --format json | jq -e --arg tag "$2" 'any(.[]; .verificationResult.statement.predicate.tag == $tag)' >/dev/null \
    || { echo "attestation.sh: $1 is not attested for $2 by release.yml run from $2 or main" >&2; return 1; }
}

case "${1:-}" in
  sums)
    [[ $# -eq 3 && "$3" =~ ^v[0-9]+(\.[0-9]+){1,3}$ ]] || { echo "usage: attestation.sh sums FILE vX.Y.Z" >&2; exit 2; }
    attested "$2" "$3"
    ;;
  tag)
    [[ $# -eq 2 ]] || { echo "usage: attestation.sh tag FILE" >&2; exit 2; }
    script_tag "$2"
    ;;
  script)
    [[ $# -eq 2 ]] || { echo "usage: attestation.sh script FILE" >&2; exit 2; }
    tag="$(script_tag "$2")"
    [[ "$tag" =~ ^v[0-9]+(\.[0-9]+){1,3}$ ]] || { echo "attestation.sh: $2 names no release tag" >&2; exit 1; }
    sums="$(mktemp -d)/SHA256SUMS"
    gh release download "$tag" --repo "${GH_REPO:?GH_REPO must name owner/repo}" --pattern SHA256SUMS --output "$sums"
    attested "$sums" "$tag"
    [[ "$(awk '$2 == "./install.ps1" { print $1 }' "$sums")" == "$(sha256sum "$2" | cut -d' ' -f1)" ]] \
      || { echo "attestation.sh: $2 is not the install.ps1 $tag attested" >&2; exit 1; }
    echo "$tag"
    ;;
  *)
    echo "usage: attestation.sh sums FILE TAG | script FILE | tag FILE" >&2
    exit 2
    ;;
esac
