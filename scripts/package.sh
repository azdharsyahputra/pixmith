#!/bin/sh
# Builds a release Pixmith.app and packs it into dist/Pixmith-<version>.dmg.
#
# The app is universal (arm64 + x86_64) when both Rust targets are available; with rustup the missing
# ones are installed, without it the script builds the native architecture only and says so.
#
# Optional environment:
#   PIXMITH_SIGN_IDENTITY   codesign identity, e.g. "Developer ID Application: Name (TEAMID)". Default: ad-hoc.
#   PIXMITH_NOTARY_PROFILE  notarytool keychain profile; submits and staples the dmg (needs PIXMITH_SIGN_IDENTITY).
#   PIXMITH_JOBS            parallel jobs for cargo and swift (default: all cores).
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
dist="$root/dist"
work="$dist/work"
min_macos=13.0

version=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$root/Info.plist")
sign_identity=${PIXMITH_SIGN_IDENTITY:-}
notary_profile=${PIXMITH_NOTARY_PROFILE:-}

if [ -n "$notary_profile" ] && [ -z "$sign_identity" ]; then
    echo "error: PIXMITH_NOTARY_PROFILE needs PIXMITH_SIGN_IDENTITY" >&2
    exit 1
fi

# Rust triple and Swift architecture name for each slice.
slices="aarch64-apple-darwin:arm64 x86_64-apple-darwin:x86_64"

have_target() {
    [ -d "$(rustc --print sysroot)/lib/rustlib/$1/lib" ]
}

if command -v rustup >/dev/null 2>&1; then
    for s in $slices; do
        have_target "${s%%:*}" || rustup target add "${s%%:*}"
    done
fi

universal=yes
for s in $slices; do
    have_target "${s%%:*}" || universal=no
done
if [ "$universal" = no ]; then
    case $(uname -m) in
        arm64) slices="aarch64-apple-darwin:arm64" ;;
        *) slices="x86_64-apple-darwin:x86_64" ;;
    esac
    echo "warning: a Rust target is missing and rustup is not installed; building for the native architecture only (${slices#*:})" >&2
fi

# Deployment target for the Rust objects, so they match the app's minimum macOS version.
MACOSX_DEPLOYMENT_TARGET=$min_macos
export MACOSX_DEPLOYMENT_TARGET

rm -rf "$work"
mkdir -p "$work/bin"
swift_bins=""
cli_bins=""
for s in $slices; do
    rust_target=${s%%:*}
    arch=${s#*:}
    echo "==> Building $arch"
    CARGO_BUILD_JOBS=${PIXMITH_JOBS:-$(sysctl -n hw.ncpu)} cargo build --release --manifest-path "$root/core/Cargo.toml" --target "$rust_target"
    # Each slice links its own architecture's static library; the executables are combined afterwards.
    PIXMITH_CORE_LIB_DIR="$root/core/target/$rust_target/release"
    export PIXMITH_CORE_LIB_DIR
    set -- --configuration release --package-path "$root/app" --scratch-path "$work/swift-$arch" \
        --triple "$arch-apple-macosx$min_macos"
    if [ -n "${PIXMITH_JOBS:-}" ]; then set -- "$@" -j "$PIXMITH_JOBS"; fi
    swift build "$@" --product Pixmith
    swift_bins="$swift_bins $(swift build "$@" --show-bin-path)/Pixmith"
    cli_bins="$cli_bins $root/core/target/$rust_target/release/pixmith-cli"
done
unset PIXMITH_CORE_LIB_DIR

# Combines one binary per architecture into a single (universal) file.
combine() {
    out=$1
    shift
    if [ $# -eq 1 ]; then cp "$1" "$out"; else lipo -create "$@" -output "$out"; fi
}

app="$work/Pixmith.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
# shellcheck disable=SC2086
combine "$app/Contents/MacOS/Pixmith" $swift_bins
# shellcheck disable=SC2086
combine "$work/bin/pixmith-cli" $cli_bins
# The command line tool ships inside the app; Pixmith > Install Command Line Tool… links it onto the PATH.
cp "$work/bin/pixmith-cli" "$app/Contents/MacOS/pixmith-cli"
cp "$root/Info.plist" "$app/Contents/Info.plist"
cp "$root/assets/AppIcon.icns" "$app/Contents/Resources/AppIcon.icns"

echo "==> Signing"
if [ -n "$sign_identity" ]; then
    codesign --force --options runtime --timestamp --sign "$sign_identity" "$app/Contents/MacOS/pixmith-cli"
    codesign --force --options runtime --timestamp --sign "$sign_identity" "$app"
else
    echo "PIXMITH_SIGN_IDENTITY is not set; signing ad-hoc (the app will not pass Gatekeeper on other Macs)"
    codesign --force --sign - "$app/Contents/MacOS/pixmith-cli"
    codesign --force --sign - "$app"
fi
codesign --verify --strict "$app"

echo "==> Creating disk image"
stage="$work/dmg"
mkdir -p "$stage"
cp -R "$app" "$stage/Pixmith.app"
ln -s /Applications "$stage/Applications"
dmg="$dist/Pixmith-$version.dmg"
rm -f "$dmg"
hdiutil create -volname "Pixmith $version" -srcfolder "$stage" -format UDZO -ov "$dmg" >/dev/null

if [ -n "$sign_identity" ]; then
    codesign --force --timestamp --sign "$sign_identity" "$dmg"
fi
if [ -n "$notary_profile" ]; then
    echo "==> Notarizing"
    xcrun notarytool submit "$dmg" --keychain-profile "$notary_profile" --wait
    xcrun stapler staple "$dmg"
fi

echo "Built $dmg"
echo "Architectures: $(lipo -archs "$app/Contents/MacOS/Pixmith")"
shasum -a 256 "$dmg"
