#!/usr/bin/env bash
# Fails unless every non-merge commit in BASE..HEAD carries "Signed-off-by:" with its author's email
# (Developer Certificate of Origin, CONTRIBUTING.md "Contribution terms").
set -euo pipefail
base="${1:?usage: check-dco.sh BASE HEAD}"
head="${2:?usage: check-dco.sh BASE HEAD}"
status=0
while IFS= read -r commit; do
  email="$(git show -s --format=%ae "$commit")"
  if ! git show -s --format='%(trailers:key=Signed-off-by,valueonly)' "$commit" | grep -qiF "<$email>"; then
    echo "::error::$(git show -s --format='%h %s' "$commit"): no 'Signed-off-by: … <$email>'; sign it off with git commit -s (git rebase --signoff $base for a whole branch)"
    status=1
  fi
done < <(git rev-list --no-merges "$base..$head")
exit "$status"
