//! Bézier curves and polygons (outline or filled), plus their C ABI. Drawing goes through the
//! shape painter, so ink modes, brushes and symmetry apply.

use crate::document::{Editor, Sprite};
use crate::paint::shapes::{line_points, reset_to_checkpoint, tracking, Painter, Pt, FILLED, OPEN};

/// Pixels of a cubic Bézier from `p0` to `p3`, gap-free (consecutive pixels touch).
pub fn bezier_points(p0: Pt, c1: Pt, c2: Pt, p3: Pt) -> Vec<Pt> {
    let dist = |a: Pt, b: Pt| (a.0 as f64 - b.0 as f64).hypot(a.1 as f64 - b.1 as f64);
    let n = (((dist(p0, c1) + dist(c1, c2) + dist(c2, p3)).ceil() as usize) * 2).clamp(2, 8192);
    let at = |t: f64| -> Pt {
        let u = 1.0 - t;
        let f = |a: i32, b: i32, c: i32, d: i32| {
            u * u * u * a as f64 + 3.0 * u * u * t * b as f64 + 3.0 * u * t * t * c as f64 + t * t * t * d as f64
        };
        (f(p0.0, c1.0, c2.0, p3.0).round() as i32, f(p0.1, c1.1, c2.1, p3.1).round() as i32)
    };
    let mut out = vec![p0];
    let mut prev = p0;
    for i in 1..=n {
        let q = if i == n { p3 } else { at(i as f64 / n as f64) };
        if q != prev {
            out.extend(line_points(prev, q).into_iter().skip(1));
            prev = q;
        }
    }
    out
}

/// Pixels of the closed polyline through `pts` (a lone point is a dot).
pub fn polygon_outline(pts: &[Pt]) -> Vec<Pt> {
    match pts {
        [] => vec![],
        [p] => vec![*p],
        _ => (0..pts.len()).flat_map(|i| line_points(pts[i], pts[(i + 1) % pts.len()])).collect(),
    }
}

/// Pixels of the open path through `pts`.
pub fn polyline_points(pts: &[Pt]) -> Vec<Pt> {
    match pts {
        [] => vec![],
        [p] => vec![*p],
        _ => pts.windows(2).flat_map(|w| line_points(w[0], w[1])).collect(),
    }
}

/// Interior rows `(y, left, right)` of the polygon by the even-odd rule, sampled at pixel centers.
pub fn polygon_spans(pts: &[Pt]) -> Vec<(i32, i32, i32)> {
    if pts.len() < 3 {
        return vec![];
    }
    let (ymin, ymax) = pts.iter().fold((i32::MAX, i32::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
    let mut out = Vec::new();
    for y in ymin..=ymax {
        let yc = y as f64 + 0.5;
        let mut xs: Vec<f64> = Vec::new();
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            let (ya, yb) = (a.1 as f64 + 0.5, b.1 as f64 + 0.5);
            if (ya <= yc) != (yb <= yc) {
                let t = (yc - ya) / (yb - ya);
                xs.push(a.0 as f64 + 0.5 + t * (b.0 - a.0) as f64);
            }
        }
        xs.sort_by(f64::total_cmp);
        for pair in xs.chunks_exact(2) {
            let (l, r) = ((pair[0] - 0.5).ceil() as i32, (pair[1] - 0.5).floor() as i32);
            if l <= r {
                out.push((y, l, r));
            }
        }
    }
    out
}

impl Sprite {
    /// Draws a cubic Bézier through `[start, control 1, control 2, end]`.
    pub fn draw_curve(&mut self, layer: usize, frame: usize, pts: [Pt; 4], color: [u8; 4], size: u32, flags: u32) {
        let dims = self.dims();
        let Some(cel) = self.cel_mut(layer, frame) else { return };
        let mut touched = tracking(dims);
        let mut p = Painter::new(cel, dims, color, size, flags).tracked(touched.as_deref_mut());
        for q in bezier_points(pts[0], pts[1], pts[2], pts[3]) {
            p.dot(q);
        }
    }

    /// Draws a closed polygon; with the filled flag the interior is painted too.
    pub fn draw_polygon(&mut self, layer: usize, frame: usize, pts: &[Pt], color: [u8; 4], size: u32, flags: u32) {
        let dims = self.dims();
        let Some(cel) = self.cel_mut(layer, frame) else { return };
        let mut touched = tracking(dims);
        let mut p = Painter::new(cel, dims, color, size, flags).tracked(touched.as_deref_mut());
        let open = flags & OPEN != 0;
        if flags & FILLED != 0 && !open {
            for (y, l, r) in polygon_spans(pts) {
                p.rect(l, y, r, y);
            }
        }
        let outline = if open { polyline_points(pts) } else { polygon_outline(pts) };
        for q in outline {
            p.dot(q);
        }
    }
}

/// Draws a Bézier curve from `(x0, y0)` to `(x3, y3)` with two control points. With `restore`,
/// first rewinds to the `pixmith_begin_stroke` checkpoint, for live previews.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_curve(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    pts: *const i32,
    color: u32,
    size: u32,
    flags: u32,
    restore: bool,
) {
    crate::ffi::guarded("pixmith_curve", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return };
        if pts.is_null() {
            return;
        }
        let v = std::slice::from_raw_parts(pts, 8);
        if restore {
            reset_to_checkpoint(e);
        }
        let c = crate::ffi::clampc;
        let p = [(c(v[0]), c(v[1])), (c(v[2]), c(v[3])), (c(v[4]), c(v[5])), (c(v[6]), c(v[7]))];
        e.sprite.draw_curve(layer as usize, frame as usize, p, color.to_be_bytes(), size, flags);
        crate::paint::selection::clip_to_selection(e, layer as usize, frame as usize);
    })
}

/// Draws the closed polygon through `n` points given as x, y pairs.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn pixmith_polygon(
    doc: *mut Editor,
    layer: u32,
    frame: u32,
    pts: *const i32,
    n: usize,
    color: u32,
    size: u32,
    flags: u32,
    restore: bool,
) {
    crate::ffi::guarded("pixmith_polygon", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return };
        if pts.is_null() || n == 0 || n > crate::paint::selection::MAX_POLY_POINTS {
            return;
        }
        let pts: Vec<Pt> = std::slice::from_raw_parts(pts, n * 2)
            .chunks_exact(2)
            .map(|c| (crate::ffi::clampc(c[0]), crate::ffi::clampc(c[1])))
            .collect();
        if restore {
            reset_to_checkpoint(e);
        }
        e.sprite.draw_polygon(layer as usize, frame as usize, &pts, color.to_be_bytes(), size, flags);
        crate::paint::selection::clip_to_selection(e, layer as usize, frame as usize);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::shapes::{OPEN, SYM_X};

    const RED: [u8; 4] = [255, 0, 0, 255];

    fn lit(s: &Sprite) -> Vec<Pt> {
        let px = s.render(0);
        let w = s.width as usize;
        (0..px.len() / 4).filter(|&i| px[i * 4 + 3] != 0).map(|i| ((i % w) as i32, (i / w) as i32)).collect()
    }

    fn connected(pts: &[Pt]) -> bool {
        pts.windows(2).all(|w| (w[0].0 - w[1].0).abs() <= 1 && (w[0].1 - w[1].1).abs() <= 1)
    }

    #[test]
    fn bezier_hits_endpoints_and_has_no_gaps() {
        let pts = bezier_points((0, 0), (10, 30), (30, -10), (40, 20));
        assert_eq!((pts[0], *pts.last().unwrap()), ((0, 0), (40, 20)));
        assert!(connected(&pts));
        assert_eq!(bezier_points((3, 3), (3, 3), (3, 3), (3, 3)), vec![(3, 3)]);
    }

    #[test]
    fn straight_bezier_matches_a_line_in_extent() {
        let pts = bezier_points((0, 0), (0, 0), (10, 10), (10, 10));
        assert!(pts.iter().all(|&(x, y)| (x - y).abs() <= 1));
        assert_eq!(*pts.last().unwrap(), (10, 10));
    }

    #[test]
    fn bezier_bulges_toward_control_points() {
        let pts = bezier_points((0, 10), (0, 0), (20, 0), (20, 10));
        assert!(pts.iter().map(|p| p.1).min().unwrap() <= 3);
        assert!(pts.iter().all(|p| p.1 <= 10));
    }

    #[test]
    fn polygon_outline_closes() {
        let o = polygon_outline(&[(0, 0), (4, 0), (4, 4)]);
        assert!(o.contains(&(2, 2)) && o.contains(&(4, 2)) && o.contains(&(2, 0)));
        assert_eq!(polygon_outline(&[(1, 1)]), vec![(1, 1)]);
        assert!(polygon_outline(&[]).is_empty());
    }

    #[test]
    fn spans_fill_a_rectangle_exactly() {
        let spans = polygon_spans(&[(1, 1), (5, 1), (5, 3), (1, 3)]);
        let area: i32 = spans.iter().map(|&(_, l, r)| r - l + 1).sum();
        // Rows are sampled at pixel centers, so the bottom edge row is left to the outline.
        assert_eq!(area, 5 * 2);
        assert!(spans.iter().all(|&(_, l, r)| l == 1 && r == 5));
        assert!(polygon_spans(&[(0, 0), (3, 3)]).is_empty());
    }

    #[test]
    fn concave_polygon_leaves_its_notch_empty() {
        // A "U" opening upward: the notch between x=3..5, y=0..3 is outside.
        let u = [(0, 0), (2, 0), (2, 4), (6, 4), (6, 0), (8, 0), (8, 8), (0, 8)];
        let spans = polygon_spans(&u);
        let row = |y: i32| spans.iter().filter(|s| s.0 == y).map(|s| (s.1, s.2)).collect::<Vec<_>>();
        assert_eq!(row(1).len(), 2);
        assert_eq!(row(6).len(), 1);
        assert!(!row(1).iter().any(|&(l, r)| l <= 4 && 4 <= r));
    }

    #[test]
    fn triangle_area_is_about_half_the_box() {
        let spans = polygon_spans(&[(0, 0), (20, 0), (0, 20)]);
        let area: i32 = spans.iter().map(|&(_, l, r)| r - l + 1).sum();
        assert!((190..=230).contains(&area), "{area}");
    }

    #[test]
    fn filled_polygon_covers_interior_and_outline_alone_does_not() {
        let sq = [(2, 2), (8, 2), (8, 8), (2, 8)];
        let mut s = Sprite::new(12, 12);
        s.draw_polygon(0, 0, &sq, RED, 1, FILLED);
        assert_eq!(lit(&s).len(), 7 * 7);
        let mut s = Sprite::new(12, 12);
        s.draw_polygon(0, 0, &sq, RED, 1, 0);
        assert_eq!(lit(&s).len(), 24);
        assert!(!lit(&s).contains(&(5, 5)));
    }

    #[test]
    fn open_polygon_leaves_the_closing_edge_out() {
        let sq = [(2, 2), (8, 2), (8, 8), (2, 8)];
        let mut s = Sprite::new(12, 12);
        s.draw_polygon(0, 0, &sq, RED, 1, OPEN | FILLED);
        let px = lit(&s);
        assert_eq!(px.len(), 7 + 6 + 6);
        assert!(!px.contains(&(2, 5)) && px.contains(&(8, 5)) && px.contains(&(2, 2)) && px.contains(&(2, 8)));
        assert!(!px.contains(&(5, 5)));
        assert_eq!(polyline_points(&[(0, 0), (3, 0)]).len(), 4);
    }

    #[test]
    fn curve_and_polygon_follow_symmetry() {
        let mut s = Sprite::new(20, 10);
        s.draw_curve(0, 0, [(1, 1), (3, 5), (6, 5), (8, 1)], RED, 1, SYM_X);
        let px = lit(&s);
        assert!(px.iter().all(|&(x, y)| px.contains(&(19 - x, y))));
        let mut s = Sprite::new(20, 10);
        s.draw_polygon(0, 0, &[(1, 1), (6, 1), (3, 6)], RED, 1, SYM_X | FILLED);
        let px = lit(&s);
        assert!(px.iter().all(|&(x, y)| px.contains(&(19 - x, y))));
    }

    #[test]
    fn preview_rewinds_to_checkpoint() {
        let mut e = Editor::new(Sprite::new(16, 16));
        e.begin_preview();
        let a = [0, 0, 5, 5, 10, 5, 15, 0];
        let b = [0, 15, 5, 5, 10, 5, 15, 15];
        unsafe {
            pixmith_curve(&mut e, 0, 0, a.as_ptr(), 0xFF0000FF, 1, 0, true);
            pixmith_curve(&mut e, 0, 0, b.as_ptr(), 0xFF0000FF, 1, 0, true);
        }
        assert!(lit(&e.sprite).iter().all(|p| p.1 >= 5));
        assert!(e.undo());
        assert!(lit(&e.sprite).is_empty());
    }

    #[test]
    fn polygon_ffi_draws_and_tolerates_bad_input() {
        let mut e = Editor::new(Sprite::new(8, 8));
        let pts = [1, 1, 6, 1, 6, 6, 1, 6];
        unsafe {
            pixmith_polygon(&mut e, 0, 0, pts.as_ptr(), 4, 0xFF0000FF, 1, FILLED, false);
            pixmith_polygon(&mut e, 0, 0, std::ptr::null(), 4, 0, 1, 0, false);
            pixmith_polygon(&mut e, 0, 0, pts.as_ptr(), 0, 0, 1, 0, false);
        }
        assert_eq!(lit(&e.sprite).len(), 36);
    }
}
