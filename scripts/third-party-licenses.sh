#!/usr/bin/env bash
# Writes packaging/third-party-licenses.txt (macOS: app and Chromium helper, normal dependencies on
# aarch64-apple-darwin), packaging/third-party-licenses-windows.txt (app on x86_64-pc-windows-msvc),
# -linux.txt (linux/ on x86_64-unknown-linux-gnu), -ios.txt and -android.txt (mobile/ on aarch64):
# every crate compiled into the release with its licence and the licence files it ships; for an MIT or Apache-2.0 crate without licence files those texts follow at the end. --check fails when the committed file is out of date.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out=""
trap '[[ -z "$out" ]] || rm -f "$out.tmp"' EXIT

generate() {
  cargo metadata --manifest-path "$root/${4:-Cargo.toml}" --format-version 1 --locked \
    --filter-platform "$1" ${2:+--features "$2"} \
    | jq -r --arg roots "$3" '
      (.resolve.nodes | map({key: .id, value: [.deps[] | select(any(.dep_kinds[]; .kind == null)) | .pkg]}) | from_entries) as $graph
      | .workspace_members as $members
      | ($roots | split(" ")) as $names | [.packages[] | select(.name as $n | $names | index($n)) | .id] as $roots
      | ([$roots, {}] | until(.[0] | length == 0;
          .[0][0] as $id | if .[1][$id] then [.[0][1:], .[1]] else [.[0][1:] + $graph[$id], .[1] + {($id): true}] end)
        | .[1]) as $used
      | .packages[] | select($used[.id] and (.id as $id | $members | index($id) | not))
      | [.name, .version, (.license // "see licence files"), (.repository // "https://crates.io/crates/\(.name)/\(.version)"), ((.authors | join(", ")) | if . == "" then "-" else . end), (.manifest_path | sub("/Cargo.toml$"; ""))] | @tsv' \
    | LC_ALL=C sort -t$'\t' -k1,1 -k2,2V \
    | while IFS=$'\t' read -r name version license repository authors dir; do
        printf '=====\n%s %s\nLicence: %s\n' "$name" "$version" "$license"
        printf 'Source: %s\n' "$repository"
        files="$(find "$dir" -maxdepth 1 -type f \( -iname 'licen[cs]e*' -o -iname 'copying*' -o -iname 'notice*' -o -iname 'copyright*' \) | LC_ALL=C sort)"
        if [[ -z "$files" ]]; then
          if [[ "$authors" != - ]]; then printf 'Authors: %s\n' "$authors"; else printf 'Copyright holders: see Source.\n'; fi
          if [[ "$license" =~ MIT|Apache-2\.0 ]]; then
            printf '(The crate ships no licence file; the licence named above applies, its text is at the end.)\n'
          else
            printf '(The crate ships no licence file; see Source.)\n'
          fi
        else
          while IFS= read -r file; do printf '\n--- %s ---\n' "$(basename "$file")"; tr -d '\r' < "$file"; printf '\n'; done <<<"$files"
        fi
      done
}

write() {
  out="$root/packaging/$1"
  {
  printf 'Third-party software in Tarn\n\n%s Rust crates:\n\n' "$2"
  generate "$3" "${4:-}" "$5" "${6:-}"
  printf '=====\nMIT licence text, for the crates above that ship none (copyright: the authors named there or at their Source)\n\n'
  sed '1,/^Copyright/d' "$root/packaging/licenses/MIT.txt" | sed '1{/^$/d;}'
  printf '\n=====\nApache License 2.0 text, for the crates above that ship none\n\n'
  cat "$root/packaging/licenses/Apache-2.0.txt"
  } > "$out.tmp"
  if [[ "$check" == --check ]]; then
    cmp -s "$out.tmp" "$out" || { echo "packaging/$1 is out of date: run scripts/third-party-licenses.sh" >&2; exit 1; }
    rm "$out.tmp"
  else
    mv "$out.tmp" "$out"
  fi
}

check="${1:-}"
write third-party-licenses.txt $'Chromium and the Chromium Embedded Framework: Chromium-CREDITS.html and CEF-LICENSE.txt.\nSparkle: Sparkle-LICENSE.txt.' \
  aarch64-apple-darwin app/chromium,chromium-helper/chromium "app chromium-helper"
write third-party-licenses-windows.txt $'Chromium and the Chromium Embedded Framework: Chromium-CREDITS.html and CEF-LICENSE.txt.
The Microsoft Visual C++ runtime (vcruntime140.dll), redistributed under the Visual Studio license terms.' \
  x86_64-pc-windows-msvc "" app
write third-party-licenses-linux.txt 'WebKitGTK, GTK and the other system libraries come from your distribution under their own licences.' \
  x86_64-unknown-linux-gnu "" tarn-linux linux/Cargo.toml
write third-party-licenses-ios.txt 'WebKit and the system frameworks are part of iOS.' \
  aarch64-apple-ios "" tarn-mobile-ios mobile/Cargo.toml
write third-party-licenses-android.txt 'Android System WebView and the system libraries are part of Android.' \
  aarch64-linux-android "" tarn-mobile-android mobile/Cargo.toml
