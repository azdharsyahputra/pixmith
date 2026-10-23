#!/bin/sh
# Builds the Rust core and the pixmith-qa scenario runner, then runs every scenario headlessly against the real app
# code (SpriteModel, the canvas view's mouse handlers, file actions, ...) and exits non-zero if any fails.
#
#   scripts/qa.sh [PATTERN]       run the scenarios whose name contains PATTERN (default: all)
#
# Environment: PIXMITH_QA_OUT (PNG snapshots, default ./qa-out), PIXMITH_QA_TIMEOUT (seconds per scenario, default 180),
# PIXMITH_QA_VERBOSE=1 (print scenario notes), PIXMITH_JOBS (parallel jobs for cargo and swift).
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

if [ -n "${PIXMITH_JOBS:-}" ]; then
    CARGO_BUILD_JOBS=$PIXMITH_JOBS
    export CARGO_BUILD_JOBS
fi
cargo build --release --manifest-path core/Cargo.toml
# SwiftPM does not notice a rebuilt static library, so force the relink.
bin=$(swift build --package-path app --show-bin-path)
rm -f "$bin/pixmith-qa"
if [ -n "${PIXMITH_JOBS:-}" ]; then
    swift build --package-path app --product pixmith-qa -j "$PIXMITH_JOBS"
else
    swift build --package-path app --product pixmith-qa
fi
mkdir -p "${PIXMITH_QA_OUT:-qa-out}"
exec "$bin/pixmith-qa" "$@"
