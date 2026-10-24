# Pixmith Live Link demo

A tiny Godot 4 project (default texture filter: Nearest) with a hero (`res://sprites/hero.tres`, laid out like
Pixmith's Godot export) playing in `main.tscn`, and the [Live Link addon](../addons/pixmith_live_link/README.md)
enabled (autoload `PixmithLiveLink`). `addons` here is a link to `../addons`; if your checkout turned it into a plain
file (Windows without symlinks), replace it with a copy of `godot/addons`.

## Run

1. Open `godot/demo/project.godot` in Godot 4 (4.3 or newer; tested with 4.7), or run it from a terminal:
   `godot --path godot/demo`.
2. Start Pixmith (Live mode on, the default). The label in the corner says "connected to Pixmith".
3. In Pixmith, create a document named `hero` (File > New, then save it as `hero.pxmt`),
   draw something and watch the sprite in the game change after each stroke. Space (or a click) switches the
   animation. The match is by name (`hero` = `hero.tres`) because the document was never exported to Godot; to link
   it for real, use File > Export > Godot and pick `godot/demo/sprites/hero.tres`.

`tools/make_hero.py` regenerates `sprites/hero.png` and `sprites/hero.tres` (standard library only).

Tests: `godot/tests/run.sh` runs this project headless against a mock Pixmith.
