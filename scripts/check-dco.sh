#!/usr/bin/env bash
# Fails unless every commit in BASE..HEAD carries "Signed-off-by:" with its author's email; a merge
# is exempt only when it adds nothing beyond combining its parents
# (Developer Certificate of Origin, CONTRIBUTING.md "Contribution terms").
set -euo pipefail
base="${1:?usage: check-dco.sh BASE HEAD}"
head="${2:?usage: check-dco.sh BASE HEAD}"
git rev-list --quiet "$base..$head"
status=0
while IFS= read -r commit; do
  if [[ "$(git rev-list --no-walk --count --merges "$commit")" == 1 && -z "$(git show --cc --format= "$commit")" ]]; then
    continue
  fi
  email="$(git show -s --format=%ae "$commit")"
  if ! git show -s --format='%(trailers:key=Signed-off-by,valueonly)' "$commit" | grep -qiF "<$email>"; then
    fix="git commit --amend -s for the last commit, git rebase --signoff $base for a branch without merges"
    [[ "$(git rev-list --no-walk --count --merges "$commit")" == 0 ]] || fix="git commit --amend -s on the merge, or redo it with git merge --signoff"
    echo "::error::$(git show -s --format='%h %s' "$commit"): no 'Signed-off-by: … <$email>'; sign it off ($fix)"
    status=1
  fi
done < <(git rev-list "$base..$head")
exit "$status"
