//! Cel content beyond the canvas edges (Aseprite semantics).
//!
//! In Aseprite a cel is an image with a position, and it may stick out of the canvas: moving a cel
//! off the canvas and back loses nothing, and a file saved with such a cel keeps it. Pixmith cels
//! are canvas-sized RGBA buffers (`Cel::pixels`, used by every drawing and rendering routine), so
//! the part beyond the edges is kept next to them:
//!
//! * What is visible inside the canvas always comes from `Cel::pixels`.
//! * `Cel::outside` (an `Arc`, so undo snapshots and linked cels share it for free) holds only what lies
//!   *beyond* the canvas edges: the tightest rectangle around the non-transparent pixels that are not
//!   inside the canvas. The part of that rectangle which overlaps the canvas is zeroed and ignored.
//!   `None` (the common case) means nothing sticks out and costs nothing.
//! * Operations that only know the canvas (drawing, filters, selections) leave `outside` alone.
//!   Operations on the whole cel (moving it, changing the canvas size, scaling the sprite, saving)
//!   go through the helpers below, which see the *full* content: `Cel::full_bounds`,
//!   `Cel::full_image`, `Cel::set_full`, `Cel::translate`.
//!
//! Limits follow the `.aseprite` format: a cel's x and y are signed 16-bit values and its width and
//! height unsigned 16-bit ones, and the stored part may not exceed [`MAX_OFFCANVAS_PIXELS`].

use crate::document::Cel;

/// Smallest and largest top-left coordinate of a cel (a signed 16-bit value in `.aseprite` files).
pub const COORD_MIN: i32 = i16::MIN as i32;
pub const COORD_MAX: i32 = i16::MAX as i32;
/// Largest width or height of a cel image.
pub const MAX_OFFCANVAS_DIM: usize = u16::MAX as usize;
/// Most pixels the stored off-canvas rectangle of one cel may have.
pub const MAX_OFFCANVAS_PIXELS: usize = crate::document::MAX_CANVAS_PIXELS;

/// The content of a cel that lies beyond the canvas: an RGBA image at (`x`, `y`) in canvas
/// coordinates (it may start at negative coordinates and extend past the canvas width and height).
/// The pixels of the image that fall inside the canvas are zero and meaningless; see the module docs.
#[derive(Clone, Debug, PartialEq)]
pub struct OffCanvas {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// An RGBA image placed in canvas coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct FullImage {
    pub x: i32,
    pub y: i32,
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

/// A rectangle (x, y, w, h) in canvas coordinates.
pub type Rect4 = (i32, i32, usize, usize);

/// Why a placement was refused (the cel is left unchanged).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceError {
    /// The image does not hold `w * h * 4` bytes.
    BadSize,
    /// The part beyond the canvas would start too far away or be too large for the file format.
    TooFar,
}

impl std::fmt::Display for PlaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlaceError::BadSize => f.write_str("the image does not match its size"),
            PlaceError::TooFar => f.write_str("the cel would reach too far beyond the canvas"),
        }
    }
}

fn intersect(a: Rect4, b: Rect4) -> Option<Rect4> {
    let x0 = a.0.max(b.0);
    let y0 = a.1.max(b.1);
    let x1 = (a.0 as i64 + a.2 as i64).min(b.0 as i64 + b.2 as i64);
    let y1 = (a.1 as i64 + a.3 as i64).min(b.1 as i64 + b.3 as i64);
    (x1 > x0 as i64 && y1 > y0 as i64).then(|| (x0, y0, (x1 - x0 as i64) as usize, (y1 - y0 as i64) as usize))
}

/// The smallest rectangle holding both.
pub fn union_rect(a: Rect4, b: Rect4) -> Rect4 {
    let x0 = a.0.min(b.0);
    let y0 = a.1.min(b.1);
    let x1 = (a.0 as i64 + a.2 as i64).max(b.0 as i64 + b.2 as i64);
    let y1 = (a.1 as i64 + a.3 as i64).max(b.1 as i64 + b.3 as i64);
    (x0, y0, (x1 - x0 as i64) as usize, (y1 - y0 as i64) as usize)
}

/// Copies the part of `src` (an image at `src_rect`) that lies inside `dst_rect` into `dst`
/// (an image at `dst_rect`), replacing.
fn blit_rect(dst: &mut [u8], dst_rect: Rect4, src: &[u8], src_rect: Rect4) {
    let Some((x, y, w, h)) = intersect(dst_rect, src_rect) else { return };
    for row in 0..h {
        let ay = y as i64 + row as i64;
        let s = (((ay - src_rect.1 as i64) as usize) * src_rect.2 + (x as i64 - src_rect.0 as i64) as usize) * 4;
        let d = (((ay - dst_rect.1 as i64) as usize) * dst_rect.2 + (x as i64 - dst_rect.0 as i64) as usize) * 4;
        dst[d..d + w * 4].copy_from_slice(&src[s..s + w * 4]);
    }
}

impl OffCanvas {
    pub fn rect(&self) -> Rect4 {
        (self.x, self.y, self.w as usize, self.h as usize)
    }

    /// Checks the invariants `Sprite::validate` relies on.
    pub fn check(&self) -> Result<(), String> {
        let (w, h) = (self.w as usize, self.h as usize);
        if w == 0 || h == 0 || w > MAX_OFFCANVAS_DIM || h > MAX_OFFCANVAS_DIM {
            return Err(format!("an off-canvas cel part of {w}x{h} pixels is out of range"));
        }
        if w * h > MAX_OFFCANVAS_PIXELS {
            return Err(format!("an off-canvas cel part of {w}x{h} pixels is too large"));
        }
        if self.rgba.len() != w * h * 4 {
            return Err(format!("an off-canvas cel part has {} bytes, expected {}", self.rgba.len(), w * h * 4));
        }
        if !(COORD_MIN..=COORD_MAX).contains(&self.x) || !(COORD_MIN..=COORD_MAX).contains(&self.y) {
            return Err(format!("an off-canvas cel part at ({}, {}) is out of range", self.x, self.y));
        }
        Ok(())
    }

    /// The part of the image `rgba` (at `x`, `y`, `w` by `h`) that lies beyond the canvas, as the
    /// tightest rectangle around its non-transparent pixels. `Ok(None)` when nothing does.
    pub fn extract(cw: usize, ch: usize, (x, y, w, h): Rect4, rgba: &[u8]) -> Result<Option<OffCanvas>, PlaceError> {
        if rgba.len() != w * h * 4 {
            return Err(PlaceError::BadSize);
        }
        let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
        let mut scan = |row: usize, from: usize, to: usize| {
            let line = &rgba[row * w * 4..(row + 1) * w * 4];
            let first = (from..to).find(|&c| line[c * 4 + 3] != 0);
            if let Some(first) = first {
                let last = (from..to).rev().find(|&c| line[c * 4 + 3] != 0).unwrap_or(first);
                let ay = y as i64 + row as i64;
                x0 = x0.min(x as i64 + first as i64);
                x1 = x1.max(x as i64 + last as i64 + 1);
                y0 = y0.min(ay);
                y1 = y1.max(ay + 1);
            }
        };
        for row in 0..h {
            let ay = y as i64 + row as i64;
            if ay < 0 || ay >= ch as i64 {
                scan(row, 0, w);
            } else {
                // Left of the canvas, then right of it.
                let left = (-(x as i64)).clamp(0, w as i64) as usize;
                scan(row, 0, left);
                let right = (cw as i64 - x as i64).clamp(0, w as i64) as usize;
                scan(row, right.max(left), w);
            }
        }
        if x1 <= x0 {
            return Ok(None);
        }
        let (bw, bh) = ((x1 - x0) as usize, (y1 - y0) as usize);
        if bw > MAX_OFFCANVAS_DIM
            || bh > MAX_OFFCANVAS_DIM
            || bw * bh > MAX_OFFCANVAS_PIXELS
            || !(COORD_MIN as i64..=COORD_MAX as i64).contains(&x0)
            || !(COORD_MIN as i64..=COORD_MAX as i64).contains(&y0)
        {
            return Err(PlaceError::TooFar);
        }
        let rect = (x0 as i32, y0 as i32, bw, bh);
        let mut out = vec![0u8; bw * bh * 4];
        blit_rect(&mut out, rect, rgba, (x, y, w, h));
        // Zero the part inside the canvas (the canonical form; see the module docs).
        if let Some((ix, iy, iw, ih)) = intersect(rect, (0, 0, cw, ch)) {
            for row in 0..ih {
                let d = (((iy as i64 - rect.1 as i64) as usize + row) * bw + (ix as i64 - rect.0 as i64) as usize) * 4;
                out[d..d + iw * 4].fill(0);
            }
        }
        Ok(Some(OffCanvas { x: rect.0, y: rect.1, w: bw as u32, h: bh as u32, rgba: out }))
    }
}

impl Cel {
    /// Whether any content lies beyond the canvas edges.
    pub fn has_outside(&self) -> bool {
        self.outside.is_some()
    }

    /// Bounding box of the non-transparent pixels inside the canvas of `cw` x `ch`.
    fn inside_bounds(&self, cw: usize, ch: usize) -> Option<Rect4> {
        if self.pixels.len() != cw * ch * 4 {
            return None;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        for (y, row) in self.pixels.chunks_exact(cw * 4).enumerate() {
            let Some(first) = row.chunks_exact(4).position(|p| p[3] != 0) else { continue };
            let last = row.chunks_exact(4).rposition(|p| p[3] != 0).unwrap_or(first);
            x0 = x0.min(first);
            x1 = x1.max(last + 1);
            y0 = y0.min(y);
            y1 = y + 1;
        }
        (x1 > x0 && y1 > y0).then(|| (x0 as i32, y0 as i32, x1 - x0, y1 - y0))
    }

    /// The box around all of the cel's non-transparent content, inside and beyond the canvas of
    /// `cw` x `ch`; `None` for an empty cel.
    pub fn full_bounds(&self, cw: usize, ch: usize) -> Option<Rect4> {
        match (self.inside_bounds(cw, ch), &self.outside) {
            (Some(a), Some(o)) => Some(union_rect(a, o.rect())),
            (Some(a), None) => Some(a),
            (None, Some(o)) => Some(o.rect()),
            (None, None) => None,
        }
    }

    /// The cel's content inside `rect` (canvas coordinates), from the canvas buffer and the
    /// off-canvas part; transparent where the cel has nothing.
    pub fn read_region(&self, cw: usize, ch: usize, rect: Rect4) -> Vec<u8> {
        let mut out = vec![0u8; rect.2 * rect.3 * 4];
        if let Some(o) = &self.outside {
            blit_rect(&mut out, rect, &o.rgba, o.rect());
        }
        if self.pixels.len() == cw * ch * 4 {
            blit_rect(&mut out, rect, &self.pixels, (0, 0, cw, ch));
        }
        out
    }

    /// The color at (x, y) in canvas coordinates, from anywhere in the cel's content.
    pub fn pixel_at(&self, cw: usize, ch: usize, x: i64, y: i64) -> [u8; 4] {
        if x >= 0 && y >= 0 && x < cw as i64 && y < ch as i64 {
            let i = (y as usize * cw + x as usize) * 4;
            return self.pixels.get(i..i + 4).and_then(|p| p.try_into().ok()).unwrap_or([0; 4]);
        }
        if let Some(o) = &self.outside {
            let (dx, dy) = (x - o.x as i64, y - o.y as i64);
            if dx >= 0 && dy >= 0 && dx < o.w as i64 && dy < o.h as i64 {
                let i = (dy as usize * o.w as usize + dx as usize) * 4;
                return o.rgba[i..i + 4].try_into().unwrap_or([0; 4]);
            }
        }
        [0; 4]
    }

    /// The cel's full content (see `full_bounds`) as an image; `None` for an empty cel.
    pub fn full_image(&self, cw: usize, ch: usize) -> Option<FullImage> {
        let (x, y, w, h) = self.full_bounds(cw, ch)?;
        Some(FullImage { x, y, w, h, rgba: self.read_region(cw, ch, (x, y, w, h)) })
    }

    /// Replaces the whole content of the cel with the image `rgba` (`w` x `h`) placed at (`x`, `y`)
    /// in canvas coordinates: the part inside the canvas of `cw` x `ch` goes to `pixels`, the part
    /// beyond it to `outside`. Fails, changing nothing, if the part beyond the canvas is too large
    /// or too far away for the `.aseprite` format.
    pub fn set_full(&mut self, cw: usize, ch: usize, (x, y, w, h): Rect4, rgba: &[u8]) -> Result<(), PlaceError> {
        let outside = OffCanvas::extract(cw, ch, (x, y, w, h), rgba)?;
        self.pixels.clear();
        self.pixels.resize(cw * ch * 4, 0);
        blit_rect(&mut self.pixels, (0, 0, cw, ch), rgba, (x, y, w, h));
        self.outside = outside.map(std::sync::Arc::new);
        Ok(())
    }

    /// Replaces what the cel holds inside `rect` with the image `rgba` (`rect.2` x `rect.3`);
    /// everything else stays. Parts beyond the canvas go to `outside`.
    pub fn write_region(&mut self, cw: usize, ch: usize, rect: Rect4, rgba: &[u8]) -> Result<(), PlaceError> {
        if rgba.len() != rect.2 * rect.3 * 4 {
            return Err(PlaceError::BadSize);
        }
        if rect.2 == 0 || rect.3 == 0 {
            return Ok(());
        }
        let canvas = (0, 0, cw, ch);
        let beyond = intersect(rect, canvas).is_none_or(|i| i != rect);
        if beyond {
            // Merge into the existing off-canvas part (its canvas area is zero, which is fine here).
            let merged = match &self.outside {
                Some(o) => {
                    let u = union_rect(o.rect(), rect);
                    // The union is only a scratch buffer (the result is trimmed again), but bound it.
                    if u.2.saturating_mul(u.3) > MAX_OFFCANVAS_PIXELS * 4 {
                        return Err(PlaceError::TooFar);
                    }
                    let mut buf = vec![0u8; u.2 * u.3 * 4];
                    blit_rect(&mut buf, u, &o.rgba, o.rect());
                    blit_rect(&mut buf, u, rgba, rect);
                    (u, buf)
                }
                None => (rect, rgba.to_vec()),
            };
            let outside = OffCanvas::extract(cw, ch, merged.0, &merged.1)?;
            self.outside = outside.map(std::sync::Arc::new);
        }
        if self.pixels.len() != cw * ch * 4 {
            self.pixels.clear();
            self.pixels.resize(cw * ch * 4, 0);
        }
        blit_rect(&mut self.pixels, canvas, rgba, rect);
        Ok(())
    }

    /// Moves the cel's whole content by (`dx`, `dy`). What leaves the canvas is kept in `outside`
    /// and what returns is revealed again, so moving out and back loses nothing. Fails, changing
    /// nothing, when the content would reach too far (see `set_full`).
    pub fn translate(&mut self, cw: usize, ch: usize, dx: i32, dy: i32) -> Result<(), PlaceError> {
        if dx == 0 && dy == 0 {
            return Ok(());
        }
        let Some(f) = self.full_image(cw, ch) else { return Ok(()) };
        let (nx, ny) = (f.x as i64 + dx as i64, f.y as i64 + dy as i64);
        if nx < COORD_MIN as i64 || ny < COORD_MIN as i64 || nx > i32::MAX as i64 / 2 || ny > i32::MAX as i64 / 2 {
            return Err(PlaceError::TooFar);
        }
        self.set_full(cw, ch, (nx as i32, ny as i32, f.w, f.h), &f.rgba)
    }

    /// Drops everything beyond the canvas.
    pub fn clear_outside(&mut self) {
        self.outside = None;
    }
}
