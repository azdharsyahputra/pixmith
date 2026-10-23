# Aseprite fixtures

Real `.aseprite` / `.ase` files, and the PNGs that Aseprite exported for them, used by `core/tests/compat.rs`.
They are data files only. Nothing here comes from the Aseprite repository or from Aseprite's own distribution.

## asefile (Rust crate) - `asefile/`

- Source: https://github.com/alpine-alpaca/asefile (commit `2274c354cea6764f85597252a0d2228e64709348`), directory `tests/data/`
- License: MIT, Copyright (c) 2020 Alpine Alpaca Games. The license applies to the whole repository, including the test data. Copy: `asefile/LICENSE`.
- Files taken (every `.aseprite` in `tests/data/` except the large blend-mode ones, plus their reference PNGs):
  `256_color_old_palette_chunk`, `background`, `basic-16x16`, `big`, `blend_hue`, `blend_multiply`, `blend_normal`, `blend_overlay`,
  `blend_saturation_bug`, `cel_overflow`, `color-curve`, `grayscale`, `indexed`, `layers_and_tags`, `linked_cels`, `palette`, `rawcel`,
  `slice`, `slice_advanced`, `tilemap`, `tilemap_empty_edges`, `tilemap_grayscale`, `tilemap_indexed`, `tilemap_multi`, `tileset`,
  `transparency`, `user_data`, `util_extrude`, `util_indexed`.
- The other 15 blend-mode files (`blend_addition`, `blend_color`, ... 525 KB each) were checked locally against their reference PNGs
  (all 19 blend modes render within 3 per channel; normal is exact) but are not committed to keep the repository small.
- What the source project documents (from its own tests) and `compat.rs` asserts: sizes, frame/layer/tag counts, palette entries,
  tileset names and tile ids, slice keys, pivots and 9-patch centers, sprite/layer/cel/tag user data, and per-frame reference images.

## ase (Go package) - `ase-go/`

- Source: https://github.com/setanarut/aseprite (commit `013284dce733d226ff65b836105ff76a5b19a40a`), directory `test_files/`
- License: MIT, Copyright (c) 2026 Barış. Copy: `ase-go/LICENSE`.
- Files taken: `prop.ase`, `test_paletted.ase`, `test_nrgba.ase`, `test_grayscale.ase` and the `tilemap_*.png` images its tests compare against
  (those PNGs are produced by that project's own tilemap renderer, not exported by Aseprite).
- `compat.rs` asserts what that project's `ase_test.go` documents: layer names/types/opacity, user data, tags (range, repeat, direction,
  color), slice bounds/center/pivot/color, tileset properties, tile ids and flip flags, durations, palette and the 1:2 pixel ratio.

## Considered and skipped

- https://github.com/damienfamed75/aseprite (Apache-2.0) ships `player.ase` example art; the license of the art itself is not stated, so it is not used.
- https://github.com/ggez/aseprite `examples/boonga.ase`: artwork with no separate license; not used.
- https://github.com/TheCyberRonin/ase-parser (MIT): contains no sample files.
