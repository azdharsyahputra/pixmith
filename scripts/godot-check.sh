#!/bin/sh
# Opt-in check: exports fixture sprites with pixmith-cli into a throwaway Godot 4 project and lets a real
# Godot (headless) import and load them. Skips (exit 0) when Godot is not installed.
#   GODOT=/path/to/Godot scripts/godot-check.sh
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)

godot=${GODOT:-}
if [ -z "$godot" ]; then
    for c in /Applications/Godot.app/Contents/MacOS/Godot "$(command -v godot 2>/dev/null || true)" "$(command -v Godot 2>/dev/null || true)"; do
        if [ -n "$c" ] && [ -x "$c" ]; then godot=$c; break; fi
    done
fi
if [ -z "$godot" ] || [ ! -x "$godot" ]; then
    echo "godot-check: Godot 4 not found (brew install --cask godot, or set GODOT=/path/to/Godot); skipping."
    exit 0
fi
echo "godot-check: using $(HOME=$(mktemp -d) "$godot" --version 2>/dev/null | tail -1)"

export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
cargo build --quiet --manifest-path "$root/core/Cargo.toml" --bin pixmith-cli --example godot_fixtures
cli=$root/core/target/debug/pixmith-cli
fixtures=$root/core/target/debug/examples/godot_fixtures

proj=$root/.scratch/godot-proj
rm -rf "$proj"
mkdir -p "$proj/art" "$root/.scratch"
cat > "$proj/project.godot" <<'EOF'
config_version=5

[application]
config/name="PixmithGodotCheck"

[rendering]
textures/canvas_textures/default_texture_filter=0
EOF
fx=$proj/fixtures
"$fixtures" "$fx"
cp "$fx/manifest.json" "$proj/manifest.json"
cp "$root/scripts/godot-check/check.gd" "$root/scripts/godot-check/resave.gd" "$proj/"
hero=$fx/hero.aseprite

export_hero() { # <tres> [flags...]
    out=$1; shift
    "$cli" export "$hero" --godot-root "$proj" --godot-tres "$out" "$@" 2>/dev/null
}
export_hero "$proj/art/hero.tres"
export_hero "$proj/art/hero_opt.tres" --trim --extrude --padding 1 --packing grid --godot-tscn
export_hero "$proj/art/pages/hero_pages.tres" --max-texture 16
export_hero "$proj/art/hero_anim.tres" --per-animation-png
export_hero "$proj/art/Spieler Ü 走.tres" --godot-tscn --trim --scale 1

# Godot writes editor settings and logs under $HOME; keep that out of the real home.
ghome=$(mktemp -d)
trap 'rm -rf "$ghome"' EXIT
run_godot() { HOME=$ghome "$godot" --headless --path "$proj" "$@"; }

echo "godot-check: importing"
run_godot --import > "$root/.scratch/godot-import.log" 2>&1 || true
if grep -E "Parse Error|Failed loading|Failed to load" "$root/.scratch/godot-import.log"; then
    echo "godot-check: FAIL import errors (see .scratch/godot-import.log)"; exit 1
fi

echo "godot-check: loading and comparing"
# Godot exits 0 even when a script fails to parse, so require the script's own OK line.
run_check() {
    log=$root/.scratch/godot-check.log
    run_godot --script res://check.gd -- "$@" > "$log" 2>&1 || true
    cat "$log"
    if ! grep -q "^OK: all checks passed" "$log"; then
        echo "godot-check: FAIL"; exit 1
    fi
}
run_check res://art/hero.tres res://art/hero_opt.tres res://art/hero_opt.tscn res://art/pages/hero_pages.tres \
    res://art/hero_anim.tres "res://art/Spieler Ü 走.tres" "res://art/Spieler Ü 走.tscn"

echo "godot-check: Godot re-saves our resource (text diff is informational)"
run_godot --script res://resave.gd -- res://art/hero.tres res://resave/hero.tres res://art/hero_opt.tscn res://resave/hero_opt.tscn
diff "$proj/art/hero.tres" "$proj/resave/hero.tres" > "$root/.scratch/godot-resave.diff" || true
diff "$proj/art/hero_opt.tscn" "$proj/resave/hero_opt.tscn" > "$root/.scratch/godot-resave-scene.diff" || true
echo "  diff: $root/.scratch/godot-resave.diff ($(wc -l < "$root/.scratch/godot-resave.diff") lines)"

echo "godot-check: uid preservation across a re-export"
# Godot's own saved file (with the uid Godot gave it) replaces ours, plus the texture's real uid.
uid=$(sed -n 's/^\[gd_resource .*uid="\(uid:\/\/[^"]*\)".*/\1/p' "$proj/resave/hero.tres" | head -1)
if [ -z "$uid" ]; then echo "godot-check: FAIL Godot saved no uid"; exit 1; fi
texuid=$(sed -n 's/^uid="\(uid:\/\/[^"]*\)"/\1/p' "$proj/art/hero.png.import" | head -1)
sed "s|^\[ext_resource type=\"Texture2D\" path=|[ext_resource type=\"Texture2D\" uid=\"$texuid\" path=|" "$proj/resave/hero.tres" > "$proj/art/hero.tres"
export_hero "$proj/art/hero.tres"
if ! head -1 "$proj/art/hero.tres" | grep -q "format=3 uid=\"$uid\"\]"; then
    echo "godot-check: FAIL header uid $uid was not preserved"; head -1 "$proj/art/hero.tres"; exit 1
fi
if ! grep -q "^\[ext_resource type=\"Texture2D\" uid=\"$texuid\" path=\"res://art/hero.png\"" "$proj/art/hero.tres"; then
    echo "godot-check: FAIL texture uid $texuid was not preserved"; exit 1
fi
run_godot --import > /dev/null 2>&1 || true
run_check res://art/hero.tres res://art/hero_opt.tres
echo "godot-check: PASS"
