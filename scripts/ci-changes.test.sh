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
expect "code" docs/vex.json
expect "code shared nsh mobile linux" crates/app/src/lib.rs README.md
expect "code shared nsh mobile linux" .github/workflows/ci.yml
expect "code" .github/workflows/ci-retry.yml .github/zizmor.yml
expect "code" .github/workflows/release.yml
expect "code shared nsh mobile linux" .github/workflows/nightly.yml scripts/appcast.sh
expect "code shared nsh mobile linux" Cargo.lock
expect "code shared nsh mobile linux" mobile/app/assets/brand/icon.png crates/store/src/lib.rs
expect "code linux" linux/src/main.rs linux/README.md
expect "code mobile" mobile/app/src/lib.rs
expect "code mobile" mobile/app/assets/brand/icon.png
expect "code nsh mobile linux" nsh-shared/src/theme.rs
[[ "$(EVENT=workflow_dispatch "$script" | grep -c '=true')" == 5 ]] || { echo "ci-changes: manual runs must run every job" >&2; status=1; }
for event in push workflow_dispatch; do
  [[ "$(EVENT=$event "$script" | grep -c '=true')" == 5 ]] || { echo "ci-changes: $event must run every job" >&2; status=1; }
done
repo="$(mktemp -d)"
(
  cd "$repo"
  git init -q
  mkdir -p crates/x && seq 1 50 > crates/x/sec.rs && git add . && git -c commit.gpgsign=false -c user.name=t -c user.email=t@t commit -qm a
  mkdir docs && git mv crates/x/sec.rs docs/sec.rs && git -c commit.gpgsign=false -c user.name=t -c user.email=t@t commit -qm b
  [[ "$(EVENT=pull_request "$script" | grep -c '=true')" == 5 ]]
) || { echo "ci-changes: moving code into docs/ must still run every job" >&2; status=1; }
# A push reuses a pull request's CI only for the exact tree that passed; anything else runs every job.
fake="$(mktemp -d)"
trap 'rm -rf "$repo" "$fake"' EXIT
cat > "$fake/gh" <<'GH'
#!/usr/bin/env bash
[[ "${FAKE_FAIL:-}" == 1 ]] && exit 1
case "$2" in
  */commits/*/pulls) echo "${FAKE_PR_HEAD:-}" ;;
  */commits/"$SHA") echo tree-main ;;
  */commits/*) echo "${FAKE_HEAD_TREE:-tree-main}" ;;
  */runs*) echo "${FAKE_RUN:-completed success}" ;;
esac
GH
chmod +x "$fake/gh"
push() { env PATH="$fake:$PATH" EVENT=push GH_REPO=o/r SHA=0123456789012345678901234567890123456789 "$@" "$script" 2>/dev/null | grep -c '=true' || true; }
[[ "$(push FAKE_PR_HEAD=abc)" == 0 ]] || { echo "ci-changes: a push of a passed pull request's exact tree must skip" >&2; status=1; }
[[ "$(push FAKE_PR_HEAD=abc FAKE_RUN='completed failure')" == 5 ]] || { echo "ci-changes: a failed pull request run must not be reused" >&2; status=1; }
[[ "$(push FAKE_PR_HEAD=abc FAKE_RUN='in_progress null')" == 5 ]] || { echo "ci-changes: an unfinished pull request run must not be reused" >&2; status=1; }
[[ "$(push FAKE_PR_HEAD=abc FAKE_HEAD_TREE=tree-other)" == 5 ]] || { echo "ci-changes: another tree must run every job" >&2; status=1; }
[[ "$(push FAKE_PR_HEAD=)" == 5 ]] || { echo "ci-changes: a push without a merged pull request must run every job" >&2; status=1; }
[[ "$(push FAKE_PR_HEAD=abc FAKE_FAIL=1)" == 5 ]] || { echo "ci-changes: an API error must run every job" >&2; status=1; }
exit "$status"
