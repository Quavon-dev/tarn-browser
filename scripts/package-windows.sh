#!/usr/bin/env bash
# Collects Tarn, the CEF runtime it loads from its own folder and the licence texts into DIR (Git
# Bash on Windows, with the Windows SDK and MSVC tools installed). tarn.exe is CEF's bootstrap
# launcher, which starts the Chromium sandbox and loads Tarn from tarn.dll (target/release/app_dll.dll);
# it gets Tarn's manifest and the 8 MiB main-thread stack the development tarn.exe links with.
# Unsigned: sign tarn.exe and tarn.dll with the same certificate, or the launcher refuses the DLL.
set -euo pipefail
cef="${CEF_PATH:?CEF_PATH must name the unpacked windows64 CEF build}"
dest="${1:?usage: package-windows.sh DIR}"
tool() {
  command -v "$1" || { find "/c/Program Files (x86)/Windows Kits/10/bin" "/c/Program Files/Microsoft Visual Studio" "/c/Program Files (x86)/Microsoft Visual Studio" -ipath "*/x64/$1" 2>/dev/null | grep -iv "/Hostx86/" || true; } | sort -V | tail -1
}
mt="$(tool mt.exe)"
editbin="$(tool editbin.exe)"
[ -n "$mt" ] && [ -n "$editbin" ] || { echo "package-windows.sh: mt.exe and editbin.exe not found" >&2; exit 1; }
# tarn.dll links the Visual C++ runtime dynamically, which a clean Windows does not have.
crt="$( { find "/c/Program Files/Microsoft Visual Studio" "/c/Program Files (x86)/Microsoft Visual Studio" -ipath "*/VC/Redist/MSVC/*/x64/Microsoft.VC*.CRT/vcruntime140.dll" 2>/dev/null | grep -iv "/onecore/" || true; } | sort -V | tail -1)"
[ -n "$crt" ] || { echo "package-windows.sh: vcruntime140.dll not found in the Visual Studio redistributables" >&2; exit 1; }
rm -rf "$dest"
mkdir -p "$dest"
cp "$cef/bootstrap.exe" "$dest/tarn.exe"
cp target/release/app_dll.dll "$dest/tarn.dll"
cp "$crt" "$dest/"
dumpbin="$(tool dumpbin.exe)"
if [ -n "$dumpbin" ]; then
  # A runtime DLL tarn.dll needs but the folder lacks would only fail on a clean machine.
  missing="$(MSYS2_ARG_CONV_EXCL='*' "$dumpbin" -nologo /dependents "$(cygpath -w "$dest/tarn.dll")" | tr -d '\r' | grep -ioE '^ +(vcruntime|msvcp|concrt)[^ ]*\.dll' | tr -d ' ' | while read -r dll; do [ -e "$dest/$dll" ] || [ -e "$dest/${dll,,}" ] || echo "$dll"; done)"
  [ -z "$missing" ] || { echo "package-windows.sh: tarn.dll needs runtime DLLs not shipped: $missing" >&2; exit 1; }
fi
"$mt" -nologo "-inputresource:$(cygpath -w "$dest/tarn.exe");#1" -manifest packaging/windows/tarn.manifest "-outputresource:$(cygpath -w "$dest/tarn.exe");#1"
"$editbin" -nologo -stack:0x800000 "$dest/tarn.exe"
cp "$cef"/*.dll "$cef"/*.pak "$cef"/*.dat "$cef"/*.bin "$dest/"
find "$cef" -maxdepth 1 -name '*.json' ! -name archive.json -exec cp {} "$dest/" \;
cp -R "$cef/locales" "$dest/"
cp LICENSE "$dest/LICENSE.txt"
cp packaging/third-party-licenses-windows.txt "$dest/Third-Party-Licenses.txt"
cp "$cef/LICENSE.txt" "$dest/CEF-LICENSE.txt"
cp "$cef/CREDITS.html" "$dest/Chromium-CREDITS.html"
