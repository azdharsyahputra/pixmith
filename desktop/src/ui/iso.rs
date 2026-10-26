//! The isometric grid: diamond cells of tile width `w` and height `h` (default 32 x 16), drawn with pixel-exact
//! stepping, and the exact pixel mask of one cell for Select > Isometric Tile.
//!
//! Layout. Cell (0, 0) has its bounding box at `(x, y)` with size `w` x `h`, so its corners are
//! `(x + w/2, y)`, `(x + w, y + h/2)`, `(x + w/2, y + h)` and `(x, y + h/2)`. Cell (i, j) is that diamond moved by
//! `(i - j) * w/2` across and `(i + j) * h/2` down; the cells tile the plane without gaps. Every grid line is
//! therefore one of two families: down-right lines through the cells' top corners, and up-right ones.
//!
//! Pixels. A line is drawn in sprite pixels: in each column (or row, for steep tiles) the pixel the line passes
//! at the column's center. For 32 x 16 that is the classic 2:1 isometric line, two pixels per step. A pixel
//! belongs to the cell its center lies in (a center on a border goes to the cell below or to the right), so the
//! masks of all cells partition the sprite and a 32 x 16 cell has `w * h / 2` = 256 pixels.

use eframe::egui::{self, Color32, Pos2, Rect};

pub const DEFAULT_W: i32 = 32;
pub const DEFAULT_H: i32 = 16;
pub const MIN_SIDE: i32 = 4;
pub const MAX_SIDE: i32 = 4096;

/// A sanitized isometric grid: even sizes of at least 4 and the offset of cell (0, 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Iso {
    pub w: i64,
    pub h: i64,
    pub x: i64,
    pub y: i64,
}

/// Rounds a tile side to the nearest allowed value: even, 4 to 4096.
pub fn clamp_side(v: i32) -> i32 {
    (v.clamp(MIN_SIDE, MAX_SIDE) + 1) & !1
}

impl Iso {
    pub fn new(w: i32, h: i32, x: i32, y: i32) -> Iso {
        Iso { w: clamp_side(w) as i64, h: clamp_side(h) as i64, x: x as i64, y: y as i64 }
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
        let (i, j) = cell;
        let left = self.x + (i - j) * self.w / 2;
        let top = self.y + (i + j) * self.h / 2;
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
    /// them) are exactly the cell's mask. The path runs down the left edge and back up the right one, turning
    /// inside the cell, so no outline pixel falls outside it.
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

    /// The same grid with the axes swapped, so steep tiles can reuse the shallow code.
    fn transposed(&self) -> Iso {
        Iso { w: self.h, h: self.w, x: self.y, y: self.x }
    }

    /// Row of the down-right line 0 in pixel column `x` (shallow tiles, `h <= w`).
    fn down_row(&self, x: i64) -> i64 {
        (2 * self.w * self.y + (2 * x + 1 - 2 * self.x - self.w) * self.h).div_euclid(2 * self.w)
    }

    /// Row of the up-right line 0 in pixel column `x`.
    fn up_row(&self, x: i64) -> i64 {
        (2 * self.w * self.y - (2 * x + 1 - 2 * self.x - self.w) * self.h).div_euclid(2 * self.w)
    }

    /// Calls `emit(x0, x1, y)` for every horizontal run of grid-line pixels inside `area`
    /// (`[x0, y0, x1, y1]`, inclusive). A pixel of both families is emitted once.
    pub fn line_runs(&self, area: [i64; 4], emit: &mut impl FnMut(i64, i64, i64)) {
        if self.h <= self.w {
            self.shallow_runs(area, &mut |a, b, y| emit(a, b, y));
        } else {
            // Steep: runs of the transposed grid are vertical runs here.
            let t = self.transposed();
            let swapped = [area[1], area[0], area[3], area[2]];
            t.shallow_runs(swapped, &mut |a, b, y| {
                for k in a..=b {
                    emit(y, y, k);
                }
            });
        }
    }

    fn shallow_runs(&self, [xa, ya, xb, yb]: [i64; 4], emit: &mut impl FnMut(i64, i64, i64)) {
        if xb < xa || yb < ya {
            return;
        }
        let h = self.h;
        let (d_lo, d_hi) = (self.down_row(xa), self.down_row(xb));
        // Down-right lines: row = down_row(x) + j * h.
        for j in (ya - d_hi).div_euclid(h)..=(yb - d_lo).div_euclid(h) + 1 {
            let mut run: Option<(i64, i64, i64)> = None;
            for x in xa..=xb {
                let y = self.down_row(x) + j * h;
                run = push_run(run, x, y, y >= ya && y <= yb, emit);
            }
            flush(run, emit);
        }
        let (u_hi, u_lo) = (self.up_row(xa), self.up_row(xb));
        for i in (ya - u_hi).div_euclid(h)..=(yb - u_lo).div_euclid(h) + 1 {
            let mut run: Option<(i64, i64, i64)> = None;
            for x in xa..=xb {
                let y = self.up_row(x) + i * h;
                // A pixel that is on a down-right line as well is drawn by that line.
                let both = (y - self.down_row(x)).rem_euclid(h) == 0;
                run = push_run(run, x, y, y >= ya && y <= yb && !both, emit);
            }
            flush(run, emit);
        }
    }

    /// Smallest and largest line index of the down-right (`.0`) and up-right (`.1`) families whose lines reach
    /// the pixel box `[x0, y0, x1, y1]`, as `i64` ranges over the continuous line equations.
    fn line_ranges(&self, [x0, y0, x1, y1]: [f64; 4]) -> ((i64, i64), (i64, i64)) {
        let (w, h) = (self.w as f64, self.h as f64);
        let oy = self.y as f64;
        let at = |x: f64| (x - self.x as f64 - w / 2.0) * h / w;
        let down = (((y0 - oy - at(x1)) / h).floor() as i64 - 1, ((y1 - oy - at(x0)) / h).ceil() as i64 + 1);
        let up = (((y0 - oy + at(x0)) / h).floor() as i64 - 1, ((y1 - oy + at(x1)) / h).ceil() as i64 + 1);
        (down, up)
    }

    /// The grid as segments across the box `[x0, y0, x1, y1]` (sprite coordinates, in pixels), for zoomed-out
    /// views where pixel stepping is not visible and too many pixels would be drawn.
    pub fn segments(&self, bx: [f64; 4]) -> Vec<[(f64, f64); 2]> {
        let (w, h) = (self.w as f64, self.h as f64);
        let (ox, oy) = (self.x as f64, self.y as f64);
        let (down, up) = self.line_ranges(bx);
        let mut out = Vec::new();
        for j in down.0..=down.1 {
            let y = |x: f64| oy + (x - ox - w / 2.0) * h / w + j as f64 * h;
            out.push([(bx[0], y(bx[0])), (bx[2], y(bx[2]))]);
        }
        for i in up.0..=up.1 {
            let y = |x: f64| oy + i as f64 * h - (x - ox - w / 2.0) * h / w;
            out.push([(bx[0], y(bx[0])), (bx[2], y(bx[2]))]);
        }
        out
    }

    /// Rough count of pixel steps `line_runs` visits for a box of `vw` x `vh` pixels.
    pub fn work(&self, vw: i64, vh: i64) -> i64 {
        let (major, across) =
            if self.h <= self.w { (vw, vh + vw * self.h / self.w) } else { (vh, vw + vh * self.w / self.h) };
        let lines = 2 * (across / self.h.min(self.w).max(1) + 3);
        lines * major
    }
}

/// Extends the run with pixel `(x, y)` when `visible`, closing it otherwise (or when `y` changes).
fn push_run(
    run: Option<(i64, i64, i64)>,
    x: i64,
    y: i64,
    visible: bool,
    emit: &mut impl FnMut(i64, i64, i64),
) -> Option<(i64, i64, i64)> {
    match run {
        Some((a, b, ry)) if visible && ry == y && b + 1 == x => Some((a, x, ry)),
        other => {
            flush(other, emit);
            visible.then_some((x, x, y))
        }
    }
}

fn flush(run: Option<(i64, i64, i64)>, emit: &mut impl FnMut(i64, i64, i64)) {
    if let Some((a, b, y)) = run {
        emit(a, b, y);
    }
}

/// Most pixel steps the pixel-exact drawing takes before it falls back to plain lines.
const PIXEL_BUDGET: i64 = 400_000;

/// Draws the grid over the visible part `vis` of the sprite. `origin` is the sprite's top-left corner in points.
pub fn paint(painter: &egui::Painter, iso: &Iso, origin: Pos2, zoom: f32, ppp: f32, vis: Rect, color: Color32) {
    let painter = painter.with_clip_rect(vis);
    let (lo, hi) = ((vis.min - origin) / zoom, (vis.max - origin) / zoom);
    let area = [lo.x.floor() as i64 - 1, lo.y.floor() as i64 - 1, hi.x.ceil() as i64 + 1, hi.y.ceil() as i64 + 1];
    let (vw, vh) = (area[2] - area[0] + 1, area[3] - area[1] + 1);
    if iso.work(vw, vh) > PIXEL_BUDGET {
        let stroke = egui::Stroke::new((1.0 / ppp).max(1.0), color);
        let bx = [area[0] as f64, area[1] as f64, area[2] as f64, area[3] as f64];
        for [a, b] in iso.segments(bx) {
            let p = |(x, y): (f64, f64)| origin + egui::vec2(x as f32, y as f32) * zoom;
            painter.line_segment([p(a), p(b)], stroke);
        }
        return;
    }
    let snap = |v: f32| (v * ppp).round() / ppp;
    let mut mesh = egui::Mesh::default();
    iso.line_runs(area, &mut |x0, x1, y| {
        let min = Pos2::new(snap(origin.x + x0 as f32 * zoom), snap(origin.y + y as f32 * zoom));
        let max = Pos2::new(snap(origin.x + (x1 + 1) as f32 * zoom), snap(origin.y + (y + 1) as f32 * zoom));
        mesh.add_colored_rect(Rect::from_min_max(min, max), color);
    });
    painter.add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn sides_are_even_and_at_least_four() {
        assert_eq!([clamp_side(0), clamp_side(5), clamp_side(32), clamp_side(99999)], [4, 6, 32, 4096]);
    }

    #[test]
    fn a_32_by_16_cell_is_the_classic_diamond() {
        let iso = Iso::new(32, 16, 0, 0);
        let rows = iso.cell_rows((0, 0));
        assert_eq!(rows.len(), 16);
        let widths: Vec<i32> = rows.iter().map(|&(_, l, r)| r - l + 1).collect();
        assert_eq!(widths, [2, 6, 10, 14, 18, 22, 26, 30, 30, 26, 22, 18, 14, 10, 6, 2]);
        assert_eq!(widths.iter().sum::<i32>(), 32 * 16 / 2);
        // Centered on the tile and symmetric.
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
        // Cell (0, 0) sits in the box at the offset.
        let rows = iso.cell_rows((0, 0));
        assert_eq!(rows.first().map(|r| r.0), Some(-3));
        assert_eq!(rows.iter().map(|r| r.1).min(), Some(5 + 1));
        assert_eq!(rows.iter().map(|r| r.2).max(), Some(5 + 30));
    }

    #[test]
    fn odd_sized_tiles_still_partition() {
        for (w, h) in [(4, 4), (6, 4), (8, 16), (104, 56), (10, 4)] {
            let iso = Iso::new(w, h, 0, 0);
            let mut count = 0;
            for cell in [(0, 0), (1, 0), (0, 1)] {
                let rows = iso.cell_rows(cell);
                assert!(!rows.is_empty());
                count += rows.iter().map(|&(_, l, r)| (r - l + 1) as i64).sum::<i64>();
            }
            assert_eq!(count, 3 * w as i64 * h as i64 / 2, "{w}x{h}");
        }
    }

    #[test]
    fn grid_lines_step_two_pixels_per_row_for_32_by_16() {
        let iso = Iso::new(32, 16, 0, 0);
        let mut runs = Vec::new();
        iso.line_runs([0, 0, 31, 15], &mut |a, b, y| runs.push((a, b, y)));
        // The top apex is a flat four pixels (two from each side) and the left edge climbs 2:1.
        assert!(runs.contains(&(14, 15, 0)) && runs.contains(&(16, 17, 0)), "{runs:?}");
        assert!(runs.contains(&(0, 1, 7)) || runs.contains(&(0, 1, 8)), "{runs:?}");
        assert!(runs.iter().all(|&(a, b, _)| b - a <= 1), "no run is longer than a 2:1 step");
        // No pixel twice.
        let mut seen = HashSet::new();
        for &(a, b, y) in &runs {
            for x in a..=b {
                assert!(seen.insert((x, y)));
            }
        }
    }

    #[test]
    fn a_tile_has_one_line_pixel_per_column_and_family() {
        let iso = Iso::new(32, 16, 0, 0);
        let mut pixels = HashSet::new();
        iso.line_runs([0, 0, 31, 15], &mut |a, b, y| (a..=b).for_each(|x| _ = pixels.insert((x, y))));
        // One pixel per column and family in a tile that is 32 columns wide.
        assert_eq!(pixels.len(), 64);
    }

    #[test]
    fn steep_tiles_draw_one_pixel_per_row() {
        let iso = Iso::new(8, 16, 0, 0);
        let mut runs = Vec::new();
        iso.line_runs([0, 0, 7, 15], &mut |a, b, y| runs.push((a, b, y)));
        assert!(!runs.is_empty());
        let columns: HashSet<i64> = runs.iter().filter(|r| r.0 == r.1).map(|r| r.0).collect();
        assert!(columns.len() > 1);
    }
}
