#!/bin/sh
# Packs an extension folder into a .pixmith-extension file (a zip with package.json at its root).
#
#   scripts/extension-template/pack.sh [folder] [output]
#
# folder defaults to the folder this script is in; output to ./<name>-<version>.pixmith-extension.
# Needs `zip`, or python3 as a fallback. Install the result with Preferences > Extensions > Add Extension...,
# by opening it in Pixmith, or by dropping it on the window.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
folder=${1:-$here}
folder=$(cd "$folder" && pwd)
[ -f "$folder/package.json" ] || { echo "pack: $folder has no package.json" >&2; exit 1; }

field() { sed -n "s/^[[:space:]]*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" "$folder/package.json" | head -n 1; }
name=$(field name)
version=$(field version)
[ -n "$name" ] && [ -n "$version" ] || { echo "pack: package.json needs \"name\" and \"version\"" >&2; exit 1; }

out=${2:-"$PWD/$name-$version.pixmith-extension"}
# A drive path (C:\... or C:/...) from Git Bash on Windows is absolute too.
case "$out" in /* | [A-Za-z]:*) ;; *) out="$PWD/$out" ;; esac
rm -f "$out"

# Everything except packing leftovers and hidden files; the script itself stays out of the package.
cd "$folder"
files=$(find . -type f ! -name '.*' ! -name 'pack.sh' ! -name '*.pixmith-extension' ! -path '*/.*' | sed 's|^\./||' | sort)
if command -v zip >/dev/null 2>&1; then
  # shellcheck disable=SC2086
  echo "$files" | zip -q -X -@ "$out"
else
  python3 - "$out" $files <<'PY'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], "w", zipfile.ZIP_DEFLATED) as z:
    for f in sys.argv[2:]:
        z.write(f, f)
PY
fi
echo "Packed $out"
