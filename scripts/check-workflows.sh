#!/usr/bin/env bash
# Fails when a workflow that someone outside the project can start (pull requests, issues, comments
# and every other event besides push, schedule, workflow_dispatch and workflow_call) runs a job on
# anything but a GitHub-hosted image Tarn uses, hands its event to a reusable workflow or enters an
# environment. Self-hosted runners and environments hold signing keys.
set -euo pipefail
shopt -s nullglob
root="$(cd "$(dirname "$0")/.." && pwd)"
[[ $# -gt 0 ]] || set -- "$root"/.github/workflows/*.yml "$root"/.github/workflows/*.yaml
[[ $# -gt 0 ]] || { echo "no workflows found" >&2; exit 1; }
status=0
for workflow in "$@"; do
  triggers="$(yq -r '.on | ((select(tag == "!!map") | keys) // (select(tag == "!!seq")) // [.]) | .[]' "$workflow")"
  jobs="$(yq -r '.jobs | to_entries | .[] | .key + "\t" + (.value.uses // "-") + "\t" + (.value.environment // "-" | to_json(0)) + "\t" + (.value["runs-on"] | to_json(0))' "$workflow")"
  untrusted="$(grep -vxE 'push|schedule|workflow_dispatch|workflow_call' <<<"$triggers" || true)"
  [[ -z "$untrusted" ]] && continue
  while IFS=$'\t' read -r job uses environment runner; do
    if [[ "$environment" != '"-"' ]]; then
      echo "$workflow: job $job enters environment $environment on ${untrusted//$'\n'/ }" >&2
      status=1
    elif [[ "$uses" != - ]]; then
      echo "$workflow: job $job calls $uses on ${untrusted//$'\n'/ }" >&2
      status=1
    elif ! [[ "$runner" =~ ^\"(ubuntu-24\.04|macos-15|windows-2025)\"$ ]]; then
      echo "$workflow: job $job runs on $runner on ${untrusted//$'\n'/ }; only GitHub-hosted images are allowed" >&2
      status=1
    fi
  done <<<"$jobs"
done
exit "$status"
