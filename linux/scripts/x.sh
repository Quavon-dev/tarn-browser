#!/bin/sh
exec docker exec -e DISPLAY=:99 -e DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/bus tarn-p2 sh -c "$*"
