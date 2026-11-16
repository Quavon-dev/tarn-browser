#!/usr/bin/env bash
# Prints which CI jobs a change needs, as GitHub step outputs (code, shared, nsh, mobile, linux).
# With --classify it reads changed paths from stdin; otherwise it diffs a pull request's merge commit
# against its base. Pushes, manual runs and calls run every job, so main is always fully checked.
set -euo pipefail

all() { printf '%s=true\n' code shared nsh mobile linux; }

classify() {
  local code=false shared=false nsh=false mobile=false linux=false path
  while IFS= read -r path; do
    [[ -n "$path" ]] || continue
    case "$path" in
      # browser-core's tests read docs/themes.md.
      docs/themes.md) ;;
      *.md | docs/* | */evidence/* | LICENSE* | .github/ISSUE_TEMPLATE/* | .github/CODEOWNERS | .editorconfig) continue ;;
    esac
    code=true
    case "$path" in
      linux/*) linux=true ;;
      mobile/*) mobile=true ;;
      nsh-shared/*) nsh=true ;;
      *) shared=true ;;
    esac
  done
  $shared && nsh=true
  $nsh && mobile=true && linux=true
  printf 'code=%s\nshared=%s\nnsh=%s\nmobile=%s\nlinux=%s\n' "$code" "$shared" "$nsh" "$mobile" "$linux"
}

if [[ "${1:-}" == --classify ]]; then
  classify
  exit 0
fi

# --no-renames lists a moved file under its old path too, so moving code into docs/ still counts.
if [[ "${EVENT:-}" == pull_request ]] && files="$(git diff --no-renames --name-only HEAD^1 HEAD)"; then
  classify <<<"$files"
else
  all
fi
