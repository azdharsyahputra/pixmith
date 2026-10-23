//! Model-based random testing of the editing core through its C ABI (the functions the Swift app
//! calls), with a fixed-seed PRNG. Every step checks structural invariants; every sequence is then
//! saved and re-read, undone to the start and redone to the end, and its dirty state is checked.
//! Failing sequences are shrunk and printed as `op(...)` lines that paste into a regression test.
//!
//! `cargo test --test stress` runs a few hundred sequences; `PIXMITH_STRESS_SEQS=20000 cargo test
//! --release --test stress -- --ignored` runs the long variant (`PIXMITH_STRESS_SEED` sets the first seed).

#![allow(clippy::too_many_arguments)]
// RGBA pixels are walked four bytes at a time with `chunks_exact(4)`, the same idiom as in the library.
#![allow(clippy::chunks_exact_to_as_chunks)]

use pixmith_core::document::color_mode as colormode;
use pixmith_core::document::{layers, slices, tilemap, Editor, Sprite};
use pixmith_core::ffi::document::*;
use pixmith_core::ffi::pixmith_string_free;
use pixmith_core::history;
use pixmith_core::paint::{curves, filters, font, ink, selection, shapes, transform};
use pixmith_core::palette::{edit as palette_edit, io as palette_io, presets as palette_presets};
use std::cell::{Cell, RefCell};
use std::ffi::CString;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::{null, null_mut};

type Doc = *mut Editor;

// MARK: PRNG

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }

    /// Inclusive range.
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + self.below((hi - lo + 1) as u64) as i64
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len() as u64) as usize]
    }
}

// MARK: Operations

macro_rules! kinds {
    ($($k:ident),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum K { $($k),* }
        const ALL: &[K] = &[$(K::$k),*];
    };
}

kinds!(
    DrawLine,
    Fill,
    FillMode,
    FillPattern,
    Shape,
    Stroke,
    Spray,
    Gradient,
    Text,
    StampMask,
    Curve,
    Polygon,
    InkSet,
    BrushCircle,
    BrushFromSel,
    BrushReset,
    SelRect,
    SelAll,
    SelClear,
    SelInvert,
    SelPoly,
    SelWand,
    SelModify,
    MoveSel,
    Nudge,
    SelDelete,
    CopyPaste,
    Paste,
    Transform,
    ResizeCanvas,
    Crop,
    FloatCommit,
    FloatCancel,
    Xf,
    AddLayer,
    RemoveLayer,
    RenameLayer,
    SetVisible,
    SetOpacity,
    LayerMove,
    LayerDup,
    MergeDown,
    AddGroup,
    Group,
    IntoGroup,
    OutOfGroup,
    LayerProps,
    AddFrame,
    RemoveFrame,
    FrameMove,
    SetDuration,
    CelOpacity,
    CelLink,
    CelUnlink,
    TagAdd,
    TagUpdate,
    TagRemove,
    CelsClear,
    CelsCopy,
    CelsPaste,
    CelsMove,
    PaletteAdd,
    PaletteRemove,
    PaletteSort,
    PeMove,
    PeInsert,
    PeDuplicate,
    PeRemove,
    PeSetLive,
    PeRamp,
    PresetLoad,
    CmPaletteSet,
    CmPaletteFromSprite,
    CmConvert,
    FilterApply,
    ScaleSprite,
    AddTilemapLayer,
    TileStamp,
    ConvertToTilemap,
    ConvertToNormal,
    DupTile,
    DelTile,
    AssignTileset,
    TilesetAddTile,
    TilesetRename,
    SliceAdd,
    SliceBounds,
    SliceProps,
    SliceRemove,
    UserData,
    Undo,
    Redo,
    HistoryGoto,
    TxnBegin,
    TxnEnd,
    Label,
    BeginStroke,
    Touch,
    MarkSaved,
    XfBegin,
    XfDrag,
    XfCommit,
    XfCancel,
    SelBegin,
    SelEnd,
    MoveBegin,
    MoveTo,
    MoveEnd,
    PreviewStroke,
    StaleReset,
    TxnStrokes,
    LongStroke,
);

/// Operations that change the document without an undo step.
fn is_raw(k: K) -> bool {
    matches!(k, K::SetVisible | K::SetOpacity | K::SetDuration | K::PaletteAdd | K::Touch | K::PeSetLive | K::MarkSaved)
}

#[derive(Clone)]
struct Op {
    k: K,
    p: [i64; 12],
    s: String,
    pts: Vec<i32>,
}

fn op(k: K, p: &[i64], s: &str, pts: &[i32]) -> Op {
    let mut a = [0i64; 12];
    a[..p.len()].copy_from_slice(p);
    Op { k, p: a, s: s.to_string(), pts: pts.to_vec() }
}

impl std::fmt::Debug for Op {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.p.iter().rposition(|v| *v != 0).map_or(0, |i| i + 1);
        write!(f, "op(K::{:?}, &{:?}, {:?}, &{:?})", self.k, &self.p[..n], self.s, self.pts)
    }
}

#[derive(Clone, Debug)]
struct Seq {
    w: u32,
    h: u32,
    ops: Vec<Op>,
}

// MARK: Document access

fn ed<'a>(d: Doc) -> &'a mut Editor {
    unsafe { &mut *d }
}

fn spr<'a>(d: Doc) -> &'a Sprite {
    unsafe { &(*d).sprite }
}

fn cs(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap()
}

struct Info {
    w: i64,
    h: i64,
    layers: i64,
    frames: i64,
    tags: i64,
    tilesets: i64,
    palette: i64,
}

fn info(d: Doc) -> Info {
    let s = spr(d);
    Info {
        w: s.width as i64,
        h: s.height as i64,
        layers: s.layers.len() as i64,
        frames: s.frame_count() as i64,
        tags: s.tags.len() as i64,
        tilesets: s.tilesets.len() as i64,
        palette: s.palette.len() as i64,
    }
}

// MARK: Generation

fn idx(r: &mut Rng, n: i64) -> i64 {
    match r.below(100) {
        0..=91 => r.range(0, (n - 1).max(0)),
        92..=95 => n + r.range(0, 3),
        96..=97 => u32::MAX as i64,
        _ => r.range(0, 1000),
    }
}

/// `PIXMITH_STRESS_EXTREME=1` adds absurd coordinates and sizes (i32 extremes, 4-billion brush sizes).
fn extreme() -> bool {
    static E: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *E.get_or_init(|| std::env::var("PIXMITH_STRESS_EXTREME").is_ok())
}

fn coord(r: &mut Rng, extent: i64) -> i64 {
    if extreme() && r.chance(15) {
        return r.pick(&[i32::MIN as i64, i32::MAX as i64, -1_000_000_000, 1_000_000_000, 200_000, -70_000]);
    }
    match r.below(100) {
        0..=84 => r.range(-2, extent + 2),
        85..=92 => r.range(-extent * 3 - 5, extent * 3 + 5),
        93..=96 => r.pick(&[1500, -1500, 1024, -1025, 1200]),
        _ => r.range(0, extent.max(1) - 1),
    }
}

fn color(r: &mut Rng) -> i64 {
    match r.below(10) {
        0 => 0,
        1 => 0xFF0000FF,
        2 => 0x00FF00FF,
        3 => 0x0000FFFF,
        4 => 0x000000FF,
        5 => 0xFFFFFFFF,
        6 => (r.next() as u32 | 0xFF) as i64,
        7 => (r.next() as u32 & !0xFF) as i64 | r.range(1, 254),
        _ => r.next() as u32 as i64,
    }
}

fn small(r: &mut Rng, hi: i64) -> i64 {
    if extreme() && r.chance(10) {
        return r.pick(&[u32::MAX as i64, 65536, 100_000, 1 << 31]);
    }
    if r.chance(95) {
        r.range(0, hi)
    } else {
        r.pick(&[100, 128, 1000])
    }
}

fn text(r: &mut Rng) -> String {
    let pool = ["", "A", "Hi!", "Layer", "héllo", "日本語", "a b\nc", "~", "😀", "x\u{0}y"];
    r.pick(&pool).to_string()
}

fn pts_xy(r: &mut Rng, n: usize, w: i64, h: i64) -> Vec<i32> {
    (0..n).flat_map(|_| [coord(r, w) as i32, coord(r, h) as i32]).collect()
}

fn gen_op(r: &mut Rng, d: Doc, step: usize) -> Op {
    let i = info(d);
    let l = |r: &mut Rng| idx(r, i.layers);
    let f = |r: &mut Rng| idx(r, i.frames);
    let x = |r: &mut Rng| coord(r, i.w);
    let y = |r: &mut Rng| coord(r, i.h);
    let k = pick_kind(r, step);
    let sp = spr(d);
    let layers_where = |pred: &dyn Fn(&pixmith_core::document::Layer) -> bool| -> Vec<i64> {
        sp.layers.iter().enumerate().filter(|(_, l)| pred(l)).map(|(n, _)| n as i64).collect()
    };
    let tilemaps = layers_where(&|l| l.kind == 2);
    let groups = layers_where(&|l| l.is_group());
    let nested = layers_where(&|l| l.child_level > 0);
    let drawable = layers_where(&|l| !l.is_group());
    // A valid pick from `v` most of the time, `fallback` (often out of range) otherwise.
    let from = |r: &mut Rng, v: &[i64], n: i64| if !v.is_empty() && r.chance(88) { r.pick(v) } else { idx(r, n) };
    // An ordered pair inside 0..n most of the time.
    let ordered = |r: &mut Rng, n: i64| -> (i64, i64) {
        if r.chance(90) {
            let (a, b) = (r.range(0, n - 1), r.range(0, n - 1));
            (a.min(b), a.max(b))
        } else {
            (idx(r, n), idx(r, n))
        }
    };
    let pl = |r: &mut Rng| from(r, &drawable, i.layers);
    use K::*;
    match k {
        DrawLine => op(k, &[pl(r), f(r), x(r), y(r), x(r), y(r), color(r), small(r, 8)], "", &[]),
        Fill => op(k, &[pl(r), f(r), x(r), y(r), color(r)], "", &[]),
        FillMode => op(k, &[pl(r), f(r), x(r), y(r), color(r), r.range(0, 1)], "", &[]),
        FillPattern => {
            op(k, &[pl(r), f(r), x(r), y(r), color(r), color(r), small(r, 9), r.range(0, 1), r.range(0, 3)], "", &[])
        }
        Shape => op(
            k,
            &[r.range(0, 3), pl(r), f(r), x(r), y(r), x(r), y(r), color(r), small(r, 8), r.range(0, 31), r.range(0, 2)],
            "",
            &[],
        ),
        Stroke => {
            let n = r.range(0, 8) as usize;
            let pts = pts_xy(r, n, i.w, i.h);
            op(k, &[pl(r), f(r), x(r), y(r), color(r), small(r, 8), r.range(0, 31)], "", &pts)
        }
        Spray => op(
            k,
            &[pl(r), f(r), x(r), y(r), color(r), small(r, 10), r.range(0, 120), r.next() as u32 as i64, r.range(0, 31)],
            "",
            &[],
        ),
        Gradient => op(
            k,
            &[
                pl(r),
                f(r),
                x(r),
                y(r),
                x(r),
                y(r),
                color(r),
                color(r),
                r.range(0, 2),
                r.pick(&[0, 2, 4, 8, 3]),
                r.range(0, 2),
            ],
            "",
            &[],
        ),
        Text => op(k, &[pl(r), f(r), x(r), y(r), small(r, 4), color(r), r.range(0, 31), r.range(0, 2)], &text(r), &[]),
        StampMask => {
            let (w, h) = (r.range(0, 10), r.range(0, 10));
            let mask: Vec<i32> = (0..w * h).map(|_| r.range(0, 255) as i32).collect();
            op(k, &[pl(r), f(r), x(r), y(r), w, h, color(r), r.range(0, 31), r.range(0, 2)], "", &mask)
        }
        Curve => {
            let pts = pts_xy(r, 4, i.w, i.h);
            op(k, &[pl(r), f(r), color(r), small(r, 6), r.range(0, 31), r.range(0, 2)], "", &pts)
        }
        Polygon => {
            let n = r.range(0, 7) as usize;
            let pts = pts_xy(r, n, i.w, i.h);
            op(k, &[pl(r), f(r), color(r), small(r, 6), r.range(0, 31), r.range(0, 2)], "", &pts)
        }
        InkSet => {
            let n = r.range(0, 5) as usize;
            let ramp: Vec<i32> = (0..n).map(|_| color(r) as u32 as i32).collect();
            op(k, &[r.range(0, 6), r.range(0, 1)], "", &ramp)
        }
        BrushCircle => op(k, &[r.range(0, 1)], "", &[]),
        BrushFromSel => op(k, &[pl(r), f(r)], "", &[]),
        BrushReset => op(k, &[], "", &[]),
        PreviewStroke => {
            op(k, &[pl(r), f(r), x(r), y(r), x(r), y(r), color(r), small(r, 6), r.range(0, 31), r.range(0, 3)], "", &[])
        }
        StaleReset => op(k, &[r.range(0, 1 << 20)], "", &[]),
        TxnStrokes => {
            op(k, &[pl(r), f(r), x(r), y(r), x(r), y(r), x(r), y(r), color(r), small(r, 5), r.range(0, 1)], "", &[])
        }
        LongStroke => {
            let pts = pts_xy(r, 30, i.w, i.h);
            op(k, &[pl(r), f(r), x(r), y(r), color(r), small(r, 6), r.range(0, 31)], "", &pts)
        }
        SelRect => op(k, &[x(r), y(r), x(r), y(r), r.range(0, 2)], "", &[]),
        SelAll | SelClear | SelInvert | Crop | FloatCommit | FloatCancel => op(k, &[], "", &[]),
        SelPoly => {
            let n = r.range(0, 8) as usize;
            let pts = pts_xy(r, n, i.w, i.h);
            op(k, &[r.range(0, 2)], "", &pts)
        }
        SelWand => op(k, &[pl(r), f(r), x(r), y(r), r.range(0, 300), r.range(0, 1), r.range(0, 2)], "", &[]),
        SelModify => op(k, &[r.range(0, 3), small(r, 5)], "", &[]),
        MoveSel => op(k, &[pl(r), f(r), coord(r, i.w), coord(r, i.h)], "", &[]),
        Nudge => op(k, &[pl(r), f(r), r.range(-12, 12), r.range(-12, 12)], "", &[]),
        SelDelete => op(k, &[pl(r), f(r)], "", &[]),
        CopyPaste => op(k, &[pl(r), f(r), pl(r), f(r), r.range(-6, 6), r.range(-6, 6)], "", &[]),
        Paste => {
            let (w, h) = (r.range(0, 8), r.range(0, 8));
            let data: Vec<i32> = (0..w * h * 4).map(|_| r.range(0, 255) as i32).collect();
            op(k, &[pl(r), f(r), w, h, x(r), y(r)], "", &data)
        }
        Transform => op(k, &[pl(r), f(r), r.range(0, 4)], "", &[]),
        ResizeCanvas => op(k, &[r.range(0, 80), r.range(0, 70), r.range(-70, 70), r.range(-70, 70)], "", &[]),
        Xf => op(
            k,
            &[
                pl(r),
                f(r),
                r.range(0, 1),
                r.range(0, 4),
                r.range(-300, 300),
                r.range(-300, 300),
                r.range(0, 7),
                r.range(-2000, 2000),
                r.range(0, 2),
            ],
            "",
            &[],
        ),
        AddLayer => op(k, &[idx(r, i.layers)], "", &[]),
        RemoveLayer => op(k, &[l(r)], "", &[]),
        RenameLayer => op(k, &[l(r)], &text(r), &[]),
        SetVisible => op(k, &[l(r), r.range(0, 1)], "", &[]),
        SetOpacity => op(k, &[l(r), r.range(0, 255)], "", &[]),
        LayerMove => op(k, &[l(r), r.range(0, 1)], "", &[]),
        LayerDup | MergeDown | Group => op(k, &[l(r)], "", &[]),
        OutOfGroup => op(k, &[from(r, &nested, i.layers)], "", &[]),
        AddGroup => op(k, &[l(r)], "", &[]),
        IntoGroup => op(k, &[l(r), from(r, &groups, i.layers)], "", &[]),
        LayerProps => op(k, &[l(r), r.range(0, 255), r.range(0, 30), r.range(0, 1)], &text(r), &[]),
        AddFrame => op(k, &[f(r), r.range(0, 1)], "", &[]),
        RemoveFrame => op(k, &[f(r)], "", &[]),
        FrameMove => op(k, &[f(r), f(r)], "", &[]),
        SetDuration => op(k, &[f(r), r.range(0, 70000)], "", &[]),
        CelOpacity => op(k, &[pl(r), f(r), r.range(0, 255)], "", &[]),
        CelLink => {
            let (a, b) = ordered(r, i.frames);
            op(k, &[from(r, &drawable, i.layers), a, b], "", &[])
        }
        CelUnlink => op(k, &[pl(r), f(r)], "", &[]),
        TagAdd => op(k, &[f(r), f(r), r.range(0, 5), small(r, 5), r.next() as u32 as i64], &text(r), &[]),
        TagUpdate => {
            op(k, &[idx(r, i.tags), f(r), f(r), r.range(0, 5), small(r, 5), r.next() as u32 as i64], &text(r), &[])
        }
        TagRemove => op(k, &[idx(r, i.tags)], "", &[]),
        CelsClear | CelsCopy => {
            let (l0, l1) = ordered(r, i.layers);
            let (f0, f1) = ordered(r, i.frames);
            op(k, &[l0, l1, f0, f1], "", &[])
        }
        CelsPaste => op(k, &[l(r), f(r)], "", &[]),
        CelsMove => {
            let (l0, l1) = ordered(r, i.layers);
            let (f0, f1) = ordered(r, i.frames);
            // Mostly shifts that keep the rectangle inside the sprite.
            let dl = if r.chance(90) { r.range(-l0, (i.layers - 1 - l1).max(-l0)) } else { r.range(-2, 2) };
            let df = if r.chance(90) { r.range(-f0, (i.frames - 1 - f1).max(-f0)) } else { r.range(-2, 2) };
            op(k, &[l0, l1, f0, f1, dl, df, r.range(0, 1)], "", &[])
        }
        PaletteAdd => op(k, &[color(r)], "", &[]),
        PaletteRemove => op(k, &[idx(r, i.palette)], "", &[]),
        PaletteSort => op(k, &[r.range(0, 6)], "", &[]),
        PeMove => {
            let n = r.range(0, 4) as usize;
            let v: Vec<i32> = (0..n).map(|_| idx(r, i.palette) as u32 as i32).collect();
            op(k, &[idx(r, i.palette + 1)], "", &v)
        }
        PeInsert => op(k, &[idx(r, i.palette + 1), color(r)], "", &[]),
        PeDuplicate | PeRemove => {
            let n = r.range(0, 4) as usize;
            let v: Vec<i32> = (0..n).map(|_| idx(r, i.palette) as u32 as i32).collect();
            op(k, &[], "", &v)
        }
        PeSetLive => op(k, &[idx(r, i.palette), color(r), r.range(0, 1)], "", &[]),
        PeRamp => op(k, &[idx(r, i.palette), idx(r, i.palette), small(r, 10), r.range(0, 1)], "", &[]),
        PresetLoad => op(k, &[r.range(0, 12)], "", &[]),
        CmPaletteSet => op(k, &[idx(r, i.palette), color(r)], "", &[]),
        CmPaletteFromSprite => op(k, &[r.pick(&[0, 2, 4, 16, 256, 1000])], "", &[]),
        CmConvert => op(k, &[r.range(0, 3), r.pick(&[0, 2, 4, 8, 3])], "", &[]),
        FilterApply => {
            let kind = r.range(0, 12);
            let c = |r: &mut Rng| color(r) as u32 as i32;
            let params: Vec<i32> = match kind {
                0 => vec![c(r), c(r), r.range(0, 255) as i32],
                3 => vec![r.range(-180, 180) as i32, r.range(-100, 100) as i32, r.range(-100, 100) as i32],
                4 => vec![r.range(-100, 100) as i32, r.range(-100, 100) as i32],
                5 => vec![r.range(-255, 255) as i32, r.range(-255, 255) as i32, r.range(-255, 255) as i32],
                6 => vec![c(r), r.range(0, 1) as i32, r.range(0, 1) as i32],
                7 => vec![r.range(-4, 4) as i32, r.range(-4, 4) as i32, c(r)],
                8 => vec![r.range(0, 1) as i32],
                11 => (0..5).flat_map(|_| vec![2, 0, r.range(0, 255) as i32, 255, r.range(0, 255) as i32]).collect(),
                _ if r.chance(10) => (0..r.range(0, 6)).map(|_| r.range(-300, 300) as i32).collect(),
                _ => vec![],
            };
            let params =
                if r.chance(8) { params.iter().take(r.range(0, 3) as usize).copied().collect() } else { params };
            let (rx, ry, rw, rh) = if r.chance(65) { (0, 0, 0, 0) } else { (x(r), y(r), small(r, i.w), small(r, i.h)) };
            op(k, &[kind, r.range(0, 3), pl(r), f(r), rx, ry, rw, rh], "", &params)
        }
        ScaleSprite => op(k, &[r.range(0, 90), r.range(0, 90)], "", &[]),
        AddTilemapLayer => {
            op(k, &[idx(r, i.layers), r.pick(&[0, 1, 2, 3, 4, 8, 16]), r.pick(&[0, 1, 2, 3, 4, 8, 16])], "", &[])
        }
        TileStamp => {
            let tiles = sp.tilesets.iter().map(|t| t.tiles.len() as i64).max().unwrap_or(1) + 1;
            let flags = r.pick(&[0, 0, 0x2000_0000, 0x4000_0000, 0x8000_0000, 0xE000_0000]);
            let id = if r.chance(92) { r.range(0, tiles - 1) } else { u32::MAX as i64 };
            op(k, &[from(r, &tilemaps, i.layers), f(r), x(r), y(r), id | flags], "", &[])
        }
        ConvertToTilemap => {
            op(k, &[l(r), r.pick(&[0, 1, 2, 3, 4, 8, 16]), r.pick(&[0, 1, 2, 3, 4, 8, 16]), r.range(0, 1)], "", &[])
        }
        ConvertToNormal => op(k, &[from(r, &tilemaps, i.layers)], "", &[]),
        DupTile | DelTile => {
            let ts = idx(r, i.tilesets);
            let n = sp.tilesets.get(ts as usize).map_or(3, |t| t.tiles.len() as i64);
            op(k, &[ts, idx(r, n)], "", &[])
        }
        AssignTileset => op(k, &[from(r, &tilemaps, i.layers), idx(r, i.tilesets)], "", &[]),
        TilesetAddTile => op(k, &[idx(r, i.tilesets)], "", &[]),
        TilesetRename => op(k, &[idx(r, i.tilesets)], &text(r), &[]),
        SliceAdd => op(k, &[x(r), y(r), small(r, 20), small(r, 20)], "", &[]),
        SliceBounds => op(k, &[r.range(0, 4), f(r), x(r), y(r), small(r, 20), small(r, 20)], "", &[]),
        SliceProps => {
            let pts: Vec<i32> = if r.chance(50) {
                vec![1, 1, 3, 3, 1, 2]
            } else if r.chance(50) {
                vec![0, 0, 2, 2]
            } else {
                vec![]
            };
            op(k, &[r.range(0, 4), f(r), r.range(0, 1)], &text(r), &pts)
        }
        SliceRemove => op(k, &[r.range(0, 4)], "", &[]),
        UserData => op(k, &[r.range(0, 5), l(r).min(i.layers + 2), f(r), r.range(0, 1), color(r)], &text(r), &[]),
        Undo | Redo | TxnBegin | TxnEnd | BeginStroke | Touch | MarkSaved | XfCommit | XfCancel | SelEnd | MoveEnd => {
            op(k, &[], "", &[])
        }
        XfBegin => op(k, &[pl(r), f(r), r.range(0, 1)], "", &[]),
        XfDrag => op(
            k,
            &[
                r.range(0, 4),
                r.range(-300, 300),
                r.range(-300, 300),
                r.range(0, 7),
                r.range(-2000, 2000),
                r.range(0, 1),
            ],
            "",
            &[],
        ),
        SelBegin => op(k, &[r.range(0, 2)], "", &[]),
        MoveBegin => op(k, &[pl(r), f(r)], "", &[]),
        MoveTo => op(k, &[r.range(-12, 12), r.range(-12, 12)], "", &[]),
        HistoryGoto => op(k, &[r.range(0, 30)], "", &[]),
        Label => op(k, &[], &text(r), &[]),
    }
}

/// `PIXMITH_STRESS_SKIP=ResizeCanvas,Crop` leaves those operation kinds out, to look past a known failure.
fn skipped(k: K) -> bool {
    static SKIP: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let list = SKIP.get_or_init(|| {
        std::env::var("PIXMITH_STRESS_SKIP").map(|v| v.split(',').map(str::to_string).collect()).unwrap_or_default()
    });
    list.iter().any(|n| *n == format!("{k:?}"))
}

fn pick_kind(r: &mut Rng, step: usize) -> K {
    loop {
        let k = pick_kind_unfiltered(r, step);
        if !skipped(k) {
            return k;
        }
    }
}

fn pick_kind_unfiltered(r: &mut Rng, step: usize) -> K {
    use K::*;
    if step < 6 && r.chance(70) {
        return r.pick(&[AddLayer, AddFrame, AddFrame, AddGroup, AddTilemapLayer, DrawLine, Fill, Shape, Group]);
    }
    // Structural and history operations are weighted up so sequences keep changing shape.
    let heavy: &[K] = &[
        AddLayer,
        AddFrame,
        AddGroup,
        AddTilemapLayer,
        RemoveLayer,
        RemoveFrame,
        Undo,
        Redo,
        DrawLine,
        Fill,
        Shape,
        SelRect,
        MoveSel,
        LayerDup,
        MergeDown,
        FrameMove,
        LayerMove,
        TagAdd,
        CelsMove,
        CelLink,
        Xf,
        ResizeCanvas,
        Transform,
        CmConvert,
        PreviewStroke,
        TxnStrokes,
        LongStroke,
        XfBegin,
        XfDrag,
        TileStamp,
        TilesetAddTile,
        DupTile,
        SliceAdd,
        IntoGroup,
        Group,
        ConvertToTilemap,
        CelsPaste,
        CelsCopy,
    ];
    if r.chance(45) {
        r.pick(heavy)
    } else {
        r.pick(ALL)
    }
}

// MARK: Applying

thread_local! {
    static OPEN_TXN: Cell<u32> = const { Cell::new(0) };
}

/// Drawing colors are constrained to the color mode before they reach the core, as the app does.
fn col(d: Doc, v: i64) -> u32 {
    unsafe { colormode::pixmith_cm_snap_color(d, v as u32) }
}

fn u(v: i64) -> u32 {
    v as u32
}

fn i32_(v: i64) -> i32 {
    v as i32
}

thread_local! {}

/// Millisecond clock shared with the watchdog thread of a child process.
static PROGRESS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static WATCHED: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

fn apply(d: Doc, o: &Op) {
    PROGRESS.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut w) = WATCHED.lock() {
        *w = format!("{o:?}");
    }
    let p = &o.p;
    let txt = cs(&o.s);
    let pts = &o.pts;
    unsafe {
        use K::*;
        match o.k {
            DrawLine => {
                pixmith_begin_stroke(d);
                pixmith_draw_line(
                    d,
                    u(p[0]),
                    u(p[1]),
                    i32_(p[2]),
                    i32_(p[3]),
                    i32_(p[4]),
                    i32_(p[5]),
                    col(d, p[6]),
                    u(p[7]),
                );
            }
            Fill => {
                pixmith_begin_stroke(d);
                pixmith_fill(d, u(p[0]), u(p[1]), i32_(p[2]), i32_(p[3]), col(d, p[4]));
            }
            FillMode => {
                pixmith_begin_stroke(d);
                shapes::pixmith_fill_mode(d, u(p[0]), u(p[1]), i32_(p[2]), i32_(p[3]), col(d, p[4]), p[5] != 0);
            }
            FillPattern => {
                pixmith_begin_stroke(d);
                ink::pixmith_fill_pattern(
                    d,
                    u(p[0]),
                    u(p[1]),
                    i32_(p[2]),
                    i32_(p[3]),
                    col(d, p[4]),
                    col(d, p[5]),
                    u(p[6]),
                    p[7] != 0,
                    u(p[8]) & 3,
                );
            }
            Shape => {
                pixmith_begin_stroke(d);
                for restore in [false, p[10] == 1] {
                    shapes::pixmith_shape(
                        d,
                        u(p[0]),
                        u(p[1]),
                        u(p[2]),
                        i32_(p[3]),
                        i32_(p[4]),
                        i32_(p[5]),
                        i32_(p[6]),
                        col(d, p[7]),
                        u(p[8]),
                        u(p[9]),
                        restore,
                    );
                }
            }
            Stroke => {
                pixmith_begin_stroke(d);
                shapes::pixmith_stroke_begin(
                    d,
                    u(p[0]),
                    u(p[1]),
                    i32_(p[2]),
                    i32_(p[3]),
                    col(d, p[4]),
                    u(p[5]),
                    u(p[6]),
                );
                for q in pts.chunks_exact(2) {
                    shapes::pixmith_stroke_to(d, q[0], q[1]);
                }
                shapes::pixmith_stroke_end();
            }
            Spray => {
                pixmith_begin_stroke(d);
                shapes::pixmith_spray(
                    d,
                    u(p[0]),
                    u(p[1]),
                    i32_(p[2]),
                    i32_(p[3]),
                    col(d, p[4]),
                    u(p[5]),
                    u(p[6]),
                    u(p[7]),
                    u(p[8]),
                );
            }
            Gradient => {
                pixmith_begin_stroke(d);
                ink::pixmith_gradient(
                    d,
                    u(p[0]),
                    u(p[1]),
                    i32_(p[2]),
                    i32_(p[3]),
                    i32_(p[4]),
                    i32_(p[5]),
                    col(d, p[6]),
                    col(d, p[7]),
                    u(p[8]),
                    u(p[9]),
                    p[10] == 1,
                );
            }
            Text => {
                pixmith_begin_stroke(d);
                font::pixmith_text(
                    d,
                    u(p[0]),
                    u(p[1]),
                    i32_(p[2]),
                    i32_(p[3]),
                    txt.as_ptr(),
                    u(p[4]),
                    col(d, p[5]),
                    u(p[6]),
                    p[7] == 1,
                );
            }
            StampMask => {
                let mask: Vec<u8> = pts.iter().map(|v| *v as u8).collect();
                pixmith_begin_stroke(d);
                let m = if mask.len() as i64 == p[4] * p[5] { mask.as_ptr() } else { null() };
                font::pixmith_stamp_mask(
                    d,
                    u(p[0]),
                    u(p[1]),
                    i32_(p[2]),
                    i32_(p[3]),
                    u(p[4]),
                    u(p[5]),
                    m,
                    col(d, p[6]),
                    u(p[7]),
                    p[8] == 1,
                );
            }
            Curve => {
                pixmith_begin_stroke(d);
                if pts.len() == 8 {
                    curves::pixmith_curve(d, u(p[0]), u(p[1]), pts.as_ptr(), col(d, p[2]), u(p[3]), u(p[4]), p[5] == 1);
                }
            }
            Polygon => {
                pixmith_begin_stroke(d);
                curves::pixmith_polygon(
                    d,
                    u(p[0]),
                    u(p[1]),
                    pts.as_ptr(),
                    pts.len() / 2,
                    col(d, p[2]),
                    u(p[3]),
                    u(p[4]),
                    p[5] == 1,
                );
            }
            InkSet => {
                let ramp: Vec<u32> = pts.iter().map(|v| *v as u32).collect();
                ink::pixmith_ink_set(u(p[0]), p[1] != 0, ramp.as_ptr(), ramp.len());
            }
            BrushCircle => ink::pixmith_brush_circle(p[0] != 0),
            BrushFromSel => {
                ink::pixmith_brush_from_selection(d, u(p[0]), u(p[1]));
            }
            BrushReset => ink::pixmith_brush_reset(),
            SelRect => {
                selection::pixmith_sel_begin(d, u(p[4]));
                selection::pixmith_sel_rect(d, i32_(p[0]), i32_(p[1]), i32_(p[2]), i32_(p[3]));
                selection::pixmith_sel_end(d);
            }
            SelAll => selection::pixmith_sel_all(d),
            SelClear => selection::pixmith_sel_clear(d),
            SelInvert => selection::pixmith_sel_invert(d),
            SelPoly => {
                selection::pixmith_sel_begin(d, u(p[0]));
                selection::pixmith_sel_poly(d, pts.as_ptr(), pts.len() / 2);
                selection::pixmith_sel_end(d);
            }
            SelWand => {
                selection::pixmith_sel_wand(d, u(p[0]), u(p[1]), i32_(p[2]), i32_(p[3]), u(p[4]), p[5] != 0, u(p[6]));
            }
            SelModify => {
                selection::pixmith_sel_modify(d, u(p[0]), u(p[1]));
            }
            MoveSel => {
                if selection::pixmith_sel_move_begin(d, u(p[0]), u(p[1])) {
                    selection::pixmith_sel_move_to(d, i32_(p[2]) / 2, i32_(p[3]) / 2);
                    selection::pixmith_sel_move_to(d, i32_(p[2]), i32_(p[3]));
                }
                selection::pixmith_sel_move_end(d);
            }
            Nudge => {
                selection::pixmith_sel_nudge(d, u(p[0]), u(p[1]), i32_(p[2]), i32_(p[3]));
            }
            SelDelete => {
                selection::pixmith_sel_delete(d, u(p[0]), u(p[1]));
            }
            CopyPaste => {
                let mut b = [0i32; 4];
                selection::pixmith_sel_copy_bounds(d, b.as_mut_ptr());
                let n = (b[2].max(0) as usize) * (b[3].max(0) as usize) * 4;
                let mut buf = vec![0u8; n];
                if selection::pixmith_sel_copy(d, u(p[0]), u(p[1]), buf.as_mut_ptr(), n) {
                    selection::pixmith_sel_paste(
                        d,
                        u(p[2]),
                        u(p[3]),
                        b[2] as u32,
                        b[3] as u32,
                        buf.as_ptr(),
                        b[0] + i32_(p[4]),
                        b[1] + i32_(p[5]),
                    );
                }
            }
            Paste => {
                let data: Vec<u8> = pts.iter().map(|v| *v as u8).collect();
                selection::pixmith_sel_paste(
                    d,
                    u(p[0]),
                    u(p[1]),
                    u(p[2]),
                    u(p[3]),
                    data.as_ptr(),
                    i32_(p[4]),
                    i32_(p[5]),
                );
            }
            Transform => {
                selection::pixmith_sel_transform(d, u(p[0]), u(p[1]), u(p[2]));
            }
            ResizeCanvas => {
                selection::pixmith_resize_canvas(d, u(p[0]), u(p[1]), i32_(p[2]), i32_(p[3]));
            }
            Crop => {
                selection::pixmith_crop_to_selection(d);
            }
            FloatCommit => selection::pixmith_float_commit(d),
            FloatCancel => {
                selection::pixmith_float_cancel(d);
            }
            Xf => {
                if transform::pixmith_xf_begin(d, u(p[0]), u(p[1]), p[2] != 0) {
                    let (a, b) = (p[4] as f64 / 10.0, p[5] as f64 / 10.0);
                    transform::pixmith_xf_drag_begin(d);
                    match p[3] {
                        0 => transform::pixmith_xf_drag(
                            d,
                            1,
                            (p[6] % 3 - 1) as i32,
                            (p[6] / 3 % 3 - 1) as i32,
                            p[8] == 1,
                            0.0,
                            0.0,
                            a,
                            b,
                        ),
                        1 => transform::pixmith_xf_drag(d, 2, 0, 0, p[8] == 1, 10.0, 10.0, a, b),
                        2 => transform::pixmith_xf_drag(
                            d,
                            3,
                            (p[6] % 3 - 1) as i32,
                            (p[6] / 3 % 3 - 1) as i32,
                            false,
                            0.0,
                            0.0,
                            a,
                            b,
                        ),
                        3 => transform::pixmith_xf_set_field(d, u(p[6]), p[7] as f64 / 10.0),
                        _ => transform::pixmith_xf_drag(d, 0, 0, 0, false, 0.0, 0.0, a, b),
                    }
                    match p[8] {
                        2 => {
                            transform::pixmith_xf_cancel(d);
                        }
                        _ => transform::pixmith_xf_commit(d),
                    }
                }
            }
            AddLayer => {
                pixmith_add_layer(d, u(p[0]));
            }
            RemoveLayer => {
                pixmith_remove_layer(d, u(p[0]));
            }
            RenameLayer => pixmith_rename_layer(d, u(p[0]), txt.as_ptr()),
            SetVisible => pixmith_set_layer_visible(d, u(p[0]), p[1] != 0),
            SetOpacity => pixmith_set_layer_opacity(d, u(p[0]), p[1] as u8),
            LayerMove => {
                layers::pixmith_layer_move(d, u(p[0]), p[1] != 0);
            }
            LayerDup => {
                layers::pixmith_layer_duplicate(d, u(p[0]));
            }
            MergeDown => {
                layers::pixmith_layer_merge_down(d, u(p[0]));
            }
            AddGroup => {
                layers::pixmith_layer_add_group(d, u(p[0]));
            }
            Group => {
                layers::pixmith_layer_group(d, u(p[0]));
            }
            IntoGroup => {
                layers::pixmith_layer_into_group(d, u(p[0]), u(p[1]));
            }
            OutOfGroup => {
                layers::pixmith_layer_out_of_group(d, u(p[0]));
            }
            LayerProps => {
                layers::pixmith_layer_set_props(d, u(p[0]), txt.as_ptr(), p[1] as u8, p[2] as u16, p[3] != 0);
            }
            AddFrame => {
                pixmith_add_frame(d, u(p[0]), p[1] != 0);
            }
            RemoveFrame => {
                pixmith_remove_frame(d, u(p[0]));
            }
            FrameMove => {
                layers::pixmith_frame_move(d, u(p[0]), u(p[1]));
            }
            SetDuration => pixmith_set_frame_duration(d, u(p[0]), p[1] as u16),
            CelOpacity => {
                layers::pixmith_cel_set_opacity(d, u(p[0]), u(p[1]), p[2] as u8);
            }
            CelLink => {
                layers::pixmith_cel_link(d, u(p[0]), u(p[1]), u(p[2]));
            }
            CelUnlink => {
                layers::pixmith_cel_unlink(d, u(p[0]), u(p[1]));
            }
            TagAdd => {
                let t = layers::PixmithTag {
                    from: u(p[0]),
                    to: u(p[1]),
                    direction: p[2] as u8,
                    repeat: p[3] as u16,
                    color: u(p[4]),
                };
                layers::pixmith_tag_add(d, txt.as_ptr(), t);
            }
            TagUpdate => {
                let t = layers::PixmithTag {
                    from: u(p[1]),
                    to: u(p[2]),
                    direction: p[3] as u8,
                    repeat: p[4] as u16,
                    color: u(p[5]),
                };
                layers::pixmith_tag_update(d, u(p[0]), txt.as_ptr(), t);
            }
            TagRemove => {
                layers::pixmith_tag_remove(d, u(p[0]));
            }
            CelsClear => {
                layers::pixmith_cels_clear(d, u(p[0]), u(p[1]), u(p[2]), u(p[3]));
            }
            CelsCopy => {
                layers::pixmith_cels_copy(d, u(p[0]), u(p[1]), u(p[2]), u(p[3]));
            }
            CelsPaste => {
                layers::pixmith_cels_paste(d, u(p[0]), u(p[1]));
            }
            CelsMove => {
                layers::pixmith_cels_move(d, u(p[0]), u(p[1]), u(p[2]), u(p[3]), i32_(p[4]), i32_(p[5]), p[6] != 0);
            }
            PaletteAdd => pixmith_palette_add(d, u(p[0])),
            PaletteRemove => {
                palette_io::pixmith_palette_remove(d, u(p[0]));
            }
            PaletteSort => palette_io::pixmith_palette_sort(d, u(p[0])),
            PeMove => {
                let v: Vec<u32> = pts.iter().map(|v| *v as u32).collect();
                palette_edit::pixmith_pe_move(d, v.as_ptr(), v.len() as u32, u(p[0]));
            }
            PeInsert => {
                palette_edit::pixmith_pe_insert(d, u(p[0]), u(p[1]));
            }
            PeDuplicate => {
                let v: Vec<u32> = pts.iter().map(|v| *v as u32).collect();
                palette_edit::pixmith_pe_duplicate(d, v.as_ptr(), v.len() as u32);
            }
            PeRemove => {
                let v: Vec<u32> = pts.iter().map(|v| *v as u32).collect();
                palette_edit::pixmith_pe_remove(d, v.as_ptr(), v.len() as u32);
            }
            PeSetLive => {
                palette_edit::pixmith_pe_set_live(d, u(p[0]), u(p[1]), p[2] != 0);
            }
            PeRamp => {
                palette_edit::pixmith_pe_ramp(d, u(p[0]), u(p[1]), u(p[2]), p[3] != 0);
            }
            PresetLoad => {
                palette_presets::pixmith_pe_preset_load(d, u(p[0]));
            }
            CmPaletteSet => {
                colormode::pixmith_cm_palette_set(d, u(p[0]), u(p[1]));
            }
            CmPaletteFromSprite => {
                colormode::pixmith_cm_palette_from_sprite(d, u(p[0]));
            }
            CmConvert => {
                colormode::pixmith_cm_convert(d, u(p[0]), u(p[1]));
            }
            FilterApply => {
                filters::pixmith_filter_apply(
                    d,
                    u(p[0]),
                    u(p[1]),
                    u(p[2]),
                    u(p[3]),
                    i32_(p[4]),
                    i32_(p[5]),
                    u(p[6]),
                    u(p[7]),
                    pts.as_ptr(),
                    pts.len(),
                );
            }
            ScaleSprite => {
                filters::pixmith_scale_sprite(d, u(p[0]), u(p[1]));
            }
            AddTilemapLayer => {
                tilemap::pixmith_add_tilemap_layer(d, u(p[0]), u(p[1]), u(p[2]));
            }
            TileStamp => {
                pixmith_begin_stroke(d);
                tilemap::pixmith_tile_stamp(d, u(p[0]), u(p[1]), i32_(p[2]), i32_(p[3]), u(p[4]));
            }
            ConvertToTilemap => {
                tilemap::pixmith_convert_to_tilemap_ex(d, u(p[0]), u(p[1]), u(p[2]), p[3] != 0);
            }
            ConvertToNormal => {
                tilemap::pixmith_convert_to_normal(d, u(p[0]));
            }
            DupTile => {
                tilemap::pixmith_tileset_duplicate_tile(d, u(p[0]), u(p[1]));
            }
            DelTile => {
                tilemap::pixmith_tileset_delete_tile(d, u(p[0]), u(p[1]));
            }
            AssignTileset => {
                tilemap::pixmith_layer_assign_tileset(d, u(p[0]), u(p[1]));
            }
            TilesetAddTile => {
                tilemap::pixmith_tileset_add_tile(d, u(p[0]));
            }
            TilesetRename => {
                tilemap::pixmith_tileset_rename(d, u(p[0]), txt.as_ptr());
            }
            SliceAdd => {
                slices::pixmith_slice_add(d, i32_(p[0]), i32_(p[1]), u(p[2]), u(p[3]));
            }
            SliceBounds => {
                pixmith_begin_stroke(d);
                slices::pixmith_slice_set_bounds(d, u(p[0]), u(p[1]), i32_(p[2]), i32_(p[3]), u(p[4]), u(p[5]));
            }
            SliceProps => {
                let center = if pts.len() >= 4 { pts.as_ptr() } else { null() };
                let pivot = if pts.len() >= 6 { pts[4..].as_ptr() } else { null() };
                let (c, pv) = if p[2] != 0 { (center, pivot) } else { (null(), null()) };
                slices::pixmith_slice_set_props(d, u(p[0]), u(p[1]), txt.as_ptr(), c, pv);
            }
            SliceRemove => {
                slices::pixmith_slice_remove(d, u(p[0]));
            }
            UserData => {
                slices::pixmith_userdata_set(d, u(p[0]), u(p[1]), u(p[2]), txt.as_ptr(), p[3] != 0, u(p[4]));
            }
            XfBegin => {
                transform::pixmith_xf_begin(d, u(p[0]), u(p[1]), p[2] != 0);
            }
            XfDrag => {
                let (a, b) = (p[1] as f64 / 10.0, p[2] as f64 / 10.0);
                transform::pixmith_xf_drag_begin(d);
                match p[0] {
                    0 => transform::pixmith_xf_drag(
                        d,
                        1,
                        (p[3] % 3 - 1) as i32,
                        (p[3] / 3 % 3 - 1) as i32,
                        p[5] == 1,
                        0.0,
                        0.0,
                        a,
                        b,
                    ),
                    1 => transform::pixmith_xf_drag(d, 2, 0, 0, p[5] == 1, 10.0, 10.0, a, b),
                    2 => transform::pixmith_xf_drag(
                        d,
                        3,
                        (p[3] % 3 - 1) as i32,
                        (p[3] / 3 % 3 - 1) as i32,
                        false,
                        0.0,
                        0.0,
                        a,
                        b,
                    ),
                    3 => transform::pixmith_xf_set_field(d, u(p[3]), p[4] as f64 / 10.0),
                    _ => transform::pixmith_xf_drag(d, 0, 0, 0, false, 0.0, 0.0, a, b),
                }
            }
            XfCommit => transform::pixmith_xf_commit(d),
            XfCancel => {
                transform::pixmith_xf_cancel(d);
            }
            SelBegin => selection::pixmith_sel_begin(d, u(p[0])),
            SelEnd => selection::pixmith_sel_end(d),
            MoveBegin => {
                selection::pixmith_sel_move_begin(d, u(p[0]), u(p[1]));
            }
            MoveTo => selection::pixmith_sel_move_to(d, i32_(p[0]), i32_(p[1])),
            MoveEnd => selection::pixmith_sel_move_end(d),
            PreviewStroke => {
                // The explicit-token API: the third and fourth variants end or replay stale sessions.
                let id = shapes::pixmith_preview_begin(d);
                assert!(id != 0, "a preview session needs a nonzero id");
                let c = col(d, p[6]);
                shapes::pixmith_shape(
                    d,
                    0,
                    u(p[0]),
                    u(p[1]),
                    i32_(p[2]),
                    i32_(p[3]),
                    i32_(p[4]),
                    i32_(p[5]),
                    c,
                    u(p[7]),
                    u(p[8]),
                    false,
                );
                assert!(shapes::pixmith_preview_reset(d, id), "the open session must be resettable");
                shapes::pixmith_shape(
                    d,
                    1,
                    u(p[0]),
                    u(p[1]),
                    i32_(p[4]),
                    i32_(p[5]),
                    i32_(p[2]),
                    i32_(p[3]),
                    c,
                    u(p[7]),
                    u(p[8]),
                    false,
                );
                assert!(!shapes::pixmith_preview_reset(d, id + 1), "an unknown token must not reset anything");
                match p[9] {
                    0 => shapes::pixmith_preview_end(d, id),
                    1 => {
                        shapes::pixmith_preview_end(d, id);
                        assert!(!shapes::pixmith_preview_reset(d, id), "an ended session cannot be reset");
                    }
                    2 => {
                        // Another gesture replaces the session; the old token is dead.
                        let next = shapes::pixmith_preview_begin(d);
                        assert!(!shapes::pixmith_preview_reset(d, id));
                        shapes::pixmith_preview_end(d, next);
                    }
                    _ => {}
                }
            }
            StaleReset => {
                assert!(
                    !shapes::pixmith_preview_reset(d, (1 << 40) + p[0] as u64),
                    "a token that was never issued must be ignored"
                );
            }
            TxnStrokes => {
                // Several strokes in one transaction, each previewed with restore: a later stroke's
                // rewind must not take earlier strokes with it.
                history::pixmith_begin_transaction(d);
                let c = col(d, p[8]);
                for (n, end) in [(p[4], p[5]), (p[6], p[7]), (p[2], p[7])].into_iter().enumerate() {
                    pixmith_begin_stroke(d);
                    shapes::pixmith_shape(
                        d,
                        (n % 3) as u32,
                        u(p[0]),
                        u(p[1]),
                        i32_(p[2]),
                        i32_(p[3]),
                        i32_(p[4]),
                        i32_(p[5]),
                        c,
                        u(p[9]),
                        0,
                        false,
                    );
                    shapes::pixmith_shape(
                        d,
                        (n % 3) as u32,
                        u(p[0]),
                        u(p[1]),
                        i32_(p[2]),
                        i32_(p[3]),
                        i32_(end.0),
                        i32_(end.1),
                        c,
                        u(p[9]),
                        0,
                        true,
                    );
                }
                history::pixmith_end_transaction(d);
            }
            LongStroke => {
                pixmith_begin_stroke(d);
                shapes::pixmith_stroke_begin(
                    d,
                    u(p[0]),
                    u(p[1]),
                    i32_(p[2]),
                    i32_(p[3]),
                    col(d, p[4]),
                    u(p[5]),
                    u(p[6]),
                );
                for q in pts.chunks_exact(2) {
                    shapes::pixmith_stroke_to(d, q[0], q[1]);
                }
                shapes::pixmith_stroke_end();
            }
            Undo => {
                pixmith_undo(d);
            }
            Redo => {
                pixmith_redo(d);
            }
            HistoryGoto => {
                history::pixmith_history_goto(d, u(p[0]));
            }
            TxnBegin => {
                history::pixmith_begin_transaction(d);
                OPEN_TXN.with(|t| t.set(t.get() + 1));
            }
            TxnEnd => {
                if OPEN_TXN.with(|t| t.get()) > 0 {
                    history::pixmith_end_transaction(d);
                    OPEN_TXN.with(|t| t.set(t.get() - 1));
                }
            }
            Label => history::pixmith_checkpoint_label(d, txt.as_ptr()),
            BeginStroke => pixmith_begin_stroke(d),
            Touch => history::pixmith_touch(d),
            MarkSaved => history::pixmith_mark_saved(d),
        }
        let _ = null_mut::<u8>();
    }
}

// MARK: Invariants

/// Everything the app's panels read after each edit; none of it may panic or return junk.
fn ui_queries(d: Doc) {
    unsafe {
        let s = spr(d);
        let mut rect = [0i32; 13];
        let free = |p: *mut std::ffi::c_char| {
            if !p.is_null() {
                pixmith_string_free(p);
            }
        };
        for l in 0..s.layers.len() as u32 + 1 {
            free(pixmith_layer_name(d, l));
            pixmith_layer_visible(d, l);
            pixmith_layer_is_group(d, l);
            pixmith_layer_level(d, l);
            pixmith_layer_opacity(d, l);
            layers::pixmith_layer_blend_mode(d, l);
            layers::pixmith_layer_can_merge_down(d, l);
            tilemap::pixmith_layer_is_tilemap(d, l);
            tilemap::pixmith_layer_tileset(d, l);
            for f in 0..s.frame_count() as u32 + 1 {
                layers::pixmith_cel_opacity(d, l, f);
                layers::pixmith_cel_is_linked(d, l, f);
            }
        }
        for f in 0..s.frame_count() as u32 + 1 {
            pixmith_frame_duration(d, f);
        }
        let mut grid = vec![0u8; s.layers.len() * s.frame_count()];
        layers::pixmith_cel_grid(d, grid.as_mut_ptr(), grid.len());
        for t in 0..s.tags.len() as u32 + 1 {
            let mut tag = layers::PixmithTag { from: 0, to: 0, direction: 0, repeat: 0, color: 0 };
            layers::pixmith_tag_get(d, t, &mut tag);
            free(layers::pixmith_tag_name(d, t));
        }
        let mut state = std::mem::MaybeUninit::<layers::PlayState>::uninit();
        layers::pixmith_play_start(state.as_mut_ptr());
        let mut state = state.assume_init();
        let mut frame = 0;
        for _ in 0..s.frame_count() * 2 {
            frame = layers::pixmith_play_next(d, frame, &mut state, true);
            assert!((frame as usize) < s.frame_count(), "pixmith_play_next left the sprite: {frame}");
        }
        for c in 0..s.palette.len() as u32 + 1 {
            pixmith_palette_color(d, c);
        }
        for i in 0..s.extra.slices.len() as u32 + 1 {
            free(slices::pixmith_slice_name(d, i));
            for f in 0..s.frame_count() as u32 {
                slices::pixmith_slice_get(d, i, f, rect.as_mut_ptr());
            }
        }
        for k in 0..5 {
            free(slices::pixmith_userdata_text(d, k, 0, 0));
        }
        for t in 0..s.tilesets.len() as u32 + 1 {
            tilemap::pixmith_tileset_tile_count(d, t);
            tilemap::pixmith_tileset_tile_width(d, t);
            tilemap::pixmith_tileset_tile_height(d, t);
            tilemap::pixmith_tileset_base_index(d, t);
            free(tilemap::pixmith_tileset_name(d, t));
        }
        let mut b = [0i32; 4];
        if selection::pixmith_sel_bounds(d, b.as_mut_ptr()) {
            assert!(b[2] > 0 && b[3] > 0, "selection bounds are empty");
            assert!(selection::pixmith_sel_active(d));
        }
        let n = selection::pixmith_sel_outline(d, std::ptr::null_mut(), 0);
        let mut outline = vec![0i32; n * 4];
        selection::pixmith_sel_outline(d, outline.as_mut_ptr(), n);
        let mut xf = [0f64; 18];
        transform::pixmith_xf_geometry(d, xf.as_mut_ptr());
        transform::pixmith_xf_fields(d, xf.as_mut_ptr());
        colormode::pixmith_cm_get_mode(d);
        colormode::pixmith_cm_transparent_index(d);
        history::pixmith_is_dirty(d);
        let (mut u_, mut r_) = (0u32, 0u32);
        history::pixmith_history_counts(d, &mut u_, &mut r_);
        for i in 0..(u_ + r_).min(40) {
            free(history::pixmith_history_label(d, i));
        }
    }
}

fn check(d: Doc) -> Result<(), String> {
    let s = spr(d);
    let (w, h) = (s.width as usize, s.height as usize);
    if w == 0 || h == 0 || w > 65535 || h > 65535 {
        return Err(format!("bad dimensions {w}x{h}"));
    }
    if s.layers.is_empty() || s.durations.is_empty() {
        return Err("no layers or frames".into());
    }
    if s.cels.len() != s.durations.len() {
        return Err(format!("{} cel rows for {} frames", s.cels.len(), s.durations.len()));
    }
    for (f, row) in s.cels.iter().enumerate() {
        if row.len() != s.layers.len() {
            return Err(format!("frame {f} has {} cels for {} layers", row.len(), s.layers.len()));
        }
        for (l, cel) in row.iter().enumerate() {
            let Some(c) = cel else { continue };
            if c.pixels.len() != w * h * 4 {
                return Err(format!("cel ({f},{l}) has {} bytes for {w}x{h}", c.pixels.len()));
            }
            if s.layers[l].is_group() {
                return Err(format!("cel ({f},{l}) on a group layer"));
            }
            if let Some(ix) = &c.indices {
                if ix.len() != w * h {
                    return Err(format!("cel ({f},{l}) has {} indices for {w}x{h}", ix.len()));
                }
            }
            if let Some(g) = &c.tiles {
                if g.tiles.len() != g.w as usize * g.h as usize {
                    return Err(format!("cel ({f},{l}) grid {}x{} has {} tiles", g.w, g.h, g.tiles.len()));
                }
                if s.layers[l].kind != 2 {
                    return Err(format!("cel ({f},{l}) has tiles on a non-tilemap layer"));
                }
            }
        }
    }
    let mut max_level = 0u16;
    for (i, l) in s.layers.iter().enumerate() {
        if l.child_level > max_level {
            return Err(format!("layer {i} level {} jumps past {}", l.child_level, max_level));
        }
        max_level = if l.is_group() { l.child_level + 1 } else { l.child_level };
        if l.kind == 2 {
            let Some(ts) = s.tilesets.get(l.tileset as usize) else {
                return Err(format!("tilemap layer {i} uses missing tileset {}", l.tileset));
            };
            for f in 0..s.frame_count() {
                if let Some(g) = s.cels[f][i].as_ref().and_then(|c| c.tiles.as_ref()) {
                    if let Some(bad) = g.tiles.iter().find(|v| (**v & 0x1FFF_FFFF) as usize >= ts.tiles.len()) {
                        return Err(format!(
                            "tilemap cel ({f},{i}) references tile {} of {}",
                            bad & 0x1FFF_FFFF,
                            ts.tiles.len()
                        ));
                    }
                }
            }
        }
    }
    for (i, t) in s.tags.iter().enumerate() {
        if t.from > t.to || t.to as usize >= s.frame_count() {
            return Err(format!("tag {i} {}..{} outside {} frames", t.from, t.to, s.frame_count()));
        }
    }
    for (i, ts) in s.tilesets.iter().enumerate() {
        if ts.tile_w == 0 || ts.tile_h == 0 || ts.tiles.is_empty() {
            return Err(format!("tileset {i} is degenerate"));
        }
        if let Some(t) = ts.tiles.iter().position(|t| t.len() != (ts.tile_w * ts.tile_h * 4) as usize) {
            return Err(format!("tileset {i} tile {t} has the wrong size"));
        }
    }
    for (i, sl) in s.extra.slices.iter().enumerate() {
        if sl.keys.windows(2).any(|k| k[0].frame >= k[1].frame) {
            return Err(format!("slice {i} keys are not strictly increasing"));
        }
    }
    if s.palette.len() > 65536 {
        return Err("palette too large".into());
    }
    for f in 0..s.frame_count() {
        if s.render(f).len() != w * h * 4 {
            return Err(format!("render of frame {f} has the wrong size"));
        }
    }
    ui_queries(d);
    let mut buf = vec![0u8; w * h * 4];
    unsafe {
        if !pixmith_render(d, 0, buf.as_mut_ptr(), buf.len()) {
            return Err("pixmith_render failed".into());
        }
        if pixmith_width(d) as usize != w || pixmith_height(d) as usize != h {
            return Err("pixmith_width/height disagree".into());
        }
    }
    Ok(())
}

// MARK: Runner

#[derive(Debug, Clone)]
struct Fail {
    step: usize,
    msg: String,
}

thread_local! {
    /// Per operation kind: how often it ran and how often it changed the document.
    static STATS: RefCell<std::collections::BTreeMap<String, (u32, u32)>> = const { RefCell::new(std::collections::BTreeMap::new()) };
    static IN_RUN: Cell<bool> = const { Cell::new(false) };
    static PANIC: RefCell<String> = const { RefCell::new(String::new()) };
}

fn install_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if IN_RUN.with(|r| r.get()) {
                let loc = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
                let msg = info
                    .payload()
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| info.payload().downcast_ref::<String>().cloned())
                    .unwrap_or_default();
                let text = format!("panic: {msg}{loc}");
                eprintln!("PIXMITH_PANIC: {text}");
                PANIC.with(|p| *p.borrow_mut() = text);
            } else {
                prev(info);
            }
        }));
    });
}

fn selection_of(d: Doc) -> Option<pixmith_core::paint::selection::Selection> {
    let e = ed(d);
    e.sel.active(e.sprite.width, e.sprite.height).cloned()
}

fn undo_len(d: Doc) -> usize {
    ed(d).history_len().0
}

fn sprite_diff(a: &Sprite, b: &Sprite) -> String {
    let mut out = Vec::new();
    if (a.width, a.height) != (b.width, b.height) {
        out.push(format!("size {}x{} vs {}x{}", a.width, a.height, b.width, b.height));
    }
    if a.layers != b.layers {
        out.push(format!(
            "layers {:?} vs {:?}",
            a.layers.iter().map(|l| &l.name).collect::<Vec<_>>(),
            b.layers.iter().map(|l| &l.name).collect::<Vec<_>>()
        ));
    }
    if a.durations != b.durations {
        out.push("durations".into());
    }
    if a.cels != b.cels {
        let (fa, fb) = (a.cels.len(), b.cels.len());
        let mut at = None;
        for f in 0..fa.min(fb) {
            for l in 0..a.cels[f].len().min(b.cels[f].len()) {
                if a.cels[f][l] != b.cels[f][l] && at.is_none() {
                    at = Some((f, l));
                }
            }
        }
        out.push(format!("cels (frames {fa} vs {fb}, first difference {at:?})"));
    }
    if a.tags != b.tags {
        out.push(format!("tags {:?} vs {:?}", a.tags, b.tags));
    }
    if a.palette != b.palette {
        out.push(format!("palette ({} vs {} entries)", a.palette.len(), b.palette.len()));
    }
    if a.tilesets != b.tilesets {
        out.push("tilesets".into());
    }
    if a.color_mode != b.color_mode || a.transparent_index != b.transparent_index {
        out.push(format!(
            "color mode {:?}/{} vs {:?}/{}",
            a.color_mode, a.transparent_index, b.color_mode, b.transparent_index
        ));
    }
    if a.extra != b.extra {
        out.push("slices/user data".into());
    }
    if out.is_empty() {
        out.push("(unknown field)".into());
    }
    out.join("; ")
}

/// Saves and re-reads. The file format drops empty cels and unused tilesets, so the comparison
/// goes through the tilemap normalisation and ignores empty cels.
fn check_round_trip(s: &Sprite) -> Result<(), String> {
    let norm = pixmith_core::document::tilemap::normalized(s).into_owned();
    let bytes = pixmith_core::formats::ase::write(s);
    let back = pixmith_core::formats::ase::read(&bytes).map_err(|e| format!("re-reading a saved file failed: {e}"))?;
    let strip = |sp: &Sprite| -> Vec<Vec<Option<Vec<u8>>>> {
        sp.cels
            .iter()
            .map(|row| {
                row.iter()
                    .map(|c| {
                        c.as_ref().filter(|c| c.pixels.chunks_exact(4).any(|p| p[3] != 0)).map(|c| {
                            // The file trims cels to their visible bounds, so invisible pixels lose their color.
                            c.pixels
                                .chunks_exact(4)
                                .flat_map(|p| if p[3] == 0 { [0; 4] } else { [p[0], p[1], p[2], p[3]] })
                                .collect()
                        })
                    })
                    .collect()
            })
            .collect()
    };
    if (norm.width, norm.height) != (back.width, back.height) {
        return Err(format!("round trip changed the size: {}", sprite_diff(&norm, &back)));
    }
    if norm.layers != back.layers {
        let only_groups = !norm.layers.iter().any(|l| !l.is_group());
        if !only_groups {
            return Err(format!("round trip changed the layers: {}", sprite_diff(&norm, &back)));
        }
    }
    if norm.durations != back.durations || norm.tags != back.tags || norm.extra != back.extra {
        return Err(format!("round trip changed frames, tags or slices: {}", sprite_diff(&norm, &back)));
    }
    if norm.color_mode != back.color_mode {
        return Err(format!("round trip changed the color mode: {}", sprite_diff(&norm, &back)));
    }
    if norm.palette != back.palette {
        return Err(format!("round trip changed the palette: {}", sprite_diff(&norm, &back)));
    }
    // Only RGBA files keep arbitrary colors; grayscale keeps the alpha plane, and indexed files
    // snap to the palette (the app constrains drawing colors itself).
    use pixmith_core::document::color_mode::ColorMode;
    if norm.color_mode == ColorMode::Rgba {
        if strip(&norm) != strip(&back) {
            return Err(format!("round trip changed cel pixels: {}", sprite_diff(&norm, &back)));
        }
        for f in 0..norm.frame_count() {
            if norm.render(f) != back.render(f) {
                return Err(format!("round trip changed the render of frame {f}"));
            }
        }
    } else if norm.color_mode == ColorMode::Indexed {
        // Tools such as paste may leave off-palette colors in memory; the file must hold only palette colors.
        for (f, row) in back.cels.iter().enumerate() {
            for (l, cel) in row.iter().enumerate() {
                let Some(c) = cel else { continue };
                if let Some(p) =
                    c.pixels.chunks_exact(4).find(|p| p[3] != 0 && !back.palette.iter().any(|e| e[..3] == p[..3]))
                {
                    return Err(format!("indexed file holds a color outside its palette: cel ({f},{l}) {p:?}"));
                }
            }
        }
    } else {
        let alpha = |sp: &Sprite| -> Vec<Vec<Option<Vec<u8>>>> {
            strip(sp)
                .into_iter()
                .map(|row| row.into_iter().map(|c| c.map(|p| p.chunks_exact(4).map(|q| q[3]).collect())).collect())
                .collect()
        };
        if alpha(&norm) != alpha(&back) {
            return Err(format!("round trip changed the alpha of grayscale cels: {}", sprite_diff(&norm, &back)));
        }
    }
    // Saving what was read must be stable.
    let again = pixmith_core::formats::ase::write(&back);
    let third = pixmith_core::formats::ase::read(&again).map_err(|e| format!("second re-read failed: {e}"))?;
    if strip(&third) != strip(&back)
        || (third.layers != back.layers && back.layers.iter().any(|l| !l.is_group()))
        || third.durations != back.durations
        || third.tags != back.tags
        || third.palette != back.palette
        || third.extra != back.extra
    {
        return Err(format!("second round trip is not stable: {}", sprite_diff(&back, &third)));
    }
    Ok(())
}

fn run_inner(seq: &Seq) -> Result<(), Fail> {
    let fail = |step: usize, msg: String| Fail { step, msg };
    OPEN_TXN.with(|t| t.set(0));
    unsafe {
        shapes::pixmith_stroke_end();
        ink::pixmith_ink_set(0, false, null(), 0);
        ink::pixmith_brush_reset();
        ink::pixmith_brush_circle(false);
    }
    let d = pixmith_new(seq.w, seq.h);
    let initial = spr(d).clone();
    let mut raw = false;
    let mut saved = false;
    let mut deep = false;
    // Between a gesture's begin and end (live selection, move, transform) edits belong to its undo step.
    let (mut move_open, mut sel_open) = (false, false);
    // Undo depth at the last save, while the saved state is still reachable by undoing.
    let mut anchor: Option<usize> = None;
    let result = (|| -> Result<(), Fail> {
        for (n, o) in seq.ops.iter().enumerate() {
            let before = (spr(d).clone(), selection_of(d), undo_len(d));
            let float_before = unsafe { selection::pixmith_float_active(d) || transform::pixmith_xf_active(d) }
                || move_open
                || sel_open;
            apply(d, o);
            // Edits to linked cels reach their siblings lazily, on the next call into the core.
            ed(d).sprite.flush_links();
            let pos = format!("after #{n} {o:?}");
            check(d).map_err(|m| fail(n, format!("invariant: {m} {pos}")))?;
            match o.k {
                K::MoveBegin => move_open = true,
                K::SelBegin => sel_open = true,
                K::MoveEnd => move_open = false,
                K::SelEnd => sel_open = false,
                // Navigating that actually moves through history abandons the gesture.
                K::Undo | K::Redo | K::HistoryGoto if spr(d) != &before.0 || undo_len(d) != before.2 => {
                    (move_open, sel_open) = (false, false)
                }
                _ => {}
            }
            if o.k == K::MarkSaved {
                anchor = Some(undo_len(d));
            } else if is_raw(o.k) || OPEN_TXN.with(|t| t.get()) > 0 || anchor.is_some_and(|a| undo_len(d) < a) {
                anchor = None;
            }
            raw |= is_raw(o.k);
            saved |= o.k == K::MarkSaved;
            let len = undo_len(d);
            deep |= len >= 90;
            let dirty = unsafe { history::pixmith_is_dirty(d) };
            let changed_any = spr(d) != &before.0 || selection_of(d) != before.1;
            STATS.with(|st| {
                let e = *st.borrow_mut().entry(format!("{:?}", o.k)).or_insert((0, 0));
                st.borrow_mut().insert(format!("{:?}", o.k), (e.0 + 1, e.1 + changed_any as u32));
            });
            let navigates = matches!(o.k, K::Undo | K::Redo | K::HistoryGoto | K::MarkSaved);
            // A floating paste moves with its own undo step; cancelling a preview drops its step.
            if !navigates && !is_raw(o.k) && !float_before && OPEN_TXN.with(|t| t.get()) == 0 {
                let changed = spr(d) != &before.0 || selection_of(d) != before.1;
                if changed && len == before.2 {
                    return Err(fail(
                        n,
                        format!("changed the document without an undo step {pos}: {}", sprite_diff(&before.0, spr(d))),
                    ));
                }
            }
            if len > before.2 && !dirty && !navigates {
                return Err(fail(n, format!("document is clean after an undoable edit {pos}")));
            }
            if o.k == K::MarkSaved && dirty {
                return Err(fail(n, format!("document is dirty right after saving {pos}")));
            }
        }
        while OPEN_TXN.with(|t| t.get()) > 0 {
            unsafe { history::pixmith_end_transaction(d) };
            OPEN_TXN.with(|t| t.set(t.get() - 1));
        }
        let end = seq.ops.len();
        check(d).map_err(|m| fail(end, format!("invariant at end: {m}")))?;
        check_round_trip(spr(d)).map_err(|m| fail(end, m))?;

        // Undo everything, then redo everything.
        let pre = (spr(d).clone(), selection_of(d));
        let dirty_pre = unsafe { history::pixmith_is_dirty(d) };
        let mut k = 0;
        while unsafe { pixmith_undo(d) } {
            k += 1;
            check(d).map_err(|m| fail(end, format!("invariant after undo {k}: {m}")))?;
            if k > 5000 {
                return Err(fail(end, "undo never ends".into()));
            }
        }
        if !raw && !deep {
            if spr(d) != &initial {
                return Err(fail(
                    end,
                    format!("undoing everything does not restore the start: {}", sprite_diff(&initial, spr(d))),
                ));
            }
            if selection_of(d).is_some() {
                return Err(fail(end, "undoing everything leaves a selection".into()));
            }
            if !saved && unsafe { history::pixmith_is_dirty(d) } {
                return Err(fail(end, "document is dirty after undoing everything".into()));
            }
        }
        for j in 0..k {
            if !unsafe { pixmith_redo(d) } {
                return Err(fail(end, format!("redo {j} of {k} failed")));
            }
            check(d).map_err(|m| fail(end, format!("invariant after redo {j}: {m}")))?;
        }
        if spr(d) != &pre.0 {
            return Err(fail(
                end,
                format!("redoing everything does not restore the end: {}", sprite_diff(&pre.0, spr(d))),
            ));
        }
        if selection_of(d) != pre.1 {
            return Err(fail(end, "redoing everything restores a different selection".into()));
        }
        if unsafe { history::pixmith_is_dirty(d) } != dirty_pre {
            return Err(fail(end, "dirty state differs after undo + redo".into()));
        }
        // Going back to the depth of the last save must make the document clean again.
        if let Some(a) = anchor.filter(|_| !deep) {
            if unsafe { history::pixmith_history_goto(d, a as u32) } && unsafe { history::pixmith_is_dirty(d) } {
                return Err(fail(end, format!("document is dirty after returning to the saved state (depth {a})")));
            }
        }
        Ok(())
    })();
    unsafe {
        shapes::pixmith_stroke_end();
        pixmith_free(d);
    }
    result
}

fn run(seq: &Seq) -> Result<(), Fail> {
    install_hook();
    IN_RUN.with(|r| r.set(true));
    let out = catch_unwind(AssertUnwindSafe(|| run_inner(seq)));
    IN_RUN.with(|r| r.set(false));
    match out {
        Ok(r) => r,
        Err(_) => Err(Fail { step: usize::MAX, msg: PANIC.with(|p| std::mem::take(&mut *p.borrow_mut())) }),
    }
}

// MARK: Serialization (sequences travel between the test process and its children as text)

fn ser_op(o: &Op) -> String {
    let p: Vec<String> = o.p.iter().map(|v| v.to_string()).collect();
    let s: String = o.s.bytes().map(|b| format!("{b:02x}")).collect();
    let pts: Vec<String> = o.pts.iter().map(|v| v.to_string()).collect();
    format!("{:?}|{}|{}|{}", o.k, p.join(" "), s, pts.join(" "))
}

fn de_op(line: &str) -> Option<Op> {
    let mut it = line.splitn(4, '|');
    let name = it.next()?;
    let k = *ALL.iter().find(|k| format!("{k:?}") == name)?;
    let mut p = [0i64; 12];
    for (i, v) in it.next()?.split_whitespace().enumerate().take(12) {
        p[i] = v.parse().ok()?;
    }
    let hex = it.next()?;
    let bytes: Vec<u8> =
        (0..hex.len() / 2).filter_map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()).collect();
    let pts = it.next()?.split_whitespace().filter_map(|v| v.parse().ok()).collect();
    Some(Op { k, p, s: String::from_utf8_lossy(&bytes).into_owned(), pts })
}

fn ser_seq(s: &Seq) -> String {
    let mut out = format!("{} {}\n", s.w, s.h);
    for o in &s.ops {
        out.push_str(&ser_op(o));
        out.push('\n');
    }
    out
}

fn de_seq(text: &str) -> Option<Seq> {
    let mut lines = text.lines();
    let mut dims = lines.next()?.split_whitespace();
    let (w, h) = (dims.next()?.parse().ok()?, dims.next()?.parse().ok()?);
    Some(Seq { w, h, ops: lines.filter(|l| !l.is_empty()).map(de_op).collect::<Option<Vec<_>>>()? })
}

// MARK: Generation

fn gen_seq(seed: u64, sink: &mut dyn FnMut(&Seq, &Op)) -> Seq {
    let mut r = Rng(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ 0xA5A5);
    let (w, h) = match r.below(10) {
        0 => (1, 1),
        1 => (r.range(1, 3) as u32, r.range(1, 3) as u32),
        2..=7 => (r.range(4, 24) as u32, r.range(4, 24) as u32),
        _ => (r.range(16, 64) as u32, r.range(16, 48) as u32),
    };
    // Generation reads counts from a live document, so each op is applied to a scratch document
    // as it is made; the real run replays the recorded list.
    reset_globals();
    let d = pixmith_new(w, h);
    let n = r.range(15, 60);
    let mut seq = Seq { w, h, ops: Vec::new() };
    for _ in 0..n {
        let o = gen_op(&mut r, d, seq.ops.len());
        sink(&seq, &o);
        apply(d, &o);
        seq.ops.push(o);
    }
    reset_globals();
    unsafe { pixmith_free(d) };
    seq
}

fn reset_globals() {
    OPEN_TXN.with(|t| t.set(0));
    unsafe {
        shapes::pixmith_stroke_end();
        ink::pixmith_ink_set(0, false, null(), 0);
        ink::pixmith_brush_reset();
        ink::pixmith_brush_circle(false);
    }
}

// MARK: Child processes
//
// The C ABI functions are `extern "C"`, so a panic inside one aborts the process. Sequences
// therefore run in child processes (this same test binary, entered through `child_entry`).

fn say(line: &str) {
    use std::io::Write;
    let out = std::io::stdout();
    let mut out = out.lock();
    let _ = writeln!(out, "@@{line}");
    let _ = out.flush();
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n")
}

#[test]
fn child_entry() {
    let Ok(mode) = std::env::var("PIXMITH_STRESS_CHILD") else { return };
    install_hook();
    PROGRESS.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let idle = now_ms().saturating_sub(PROGRESS.load(std::sync::atomic::Ordering::Relaxed));
        if idle > 15000 {
            let op = WATCHED.lock().map(|w| w.clone()).unwrap_or_default();
            // The kind name identifies the hang for shrinking; parameters are left out.
            let kind = op.split(',').next().unwrap_or("").to_string();
            eprintln!("PIXMITH_PANIC: hang: no progress for 15 s in {kind}");
            std::process::abort();
        }
    });
    if let Some(range) = mode.strip_prefix("seeds:") {
        let (first, count) =
            range.split_once(':').map(|(a, b)| (a.parse::<u64>().unwrap(), b.parse::<u64>().unwrap())).unwrap();
        for seed in first..first + count {
            say(&format!("SEED {seed}"));
            IN_RUN.with(|r| r.set(true));
            let seq = gen_seq(seed, &mut |s, o| {
                if s.ops.is_empty() {
                    say(&format!("DIMS {} {}", s.w, s.h));
                }
                say(&format!("OP {}", ser_op(o)));
            });
            IN_RUN.with(|r| r.set(false));
            if let Err(f) = run(&seq) {
                say(&format!("FAIL {} {}", f.step, escape(&f.msg)));
            }
        }
        say("DONE");
    } else if let Some(path) = mode.strip_prefix("seq:") {
        let seq = de_seq(&std::fs::read_to_string(path).unwrap()).unwrap();
        match run(&seq) {
            Ok(()) => say("OK"),
            Err(f) => say(&format!("FAIL {} {}", f.step, escape(&f.msg))),
        }
        say("DONE");
    }
}

struct ChildResult {
    lines: Vec<String>,
    done: bool,
    panic: Option<String>,
    status: String,
}

fn spawn_child(mode: &str) -> ChildResult {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["child_entry", "--exact", "--nocapture", "--test-threads=1"])
        .env("PIXMITH_STRESS_CHILD", mode)
        .output()
        .expect("spawning the stress child");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<String> = stdout.lines().filter_map(|l| l.find("@@").map(|i| l[i + 2..].to_string())).collect();
    let panic = stderr.lines().find_map(|l| l.find("PIXMITH_PANIC: ").map(|i| l[i + 12..].to_string()));
    ChildResult { done: lines.last().is_some_and(|l| l == "DONE"), lines, panic, status: format!("{}", out.status) }
}

fn abort_message(c: &ChildResult) -> String {
    c.panic.clone().unwrap_or_else(|| format!("the process died ({})", c.status))
}

/// Runs a sequence in a child; `None` means it passed.
fn run_in_child(seq: &Seq) -> Option<Fail> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "pixmith-stress-{}-{}.txt",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, ser_seq(seq)).unwrap();
    let c = spawn_child(&format!("seq:{}", path.display()));
    let _ = std::fs::remove_file(&path);
    if !c.done {
        return Some(Fail { step: usize::MAX, msg: abort_message(&c) });
    }
    c.lines.iter().find_map(|l| l.strip_prefix("FAIL ")).map(|rest| {
        let (step, msg) = rest.split_once(' ').unwrap_or(("0", rest));
        Fail { step: step.parse().unwrap_or(usize::MAX), msg: msg.replace("\\n", "\n") }
    })
}

/// Runs seeds in children, restarting after aborts. Returns failing (seed, sequence, failure).
fn run_seeds_in_children(first: u64, count: u64, max_failures: usize) -> Vec<(u64, Seq, Fail)> {
    let mut failures = Vec::new();
    let mut next = first;
    let end = first + count;
    while next < end && failures.len() < max_failures {
        let c = spawn_child(&format!("seeds:{}:{}", next, end - next));
        let mut cur: Option<(u64, Seq)> = None;
        for l in &c.lines {
            if let Some(s) = l.strip_prefix("SEED ") {
                cur = Some((s.parse().unwrap(), Seq { w: 1, h: 1, ops: Vec::new() }));
            } else if let Some(d) = l.strip_prefix("DIMS ") {
                let mut it = d.split_whitespace();
                if let Some((_, s)) = cur.as_mut() {
                    s.w = it.next().unwrap().parse().unwrap();
                    s.h = it.next().unwrap().parse().unwrap();
                }
            } else if let Some(o) = l.strip_prefix("OP ") {
                if let (Some((_, s)), Some(o)) = (cur.as_mut(), de_op(o)) {
                    s.ops.push(o);
                }
            } else if let Some(rest) = l.strip_prefix("FAIL ") {
                let (step, msg) = rest.split_once(' ').unwrap_or(("0", rest));
                if let Some((seed, s)) = cur.clone() {
                    failures.push((
                        seed,
                        s,
                        Fail { step: step.parse().unwrap_or(usize::MAX), msg: msg.replace("\\n", "\n") },
                    ));
                }
            }
        }
        if c.done {
            break;
        }
        // The child died mid-sequence: that sequence is the culprit; carry on after it.
        let Some((seed, s)) = cur else { panic!("stress child died before starting: {}", abort_message(&c)) };
        failures.push((seed, s, Fail { step: usize::MAX, msg: abort_message(&c) }));
        next = seed + 1;
    }
    failures
}

// MARK: Shrinking and reporting

fn signature(f: &Fail) -> String {
    let head = f.msg.split(" after #").next().unwrap_or(&f.msg);
    head.chars().filter(|c| !c.is_ascii_digit()).collect()
}

fn shrink(seq: &Seq, fail: &Fail) -> Seq {
    let sig = signature(fail);
    let still_fails = |s: &Seq| run_in_child(s).is_some_and(|f| signature(&f) == sig);
    let mut cur = seq.clone();
    if fail.step < cur.ops.len() {
        let mut cut = cur.clone();
        cut.ops.truncate(fail.step + 1);
        if still_fails(&cut) {
            cur = cut;
        }
    }
    let mut chunk = (cur.ops.len() / 2).max(1);
    loop {
        let mut i = 0;
        while i < cur.ops.len() {
            let mut t = cur.clone();
            let end = (i + chunk).min(t.ops.len());
            t.ops.drain(i..end);
            if still_fails(&t) {
                cur = t;
            } else {
                i += chunk;
            }
        }
        if chunk == 1 {
            break;
        }
        chunk = (chunk / 2).max(1);
    }
    for (w, h) in [(1, 1), (2, 2), (4, 4), (8, 8), (16, 16), (32, 24)] {
        let mut t = cur.clone();
        (t.w, t.h) = (w, h);
        if (w, h) != (cur.w, cur.h) && still_fails(&t) {
            cur = t;
            break;
        }
    }
    cur
}

fn report(seed: u64, seq: &Seq, fail: &Fail) -> String {
    let small = shrink(seq, fail);
    let again = run_in_child(&small).unwrap_or_else(|| fail.clone());
    let mut out =
        format!("seed {seed}: {}\nminimal sequence ({}x{}, {} ops):\n", again.msg, small.w, small.h, small.ops.len());
    for o in &small.ops {
        out.push_str(&format!("    {o:?},\n"));
    }
    out
}

fn run_seeds(first: u64, count: u64) {
    let found = run_seeds_in_children(first, count, 5);
    let reports: Vec<String> = found.iter().map(|(seed, seq, f)| report(*seed, seq, f)).collect();
    assert!(reports.is_empty(), "{} failing sequence(s):\n{}", reports.len(), reports.join("\n"));
}

#[test]
fn random_sequences_keep_the_document_consistent() {
    run_seeds(1, 200);
}

#[test]
#[ignore = "long: set PIXMITH_STRESS_SEQS / PIXMITH_STRESS_SEED"]
fn random_sequences_long() {
    let n = std::env::var("PIXMITH_STRESS_SEQS").ok().and_then(|v| v.parse().ok()).unwrap_or(20000);
    let first = std::env::var("PIXMITH_STRESS_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(1000);
    run_seeds(first, n);
}

// MARK: Regression tests (minimal sequences found by the random runs)

fn regress(w: u32, h: u32, ops: &[Op]) {
    if let Err(f) = run(&Seq { w, h, ops: ops.to_vec() }) {
        panic!("{} (step {})", f.msg, f.step);
    }
}

#[test]
fn regress_cels_move_with_reversed_range_changes_nothing() {
    regress(
        1,
        1,
        &[
            op(K::ResizeCanvas, &[79, 28, 6, -39], "", &[]),
            op(K::AddFrame, &[], "", &[]),
            op(K::SelAll, &[], "", &[]),
            op(K::Fill, &[0, 0, 38, 21, 65535], "", &[]),
            op(K::CelLink, &[0, 0, 1], "", &[]),
            op(K::AddFrame, &[], "", &[]),
            op(K::Fill, &[0, 2, 32, 11, 1246929844], "", &[]),
            op(K::CelsMove, &[0, 1, 2, 0, 0, 3], "", &[]),
        ],
    );
}

#[test]
#[ignore = "prints which operations actually change documents"]
fn coverage_report() {
    for seed in 1..=200 {
        let seq = gen_seq(seed, &mut |_, _| {});
        let _ = run(&seq);
    }
    STATS.with(|st| {
        for (k, (n, c)) in st.borrow().iter() {
            println!("{k:22} ran {n:5} changed {c:5} ({}%)", c * 100 / n.max(&1));
        }
    });
}

#[test]
fn regress_pasted_tilemap_cels_never_reference_missing_tiles() {
    // Copy a tilemap cel, undo the edit that created its tile, paste: the clipboard still names the tile.
    regress(
        8,
        8,
        &[
            op(K::AddTilemapLayer, &[0, 4, 4], "", &[]),
            op(K::DrawLine, &[1, 0, 1, 1, 2, 2, 0xFF0000FF, 1], "", &[]),
            op(K::CelsCopy, &[1, 1, 0, 0], "", &[]),
            op(K::Undo, &[], "", &[]),
            op(K::CelsPaste, &[1, 0], "", &[]),
        ],
    );
}

#[test]
fn regress_absurd_palette_ramp_length_is_capped() {
    regress(4, 4, &[op(K::PeRamp, &[1, 2, 4294967295, 1], "", &[]), op(K::PeRamp, &[3, 1, 2147483648], "", &[])]);
}

#[test]
fn regress_out_of_group_with_a_bad_index_is_ignored() {
    regress(1, 1, &[op(K::OutOfGroup, &[4], "", &[]), op(K::OutOfGroup, &[4294967295], "", &[])]);
}

#[test]
fn regress_curve_with_huge_control_points_does_not_overflow() {
    regress(1, 1, &[op(K::Curve, &[0, 0, 65535, 3, 24, 1], "", &[11, -41, 16, 2, -1, 6, 70000, -32769])]);
}

#[test]
fn regress_absurd_coordinates_stay_bounded() {
    let big = [i32::MIN, i32::MAX, -1_000_000_000, 1_000_000_000];
    for (n, &v) in big.iter().enumerate() {
        let v = v as i64;
        regress(
            8,
            8,
            &[
                op(K::Shape, &[(n % 3) as i64, 0, 0, 1, 2, v, -v.abs().min(3), 0xFF0000FF, 3, 8], "", &[]),
                op(K::Shape, &[2, 0, 0, v, v, 3, 3, 0x00FF00FF, 1, 8], "", &[]),
                op(K::Spray, &[0, 0, v, 2, 0x0000FFFF, 9, 100, 7, 0], "", &[]),
                op(K::DrawLine, &[0, 0, v, v, 0, 0, 0xFFFFFFFF, 1], "", &[]),
                op(K::Polygon, &[0, 0, 0xFF00FFFF, 2, 8], "", &[1, 1, v as i32, 5, 3, v as i32]),
                op(K::SelPoly, &[0], "", &[0, 0, v as i32, 3, 2, v as i32]),
                op(K::SelRect, &[v, v, 3, 3, 1], "", &[]),
                op(K::Stroke, &[0, 0, 1, 1, 0xFF0000FF, 2, 0], "", &[v as i32, 2, 3, v as i32]),
                op(K::Text, &[0, 0, v, 0, 1, 0xFFFFFFFF], "Hi", &[]),
                op(K::StampMask, &[0, 0, v, v, 1, 1, 0xFF0000FF], "", &[255]),
                op(K::Paste, &[0, 0, 1, 1, v, v], "", &[1, 2, 3, 4]),
                op(K::SelAll, &[], "", &[]),
                op(K::MoveSel, &[0, 0, v, v], "", &[]),
                op(K::Nudge, &[0, 0, v, v], "", &[]),
                op(K::ResizeCanvas, &[9, 9, v, v], "", &[]),
            ],
        );
    }
}

#[test]
fn regress_edits_after_undo_inside_a_transaction_still_get_an_undo_step() {
    // The transaction took its checkpoint before the history moved; the Group that follows needs its own.
    regress(
        8,
        8,
        &[
            op(K::TxnBegin, &[], "", &[]),
            op(K::SelRect, &[5, 2, 3, -8, 1], "", &[]),
            op(K::HistoryGoto, &[], "", &[]),
            op(K::Group, &[], "", &[]),
        ],
    );
}

#[test]
fn regress_indexed_files_are_stable_across_saves() {
    regress(
        16,
        16,
        &[
            op(K::DrawLine, &[0, 0, 18, 5, 9, 2, 4278190335, 2], "", &[]),
            op(K::CmPaletteFromSprite, &[16], "", &[]),
            op(K::ResizeCanvas, &[66, 52, 61, -57], "", &[]),
            op(K::CmConvert, &[2, 2], "", &[]),
            op(K::PaletteRemove, &[], "", &[]),
            op(
                K::FilterApply,
                &[11, 1, 0, 3, 23, 51, 44, 7],
                "",
                &[2, 0, 7, 255, 57, 2, 0, 12, 255, 81, 2, 0, 243, 255, 51, 2, 0, 90, 255, 230, 2, 0, 214, 255, 123],
            ),
        ],
    );
}

#[test]
fn regress_a_move_in_progress_survives_a_canvas_change() {
    // The drag saved the 1x1 cel; scaling the sprite must end the drag instead of corrupting the cel.
    regress(
        1,
        1,
        &[op(K::MoveBegin, &[], "", &[]), op(K::ScaleSprite, &[39, 83], "", &[]), op(K::MoveTo, &[-10, 7], "", &[])],
    );
    // The same inside a transaction, where the scale does not take a checkpoint of its own.
    regress(
        1,
        1,
        &[
            op(K::TxnBegin, &[], "", &[]),
            op(K::MoveBegin, &[], "", &[]),
            op(K::ScaleSprite, &[39, 83], "", &[]),
            op(K::MoveTo, &[-10, 7], "", &[]),
            op(K::TxnEnd, &[], "", &[]),
        ],
    );
}

#[test]
fn regress_previews_survive_a_canvas_change_inside_a_transaction() {
    for open in [
        op(K::XfBegin, &[0], "", &[]),
        op(K::Paste, &[0, 0, 2, 2, 1, 1], "", &[9, 9, 9, 255, 9, 9, 9, 255, 9, 9, 9, 255, 9, 9, 9, 255]),
    ] {
        regress(
            4,
            4,
            &[
                op(K::DrawLine, &[0, 0, 1, 1, 2, 2, 0xFF0000FF, 1], "", &[]),
                op(K::TxnBegin, &[], "", &[]),
                open.clone(),
                op(K::ScaleSprite, &[9, 7], "", &[]),
                op(K::XfDrag, &[1, -54, -98, 1, -1113], "", &[]),
                op(K::MoveBegin, &[0, 0], "", &[]),
                op(K::MoveTo, &[2, 2], "", &[]),
                op(K::TxnEnd, &[], "", &[]),
            ],
        );
    }
}

#[test]
fn regress_strokes_in_one_transaction_do_not_erase_each_other() {
    // Two strokes in a transaction; the second is previewed with restore. Rewinding for the preview
    // must go back to where the second stroke began, not to the transaction's first checkpoint.
    let d = pixmith_new(12, 12);
    let at = |x: i32, y: i32| spr(d).cels[0][0].as_ref().map_or(0, |c| c.pixels[((y * 12 + x) * 4 + 3) as usize]);
    unsafe {
        history::pixmith_begin_transaction(d);
        pixmith_begin_stroke(d);
        shapes::pixmith_shape(d, 0, 0, 0, 0, 0, 5, 0, 0xFF0000FF, 1, 0, false);
        pixmith_begin_stroke(d);
        shapes::pixmith_shape(d, 0, 0, 0, 0, 5, 0, 8, 0x00FF00FF, 1, 0, false);
        shapes::pixmith_shape(d, 0, 0, 0, 0, 5, 3, 5, 0x00FF00FF, 1, 0, true);
        history::pixmith_end_transaction(d);
    }
    assert_eq!(at(5, 0), 255, "the first stroke survives");
    assert_eq!(at(0, 5), 255, "the second stroke shows its last preview");
    assert_eq!(at(0, 8), 0, "an earlier preview of the second stroke is gone");
    unsafe {
        assert!(pixmith_undo(d));
        assert_eq!(at(5, 0), 0, "the transaction undoes as one step");
        pixmith_free(d);
    }
}

#[test]
fn regress_selection_clipping_uses_the_current_strokes_baseline_in_a_transaction() {
    let d = pixmith_new(12, 12);
    let at = |x: i32, y: i32| spr(d).cels[0][0].as_ref().map_or(0, |c| c.pixels[((y * 12 + x) * 4 + 3) as usize]);
    unsafe {
        history::pixmith_begin_transaction(d);
        pixmith_begin_stroke(d);
        pixmith_draw_line(d, 0, 0, 0, 0, 3, 0, 0xFF0000FF, 1); // outside the coming selection
        selection::pixmith_sel_rect(d, 6, 0, 11, 11);
        pixmith_begin_stroke(d);
        shapes::pixmith_shape(d, 0, 0, 0, 0, 4, 11, 4, 0x00FF00FF, 1, 0, false); // crosses the selection edge
        history::pixmith_end_transaction(d);
    }
    assert_eq!(at(1, 0), 255, "the first stroke is outside the selection and must survive the second one's clipping");
    assert_eq!(at(2, 4), 0, "the second stroke is clipped to the selection");
    assert_eq!(at(7, 4), 255);
    unsafe { pixmith_free(d) };
}

#[test]
fn regress_redo_after_two_canvas_resizes_restores_the_end_state() {
    regress(
        16,
        16,
        &[
            op(K::Group, &[], "", &[]),
            op(K::DrawLine, &[1, 0, 20, 9, 2, 11, 4278190335], "", &[]),
            op(K::ResizeCanvas, &[72, 25, 19, 3], "", &[]),
            op(K::ResizeCanvas, &[50, 36, -42, 31], "", &[]),
        ],
    );
}
