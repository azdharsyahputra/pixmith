// Tweening API (core/src/tween.rs). Layers are stored as keyframed tracks in the layer's user data.
// prop: 0 PosX, 1 PosY, 2 Rotation (degrees, clockwise), 3 ScaleX, 4 ScaleY, 5 Opacity (0..1).
// ease: 0 Linear, 1 Hold, 2 InQuad, 3 OutQuad, 4 InOutQuad, 5 InOutCubic, 6 OutBack, 7 OutBounce,
//       8 Bezier (x1, y1, x2, y2 are used only for 8).
// Every mutating call is one undo step.
#pragma once
#include "pixmith.h"

bool pixmith_tween_set_key(PixmithDoc *doc, uint32_t layer, uint32_t prop, uint32_t frame, float value,
                        uint32_t ease, float x1, float y1, float x2, float y2);
bool pixmith_tween_remove_key(PixmithDoc *doc, uint32_t layer, uint32_t prop, uint32_t frame);
uint32_t pixmith_tween_key_count(const PixmithDoc *doc, uint32_t layer, uint32_t prop);
// Writes 7 floats: frame, value, ease code, x1, y1, x2, y2.
bool pixmith_tween_key_at(const PixmithDoc *doc, uint32_t layer, uint32_t prop, uint32_t index, float *out7);
// Value at a frame (the default when the property has no track).
float pixmith_tween_value_at(const PixmithDoc *doc, uint32_t layer, uint32_t prop, uint32_t frame);
// custom=false resets the pivot to the content center.
bool pixmith_tween_set_pivot(PixmithDoc *doc, uint32_t layer, float x, float y, bool custom);
// Writes the effective pivot (x, y).
bool pixmith_tween_get_pivot(const PixmithDoc *doc, uint32_t layer, float *out2);
bool pixmith_tween_set_rotsprite(PixmithDoc *doc, uint32_t layer, bool on);
// Bit 0 has keys, bit 1 RotSprite on, bit 2 custom pivot.
uint32_t pixmith_tween_options(const PixmithDoc *doc, uint32_t layer);
// Writes tweened cels for the key range; keep_source keeps the tracks and the source cel.
bool pixmith_tween_bake(PixmithDoc *doc, uint32_t layer, bool keep_source);
// Bit 0: non-integer scale (uneven pixel sizes).
uint32_t pixmith_tween_warnings(const PixmithDoc *doc, uint32_t layer);

// Flag 0 RotSprite, 1 snap scale to integer, 2 ignore opacity. Options bits: 0 keys, 1 RotSprite,
// 2 custom pivot, 3 snap scale, 4 opacity ignored.
bool pixmith_tween_set_flag(PixmithDoc *doc, uint32_t layer, uint32_t flag, bool on);
// mode: 0 all keys, 1 frames from..=to, 2 current frame (from). Returns the layer the cels went to or -1.
// Groups always bake into a new layer.
int32_t pixmith_tween_bake_ex(PixmithDoc *doc, uint32_t layer, uint32_t mode, uint32_t from, uint32_t to,
                           bool new_layer, bool keep_source);
uint32_t pixmith_tween_bake_all(PixmithDoc *doc, bool keep_source, bool new_layer);
bool pixmith_tween_move_key(PixmithDoc *doc, uint32_t layer, uint32_t prop, uint32_t from, uint32_t to);
bool pixmith_tween_set_ease(PixmithDoc *doc, uint32_t layer, uint32_t prop, uint32_t frame, uint32_t ease,
                         float x1, float y1, float x2, float y2);
uint32_t pixmith_tween_copy_keys(const PixmithDoc *doc, uint32_t layer, uint32_t from, uint32_t to);
uint32_t pixmith_tween_paste_keys(PixmithDoc *doc, uint32_t layer, uint32_t at);
// Sets keys for the properties selected by bit i of mask at one frame; values: 6 floats by property.
// ease must not be 8 (Bezier). One undo step.
bool pixmith_tween_set_keys_at(PixmithDoc *doc, uint32_t layer, uint32_t frame, uint32_t mask,
                            const float *values6, uint32_t ease);
// Writes 8 floats: x, y (rounded), rotation, scale x, scale y, opacity, pivot x, pivot y.
bool pixmith_tween_transform_at(const PixmithDoc *doc, uint32_t layer, uint32_t frame, float *out8);
// Writes first and last key frame.
bool pixmith_tween_range(const PixmithDoc *doc, uint32_t layer, uint32_t *out2);
// Warning text (free with pixmith_string_free) or NULL.
char *pixmith_tween_warning_text(const PixmithDoc *doc, uint32_t layer);
// Writes 4 floats: x0, y0, x1, y1 (pixel corners) of the tween source content.
bool pixmith_tween_source_bounds(const PixmithDoc *doc, uint32_t layer, float *out4);

// MARK: Tween rig (parented layers; core/src/gamedev/rig.rs)
// A layer can have a tween parent: its transform is the parent's transform composed with its own, about its own
// pivot. The link survives reordering, inserting and deleting layers. Options bit 5 (pixmith_tween_options) says a
// layer has a parent link.
// parent -1 unlinks. One undo step. Fails (false, nothing changed) for a cycle or a layer that cannot be tweened.
bool pixmith_tween_set_parent(PixmithDoc *doc, uint32_t layer, int32_t parent);
// The parent's layer index, or -1.
int32_t pixmith_tween_get_parent(const PixmithDoc *doc, uint32_t layer);
// Whether `parent` is a valid parent for `layer` (fills a menu: excludes the layer, its descendants, groups that
// contain it or that it contains).
bool pixmith_tween_can_parent(const PixmithDoc *doc, uint32_t layer, uint32_t parent);
// Returns the number of children; writes up to `cap` of their layer indices to `out` (out may be NULL).
uint32_t pixmith_tween_children(const PixmithDoc *doc, uint32_t layer, uint32_t *out, uint32_t cap);
// Writes 11 floats: composed matrix a b c d e f (x' = a x + c y + e, y' = b x + d y + f), rotation (degrees,
// clockwise), scale x, scale y, world position of the layer's pivot (x, y). False when the layer is not tweened.
bool pixmith_tween_composed_at(const PixmithDoc *doc, uint32_t layer, uint32_t frame, float *out11);
// First and last frame the layer is driven in: its own keys and its ancestors' together.
bool pixmith_tween_effective_range(const PixmithDoc *doc, uint32_t layer, uint32_t *out2);
