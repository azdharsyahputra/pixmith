//! Isometric grid math shared by tilemaps, terrain and export (2:1 diamonds, Godot `DIAMOND_DOWN`).
//!
//! Cell (0, 0) has its bounding box at `(x, y)` with size `w` x `h`; its corners are `(x + w/2, y)`,
//! `(x + w, y + h/2)`, `(x + w/2, y + h)` and `(x, y + h/2)`. Cell (i, j) is that diamond moved by
//! `(i - j) * w/2` across and `(i + j) * h/2` down, so the `i` axis runs down-right and the `j` axis
//! runs down-left, and the cells tile the plane without gaps. A pixel belongs to the cell its center
//! lies in (a center on a border goes to the cell below or to the right), so the masks of all cells
//! partition the plane and a 32 x 16 cell has `w * h / 2` = 256 pixels.
//!
//! The desktop app's `ui::iso::Iso` is the same math; this is the copy tilemaps depend on.

/// Smallest cell side (cells are even so that half a cell is a whole number of pixels).
pub const MIN_SIDE: i32 = 2;
pub const MAX_SIDE: i32 = 4096;
/// Cell coordinates are kept within this range (they are stored as 16-bit values in the file).
pub const MAX_CELL: i64 = 30_000;

/// Rounds a cell side to the nearest allowed value: even, 2 to 4096.
pub fn clamp_side(v: i32) -> i32 {
    (v.clamp(MIN_SIDE, MAX_SIDE) + 1) & !1
}

/// An isometric grid: even cell size and the pixel position of cell (0, 0)'s bounding box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Iso {
    pub w: i64,
    pub h: i64,
    pub x: i64,
    pub y: i64,
}

impl Iso {
    /// A grid with sanitized (even, in range) cell sides.
    pub fn new(w: i32, h: i32, x: i32, y: i32) -> Iso {
        Iso { w: clamp_side(w) as i64, h: clamp_side(h) as i64, x: x as i64, y: y as i64 }
    }

    /// Pixel position of the bounding box's top-left corner of cell `(i, j)`.
    pub fn cell_origin(&self, cell: (i64, i64)) -> (i64, i64) {
        (self.x + (cell.0 - cell.1) * self.w / 2, self.y + (cell.0 + cell.1) * self.h / 2)
    }

    /// Pixel position of the center of cell `(i, j)`'s diamond, as a point in doubled-free pixel
    /// edge coordinates (the box's top-left plus half the cell size).
    pub fn cell_center(&self, cell: (i64, i64)) -> (i64, i64) {
        let (ox, oy) = self.cell_origin(cell);
        (ox + self.w / 2, oy + self.h / 2)
    }

    /// The cell `(i, j)` that contains pixel `(px, py)`.
    pub fn cell_at(&self, px: i32, py: i32) -> (i64, i64) {
        let (w, h) = (self.w, self.h);
        // Doubled coordinates of the pixel's center, relative to the center of the top corner of cell (0, 0).
        let dx = 2 * px as i64 + 1 - 2 * self.x - w;
        let dy = 2 * py as i64 + 1 - 2 * self.y;
        // a = dx / w and b = dy / (h/2) are the coordinates in units of half a tile; cell (i, j) has its top
        // corner at a = i - j, b = i + j, so i and j are the rounded half sums and differences.
        let (sum, diff) = (h * dx + w * (dy - h) + w * h, w * (dy - h) - h * dx + w * h);
        (sum.div_euclid(2 * w * h), diff.div_euclid(2 * w * h))
    }

    /// The pixel rows `(y, left, right)` (inclusive) of cell `(i, j)`, top to bottom; rows without a pixel are left out.
    pub fn cell_rows(&self, cell: (i64, i64)) -> Vec<(i32, i32, i32)> {
        let (left, top) = self.cell_origin(cell);
        let mut rows = Vec::new();
        for y in top..top + self.h {
            let mut span: Option<(i32, i32)> = None;
            for x in left..left + self.w {
                if self.cell_at(x as i32, y as i32) == cell {
                    span = Some(span.map_or((x as i32, x as i32), |(l, _)| (l, x as i32)));
                }
            }
            if let Some((l, r)) = span {
                rows.push((y as i32, l, r));
            }
        }
        rows
    }

    /// A closed polygon whose pixels (outline plus even-odd interior, as the core's polygon selection takes
    /// them) are exactly the cell's mask.
    pub fn cell_polygon(&self, cell: (i64, i64)) -> Vec<(i32, i32)> {
        let rows = self.cell_rows(cell);
        let Some(&(y0, l0, _)) = rows.first() else { return Vec::new() };
        let mut pts = vec![(l0, y0)];
        for pair in rows.windows(2) {
            let ((yp, lp, _), (yr, lr, _)) = (pair[0], pair[1]);
            pts.push(if lr <= lp { (lp, yr) } else { (lr, yp) });
            pts.push((lr, yr));
        }
        let &(yb, _, rb) = rows.last().unwrap();
        pts.push((rb, yb));
        for pair in rows.windows(2).rev() {
            let ((yr, _, rr), (yq, _, rq)) = (pair[0], pair[1]);
            pts.push(if rr >= rq { (rq, yr) } else { (rr, yq) });
            pts.push((rr, yr));
        }
        pts.dedup();
        if pts.len() > 1 && pts.first() == pts.last() {
            pts.pop();
        }
        pts
    }

    /// Inclusive cell ranges `(min_i, max_i, min_j, max_j)` of every cell whose diamond can touch the
    /// pixel rectangle `x0..x1` x `y0..y1` (exclusive ends), widened by `grow` pixels on each side.
    pub fn cells_touching(&self, x0: i64, y0: i64, x1: i64, y1: i64, grow: i64) -> (i64, i64, i64, i64) {
        let corners = [(x0 - grow, y0 - grow), (x1 + grow, y0 - grow), (x0 - grow, y1 + grow), (x1 + grow, y1 + grow)];
        let (mut lo_i, mut hi_i, mut lo_j, mut hi_j) = (i64::MAX, i64::MIN, i64::MAX, i64::MIN);
        for (px, py) in corners {
            // Continuous cell coordinates of the corner, rounded outwards.
            let a2 = 2 * (px - self.x) - self.w; // (i - j) * w
            let b2 = 2 * (py - self.y); // (i + j) * h  (doubled)
            let i_num = self.h * a2 + self.w * b2; // 2 * w * h * i, up to the half-cell offset
            let j_num = self.w * b2 - self.h * a2;
            let den = 2 * self.w * self.h;
            let (fi, ci) = (i_num.div_euclid(den), (i_num + den - 1).div_euclid(den));
            let (fj, cj) = (j_num.div_euclid(den), (j_num + den - 1).div_euclid(den));
            lo_i = lo_i.min(fi - 1);
            hi_i = hi_i.max(ci + 1);
            lo_j = lo_j.min(fj - 1);
            hi_j = hi_j.max(cj + 1);
        }
        (lo_i, hi_i, lo_j, hi_j)
    }
}

/// Order in which cells are drawn, back to front: by `i + j` (rows from the top of the screen), then by
/// `i` (left to right within a row).
pub fn draw_order(a: (i64, i64), b: (i64, i64)) -> std::cmp::Ordering {
    (a.0 + a.1, a.0).cmp(&(b.0 + b.1, b.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn sides_are_even_and_in_range() {
        assert_eq!([clamp_side(0), clamp_side(5), clamp_side(32), clamp_side(99999)], [2, 6, 32, 4096]);
    }

    #[test]
    fn cell_origin_matches_the_user_map_formula() {
        // 96x48 tiles, map origin (592, 360): cell (i, j) at (x + (i-j)*w/2, y + (i+j)*h/2).
        let iso = Iso::new(96, 48, 592, 360);
        assert_eq!(iso.cell_origin((0, 0)), (592, 360));
        assert_eq!(iso.cell_origin((1, 0)), (640, 384));
        assert_eq!(iso.cell_origin((0, 1)), (544, 384));
        assert_eq!(iso.cell_origin((3, 5)), (592 - 96, 360 + 8 * 24));
        assert_eq!(iso.cell_center((0, 0)), (640, 384));
        let small = Iso::new(32, 16, 0, 0);
        assert_eq!(small.cell_origin((2, 7)), (-80, 72));
    }

    #[test]
    fn cell_at_inverts_cell_origin_for_96_by_48() {
        let iso = Iso::new(96, 48, 592, 360);
        for &(i, j) in &[(0, 0), (1, 0), (0, 1), (5, 3), (-2, 4), (10, 10)] {
            let (ox, oy) = iso.cell_origin((i, j));
            // The center pixel of the diamond and one pixel inside each corner lie in the cell.
            let (cx, cy) = (ox as i32 + 48, oy as i32 + 24);
            assert_eq!(iso.cell_at(cx, cy), (i, j));
            assert_eq!(iso.cell_at(cx - 1, cy), (i, j));
            assert_eq!(iso.cell_at(cx, cy - 1), (i, j));
            assert_eq!(iso.cell_at(cx - 40, cy), (i, j));
            assert_eq!(iso.cell_at(cx + 39, cy), (i, j));
            assert_eq!(iso.cell_at(cx, cy - 20), (i, j));
            assert_eq!(iso.cell_at(cx, cy + 20), (i, j));
        }
        // Just outside the top-left edge of cell (0, 0) is the cell up and to the left (-1, 0) region.
        assert_eq!(iso.cell_at(592 + 1, 360), (-1, 0));
    }

    #[test]
    fn a_32_by_16_cell_is_the_classic_diamond() {
        let iso = Iso::new(32, 16, 0, 0);
        let rows = iso.cell_rows((0, 0));
        assert_eq!(rows.len(), 16);
        let widths: Vec<i32> = rows.iter().map(|&(_, l, r)| r - l + 1).collect();
        assert_eq!(widths, [2, 6, 10, 14, 18, 22, 26, 30, 30, 26, 22, 18, 14, 10, 6, 2]);
        assert_eq!(widths.iter().sum::<i32>(), 32 * 16 / 2);
        for (y, l, r) in rows {
            assert_eq!(l + r + 1, 32, "row {y} is centered");
        }
    }

    #[test]
    fn cells_partition_the_plane_and_follow_the_offset() {
        let iso = Iso::new(32, 16, 5, -3);
        let mut seen = HashSet::new();
        for cell in [(0, 0), (1, 0), (0, 1), (-1, 2), (2, 2)] {
            for (y, l, r) in iso.cell_rows(cell) {
                for x in l..=r {
                    assert!(seen.insert((x, y)), "pixel {x},{y} in two cells");
                    assert_eq!(iso.cell_at(x, y), cell);
                }
            }
        }
        let rows = iso.cell_rows((0, 0));
        assert_eq!(rows.first().map(|r| r.0), Some(-3));
        assert_eq!(rows.iter().map(|r| r.1).min(), Some(5 + 1));
        assert_eq!(rows.iter().map(|r| r.2).max(), Some(5 + 30));
    }

    #[test]
    fn draw_order_is_back_to_front_then_left_to_right() {
        let mut cells = vec![(2, 0), (0, 2), (1, 1), (0, 0), (1, 0), (0, 1)];
        cells.sort_by(|a, b| draw_order(*a, *b));
        assert_eq!(cells, [(0, 0), (0, 1), (1, 0), (0, 2), (1, 1), (2, 0)]);
    }

    #[test]
    fn cells_touching_covers_every_cell_that_has_a_pixel_in_the_rect() {
        let iso = Iso::new(96, 48, 592, 360);
        let (lo_i, hi_i, lo_j, hi_j) = iso.cells_touching(0, 0, 1280, 720, 0);
        let mut found = HashSet::new();
        for y in (0..720).step_by(3) {
            for x in (0..1280).step_by(3) {
                found.insert(iso.cell_at(x, y));
            }
        }
        for (i, j) in found {
            assert!((lo_i..=hi_i).contains(&i) && (lo_j..=hi_j).contains(&j), "cell {i},{j}");
        }
    }
}
