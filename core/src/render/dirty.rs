//! Which pixels of the document changed since a viewer last looked, so it can re-render only that part.

use crate::document::Editor;

/// Everything starts out (and defaults to) fully dirty; only edits that know exactly what they touched narrow it
/// with `mark_rect`. A rectangle covers one frame; edits to a second frame or to anything shared make it `all`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dirty {
    all: bool,
    frame: usize,
    /// `(x0, y0, x1, y1)`, end exclusive.
    rect: Option<(i32, i32, i32, i32)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DirtyTake {
    Nothing,
    Rect { frame: usize, x: i32, y: i32, w: i32, h: i32 },
    All,
}

impl Default for Dirty {
    fn default() -> Self {
        Dirty { all: true, frame: 0, rect: None }
    }
}

impl Dirty {
    pub fn mark_all(&mut self) {
        self.all = true;
        self.rect = None;
    }

    /// Records a changed rectangle `(x0, y0, x1, y1)` (end exclusive) of `frame`, clipped to the canvas.
    pub fn mark_rect(&mut self, frame: usize, r: (i32, i32, i32, i32), size: (u32, u32)) {
        if self.all {
            return;
        }
        let (x0, y0) = (r.0.max(0), r.1.max(0));
        let (x1, y1) = (r.2.min(size.0 as i32), r.3.min(size.1 as i32));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        match self.rect {
            None => (self.frame, self.rect) = (frame, Some((x0, y0, x1, y1))),
            Some(_) if self.frame != frame => self.mark_all(),
            Some(o) => self.rect = Some((o.0.min(x0), o.1.min(y0), o.2.max(x1), o.3.max(y1))),
        }
    }

    /// What changed since the previous call; resets to clean.
    pub fn take(&mut self) -> DirtyTake {
        let t = match (self.all, self.rect) {
            (true, _) => DirtyTake::All,
            (false, Some((x0, y0, x1, y1))) => {
                DirtyTake::Rect { frame: self.frame, x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
            }
            (false, None) => DirtyTake::Nothing,
        };
        *self = Dirty { all: false, frame: 0, rect: None };
        t
    }
}

impl Editor {
    /// Whether other frames show the same pixels as the cel at (`layer`, `frame`) (a linked cel).
    pub fn cel_shared(&self, layer: usize, frame: usize) -> bool {
        self.sprite.is_cel_linked(layer, frame)
    }

    /// Reports that an edit changed `rect` (end exclusive) of the cel at (`layer`, `frame`). Anything that can
    /// show elsewhere counts as a full change instead: tilemaps share tiles, linked cels share pixels, and a
    /// tweened layer is computed from the cels of other frames. `shared` is `cel_shared` as it was *before* the
    /// edit (an edit unshares the buffer it writes to).
    pub fn mark_cel_rect(&mut self, layer: usize, frame: usize, rect: (i32, i32, i32, i32), shared: bool) {
        let plain = self.sprite.layers.get(layer).is_some_and(|l| l.kind == 0);
        if !plain || shared || self.sprite.has_pending_links() || crate::gamedev::tween::any_tween(&self.sprite) {
            self.dirty.mark_all();
        } else {
            self.dirty.mark_rect(frame, rect, (self.sprite.width, self.sprite.height));
        }
    }
}
