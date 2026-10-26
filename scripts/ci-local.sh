#!/bin/sh
# Runs the same checks as .github/workflows/ci.yml for core and desktop (fmt, clippy, tests), so a push never
# reaches GitHub red. The pre-push hook runs it; run it by hand with `scripts/ci-local.sh`.
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
echo "ci-local: all green"
