#!/usr/bin/env bash
# Every advisory a deny.toml ignores needs a not_affected statement in docs/vex.json (SECURITY.md).
set -euo pipefail
cd "$(dirname "$0")/.."
exec python3 -I - deny.toml ./*/deny.toml <<'PY'
import json, re, sys, tomllib

ID = re.compile(r"(RUSTSEC-\d{4}-\d{4}|GHSA(-[a-z0-9]{4}){3})")
covered = set()
for s in json.load(open("docs/vex.json"))["statements"]:
    if s.get("status") == "not_affected":
        covered.update([s["vulnerability"]["name"], *s["vulnerability"].get("aliases", [])])

status = 0
for path in sys.argv[1:]:
    with open(path, "rb") as f:
        ignores = tomllib.load(f).get("advisories", {}).get("ignore", [])
    for entry in ignores:
        if isinstance(entry, dict) and set(entry) <= {"id", "reason"} and isinstance(entry.get("id"), str):
            entry = entry["id"]
        if not isinstance(entry, str) or not ID.fullmatch(entry):
            print(f"::error::{path} ignores {entry!r}; ignore an advisory only by its id, with a VEX statement")
            status = 1
        elif entry not in covered:
            print(f"::error::{entry} is ignored in {path} but has no not_affected statement in docs/vex.json")
            status = 1
sys.exit(status)
PY
