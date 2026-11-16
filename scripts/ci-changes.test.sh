#!/usr/bin/env bash
set -euo pipefail
script="$(cd "$(dirname "$0")" && pwd)/ci-changes.sh"
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
for event in push workflow_dispatch workflow_call; do
  [[ "$(EVENT=$event "$script" | grep -c '=true')" == 5 ]] || { echo "ci-changes: $event must run every job" >&2; status=1; }
done
repo="$(mktemp -d)"
trap 'rm -rf "$repo"' EXIT
(
  cd "$repo"
  git init -q
  mkdir -p crates/x && seq 1 50 > crates/x/sec.rs && git add . && git -c commit.gpgsign=false -c user.name=t -c user.email=t@t commit -qm a
  mkdir docs && git mv crates/x/sec.rs docs/sec.rs && git -c commit.gpgsign=false -c user.name=t -c user.email=t@t commit -qm b
  [[ "$(EVENT=pull_request "$script" | grep -c '=true')" == 5 ]]
) || { echo "ci-changes: moving code into docs/ must still run every job" >&2; status=1; }
exit "$status"
