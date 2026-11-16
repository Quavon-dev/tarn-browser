#!/bin/sh
# Fails when cargo-sources.json no longer matches linux/Cargo.lock; regenerate it with
# update-cargo-sources.sh.
set -e
here=$(cd "$(dirname "$0")" && pwd)
python3 -I - "$here/../../Cargo.lock" "$here/cargo-sources.json" <<'EOF'
import json, sys, tomllib
lock = tomllib.load(open(sys.argv[1], "rb"))
wanted = {(p["name"], p["version"], p["checksum"]) for p in lock["package"] if p.get("source", "").startswith("registry+")}
vendored = set()
for source in json.load(open(sys.argv[2])):
    if isinstance(source, dict) and source.get("type") == "archive":
        name, file = source["url"].split("/crates/", 1)[1].split("/", 1)
        vendored.add((name, file.removeprefix(name + "-").removesuffix(".crate"), source["sha256"]))
missing = sorted(wanted - vendored)
extra = sorted(vendored - wanted)
for name, version, _ in missing:
    print(f"missing from cargo-sources.json: {name} {version}", file=sys.stderr)
for name, version, _ in extra:
    print(f"not in linux/Cargo.lock: {name} {version}", file=sys.stderr)
if missing or extra:
    sys.exit("cargo-sources.json is out of date: run linux/packaging/flatpak/update-cargo-sources.sh")
print(f"cargo-sources.json matches linux/Cargo.lock ({len(wanted)} crates)")
EOF
