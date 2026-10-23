#!/bin/sh
# Regenerates assets/AppIcon.icns (and the 512 px assets/logo.png used by the README) from scripts/make-icon.swift. The .icns is committed, so building
# the app does not need this.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

swift "$root/scripts/make-icon.swift" "$tmp/AppIcon.iconset"
mkdir -p "$root/assets"
iconutil --convert icns --output "$root/assets/AppIcon.icns" "$tmp/AppIcon.iconset"
cp "$tmp/AppIcon.iconset/icon_256x256@2x.png" "$root/assets/logo.png"
echo "Wrote $root/assets/AppIcon.icns and $root/assets/logo.png"
