#!/bin/sh
# Regenerates assets/AppIcon.icns from scripts/make-icon.swift. The .icns is committed, so building
# the app does not need this.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

swift "$root/scripts/make-icon.swift" "$tmp/AppIcon.iconset"
mkdir -p "$root/assets"
iconutil --convert icns --output "$root/assets/AppIcon.icns" "$tmp/AppIcon.iconset"
echo "Wrote $root/assets/AppIcon.icns"
