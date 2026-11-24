#!/usr/bin/env bash
# Starts a release: checks that CI passed on main and that every version file carries main's
# version, then pushes a signed tag v<version> of main. release.yml builds, signs, notarises and
# publishes it once a maintainer approves its release-signing deployment (docs/release.md).
# --critical marks the release critical in both update feeds.
# Usage: scripts/publish-release.sh [--skip-ci] [--critical]
set -euo pipefail

R=Quavon-dev/tarn-browser
NEW="$(cd "$(dirname "$0")/.." && pwd)"

ask() { local a; read -r -p "$1 [y/N] " a; [[ $a == [yY]* ]]; }

SKIP_CI=false; CRITICAL=false
for arg in "$@"; do
  case "$arg" in
    --skip-ci) SKIP_CI=true ;;
    --critical) CRITICAL=true ;;
    *) echo "unknown option $arg"; exit 1 ;;
  esac
done

git -C "$NEW" fetch -q origin main 'refs/tags/v*:refs/tags/v*'
VERSION=$(git -C "$NEW" show origin/main:Cargo.toml | sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' | head -n1)
TAG=v$VERSION
if git -C "$NEW" ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
  echo "$TAG exists; its release run is at https://github.com/$R/actions/workflows/release.yml"
  exit 0
fi

run=$(gh run list -R "$R" --workflow CI --branch main --event push -L 1 --json databaseId,headSha --jq '.[0] | "\(.databaseId) \(.headSha)"')
read -r run_id run_sha <<<"$run"
[ "$run_sha" = "$(git -C "$NEW" rev-parse origin/main)" ] || { echo "The latest CI run is not for origin/main yet; wait a moment."; exit 1; }
jobs=$(gh api "repos/$R/actions/runs/$run_id/jobs" --jq '.jobs[] | "\(.name) \(.status) \(.conclusion)"')
failed=$(awk '$2=="completed" && $3!="success" && $3!="skipped"' <<<"$jobs")
pending=$(awk '$2!="completed"' <<<"$jobs")
[ -z "$failed" ] || { echo "CI on main failed, not releasing:"; echo "$failed"; exit 1; }
if [ -n "$pending" ]; then
  $SKIP_CI || { echo "CI on main is still running:"; echo "$pending"; echo "Wait, or run with --skip-ci to release without those results."; exit 1; }
  echo "Releasing without waiting for:"; echo "$pending"
fi

tmp=$(mktemp -d)
git -C "$NEW" worktree add -q --detach "$tmp/main" origin/main
if "$tmp/main/scripts/check-version.sh" "$TAG"; then ok=true; else ok=false; fi
git -C "$NEW" worktree remove --force "$tmp/main"; rm -rf "$tmp"
$ok || { echo "Version files or CHANGELOG.md disagree with $TAG; fix them first."; exit 1; }

message=(-m "$TAG")
$CRITICAL && message+=(-m critical)
ask "Tag origin/main ($(git -C "$NEW" log -1 --format='%h %s' origin/main)) as $TAG$($CRITICAL && echo ', critical'), signed, and push it?" || exit 0
git -C "$NEW" tag -s "$TAG" origin/main "${message[@]}"
git -C "$NEW" push origin "refs/tags/$TAG"
echo "Pushed $TAG. Approve its release-signing deployments (sign and sign-android) when the builds finish:"
echo "  https://github.com/$R/actions/workflows/release.yml"
