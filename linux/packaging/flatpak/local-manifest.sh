#!/bin/sh
# Writes de.quavon.tarn.local.yml: the Flathub manifest with this checkout as the source instead of the
# release tag, for CI and local builds.
set -e
here=$(cd "$(dirname "$0")" && pwd)
python3 -I - "$here/de.quavon.tarn.yml" "$here/de.quavon.tarn.local.yml" <<'EOF'
import sys
lines = open(sys.argv[1]).read().split("\n")
start = lines.index("      - type: git")
end = lines.index("      - cargo-sources.json", start)
local = [
    "      - type: dir",
    "        path: ../../..",
    "        skip: [.git, target, linux/target, linux/dist, mobile, linux/evidence, docs/media]",
]
open(sys.argv[2], "w").write("\n".join(lines[:start] + local + lines[end:]))
EOF
