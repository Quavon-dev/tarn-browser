#!/usr/bin/env bash
# Builds the unsigned drag-to-install disk image: Tarn.app beside an Applications link in a Finder
# window with packaging/dmg-background.tiff and the Tarn volume icon. dmg.sh signs and notarises it.
#
#   scripts/dmg-layout.sh APP OUT.dmg
set -euo pipefail

app="${1:-}"
out="${2:-}"
[[ -d "$app" && "$(basename "$app")" == Tarn.app && "$out" == *.dmg ]] || { echo "usage: scripts/dmg-layout.sh <Tarn.app> <out.dmg>" >&2; exit 1; }
packaging="$(cd "$(dirname "$0")/../packaging" && pwd)"
tarn_mounted="$(osascript -e 'tell application "Finder" to exists disk "Tarn"')"
[[ "$tarn_mounted" == false ]] || { echo "a volume named Tarn is mounted: eject it first" >&2; exit 1; }

tmp="$(mktemp -d)"
detach() {
  local i
  for i in 1 2 3 4 5; do
    hdiutil detach -quiet "$1" && return 0
    sleep "$i"
  done
  hdiutil detach -force "$1"
}
mounted=
trap '[[ -z "$mounted" ]] || detach "$tmp/rw" 2>/dev/null || true; rm -rf "$tmp"' EXIT
mkdir -p "$tmp/stage/.background"
ditto "$app" "$tmp/stage/Tarn.app"
ln -s /Applications "$tmp/stage/Applications"
cp "$packaging/dmg-background.tiff" "$tmp/stage/.background/background.tiff"
size_mb=$(( $(du -sm "$tmp/stage" | cut -f1) + 32 ))
hdiutil create -quiet -volname Tarn -srcfolder "$tmp/stage" -size "${size_mb}m" -format UDRW -fs HFS+ "$tmp/rw.dmg"
hdiutil attach -quiet -noautoopen -mountpoint "$tmp/rw" "$tmp/rw.dmg"
mounted=1

layout() {
  osascript - "$tmp/rw" <<'APPLESCRIPT'
on show(d)
  tell application "Finder"
    repeat with attempt from 1 to 40
      try
        open d
        set w to container window of d
        set current view of w to icon view
        set toolbar visible of w to false
        set statusbar visible of w to false
        set bounds of w to {200, 120, 860, 648}
        return w
      on error message
        if attempt is 40 then error message
        delay 0.5
      end try
    end repeat
  end tell
end show

on run argv
  with timeout of 120 seconds
    tell application "Finder"
      set d to (POSIX file (item 1 of argv) as alias)
      set w to my show(d)
      set o to icon view options of w
      set arrangement of o to not arranged
      set icon size of o to 128
      set text size of o to 13
      set background picture of o to file ".background:background.tiff" of d
      set position of item "Tarn.app" of d to {165, 150}
      set position of item "Applications" of d to {495, 150}
      close w
      set w to my show(d)
      update d without registering applications
      delay 2
      close w
    end tell
  end timeout
end run
APPLESCRIPT
}
for attempt in 1 2 3; do
  layout && break
  [[ "$attempt" -lt 3 ]] || { echo "Finder could not lay out the disk image window" >&2; exit 1; }
  sleep 5
done
saved() { grep -q Iloc "$tmp/rw/.DS_Store" && grep -q icvp "$tmp/rw/.DS_Store"; } 2>/dev/null
for _ in $(seq 30); do
  saved && break
  sleep 1
done
saved || { echo "Finder did not save the window layout" >&2; exit 1; }

cp "$packaging/Tarn.icns" "$tmp/rw/.VolumeIcon.icns"
SetFile -a C "$tmp/rw"
rm -rf "$tmp/rw/.fseventsd" "$tmp/rw/.Trashes"
detach "$tmp/rw"
mounted=
rm -f "$out"
hdiutil convert -quiet "$tmp/rw.dmg" -format UDZO -imagekey zlib-level=9 -o "$out"
