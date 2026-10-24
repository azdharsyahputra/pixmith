#!/bin/sh
# Tests the Godot addon against a mock of the Pixmith app (no Pixmith needed). Headless Godot only.
#   godot/tests/run.sh            GODOT=/path/to/godot and PYTHON=python3 override the lookups
# Every scenario gets its own temporary PIXMITH_SUPPORT_DIR, used by both the mock and the game, so the real
# Pixmith app (and its discovery files) is never found or touched. Exits non-zero when anything fails.
set -u
here=$(cd "$(dirname "$0")" && pwd)
demo=$here/../demo
python=${PYTHON:-python3}

godot=${GODOT:-}
if [ -z "$godot" ]; then
    for c in "$(command -v godot 2>/dev/null || true)" "$(command -v Godot 2>/dev/null || true)" \
        /Applications/Godot.app/Contents/MacOS/Godot; do
        if [ -n "$c" ] && [ -x "$c" ]; then godot=$c; break; fi
    done
fi
if [ -z "$godot" ] || [ ! -x "$godot" ]; then
    echo "run.sh: Godot 4 not found (brew install --cask godot, or set GODOT=/path/to/godot)" >&2
    exit 2
fi

tmp=$(mktemp -d)
mock_pid=
cleanup() {
    if [ -n "$mock_pid" ]; then kill "$mock_pid" 2>/dev/null; fi
    rm -rf "$tmp"
}
trap cleanup EXIT INT TERM

echo "run.sh: $("$godot" --version 2>/dev/null | tail -1), $("$python" --version 2>&1)"

# Import the demo's PNG once (creates demo/.godot, which is git-ignored).
mkdir -p "$tmp/import-home"
HOME="$tmp/import-home" PIXMITH_SUPPORT_DIR="$tmp/import-support" "$godot" --headless --path "$demo" --import >"$tmp/import.log" 2>&1 || true

failed=0
total=0

# run_scenario NAME SCENARIO MODE [mock args...]
run_scenario() {
    name=$1; scenario=$2; mode=$3; shift 3
    total=$((total + 1))
    dir=$tmp/$name
    mkdir -p "$dir/home"
    "$python" "$here/mock_pixmith.py" --support-dir "$dir" --mode "$mode" "$@" >"$dir/mock.out" 2>&1 &
    mock_pid=$!
    i=0
    while [ ! -f "$dir/mock_state.json" ] && [ "$i" -lt 100 ]; do sleep 0.1; i=$((i + 1)); done
    if [ ! -f "$dir/mock_state.json" ]; then
        echo "FAILED  $name: the mock did not start"; cat "$dir/mock.out"
        kill "$mock_pid" 2>/dev/null; mock_pid=; failed=$((failed + 1)); return
    fi
    extra=
    case " $* " in *" --decoys "*) extra=--decoys ;; esac
    off=
    if [ "$scenario" = disabled ]; then off=0; fi
    env HOME="$dir/home" PIXMITH_SUPPORT_DIR="$dir" PIXMITH_LIVE_LINK="$off" \
        "$godot" --headless --path "$demo" -s "$here/live_link_test.gd" -- \
        --support-dir="$dir" --scenario="$scenario" --mode="$mode" $extra >"$dir/godot.out" 2>&1 &
    gpid=$!
    ( sleep 90; kill "$gpid" 2>/dev/null ) >/dev/null 2>&1 &
    watchdog=$!
    wait "$gpid"
    rc=$?
    kill "$watchdog" 2>/dev/null
    kill "$mock_pid" 2>/dev/null
    wait "$mock_pid" 2>/dev/null
    mock_pid=
    passes=$(grep -c '^  pass:' "$dir/godot.out")
    # Godot's own script errors and warnings (a parse error in the addon or demo, push_warning) fail the run too.
    if [ "$rc" -eq 0 ] && grep -q -E 'SCRIPT ERROR|Parse Error|^ERROR:|^WARNING:' "$dir/godot.out"; then rc=3; fi
    if [ "$rc" -eq 0 ]; then
        echo "ok      $name ($passes checks)"
    else
        failed=$((failed + 1))
        echo "FAILED  $name (exit $rc, $passes checks passed)"
        cat "$dir/godot.out"
        echo "--- mock.log"; cat "$dir/mock.log" 2>/dev/null
        echo "--- mock.out"; cat "$dir/mock.out"
    fi
}

# PIXMITH_LIVE_LINK is empty for the normal scenarios (switch not set) and 0 for "disabled".
run_scenario linked flow linked
run_scenario unlinked flow unlinked
run_scenario chunked-big-decoys flow linked --chunk 61 --big --decoys
run_scenario disabled disabled linked

if [ "$failed" -eq 0 ]; then
    echo "run.sh: all $total scenarios passed"
    exit 0
fi
echo "run.sh: $failed of $total scenarios FAILED"
exit 1
