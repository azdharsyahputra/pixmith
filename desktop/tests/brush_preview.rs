//! The brush cursor preview shows exactly the pixels a one-click stroke paints: checked against the core on a
//! scratch document.

use pixmith_desktop::app::Tool;
use pixmith_desktop::doc::Doc;
use pixmith_desktop::ui::brush_preview::{Look, look, painted_cells};
use pixmith_desktop::ui::canvas::flag;
use std::collections::HashSet;

const RED: u32 = 0xFF0000FF;

/// The pixels a single click of the pencil paints on a blank `w` x `h` sprite.
fn clicked(w: u32, h: u32, size: u32, p: (i32, i32), sym: u32) -> HashSet<(i32, i32)> {
    let mut doc = Doc::new(w, h);
    let layer = doc.layer_count() - 1;
    doc.begin_edit("Draw");
    doc.set_ink(0, false, &[]);
    doc.stroke_begin(layer, 0, p.0, p.1, RED, size, sym);
    doc.stroke_end();
    let mut buf = Vec::new();
    assert!(doc.render(0, &mut buf));
    let mut out = HashSet::new();
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if Doc::pixel_of(&buf, w, x, y).is_some_and(|c| c & 0xFF != 0) {
                out.insert((x, y));
            }
        }
    }
    out
}

fn sorted(set: &HashSet<(i32, i32)>) -> Vec<(i32, i32)> {
    let mut v: Vec<_> = set.iter().copied().collect();
    v.sort_unstable();
    v
}

#[test]
fn square_and_round_brushes_match_a_click() {
    for circle in [false, true] {
        Doc::set_brush_circle(circle);
        for size in (1..=12).chain([17, 33, 64]) {
            let p = (40, 41);
            assert_eq!(
                sorted(&painted_cells(size, p, (90, 90), (90, 90), 0)),
                sorted(&clicked(90, 90, size, p, 0)),
                "size {size}, round {circle}"
            );
        }
    }
}

#[test]
fn mirrors_and_sprite_edges_match_a_click() {
    let sym = [0, flag::MIRROR_X, flag::MIRROR_Y, flag::MIRROR_X | flag::MIRROR_Y];
    for circle in [false, true] {
        Doc::set_brush_circle(circle);
        for size in [1, 2, 3, 4, 5, 8] {
            for p in [(3, 2), (0, 0), (-1, -2), (19, 13), (21, 4), (10, 7), (-9, 3), (30, 30)] {
                for s in sym {
                    assert_eq!(
                        sorted(&painted_cells(size, p, (20, 14), (20, 14), s)),
                        sorted(&clicked(20, 14, size, p, s)),
                        "size {size}, round {circle}, at {p:?}, symmetry {s}"
                    );
                }
            }
        }
    }
}

#[test]
fn moved_mirror_axes_match_a_click() {
    Doc::set_brush_circle(false);
    // Twice the axis positions: one between two pixels, one through the middle of a pixel.
    let axes = (13, 6);
    pixmith_core::paint::shapes::set_mirror_axes(Some(axes.0), Some(axes.1));
    for size in [1, 3, 4] {
        for p in [(2, 3), (8, 1), (12, 9)] {
            let sym = flag::MIRROR_X | flag::MIRROR_Y;
            assert_eq!(
                sorted(&painted_cells(size, p, (20, 14), axes, sym)),
                sorted(&clicked(20, 14, size, p, sym)),
                "size {size} at {p:?}"
            );
        }
    }
    pixmith_core::paint::shapes::set_mirror_axes(None, None);
}

#[test]
fn a_custom_brush_previews_its_pixels() {
    Doc::set_brush_circle(false);
    let mut doc = Doc::new(30, 30);
    let layer = doc.layer_count() - 1;
    doc.begin_edit("Draw");
    for (x, y) in [(4, 4), (5, 4), (6, 4), (4, 5), (6, 6)] {
        doc.stroke_begin(layer, 0, x, y, RED, 1, 0);
        doc.stroke_end();
    }
    // SAFETY: the document is live.
    unsafe { pixmith_core::paint::selection::pixmith_sel_rect(doc.raw(), 4, 4, 6, 6) };
    assert!(doc.brush_from_selection(layer, 0));
    for p in [(15, 15), (1, 1), (29, 29)] {
        for s in [0, flag::MIRROR_X | flag::MIRROR_Y] {
            assert_eq!(
                sorted(&painted_cells(3, p, (30, 30), (30, 30), s)),
                sorted(&clicked(30, 30, 3, p, s)),
                "custom brush at {p:?}, symmetry {s}"
            );
        }
    }
    Doc::brush_reset();
}

#[test]
fn only_painting_tools_preview() {
    for tool in [Tool::Pencil, Tool::Line, Tool::Rectangle, Tool::Ellipse, Tool::Curve, Tool::ShapePolygon] {
        assert_eq!(look(tool), Some(Look::Filled), "{tool:?}");
    }
    for tool in [Tool::Eraser, Tool::Blur, Tool::Jumble] {
        assert_eq!(look(tool), Some(Look::Outline), "{tool:?}");
    }
    assert_eq!(look(Tool::Spray), Some(Look::Spray));
    for tool in [Tool::Marquee, Tool::Move, Tool::Fill, Tool::Picker, Tool::Slice, Tool::Text, Tool::Gradient] {
        assert_eq!(look(tool), None, "{tool:?}");
    }
}
