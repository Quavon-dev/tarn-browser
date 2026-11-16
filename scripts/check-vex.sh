#!/usr/bin/env bash
# Every advisory a deny.toml ignores needs a not_affected statement in docs/vex.json (SECURITY.md).
set -euo pipefail
cd "$(dirname "$0")/.."
status=0
for file in deny.toml ./*/deny.toml; do
  if awk '/^\[/ { section = $0 } section == "[advisories]" && /crate[[:space:]]*=/' "$file" | grep -q .; then
    echo "::error::$file ignores an advisory by crate; ignore it by its id with a VEX statement"
    status=1
  fi
  while read -r id; do
    jq -e --arg id "$id" 'any(.statements[]; (.vulnerability.name == $id or any(.vulnerability.aliases[]?; . == $id)) and .status == "not_affected")' docs/vex.json >/dev/null \
      || { echo "::error::$id is ignored in $file but has no not_affected statement in docs/vex.json"; status=1; }
  done < <(grep -oE '(RUSTSEC-[0-9]{4}-[0-9]{4}|GHSA(-[a-z0-9]{4}){3})' "$file" | sort -u)
done
exit "$status"
