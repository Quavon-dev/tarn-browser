#!/usr/bin/env bash
# The feed format and every attestation refusal of windows-feed.sh against fixtures; no network or keys.
set -euo pipefail
script="$(cd "$(dirname "$0")" && pwd)/windows-feed.sh"
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
failed=0
digest=$(printf 'a%.0s' {1..64})
commit=$(printf 'c%.0s' {1..40})
repo=https://github.com/Quavon-dev/tarn-browser

expect() {
  local want="$1" name="$2" got
  shift 2
  if bash -c 'source "$1"; shift; "$@"' windows-feed-test "$script" "$@" >"$dir/out" 2>&1; then got=pass; else got=refused; fi
  [[ "$got" == "$want" ]] || { echo "FAIL $name: expected $want, got $got: $(cat "$dir/out")" >&2; failed=1; }
}

feed=$(bash -c 'source "$1"; feed_json 0.4.0 false "$2" 123 SIG' windows-feed-test "$script" "$digest")
[[ "$feed" == '{"version":"0.4.0","critical":false,"installer":{"url":"https://github.com/Quavon-dev/tarn-browser/releases/download/v0.4.0/Tarn-v0.4.0-windows-x64-setup-unsigned.exe","sha256":"'"$digest"'","length":123,"signature":"SIG"}}' ]] \
  || { echo "FAIL feed format: $feed" >&2; failed=1; }

attestation() {
  jq -n --arg digest "$digest" --arg commit "$commit" --arg repo "$repo" '[{verificationResult: {
    statement: {subject: [{digest: {sha256: $digest}}], predicateType: "https://github.com/Quavon-dev/tarn-browser/attestations/release/v1",
      predicate: {commit: $commit, tag: "v0.4.0"}},
    signature: {certificate: {runnerEnvironment: "github-hosted", sourceRepositoryURI: $repo,
      buildSignerURI: "\($repo)/.github/workflows/release.yml@refs/tags/v0.4.0",
      buildConfigURI: "\($repo)/.github/workflows/release.yml@refs/tags/v0.4.0",
      buildSignerDigest: $commit, sourceRepositoryRef: "refs/tags/v0.4.0", sourceRepositoryDigest: $commit}}}}]' \
    | jq "$1" >"$dir/$2.json"
  echo "$dir/$2.json"
}
expect pass attestation check_attestation "$(attestation . good)" "$digest" "$commit" v0.4.0
expect refused other-digest check_attestation "$(attestation . good)" "$(printf 'b%.0s' {1..64})" "$commit" v0.4.0
expect refused other-tag check_attestation "$(attestation . good)" "$digest" "$commit" v0.4.1
expect refused other-commit check_attestation "$(attestation . good)" "$digest" "$(printf 'd%.0s' {1..40})" v0.4.0
expect refused branch-workflow check_attestation \
  "$(attestation '.[0].verificationResult.signature.certificate.buildSignerURI |= sub("refs/tags/v0.4.0"; "refs/heads/evil")' branch)" "$digest" "$commit" v0.4.0
expect refused branch-ref check_attestation \
  "$(attestation '.[0].verificationResult.signature.certificate.sourceRepositoryRef = "refs/heads/main"' ref)" "$digest" "$commit" v0.4.0
expect refused other-source-digest check_attestation \
  "$(attestation '.[0].verificationResult.signature.certificate.sourceRepositoryDigest = "x"' source)" "$digest" "$commit" v0.4.0
expect refused other-signer-digest check_attestation \
  "$(attestation '.[0].verificationResult.signature.certificate.buildSignerDigest = "x"' signer)" "$digest" "$commit" v0.4.0
expect refused self-hosted check_attestation \
  "$(attestation '.[0].verificationResult.signature.certificate.runnerEnvironment = "self-hosted"' runner)" "$digest" "$commit" v0.4.0
expect refused other-predicate-type check_attestation \
  "$(attestation '.[0].verificationResult.statement.predicateType = "https://slsa.dev/provenance/v1"' type)" "$digest" "$commit" v0.4.0
expect refused called-by-chromium-update check_attestation \
  "$(attestation '.[0].verificationResult.signature.certificate.buildConfigURI |= sub("release.yml@refs/tags/v0.4.0"; "chromium-update.yml@refs/heads/main")' config)" "$digest" "$commit" v0.4.0
expect refused fork check_attestation \
  "$(attestation '.[0].verificationResult.signature.certificate.sourceRepositoryURI = "https://github.com/someone/tarn-browser"' fork)" "$digest" "$commit" v0.4.0
expect refused predicate-commit check_attestation \
  "$(attestation '.[0].verificationResult.statement.predicate.commit = "x"' pcommit)" "$digest" "$commit" v0.4.0
expect refused predicate-tag check_attestation \
  "$(attestation '.[0].verificationResult.statement.predicate.tag = "v0.4.1"' ptag)" "$digest" "$commit" v0.4.0
expect refused no-signers env TARN_ALLOWED_SIGNERS="$dir/missing" bash "$script" v0.4.0
grep -q "no allowed signers file" "$dir/out" || { echo "FAIL no-signers: $(cat "$dir/out")" >&2; failed=1; }
expect refused no-attestation check_attestation "$(attestation '[]' none)" "$digest" "$commit" v0.4.0
expect refused bad-tag main v0.4
expect refused bad-option main v0.4.0 --later

[[ "$failed" == 0 ]] || exit 1
echo "windows-feed.test.sh: all passed"
