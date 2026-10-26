//! Tween rig: parented layers (a skeleton made of layers: thigh -> shin -> boot, body -> arm).
//!
//! A layer can have a tween parent. Its **world transform** at a frame is the parent's world
//! transform composed with its own:
//!
//! ```text
//! world(layer) = world(parent) * local(layer)        local = translate(pivot + t) * rotate * scale * translate(-pivot)
//! ```
//!
//! Both are affine matrices in canvas coordinates, so the pivot of a child (a point in the child's
//! own rest-pose space, e.g. the knee for a shin) is carried along by the parent: rotating the thigh
//! moves the knee, and the shin then rotates about the moved knee. The chain is evaluated root first;
//! each layer's own translation is rounded the way the single-layer tween rounds it, everything else
//! is exact until the final nearest-neighbor / RotSprite resample of the child's source cel.
//!
//! Rules:
//! * A layer's source is still its own cel at its first own key frame. A child **without keys of its
//!   own** follows its parent with a plain rest pose (its cel at the first frame of the range it
//!   inherits). The frames a layer is driven in are the union of its own key range and those of all
//!   its ancestors; outside them its own cels show.
//! * Opacity is not inherited; scale is (a scaled thigh scales the leg).
//! * Parents are stored as numbers, not layer indices: a parent gets an `id` the first time a child is
//!   linked to it, the child stores `pa=<id>`. Both live in the layer's tween payload in its user data
//!   text (`pixmith.tween.v1:` ... `id=3;pa=1`), so they survive `.aseprite` and `.pxmt` files, undo,
//!   and reordering, inserting and deleting layers. A link to a missing id is ignored. If two layers
//!   carry the same id (a hand edited file), the lowest layer is the parent.
//! * Cycles cannot be created (`set_parent` refuses them); a cyclic file is read with the walk
//!   stopping where it would revisit a layer.

use super::tween::{self, load, mul, store_or_clear, Matrix, TweenState, IDENTITY, SEP};
use crate::document::{Editor, Sprite};

/// The longest chain that is followed (a safety net; real rigs are a handful of layers deep).
const MAX_DEPTH: usize = 64;

/// Applies the matrix to a point.
pub fn map_point(m: &Matrix, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

fn has_payload(s: &Sprite, layer: usize) -> bool {
    s.layers.get(layer).and_then(|l| l.user_data.text.as_deref()).is_some_and(|t| t.contains(SEP))
}

/// The layer carrying rig id `id` (the lowest one when a file has duplicates).
fn find_id(s: &Sprite, id: u32) -> Option<usize> {
    (0..s.layers.len()).find(|&l| has_payload(s, l) && load(s, l).is_some_and(|st| st.id == Some(id)))
}

/// The ancestors of `layer` (nearest parent first) with their states.
pub(crate) fn ancestors(s: &Sprite, layer: usize, st: &TweenState) -> Vec<(usize, TweenState)> {
    let mut out: Vec<(usize, TweenState)> = Vec::new();
    let mut seen = vec![layer];
    let mut cur = st.parent;
    while let Some(id) = cur {
        let Some(p) = find_id(s, id) else { break };
        if seen.contains(&p) || out.len() >= MAX_DEPTH {
            break;
        }
        seen.push(p);
        let Some(pst) = load(s, p) else { break };
        cur = pst.parent;
        out.push((p, pst));
    }
    out
}

/// First and last frame a layer is driven in: its own key range joined with its ancestors'.
pub(crate) fn hull(st: &TweenState, anc: &[(usize, TweenState)]) -> Option<(u32, u32)> {
    std::iter::once(st)
        .chain(anc.iter().map(|(_, a)| a))
        .filter_map(|s| s.range())
        .fold(None, |acc, (lo, hi)| Some(acc.map_or((lo, hi), |(a, b): (u32, u32)| (a.min(lo), b.max(hi)))))
}

/// The product of the ancestors' local transforms at `frame` (root outermost); identity without any.
pub(crate) fn ancestors_world(s: &Sprite, anc: &[(usize, TweenState)], frame: u32) -> Matrix {
    anc.iter().rev().fold(IDENTITY, |world, (l, st)| mul(&world, &tween::local_matrix(s, *l, st, frame)))
}

/// Frames in which the layer is driven by its own keys or its ancestors' (`None`: not tweened at all).
pub fn effective_range(s: &Sprite, layer: usize) -> Option<(u32, u32)> {
    let st = load(s, layer)?;
    hull(&st, &ancestors(s, layer, &st))
}

/// The layer's tween parent, if it has one that exists.
pub fn parent_of(s: &Sprite, layer: usize) -> Option<usize> {
    let st = load(s, layer)?;
    ancestors(s, layer, &st).first().map(|a| a.0)
}

/// `[layer, parent, grandparent, ...]`.
pub fn chain(s: &Sprite, layer: usize) -> Vec<usize> {
    let mut v = vec![layer];
    if let Some(st) = load(s, layer) {
        v.extend(ancestors(s, layer, &st).into_iter().map(|a| a.0));
    }
    v
}

/// Layers whose tween parent is `layer`, bottom to top.
pub fn children_of(s: &Sprite, layer: usize) -> Vec<usize> {
    let Some(id) = load(s, layer).and_then(|st| st.id) else { return Vec::new() };
    if find_id(s, id) != Some(layer) {
        return Vec::new();
    }
    (0..s.layers.len())
        .filter(|&l| l != layer && has_payload(s, l) && load(s, l).is_some_and(|st| st.parent == Some(id)))
        .collect()
}

/// Whether `parent` may become the tween parent of `layer`: both can be tweened (normal or group
/// layers), they differ, it makes no cycle, and neither is a group that contains the other (a group
/// already transforms its content).
pub fn can_parent(s: &Sprite, layer: usize, parent: usize) -> bool {
    if layer == parent || !tween::editable(s, layer) || !tween::editable(s, parent) {
        return false;
    }
    if chain(s, parent).contains(&layer) {
        return false;
    }
    let inside =
        |outer: usize, inner: usize| s.layers[outer].is_group() && (outer + 1..s.subtree_end(outer)).contains(&inner);
    !inside(parent, layer) && !inside(layer, parent)
}

fn next_id(s: &Sprite) -> u32 {
    // Dangling links count too, so a new id never revives a link to a deleted layer.
    (0..s.layers.len())
        .filter(|&l| has_payload(s, l))
        .filter_map(|l| load(s, l))
        .flat_map(|st| [st.id, st.parent])
        .flatten()
        .max()
        .map_or(1, |m| m.saturating_add(1))
}

fn ensure_id(s: &mut Sprite, layer: usize) -> u32 {
    let mut st = load(s, layer).unwrap_or_else(TweenState::new);
    if let Some(id) = st.id.filter(|&id| find_id(s, id) == Some(layer)) {
        return id;
    }
    let id = next_id(s);
    st.id = Some(id);
    store_or_clear(s, layer, &st);
    id
}

/// Drops the ids nobody links to any more, so unlinking leaves a layer exactly as it was before.
pub(crate) fn forget_unused_ids(s: &mut Sprite) {
    let states: Vec<(usize, TweenState)> =
        (0..s.layers.len()).filter(|&l| has_payload(s, l)).filter_map(|l| load(s, l).map(|st| (l, st))).collect();
    let used: Vec<u32> = states.iter().filter_map(|(_, st)| st.parent).collect();
    for (l, mut st) in states {
        if st.id.is_some_and(|id| !used.contains(&id)) {
            st.id = None;
            store_or_clear(s, l, &st);
        }
    }
}

/// `set_parent` without the undo step (scripts own theirs). `None` unlinks.
pub(crate) fn set_parent_raw(s: &mut Sprite, layer: usize, parent: Option<usize>) -> bool {
    match parent {
        Some(p) => {
            if !can_parent(s, layer, p) {
                return false;
            }
            let id = ensure_id(s, p);
            let mut st = load(s, layer).unwrap_or_else(TweenState::new);
            st.parent = Some(id);
            store_or_clear(s, layer, &st);
        }
        None => {
            if !tween::editable(s, layer) {
                return false;
            }
            if let Some(mut st) = load(s, layer) {
                st.parent = None;
                store_or_clear(s, layer, &st);
            }
        }
    }
    forget_unused_ids(s);
    true
}

/// Makes `parent` the tween parent of `layer` (`None` unlinks). One undo step. Returns false, changing
/// nothing, for layers that do not exist or cannot be tweened and for links that would make a cycle.
/// The child keeps its own keys: they are now relative to the parent's motion.
pub fn set_parent(e: &mut Editor, layer: usize, parent: Option<usize>) -> bool {
    let ok = match parent {
        Some(p) => can_parent(&e.sprite, layer, p),
        None => tween::editable(&e.sprite, layer),
    };
    if !ok {
        return false;
    }
    let unchanged = match parent {
        Some(p) => parent_of(&e.sprite, layer) == Some(p),
        None => load(&e.sprite, layer).is_none_or(|st| st.parent.is_none()),
    };
    if unchanged {
        return true;
    }
    e.checkpoint();
    set_parent_raw(&mut e.sprite, layer, parent)
}

/// World matrix of a layer at a frame (parent chain included), or `None` when it is not tweened.
pub fn world_matrix(s: &Sprite, layer: usize, frame: u32) -> Option<Matrix> {
    let st = load(s, layer)?;
    let anc = ancestors(s, layer, &st);
    hull(&st, &anc)?;
    Some(mul(&ancestors_world(s, &anc, frame), &tween::local_matrix(s, layer, &st, frame)))
}

/// The composed transform of a layer at a frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Composed {
    /// Source (rest pose) to canvas, parents included.
    pub matrix: Matrix,
    /// The layer's own transform alone.
    pub local: Matrix,
    /// Where the layer's own pivot is on the canvas.
    pub pivot_world: (f64, f64),
    /// Rotation of the composed matrix in degrees, clockwise (exact when no layer in the chain skews).
    pub rotation: f64,
    pub scale_x: f64,
    pub scale_y: f64,
}

pub fn composed_transform(s: &Sprite, layer: usize, frame: u32) -> Option<Composed> {
    let st = load(s, layer)?;
    let anc = ancestors(s, layer, &st);
    hull(&st, &anc)?;
    let local = tween::local_matrix(s, layer, &st, frame);
    let matrix = mul(&ancestors_world(s, &anc, frame), &local);
    let pivot = tween::effective_pivot(s, layer).unwrap_or((0.0, 0.0));
    let sx = matrix[0].hypot(matrix[1]);
    let det = matrix[0] * matrix[3] - matrix[1] * matrix[2];
    Some(Composed {
        matrix,
        local,
        pivot_world: map_point(&matrix, pivot.0, pivot.1),
        rotation: matrix[1].atan2(matrix[0]).to_degrees(),
        scale_x: sx,
        scale_y: if sx > 0.0 { det / sx } else { 0.0 },
    })
}

// MARK: C ABI

/// Sets the tween parent of `layer` (`parent` -1: none). One undo step; false for cycles and layers that
/// cannot be tweened.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_set_parent(doc: *mut Editor, layer: u32, parent: i32) -> bool {
    crate::ffi::guarded("pixmith_tween_set_parent", doc, || {
        let Some(e) = doc.as_mut() else { return false };
        set_parent(e, layer as usize, usize::try_from(parent).ok())
    })
}

/// The tween parent layer index, or -1.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_get_parent(doc: *const Editor, layer: u32) -> i32 {
    crate::ffi::guarded("pixmith_tween_get_parent", std::ptr::null_mut(), || {
        doc.as_ref().and_then(|e| parent_of(&e.sprite, layer as usize)).map_or(-1, |p| p as i32)
    })
}

/// Whether `parent` is a valid tween parent for `layer` (no cycle, both tweenable); for filling a menu.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_can_parent(doc: *const Editor, layer: u32, parent: u32) -> bool {
    crate::ffi::guarded("pixmith_tween_can_parent", std::ptr::null_mut(), || {
        doc.as_ref().is_some_and(|e| can_parent(&e.sprite, layer as usize, parent as usize))
    })
}

/// Writes up to `cap` child layer indices to `out` and returns how many children the layer has.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_children(doc: *const Editor, layer: u32, out: *mut u32, cap: u32) -> u32 {
    crate::ffi::guarded("pixmith_tween_children", std::ptr::null_mut(), || {
        let Some(e) = doc.as_ref() else { return 0 };
        let kids = children_of(&e.sprite, layer as usize);
        if !out.is_null() && cap > 0 {
            let n = kids.len().min(cap as usize);
            let dst = std::slice::from_raw_parts_mut(out, n);
            for (d, k) in dst.iter_mut().zip(&kids) {
                *d = *k as u32;
            }
        }
        kids.len() as u32
    })
}

/// Writes 11 floats: the composed matrix a, b, c, d, e, f (`x' = a x + c y + e`, `y' = b x + d y + f`),
/// rotation (degrees, clockwise), scale x, scale y, and the world position of the layer's pivot (x, y).
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_composed_at(doc: *const Editor, layer: u32, frame: u32, out: *mut f32) -> bool {
    crate::ffi::guarded("pixmith_tween_composed_at", std::ptr::null_mut(), || {
        let (Some(e), false) = (doc.as_ref(), out.is_null()) else { return false };
        let Some(c) = composed_transform(&e.sprite, layer as usize, frame) else { return false };
        let m = c.matrix;
        let v =
            [m[0], m[1], m[2], m[3], m[4], m[5], c.rotation, c.scale_x, c.scale_y, c.pivot_world.0, c.pivot_world.1];
        std::slice::from_raw_parts_mut(out, 11).iter_mut().zip(v).for_each(|(o, x)| *o = x as f32);
        true
    })
}

/// Writes the first and last frame the layer is driven in, its own keys and its ancestors' together.
#[no_mangle]
pub unsafe extern "C" fn pixmith_tween_effective_range(doc: *const Editor, layer: u32, out: *mut u32) -> bool {
    crate::ffi::guarded("pixmith_tween_effective_range", std::ptr::null_mut(), || {
        let (Some(e), false) = (doc.as_ref(), out.is_null()) else { return false };
        let Some((a, b)) = effective_range(&e.sprite, layer as usize) else { return false };
        std::slice::from_raw_parts_mut(out, 2).copy_from_slice(&[a, b]);
        true
    })
}
