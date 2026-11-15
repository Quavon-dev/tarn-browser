#!/bin/sh
quoted=$(printf '%s' "$1" | sed "s/'/'\\\\''/g")
"$(dirname "$0")/x.sh" "xdotool mousemove 780 31 click 1 key ctrl+a BackSpace && xdotool type --delay 10 '$quoted' && xdotool key Return"
