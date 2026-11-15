#!/usr/bin/env bash
set -euo pipefail
script="$(dirname "$0")/ci-changes.sh"
status=0
expect() {
  local want="$1" got
  shift
  got="$(printf '%s\n' "$@" | "$script" --classify | grep '=true' | cut -d= -f1 | paste -sd' ' - || true)"
  [[ "$got" == "$want" ]] || { echo "ci-changes: $* -> '$got', expected '$want'" >&2; status=1; }
}
expect "" README.md docs/release.md mobile/evidence/a.png .github/ISSUE_TEMPLATE/bug.yml
expect "" ""
expect "code shared nsh mobile linux" docs/themes.md
expect "code shared nsh mobile linux" crates/app/src/lib.rs README.md
expect "code shared nsh mobile linux" .github/workflows/ci.yml
expect "code shared nsh mobile linux" Cargo.lock
expect "code shared nsh mobile linux" mobile/app/assets/brand/icon.png crates/store/src/lib.rs
expect "code linux" linux/src/main.rs linux/README.md
expect "code mobile" mobile/app/src/lib.rs
expect "code mobile" mobile/app/assets/brand/icon.png
expect "code nsh mobile linux" nsh-shared/src/theme.rs
[[ "$(EVENT=workflow_dispatch "$script" | grep -c '=true')" == 5 ]] || { echo "ci-changes: manual runs must run every job" >&2; status=1; }
[[ "$(EVENT=push BEFORE=0000000000000000000000000000000000000000 "$script" | grep -c '=true')" == 5 ]] || { echo "ci-changes: a new branch must run every job" >&2; status=1; }
[[ "$(EVENT=push BEFORE=deadbeef "$script" | grep -c '=true')" == 5 ]] || { echo "ci-changes: an unknown range must run every job" >&2; status=1; }
[[ "$(EVENT=push BEFORE=ffffffffffffffffffffffffffffffffffffffff "$script" | grep -c '=true')" == 5 ]] || { echo "ci-changes: a missing commit must run every job" >&2; status=1; }
exit "$status"
