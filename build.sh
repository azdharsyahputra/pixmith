#!/bin/sh
# Builds the Rust core and the Swift app, then assembles build/Pixmith.app.
set -eu
root=$(cd "$(dirname "$0")" && pwd)
# Match the app's minimum macOS so the Rust objects link without version warnings.
export MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET:-13.0}

cargo build --release --manifest-path "$root/core/Cargo.toml"
swift build -c release --package-path "$root/app" --product Pixmith

app="$root/build/Pixmith.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$(swift build -c release --package-path "$root/app" --show-bin-path)/Pixmith" "$app/Contents/MacOS/Pixmith"
cp "$root/Info.plist" "$app/Contents/Info.plist"
mkdir -p "$app/Contents/Resources"
cp "$root/assets/AppIcon.icns" "$app/Contents/Resources/AppIcon.icns"
codesign --force --sign - "$app"
echo "Built $app"
cp "$root/core/target/release/pixmith-cli" "$root/build/pixmith-cli"
echo "Built $root/build/pixmith-cli"
