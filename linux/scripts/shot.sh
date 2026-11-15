#!/bin/sh
set -e
dir=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$dir/evidence"
docker exec -e DISPLAY=:99 tarn-p2 sh -c "import -window root /tmp/shot.png"
docker cp tarn-p2:/tmp/shot.png "$dir/evidence/$1.png" >/dev/null
echo "evidence/$1.png"
