#!/usr/bin/env bash
set -euo pipefail
check="$(cd "$(dirname "$0")" && pwd)/check-workflows.sh"
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
failed=0

expect() {
  local want="$1" name="$2" body="$3"
  printf '%s\n' "$body" > "$dir/$name.yml"
  if "$check" "$dir/$name.yml" 2>/dev/null; then got=pass; else got=fail; fi
  [[ "$got" == "$want" ]] || { echo "FAIL $name: expected $want, got $got" >&2; failed=1; }
}

expect pass trusted $'on:\n  push:\n    branches: [main]\n  schedule:\n    - cron: "1 * * * *"\njobs:\n  a:\n    runs-on: [self-hosted, Linux]'
expect pass hosted $'on: [pull_request, issues]\njobs:\n  a:\n    runs-on: ubuntu-24.04\n  b:\n    runs-on: macos-15\n  c:\n    runs-on: windows-2025'
expect fail lookalike $'on: [pull_request]\njobs:\n  a:\n    runs-on: ubuntu-fast'
expect fail lookalike_version $'on: [pull_request]\njobs:\n  a:\n    runs-on: macos-15-arm64-private'
expect fail labels $'on:\n  pull_request:\njobs:\n  a:\n    runs-on: [self-hosted, Linux]'
expect fail string $'on: issue_comment\njobs:\n  a:\n    runs-on: self-hosted'
expect fail expression $'on: [pull_request_target, schedule]\njobs:\n  a:\n    runs-on: ${{ github.event_name == \'schedule\' && \'self-hosted\' || \'ubuntu-24.04\' }}'
expect fail group $'on: [issues]\njobs:\n  a:\n    runs-on:\n      group: fleet'
expect fail reusable $'on: [pull_request]\njobs:\n  a:\n    uses: ./.github/workflows/release.yml'
expect fail other_event $'on: [workflow_run]\njobs:\n  a:\n    runs-on: [self-hosted]'
expect fail missing $'on: [pull_request]\njobs:\n  a:\n    steps: []'
expect fail environment $'on: [issue_comment]\njobs:\n  a:\n    runs-on: ubuntu-24.04\n    environment: chromium-update'
expect fail environment_map $'on: [pull_request_target]\njobs:\n  a:\n    runs-on: ubuntu-24.04\n    environment:\n      name: x'
expect pass trusted_environment $'on: [schedule]\njobs:\n  a:\n    runs-on: ubuntu-24.04\n    environment: chromium-update'
expect fail unparsable $'on: [pull_request]\njobs: [\n  a: {'

[[ "$failed" == 0 ]] && echo "check-workflows: all cases pass"
exit "$failed"
