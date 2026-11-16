#!/bin/sh
# Writes de.quavon.tarn.local.yml: the Flathub manifest with this checkout as the source instead of the
# release tag, for CI and local builds.
set -e
here=$(cd "$(dirname "$0")" && pwd)
python3 -I - "$here/de.quavon.tarn.yml" "$here/de.quavon.tarn.local.yml" <<'EOF'
import sys, yaml
manifest = yaml.safe_load(open(sys.argv[1]))
sources = manifest["modules"][-1]["sources"]
sources[[s.get("type") if isinstance(s, dict) else None for s in sources].index("git")] = {
    "type": "dir",
    "path": "../../..",
    "skip": [".git", "target", "linux/target", "linux/dist", "mobile", "linux/evidence", "docs/media"],
}
yaml.safe_dump(manifest, open(sys.argv[2], "w"), sort_keys=False)
EOF
