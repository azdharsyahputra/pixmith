#!/bin/sh
# Runs the same checks as .github/workflows/ci.yml (core and desktop: fmt, clippy, tests; on macOS also the app's
# scenarios), so a push never reaches GitHub red. The pre-push hook runs it; run it by hand with `scripts/ci-local.sh`.
#
# Tests use a throwaway HOME and support folder (never the real Pixmith data) but the real cargo cache.
# PIXMITH_JOBS sets the build jobs (default 4).
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
jobs=${PIXMITH_JOBS:-4}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
run() { echo "ci-local: $*"; HOME="$tmp" PIXMITH_SUPPORT_DIR="$tmp/support" CARGO_BUILD_JOBS="$jobs" nice -n 5 "$@"; }

for crate in core desktop; do
    cd "$root/$crate"
    run cargo fmt --check
    run cargo clippy -j "$jobs" --all-targets -- -D warnings
    run cargo test -j "$jobs"
done
# The macOS app's scenario runner (the `app` job): build.sh, then every pixmith-qa scenario.
if [ "$(uname)" = Darwin ]; then
    cd "$root"
    run ./build.sh
    run env PIXMITH_JOBS="$jobs" PIXMITH_QA_OUT="$tmp/qa-out" ./scripts/qa.sh
fi
echo "ci-local: all green"
