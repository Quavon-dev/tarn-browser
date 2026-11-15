#!/bin/sh
# Runs the display tests (WebKitGTK under Xvfb) with HOME and the XDG directories in a fresh
# temporary directory, so the ephemeral-store test can scan everything the engine wrote.
set -e
cd "$(dirname "$0")/.."
bin=$(cargo test --locked --no-run --message-format=json | python3 -c "import json,sys
for l in sys.stdin:
    m = json.loads(l)
    if m.get(\"executable\") and m.get(\"profile\", {}).get(\"test\"): print(m[\"executable\"])")
[ -n "$bin" ] || { echo "no test binary was built" >&2; exit 1; }
Xvfb :98 -screen 0 1024x768x24 >/dev/null 2>&1 &
dbus-daemon --session --address=unix:path=/tmp/bus --fork >/dev/null
tests=$("$bin" --ignored --list --format=terse 2>/dev/null | sed -n 's/: test$//p')
[ -n "$tests" ] || { echo "no display tests" >&2; exit 1; }
for t in $tests; do
  h=$(mktemp -d)
  env -i PATH=/usr/bin:/bin DISPLAY=:98 HOME=$h XDG_DATA_HOME=$h/data XDG_CACHE_HOME=$h/cache XDG_CONFIG_HOME=$h/config \
    XDG_RUNTIME_DIR=$h/run DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/bus GDK_BACKEND=x11 LIBGL_ALWAYS_SOFTWARE=1 \
    WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1 "$bin" --ignored --exact "$t" --test-threads=1 --nocapture "$@" >"$h/log" 2>&1 && status=0 || status=$?
  cat "$h/log"
  # libtest runs the tests off the main thread; WebKitGTK disposes its default context in an exit
  # handler on the main thread and crashes there (SIGTRAP or SIGABRT) once the tests have finished.
  # GTK runs on one thread per process, so each display test gets its own process.
  case $status in
    0) ;;
    133|134) grep -Eq '^test result: ok\. 1 passed; 0 failed' "$h/log" || exit "$status"
      echo "$t passed; WebKitGTK crashed at process exit (status $status)" >&2 ;;
    *) exit "$status" ;;
  esac
done
