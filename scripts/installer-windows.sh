#!/usr/bin/env bash
# Compiles packaging/windows/tarn.iss into OUT/Tarn-v<version>-windows-x64-setup.exe from the folder
# scripts/package-windows.sh made (Git Bash on Windows). Unsigned: scripts/sign-windows.ps1 builds the
# signed one. ISCC names Inno Setup's compiler when it is not in a default place.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
src="${1:?usage: installer-windows.sh PACKAGE_DIR OUT_DIR}"
out="${2:?usage: installer-windows.sh PACKAGE_DIR OUT_DIR}"
[[ -f "$src/tarn.exe" ]] || { echo "$src holds no tarn.exe" >&2; exit 1; }
version="${TARN_BUILD_VERSION:-$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n1)}"
[[ "$version" =~ ^[0-9]+(\.[0-9]+){0,3}$ ]] || { echo "Cargo.toml has no usable version: '$version'" >&2; exit 1; }
iscc="${ISCC:-}"
for candidate in "$iscc" "/c/Program Files (x86)/Inno Setup 6/ISCC.exe" "${LOCALAPPDATA:-}/Programs/Inno Setup 6/ISCC.exe"; do
  [[ -n "$candidate" && -f "$candidate" ]] && { iscc="$candidate"; break; }
done
[[ -f "$iscc" ]] || { echo "Inno Setup 6 not found; set ISCC to ISCC.exe" >&2; exit 1; }
mkdir -p "$out"
MSYS2_ARG_CONV_EXCL='*' "$iscc" /Q "/DAppVersion=$version" "/DPackageDir=$(cygpath -w "$(cd "$src" && pwd)")" "/DOutputDir=$(cygpath -w "$(cd "$out" && pwd)")" "$(cygpath -w "$root/packaging/windows/tarn.iss")"
echo "$out/Tarn-v$version-windows-x64-setup.exe"
