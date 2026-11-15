#!/bin/sh
docker exec tarn-p2 sh -c "pkill -x tarn; pkill openbox" >/dev/null 2>&1 || true
sleep 1
docker exec -d -e DISPLAY=:99 -e GDK_BACKEND=x11 -e LIBGL_ALWAYS_SOFTWARE=1 \
  -e WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1 -e TARN_LOG_EVENTS=1 \
  -e DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/bus ${TARN_TEST_FILTER_KEY:+-e TARN_TEST_FILTER_KEY=$TARN_TEST_FILTER_KEY} \
  tarn-p2 sh -c "${WM:+$WM & }exec ${TARN_BIN:-/target/debug/tarn} >>/tmp/app.log 2>&1"
