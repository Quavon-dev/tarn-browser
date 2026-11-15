#!/bin/sh
# usage: scenario.sh <name> <address> [x y]... ; loads the address, clicks each point, saves the event log and HTTP log lines
d=$(dirname "$0")
name=$1; address=$2; shift 2
n=$("$d/x.sh" "wc -l < /tmp/app.log"); h=$("$d/x.sh" "wc -l < /tmp/http.log")
"$d/click.sh" 130 600; "$d/x.sh" "xdotool key Escape"; sleep 1; "$d/go.sh" "$address"; sleep 3
while [ $# -ge 2 ]; do "$d/click.sh" "$1" "$2"; sleep 2; shift 2; done
sleep 2
mkdir -p "$d/../evidence"
{ echo "# $name: $address"; "$d/x.sh" "tail -n +$((n+1)) /tmp/app.log | grep '\[tarn\]\|^tarn'"; echo "# server"; "$d/x.sh" "tail -n +$((h+1)) /tmp/http.log | grep -a GET"; } > "$d/../evidence/$name.txt"
cat "$d/../evidence/$name.txt"
"$d/shot.sh" "$name" >/dev/null
