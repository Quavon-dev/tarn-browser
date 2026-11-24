#!/usr/bin/env bash
# attestation.sh's checks with `gh` replaced by a stub: `release download` copies FAKE_SUMS and
# `attestation verify` vouches for FAKE_TAG under FAKE_IDENTITY's identity regex, or fails.
set -euo pipefail
script="$(cd "$(dirname "$0")" && pwd)/attestation.sh"
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
failed=0
mkdir "$dir/bin"
cat >"$dir/bin/gh" <<'STUB'
#!/usr/bin/env bash
case "$1 $2" in
  "release download")
    while [[ $# -gt 0 ]]; do [[ "$1" == --output ]] && cp "$FAKE_SUMS" "$2"; shift; done ;;
  "attestation verify")
    while [[ $# -gt 0 ]]; do [[ "$1" == --cert-identity-regex ]] && printf '%s' "$2" >"$FAKE_IDENTITY"; shift; done
    [[ -n "${FAKE_TAG:-}" ]] || exit 1
    printf '[{"verificationResult":{"statement":{"predicate":{"tag":"%s"}}}}]' "$FAKE_TAG" ;;
  *) exit 1 ;;
esac
STUB
chmod +x "$dir/bin/gh"
export PATH="$dir/bin:$PATH" GH_REPO=Quavon-dev/tarn-browser FAKE_SUMS="$dir/SHA256SUMS" FAKE_IDENTITY="$dir/identity"
sha() { if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }

printf "\$Tag = 'v1.2.3'\n\$Base = 'x'\n" >"$dir/install.ps1"
[[ "$("$script" tag "$dir/install.ps1")" == v1.2.3 ]] || { echo "FAIL tag" >&2; failed=1; }
printf "\$Tag = 'v1.2.3'; rm -rf /'\n" >"$dir/odd.ps1"
[[ -z "$("$script" tag "$dir/odd.ps1")" ]] || { echo "FAIL odd tag line" >&2; failed=1; }

printf '%s  ./install.ps1\n' "$(sha "$dir/install.ps1")" >"$dir/SHA256SUMS"
[[ "$(FAKE_TAG=v1.2.3 "$script" script "$dir/install.ps1")" == v1.2.3 ]] || { echo "FAIL attested script" >&2; failed=1; }
[[ "$(cat "$dir/identity")" == '^https://github\.com/Quavon-dev/tarn-browser/\.github/workflows/release\.yml@refs/(tags/v1\.2\.3|heads/main)$' ]] \
  || { echo "FAIL identity regex: $(cat "$dir/identity")" >&2; failed=1; }

refused() {
  local name="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    echo "FAIL $name: not refused" >&2
    failed=1
  fi
}
refused other-tag env FAKE_TAG=v9.9.9 "$script" script "$dir/install.ps1"
refused unattested "$script" script "$dir/install.ps1"
refused unattested-sums "$script" sums "$dir/SHA256SUMS" v1.2.3
refused bad-tag env FAKE_TAG='v1.2.3;x' "$script" sums "$dir/SHA256SUMS" 'v1.2.3;x'
refused no-tag env FAKE_TAG=v1.2.3 "$script" script "$dir/odd.ps1"
printf "\$Tag = 'v1.2.3'\n\$Base = 'changed'\n" >"$dir/changed.ps1"
refused changed-bytes env FAKE_TAG=v1.2.3 "$script" script "$dir/changed.ps1"
refused no-command "$script"
refused missing-arg "$script" sums "$dir/SHA256SUMS"

[[ "$failed" == 0 ]] || exit 1
echo "attestation.test.sh: all passed"
