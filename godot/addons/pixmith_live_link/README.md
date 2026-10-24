# Pixmith Live Link for Godot 4

Draw in [Pixmith](../../../README.md) and watch your running Godot game change, with no save, export or restart.
The addon connects to the Pixmith app on the same computer and swaps the `SpriteFrames` the game already loaded for
what is on the canvas, after every committed stroke. Playing animations keep playing. It also lets tools such as the
Pixmith MCP server take a screenshot of the running game.

## Install

1. Copy the folder `addons/pixmith_live_link` into your project's `addons/` folder.
2. Project > Project Settings > Plugins: enable **Pixmith Live Link**. (This registers the autoload
   `PixmithLiveLink`; disabling the plugin removes it.)
3. Run the game from the editor (or any debug build) with Pixmith open. Pixmith's Live mode must be on
   (Preferences > Allow AI agents to control Pixmith, on by default).

Written for Godot 4.x and tested with 4.7.2. The demo project in [`godot/demo`](../../demo/README.md) has it set up.

## How a document finds its sprite

Export the sprite once from Pixmith with **File > Export > Godot**. Pixmith remembers the project and the `.tres`
path in the document, so from then on it knows the `res://` path of the resource (`res://chars/hero.tres`). Live
Link lays the document out with the same export settings (scale, trim, padding, packing, layers, tags) and applies
it to the `SpriteFrames` the game has loaded from that path (`ResourceLoader` cache). If the game has not loaded
that resource yet, the sprite is kept and applied as soon as it is.

A document that was **never exported** has no path. Then Live Link matches by name: every `SpriteFrames` used by an
`AnimatedSprite2D` or `AnimatedSprite3D` in the running scene tree whose file name without `.tres` equals the
document name without its extension (`hero.pxmt` matches `res://anything/hero.tres`). Names must match exactly.

What happens to the `SpriteFrames`, in place (every node using it updates at once):

- each atlas page becomes an `ImageTexture`; each frame's `AtlasTexture` gets the new atlas, region and margin;
- frames and animations are added or removed to match the document; speed, loop and frame durations are set;
- an `AnimatedSprite` that plays an animation which still exists keeps playing it. If its animation was removed
  it switches to the first one.

The change only lives in memory. Nothing in your project is written; the `.tres` and PNG on disk stay as exported.
Export from Pixmith again (Cmd-Shift-G) when you want to keep what you drew.

## Safe by default

- It runs only when `OS.is_debug_build()` is true. A release export never connects to anything. It also stays out of
  the editor itself (`Engine.is_editor_hint()`), and out of web exports.
- Off switches: the project setting `pixmith/live_link/enabled` (Project Settings > Pixmith > Live Link, with
  Advanced Settings on) or the environment variable `PIXMITH_LIVE_LINK=0`.
- It only talks to `127.0.0.1`, with the token from the discovery file Pixmith writes in its own app data folder
  (`PIXMITH_SUPPORT_DIR` overrides that folder). If you press **Disconnect** in Pixmith it stops reconnecting.
- The socket is polled once per frame and never blocks. PNG decoding of a page happens on the main thread
  (a few milliseconds for pixel-art atlases); screenshot encoding runs on a worker thread.
- A lost connection is not an error: the game keeps the last sprites and the addon looks for Pixmith again every
  2 seconds, quietly. Real problems (a refused hello, a bad PNG) go to `push_warning`, once per kind.

If you want the addon out of your shipped files entirely, exclude `addons/pixmith_live_link/*` in the export preset's
Resources filter (and remove the autoload) for release presets.

## Game view

Next to `documents.list` the addon calls `game.poll`. When a tool asks for a screenshot (`game.capture`), the addon
grabs the game's viewport after the frame is drawn, scales it to at most 1280 px on the long side and answers with
`game.frame`.

## Limits

- Updates arrive after each **committed** stroke (when the mouse button is released), about 150 ms later.
- `SpriteFrames` only. TileSets and tilemaps are not updated.
- Only resources the game has loaded are touched; it never loads or creates resources by itself.
- `SpriteFrames` that are embedded in a scene (not a file) can only be matched through a linked `res://` path if they
  were loaded from that file; embedded ones are skipped when matching by name.
- The `.tres` on disk is not changed, so after a restart the game shows the exported version until Live Link
  connects again (a moment after start-up).
- Matching by name looks at the scene tree twice a second while a document is unlinked; export the document from
  Pixmith once to avoid that.
- In headless runs the viewport has no pixels; the screenshot answer is then a black image of the viewport's size.

## Protocol

The addon is a client of the [Pixmith Live protocol](../../../docs/LIVE_PROTOCOL.md): newline-delimited JSON-RPC 2.0
over loopback TCP; `hello` as `pixmith-godot`, `documents.list`, `godot.atlas`, `game.poll`, `game.frame`. Tests:
`godot/tests/run.sh` (a mock of the Pixmith app plus a headless Godot).
