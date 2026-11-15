#!/bin/sh
set -e
root=$(cd "$(dirname "$0")/../.." && pwd)
image=${TARN_LINUX_IMAGE:-tarn-linux-fedora}
docker rm -f tarn-p2 >/dev/null 2>&1 || true
docker run -d --name tarn-p2 -v tarn-linux-p2-target:/target:ro -v "$root":/work \
  -w /work/linux "$image" sleep infinity >/dev/null
docker exec -d tarn-p2 sh -c 'Xvfb :99 -screen 0 1440x900x24 >/tmp/xvfb.log 2>&1'
docker exec -d tarn-p2 sh -c 'cd /work/linux/tests/pages && python3 /work/linux/scripts/server.py 8765 >/tmp/http.log 2>&1'
docker exec tarn-p2 sh -c "echo 127.0.0.1 public.test >> /etc/hosts"
docker exec tarn-p2 dbus-daemon --session --address=unix:path=/tmp/bus --fork >/dev/null
echo "container tarn-p2 up"
