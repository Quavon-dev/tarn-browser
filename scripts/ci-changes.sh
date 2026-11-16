#!/usr/bin/env bash
# Prints which CI jobs a change needs, as GitHub step outputs (code, shared, nsh, mobile, linux).
# With --classify it reads changed paths from stdin; otherwise it diffs a pull request's merge commit
# against its base. A push skips every job only when its tree is exactly the head of a merged pull
# request whose latest CI run passed (that code was already checked); other pushes, manual runs,
# calls and any API error run every job.
set -euo pipefail

all() { printf '%s=true\n' code shared nsh mobile linux; }
none() { printf '%s=false\n' code shared nsh mobile linux; }

# Prints the head SHA of a merged pull request whose tree equals $SHA's and whose latest CI run on
# that head succeeded; prints nothing otherwise.
tested_head() {
  local tree head result
  tree=$(gh api "repos/$GH_REPO/commits/$SHA" --jq .commit.tree.sha) || return 0
  for head in $(gh api "repos/$GH_REPO/commits/$SHA/pulls" --jq '.[] | select(.merged_at != null) | .head.sha'); do
    [[ "$(gh api "repos/$GH_REPO/commits/$head" --jq .commit.tree.sha)" == "$tree" ]] || continue
    result=$(gh api "repos/$GH_REPO/actions/workflows/ci.yml/runs?head_sha=$head&event=pull_request" \
      --jq '.workflow_runs | sort_by(.created_at) | last | "\(.status) \(.conclusion)"') || continue
    [[ "$result" == "completed success" ]] && { echo "$head"; return 0; }
  done
}

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
      # Other workflows and GitHub settings run none of the builds; the check job lints them.
      .github/workflows/ci.yml) shared=true ;;
      .github/*) ;;
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
elif [[ "${EVENT:-}" == push && -n "${GH_REPO:-}" && "${SHA:-}" =~ ^[0-9a-f]{40}$ ]] && head="$(tested_head)" && [[ -n "$head" ]]; then
  echo "pull request head $head has this exact tree and passed CI" >&2
  none
else
  all
fi
