#!/usr/bin/env bash
# Times Tarn's startup on this Mac: builds the release binary, launches it RUNS times against a
# throwaway data folder with TARN_STARTUP_TRACE=1 and prints the median of each step. A first,
# unmeasured launch opens URL so that every measured launch restores one tab and commits it.
# It opens windows and loads URL over the network, so it runs on a desktop session, not in CI.
# Usage: scripts/startup-trace.sh [runs] [url]
set -euo pipefail
runs="${1:-5}"
url="${2:-https://example.com/}"
[[ "$runs" =~ ^[1-9][0-9]*$ ]] || { echo "runs must be a positive number" >&2; exit 2; }
root="$(cd "$(dirname "$0")/.." && pwd)"
cargo build --manifest-path "$root/Cargo.toml" --release --locked -p app --bin tarn
bin="${CARGO_TARGET_DIR:-$root/target}/release/tarn"
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
now_ms() { perl -MTime::HiRes=time -e 'printf "%d\n", time * 1000'; }

launch() {
  local log="$1"
  shift
  local started pid
  started="$(now_ms)"
  TARN_DATA_DIR="$dir/data" TARN_STARTUP_TRACE=1 "$bin" "$@" 2>"$log" &
  pid=$!
  for _ in $(seq 600); do
    grep -q ' first page commit$' "$log" && break
    sleep 0.05
  done
  sleep 2
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  grep -q ' first page commit$' "$log" || { echo "no page committed within 30 s; see $log" >&2; cat "$log" >&2; exit 1; }
  awk -v started="$started" '$1 == "tarn-startup" && $4 == "main" { print "-", $2 - started, "exec to main" }' "$log"
  awk '$1 == "tarn-startup" { step = $4; for (i = 5; i <= NF; i++) step = step " " $i; print "-", $3, step }' "$log"
}

launch "$dir/warmup.log" "$url" >/dev/null
for run in $(seq "$runs"); do
  launch "$dir/run$run.log" | sed "s/^-/$run/"
done | awk '
  { step = $3; for (i = 4; i <= NF; i++) step = step " " $i; if (!(step in ms)) names[++n] = step; ms[step] = ms[step] " " $2 }
  END {
    printf "%-28s %8s  %s\n", "step", "median", "runs (ms)"
    for (i = 1; i <= n; i++) {
      s = names[i]; c = split(substr(ms[s], 2), v, " ")
      for (a = 1; a <= c; a++) for (b = a + 1; b <= c; b++) if (v[b] + 0 < v[a] + 0) { t = v[a]; v[a] = v[b]; v[b] = t }
      median = c % 2 ? v[(c + 1) / 2] : (v[c / 2] + v[c / 2 + 1]) / 2
      printf "%-28s %8s  %s\n", s, median, substr(ms[s], 2)
    }
  }'
