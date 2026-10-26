//! Screenshots for the user guide: one PNG per tool or feature, dark theme only, cropped to the part of the window
//! that matters. Needs a GPU (wgpu), so it is ignored by default:
//!
//! ```sh
//! cargo test --test guide_screens -- --ignored      # writes target/guide/<id>.png
//! GUIDE_ONLY=pencil,eraser cargo test --test guide_screens -- --ignored   # only some ids
//! ```
//!
//! All scenes run in one test, one after the other, because the core keeps the ink mode and the brush shape in
//! globals that the scenes change.

use eframe::egui;
use egui_kittest::Harness;
use pixmith_desktop::app::{DocTab, Tool};
use pixmith_desktop::doc::Doc;
use pixmith_desktop::ui::canvas::{ShapeKind, flag};
use pixmith_desktop::ui::selection::SelMode;
use pixmith_desktop::ui::timeline::TagInfo;
use pixmith_desktop::{PixmithApp, theme};

const W: f32 = 1280.0;
const H: f32 = 800.0;
const INSPECTOR: f32 = 272.0;
const TITLE_ROW: f32 = 40.0;

/// Which part of the window a shot keeps.
#[derive(Clone, Copy)]
enum Region {
    /// Tool rail, options bar and canvas (no inspector, no timeline).
    Canvas,
    /// Like `Canvas`, with the timeline under it.
    WithTimeline,
    /// Everything but the title row.
    Wide,
    /// The whole window.
    Whole,
}

struct Shot {
    id: &'static str,
    size: [f32; 2],
    region: Region,
    collapse: &'static [&'static str],
    setup: fn(&mut PixmithApp),
}

const fn shot(id: &'static str, setup: fn(&mut PixmithApp)) -> Shot {
    Shot { id, size: [W, H], region: Region::Canvas, collapse: &[], setup }
}

/// Renders one shot and saves the cropped PNG. Returns the size written.
fn render(s: &Shot) -> (u32, u32) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/guide");
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = PixmithApp { dark: true, ..Default::default() };
    (s.setup)(&mut app);
    let timeline_h = pixmith_desktop::ui::timeline::panel_height(&app);
    let [w, h] = s.size;
    let collapse = s.collapse;
    let mut installed = false;
    // The snapshot options (`PREDICTABLE`) filter every texture bilinearly in the shader, which blurs the pixels of
    // the sprite; switch that off so the NEAREST texture of the canvas stays sharp.
    let mut options = eframe::egui_wgpu::RendererOptions::PREDICTABLE;
    options.predictable_texture_filtering = false;
    let mut harness = Harness::builder()
        .with_size([w, h])
        .with_pixels_per_point(2.0)
        .with_render_options(options)
        .wgpu()
        .build_ui_state(
        |ui, app: &mut PixmithApp| {
            if !installed {
                // New fonts take effect on the next pass; the first pass draws nothing.
                theme::install(ui.ctx());
                theme::apply(ui.ctx(), app.dark);
                for id in collapse {
                    pixmith_desktop::ui::widgets::set_section_collapsed(ui.ctx(), id, true);
                }
                installed = true;
                ui.ctx().request_discard("fonts installed");
                return;
            }
            // The harness insets its root ui; the real window has no inset.
            let full = ui.ctx().content_rect();
            let mut root = ui.new_child(egui::UiBuilder::new().max_rect(full));
            root.set_clip_rect(full);
            app.ui(&mut root);
        },
        app,
    );
    // The marching ants and the text cursor ask for repaints, so `run` would complain; `run_ok` does not.
    harness.run_ok();
    harness.run_ok();
    let image = harness.render().expect("render");
    let px = |v: f32| (v * 2.0).round() as u32;
    let (x0, y0, x1, y1) = match s.region {
        Region::Canvas => (0, px(TITLE_ROW), px(w - INSPECTOR), px(h - timeline_h)),
        Region::WithTimeline => (0, px(TITLE_ROW), px(w - INSPECTOR), px(h)),
        Region::Wide => (0, px(TITLE_ROW), px(w), px(h)),
        Region::Whole => (0, 0, px(w), px(h)),
    };
    let (x1, y1) = (x1.min(image.width()), y1.min(image.height()));
    let cropped = image::imageops::crop_imm(&image, x0, y0, x1 - x0, y1 - y0).to_image();
    cropped.save(dir.join(format!("{}.png", s.id))).unwrap();
    cropped.dimensions()
}

// ---- Sprites ----

/// The face of the `canvas` scenario of `screens.rs`, 48 x 48.
fn face_doc() -> Doc {
    let mut doc = Doc::new(48, 48);
    let mut shape = |kind, a, b, color, flags| {
        doc.begin_edit("Shape");
        doc.draw_shape(kind, 0, 0, a, b, color, 1, flags, false);
        doc.stroke_end();
    };
    shape(ShapeKind::Rect, (4, 4), (43, 43), 0x2B3566FF, flag::FILLED);
    shape(ShapeKind::Ellipse, (11, 7), (36, 41), 0x1A1A33FF, flag::FILLED);
    shape(ShapeKind::Ellipse, (12, 8), (35, 40), 0xE8A862FF, flag::FILLED);
    shape(ShapeKind::Rect, (17, 21), (20, 25), 0x1A1A33FF, flag::FILLED);
    shape(ShapeKind::Rect, (27, 21), (30, 25), 0x1A1A33FF, flag::FILLED);
    shape(ShapeKind::Rect, (14, 12), (17, 14), 0xFBE3A0FF, flag::FILLED);
    doc
}

/// A plain dark blue sprite to draw on.
fn plain_doc() -> Doc {
    let mut doc = Doc::new(48, 48);
    doc.begin_edit("Backdrop");
    doc.draw_shape(ShapeKind::Rect, 0, 0, (0, 0), (47, 47), 0x232B4DFF, 1, flag::FILLED, false);
    doc.stroke_end();
    doc
}

/// A 24 x 24 sprite with `frames` frames; the ball moves two pixels right per frame.
fn ball_doc(frames: u32) -> Doc {
    let mut doc = Doc::new(24, 24);
    for _ in 1..frames {
        let last = doc.frame_count() - 1;
        doc.add_frame(last, false);
    }
    let layer = doc.layer_count() - 1;
    for f in 0..frames {
        let dx = f as i32 * 2;
        doc.begin_edit("Shape");
        doc.draw_shape(ShapeKind::Ellipse, layer, f, (4 + dx, 6), (13 + dx, 17), 0xE8A862FF, 1, flag::FILLED, false);
        doc.draw_shape(ShapeKind::Rect, layer, f, (7 + dx, 9), (8 + dx, 11), 0x1A1A33FF, 1, flag::FILLED, false);
        doc.stroke_end();
    }
    doc
}

/// Puts `doc` in tab 0 at `zoom` and centers the view on it.
fn put(app: &mut PixmithApp, doc: Doc, zoom: f32, tool: Tool) {
    app.tabs[0] = DocTab::new(doc);
    app.tabs[0].zoom = zoom;
    app.tool = tool;
    app.primary = 0xF2A65AFF;
    app.secondary = 0x2B3A67FF;
    let tab = &app.tabs[0];
    app.canvas.keep_view(tab);
    app.selection.system_clipboard = false;
}

fn top(doc: &Doc) -> u32 {
    doc.layer_count() - 1
}

/// One freehand stroke through `pts` on the top layer, as the pencil does it.
fn stroke(doc: &mut Doc, pts: &[(i32, i32)], color: u32, size: u32, flags: u32) {
    let layer = top(doc);
    doc.begin_edit("Draw");
    doc.stroke_begin(layer, 0, pts[0].0, pts[0].1, color, size, flags);
    for p in &pts[1..] {
        doc.stroke_to(p.0, p.1);
    }
    doc.stroke_end();
}

fn select_rect(doc: &mut Doc, a: (i32, i32), b: (i32, i32)) {
    doc.sel_begin(SelMode::Replace);
    doc.sel_rect(a, b);
    doc.sel_end();
}

// ---- Scenes ----

fn colors(app: &mut PixmithApp) {
    let mut doc = plain_doc();
    let (pink, teal) = (0xF2557FFF, 0x35D0BAFF);
    stroke(&mut doc, &[(6, 10), (14, 6), (22, 14), (30, 8), (42, 12)], pink, 2, 0);
    stroke(&mut doc, &[(6, 24), (14, 20), (22, 28), (30, 22), (42, 26)], teal, 2, 0);
    stroke(&mut doc, &[(6, 38), (14, 34), (22, 42), (30, 36), (42, 40)], pink, 2, 0);
    put(app, doc, 8.0, Tool::Pencil);
    app.primary = pink;
    app.secondary = teal;
    app.timeline.collapsed = true;
}

fn brush_size(app: &mut PixmithApp) {
    let mut doc = face_doc();
    stroke(&mut doc, &[(8, 36), (16, 42), (26, 38), (34, 43), (41, 36)], 0xFBE3A0FF, 5, 0);
    put(app, doc, 8.0, Tool::Pencil);
    app.brush_size = 5;
    app.canvas.opts.circle_brush = true;
    Doc::set_brush_circle(true);
}

fn view(app: &mut PixmithApp) {
    let mut doc = Doc::new(24, 24);
    doc.begin_edit("Face");
    doc.draw_shape(ShapeKind::Ellipse, 0, 0, (3, 2), (20, 21), 0x1A1A33FF, 1, flag::FILLED, false);
    doc.draw_shape(ShapeKind::Ellipse, 0, 0, (4, 3), (19, 20), 0xE8A862FF, 1, flag::FILLED, false);
    doc.draw_shape(ShapeKind::Rect, 0, 0, (8, 9), (9, 11), 0x1A1A33FF, 1, flag::FILLED, false);
    doc.draw_shape(ShapeKind::Rect, 0, 0, (14, 9), (15, 11), 0x1A1A33FF, 1, flag::FILLED, false);
    doc.draw_shape(ShapeKind::Line, 0, 0, (8, 16), (15, 16), 0x1A1A33FF, 1, 0, false);
    doc.stroke_end();
    put(app, doc, 16.0, Tool::Pencil);
    app.timeline.show_grid = true;
    let s = &mut app.canvas.tools.settings;
    s.grid_color = [255, 255, 255];
    s.grid_opacity = 0.35;
}

fn pencil(app: &mut PixmithApp) {
    let mut doc = face_doc();
    stroke(&mut doc, &[(13, 30), (16, 34), (20, 36), (24, 37), (28, 36), (32, 34), (35, 30)], 0x1A1A33FF, 1, 0);
    put(app, doc, 8.0, Tool::Pencil);
}

fn eraser(app: &mut PixmithApp) {
    let mut doc = face_doc();
    stroke(&mut doc, &[(8, 12), (14, 20), (22, 28), (30, 22), (40, 34)], 0, 3, 0);
    put(app, doc, 8.0, Tool::Eraser);
    app.brush_size = 3;
}

fn spray(app: &mut PixmithApp) {
    let mut doc = plain_doc();
    let layer = top(&doc);
    doc.begin_edit("Spray");
    doc.set_ink(0, false, &[]);
    let path = [(8, 34), (13, 28), (19, 22), (25, 20), (31, 22), (36, 27), (40, 33)];
    for (i, p) in path.iter().enumerate() {
        doc.spray_at(layer, 0, *p, 0xF2A65AFF, 6, 40, 11 + i as u32 * 7, 0);
        doc.spray_at(layer, 0, *p, 0xF2A65AFF, 6, 40, 91 + i as u32 * 13, 0);
    }
    doc.stroke_end();
    put(app, doc, 8.0, Tool::Spray);
    app.brush_size = 2;
}

fn blur(app: &mut PixmithApp) {
    let mut doc = face_doc();
    let layer = top(&doc);
    doc.begin_edit("Blur");
    doc.retouch_begin(layer, 0, (10, 16), false, 6, 80, 5, 0);
    for p in [(18, 20), (26, 24), (34, 20), (40, 16), (36, 30), (24, 34), (14, 30)] {
        doc.retouch_to(p);
    }
    doc.retouch_end();
    put(app, doc, 8.0, Tool::Blur);
    app.brush_size = 6;
    app.canvas.opts.retouch_strength = 80.0;
}

fn jumble(app: &mut PixmithApp) {
    let mut doc = face_doc();
    let layer = top(&doc);
    doc.begin_edit("Jumble");
    doc.retouch_begin(layer, 0, (10, 30), true, 5, 100, 3, 0);
    for p in [(16, 36), (24, 38), (32, 36), (38, 30)] {
        doc.retouch_to(p);
    }
    doc.retouch_end();
    put(app, doc, 8.0, Tool::Jumble);
    app.brush_size = 5;
}

fn bucket(app: &mut PixmithApp) {
    let mut doc = plain_doc();
    let layer = top(&doc);
    let mut outline = |kind, a, b| {
        doc.begin_edit("Outline");
        doc.draw_shape(kind, layer, 0, a, b, 0xFBE3A0FF, 1, 0, false);
        doc.stroke_end();
    };
    outline(ShapeKind::Rect, (4, 4), (43, 43));
    outline(ShapeKind::Ellipse, (12, 10), (35, 38));
    outline(ShapeKind::Rect, (20, 19), (27, 29));
    doc.begin_edit("Fill");
    doc.fill_at(layer, 0, 7, 7, 0x35D0BAFF, 0x232B4DFF, true, 0);
    doc.stroke_end();
    doc.begin_edit("Fill");
    doc.fill_at(layer, 0, 16, 24, 0xF2A65AFF, 0x232B4DFF, true, 0);
    doc.stroke_end();
    put(app, doc, 8.0, Tool::Fill);
    app.primary = 0x35D0BAFF;
}

fn gradient(app: &mut PixmithApp) {
    let mut doc = plain_doc();
    let layer = top(&doc);
    doc.begin_edit("Gradient");
    doc.gradient_draw(layer, 0, (6, 6), (42, 42), 0xF2A65AFF, 0x2B3A67FF, false, 4, false);
    doc.stroke_end();
    put(app, doc, 8.0, Tool::Gradient);
    app.canvas.opts.gradient_dither = 2;
}

fn line(app: &mut PixmithApp) {
    let mut doc = plain_doc();
    let layer = top(&doc);
    doc.begin_edit("Line");
    doc.draw_shape(ShapeKind::Line, layer, 0, (6, 40), (40, 8), 0xF2A65AFF, 1, 0, false);
    doc.draw_shape(ShapeKind::Line, layer, 0, (6, 10), (42, 28), 0x35D0BAFF, 2, 0, false);
    doc.draw_shape(ShapeKind::Line, layer, 0, (10, 44), (44, 44), 0xFBE3A0FF, 3, 0, false);
    doc.stroke_end();
    put(app, doc, 8.0, Tool::Line);
}

fn rectangle(app: &mut PixmithApp) {
    let mut doc = plain_doc();
    let layer = top(&doc);
    doc.begin_edit("Rectangle");
    doc.draw_shape(ShapeKind::Rect, layer, 0, (6, 8), (28, 30), 0xF2A65AFF, 1, flag::FILLED, false);
    doc.draw_shape(ShapeKind::Rect, layer, 0, (20, 20), (42, 42), 0x35D0BAFF, 1, flag::FILLED, false);
    doc.stroke_end();
    put(app, doc, 8.0, Tool::Rectangle);
    app.canvas.opts.filled = true;
}

fn ellipse(app: &mut PixmithApp) {
    let mut doc = plain_doc();
    let layer = top(&doc);
    doc.begin_edit("Ellipse");
    doc.draw_shape(ShapeKind::Ellipse, layer, 0, (5, 8), (30, 32), 0xF2A65AFF, 1, flag::FILLED, false);
    doc.draw_shape(ShapeKind::Ellipse, layer, 0, (22, 18), (43, 43), 0x35D0BAFF, 1, flag::FILLED, false);
    doc.stroke_end();
    put(app, doc, 8.0, Tool::Ellipse);
    app.canvas.opts.filled = true;
}

fn curve(app: &mut PixmithApp) {
    let mut doc = plain_doc();
    let layer = top(&doc);
    doc.begin_edit("Curve");
    doc.curve_draw(layer, 0, &[(5, 38), (14, 4), (34, 4), (43, 38)], 0xF2A65AFF, 2, 0);
    doc.stroke_end();
    put(app, doc, 8.0, Tool::Curve);
    app.brush_size = 2;
}

fn polygon(app: &mut PixmithApp) {
    let mut doc = plain_doc();
    let layer = top(&doc);
    doc.begin_edit("Polygon");
    doc.polygon_draw(layer, 0, &[(24, 5), (42, 17), (36, 40), (12, 40), (6, 17)], 0xF2A65AFF, 1, flag::FILLED);
    doc.stroke_end();
    put(app, doc, 8.0, Tool::ShapePolygon);
    app.canvas.opts.filled = true;
}

fn contour(app: &mut PixmithApp) {
    let mut doc = plain_doc();
    let layer = top(&doc);
    doc.begin_edit("Contour");
    let blob = [
        (10, 20),
        (12, 13),
        (18, 8),
        (25, 9),
        (28, 14),
        (35, 12),
        (41, 18),
        (40, 27),
        (34, 33),
        (35, 40),
        (26, 41),
        (19, 36),
        (12, 33),
    ];
    doc.polygon_draw(layer, 0, &blob, 0x35D0BAFF, 1, flag::FILLED);
    doc.stroke_end();
    put(app, doc, 8.0, Tool::Contour);
    app.canvas.opts.filled = true;
}

fn text(app: &mut PixmithApp) {
    let mut doc = plain_doc();
    let layer = top(&doc);
    doc.begin_edit("Text");
    doc.text_draw(layer, 0, (9, 14), "PIXEL", 1, 0xF2A65AFF, 0, false);
    doc.stroke_end();
    doc.begin_edit("Text");
    doc.text_draw(layer, 0, (9, 28), "smith", 1, 0x35D0BAFF, 0, false);
    doc.stroke_end();
    put(app, doc, 8.0, Tool::Text);
}

fn marquee(app: &mut PixmithApp) {
    let mut doc = face_doc();
    select_rect(&mut doc, (14, 16), (33, 30));
    put(app, doc, 8.0, Tool::Marquee);
}

fn lasso(app: &mut PixmithApp) {
    let mut doc = face_doc();
    doc.sel_begin(SelMode::Replace);
    let pts = [
        (13, 14),
        (17, 11),
        (22, 12),
        (27, 10),
        (32, 13),
        (35, 18),
        (34, 24),
        (36, 30),
        (31, 35),
        (24, 33),
        (19, 36),
        (14, 31),
        (12, 24),
        (14, 19),
    ];
    doc.sel_poly(&pts);
    doc.sel_end();
    put(app, doc, 8.0, Tool::Lasso);
}

fn polylasso(app: &mut PixmithApp) {
    let mut doc = face_doc();
    doc.sel_begin(SelMode::Replace);
    doc.sel_poly(&[(24, 8), (36, 20), (32, 38), (16, 38), (12, 20)]);
    doc.sel_end();
    put(app, doc, 8.0, Tool::Polygon);
}

fn wand(app: &mut PixmithApp) {
    let mut doc = face_doc();
    let layer = top(&doc);
    assert!(doc.sel_wand(layer, 0, 24, 33, 8, true, SelMode::Replace));
    put(app, doc, 8.0, Tool::Wand);
    app.selection.tolerance = 8;
}

fn move_tool(app: &mut PixmithApp) {
    let mut doc = face_doc();
    let layer = top(&doc);
    select_rect(&mut doc, (14, 16), (33, 30));
    assert!(doc.sel_move_begin(layer, 0));
    doc.sel_move_to(5, -6);
    put(app, doc, 8.0, Tool::Move);
}

fn transform(app: &mut PixmithApp) {
    let mut doc = face_doc();
    select_rect(&mut doc, (14, 16), (33, 30));
    let layer = top(&doc);
    assert!(doc.xf_begin(layer, 0, false));
    doc.xf_set_field(4, 12.0);
    put(app, doc, 8.0, Tool::Marquee);
}

fn eyedropper(app: &mut PixmithApp) {
    put(app, face_doc(), 8.0, Tool::Picker);
    app.primary = 0xE8A862FF;
    app.secondary = 0x1A1A33FF;
    app.timeline.collapsed = true;
}

fn slice(app: &mut PixmithApp) {
    let mut doc = face_doc();
    let a = doc.add_slice(10, 6, 28, 20).unwrap();
    doc.set_slice_props(a, 0, "head", None, Some([14, 10]));
    doc.add_slice(10, 28, 28, 14).unwrap();
    doc.set_slice_props(1, 0, "chin", None, None);
    put(app, doc, 8.0, Tool::Slice);
    app.tilemap.slice_selected = Some(0);
}

fn ink(app: &mut PixmithApp) {
    let mut doc = face_doc();
    let ramp = [0xE8A862FF, 0xC98749FF, 0xA06431FF, 0x6E4223FF];
    for c in ramp {
        doc.palette_add(c);
    }
    let layer = top(&doc);
    // Each pass over the cheek steps the pixels one color further along the ramp.
    let passes: [&[(i32, i32)]; 4] = [
        &[(14, 28), (20, 36), (30, 36), (34, 28)],
        &[(16, 31), (21, 36), (30, 36), (33, 31)],
        &[(18, 34), (22, 37), (28, 37), (31, 34)],
        &[(20, 36), (24, 38), (27, 37)],
    ];
    for pts in passes {
        doc.begin_edit("Shade");
        doc.set_ink(4, false, &ramp);
        doc.stroke_begin(layer, 0, pts[0].0, pts[0].1, ramp[0], 3, 0);
        for p in &pts[1..] {
            doc.stroke_to(p.0, p.1);
        }
        doc.stroke_end();
    }
    doc.set_ink(0, false, &[]);
    put(app, doc, 8.0, Tool::Pencil);
    app.primary = ramp[0];
    app.secondary = ramp[3];
    app.brush_size = 3;
    app.canvas.opts.ink = 4;
}

fn pixel_perfect(app: &mut PixmithApp) {
    let mut doc = plain_doc();
    // A staircase of one-pixel steps: right, up, right, up. Pixel perfect drops the corner pixels.
    let mut pts = vec![(6, 40)];
    let (mut x, y0) = (6, 40);
    let mut y = y0;
    while x < 40 {
        x += 1;
        pts.push((x, y));
        y -= 1;
        pts.push((x, y));
    }
    stroke(&mut doc, &pts, 0xF2A65AFF, 1, flag::PIXEL_PERFECT);
    // The same stroke without it, for comparison.
    let shifted: Vec<(i32, i32)> = pts.iter().map(|p| (p.0, p.1 + 6)).filter(|p| p.1 < 47).collect();
    stroke(&mut doc, &shifted, 0x35D0BAFF, 1, 0);
    put(app, doc, 8.0, Tool::Pencil);
    app.canvas.opts.pixel_perfect = true;
}

fn symmetry(app: &mut PixmithApp) {
    let mut doc = plain_doc();
    let flags = flag::MIRROR_X;
    stroke(&mut doc, &[(5, 40), (8, 30), (13, 22), (11, 12), (17, 8), (21, 16)], 0xF2557FFF, 2, flags);
    stroke(&mut doc, &[(4, 20), (10, 28), (16, 36), (20, 40)], 0x35D0BAFF, 2, flags);
    put(app, doc, 8.0, Tool::Pencil);
    app.brush_size = 2;
    app.canvas.opts.mirror_x = true;
}

/// Six frames with tags idle / move, as the timeline of the macOS screenshots.
fn frames(app: &mut PixmithApp) {
    let mut doc = ball_doc(6);
    doc.rename_layer(0, "Body");
    let eyes = doc.add_layer(0);
    doc.rename_layer(eyes, "Eyes");
    for f in 0..6 {
        let x = 9 + 2 * f as i32;
        doc.begin_edit("Eyes");
        doc.draw_shape(ShapeKind::Rect, eyes, f, (x, 9), (x, 11), 0xFFFFFFFF, 1, flag::FILLED, false);
        doc.stroke_end();
    }
    doc.add_tag(&TagInfo { name: "idle".into(), from: 0, to: 2, direction: 0, repeat: 0, color: 0x3B82F6 });
    doc.add_tag(&TagInfo { name: "move".into(), from: 2, to: 5, direction: 0, repeat: 0, color: 0xEF4444 });
    put(app, doc, 12.0, Tool::Pencil);
    app.tabs[0].layer = 1;
    app.tabs[0].frame = 2;
    app.tabs[0].changed();
}

fn onion(app: &mut PixmithApp) {
    put(app, ball_doc(3), 14.0, Tool::Pencil);
    app.tabs[0].frame = 1;
    app.timeline.onion_skin = true;
    let s = &mut app.canvas.tools.settings;
    s.onion_past = 1;
    s.onion_future = 1;
    s.onion_tint = true;
    s.onion_opacity = 0.5;
}

fn tilemap_scene(app: &mut PixmithApp) {
    let mut doc = Doc::new(48, 48);
    doc.rename_layer(0, "Tiles");
    let mut shape = |kind, a, b, color, flags| {
        doc.begin_edit("Shape");
        doc.draw_shape(kind, 0, 0, a, b, color, 1, flags, false);
        doc.stroke_end();
    };
    shape(ShapeKind::Rect, (4, 4), (43, 43), 0x2B3566FF, flag::FILLED);
    shape(ShapeKind::Ellipse, (12, 8), (35, 40), 0xE8A862FF, flag::FILLED);
    assert!(doc.convert_layer_to_tilemap(0, 16, 16, true));
    put(app, doc, 8.0, Tool::Pencil);
    app.tabs[0].layer = 0;
    app.tilemap.set_stamp(true);
    app.tilemap.selected = 2;
}

fn terrain_scene(app: &mut PixmithApp) {
    let mut doc = Doc::new(128, 96);
    doc.rename_layer(0, "Layer 1");
    let layer = doc.new_tilemap_layer(0, 16, 16);
    doc.rename_layer(layer, "Ground");
    assert!(doc.create_terrain(layer, "Grass", 1));
    doc.add_terrain(layer);
    doc.add_terrain(layer);
    assert!(doc.generate_template(layer, true));
    doc.set_tile_mask(layer, 3, -1);
    doc.begin_edit("Terrain Rectangle");
    doc.paint_terrain_rect(layer, 0, (16, 16), (63, 47), 1, 0);
    doc.paint_terrain_rect(layer, 0, (32, 48), (47, 63), 1, 0);
    doc.paint_terrain_rect(layer, 0, (96, 64), (111, 79), 1, 0);
    app.tabs[0] = DocTab::new(doc);
    app.tabs[0].layer = layer;
    app.tabs[0].zoom = 6.0;
    app.tool = Tool::Pencil;
    app.tilemap.brush = true;
    app.tilemap.show_masks = false;
    app.tilemap.selected = 6;
    let tab = &app.tabs[0];
    app.canvas.keep_view(tab);
}

fn iso(app: &mut PixmithApp) {
    use pixmith_desktop::ui::iso::Iso;
    use pixmith_desktop::ui::selection::select_iso_cell;
    use pixmith_desktop::ui::tools::view::GridType;
    let mut doc = plain_doc();
    let iso = Iso::new(32, 16, 0, 0);
    select_iso_cell(&mut doc, &iso, (24, 20), SelMode::Replace);
    put(app, doc, 8.0, Tool::Marquee);
    let s = &mut app.canvas.tools.settings;
    s.custom_grid = true;
    s.custom_grid_type = GridType::Isometric;
    s.iso_grid_w = 32;
    s.iso_grid_h = 16;
    s.iso_grid_x = 0;
    s.iso_grid_y = 0;
}

fn export_godot(app: &mut PixmithApp) {
    let project = std::path::PathBuf::from("/tmp/my-game");
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("project.godot"), "config_version=5\n").unwrap();
    let mut doc = ball_doc(6);
    doc.rename_layer(0, "Hero");
    doc.add_tag(&TagInfo { name: "idle".into(), from: 0, to: 2, direction: 0, repeat: 0, color: 0x3B82F6 });
    doc.add_tag(&TagInfo { name: "move".into(), from: 3, to: 5, direction: 0, repeat: 0, color: 0xEF4444 });
    put(app, doc, 12.0, Tool::Pencil);
    app.gamedev.prefs.set_godot_root(&project.to_string_lossy());
    app.open_godot_export();
}

fn live_link(app: &mut PixmithApp) {
    put(app, face_doc(), 8.0, Tool::Pencil);
    app.menus.prefs_open = true;
    app.menus.prefs_tab = pixmith_desktop::ui::menus::PrefsTab::General;
}

#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn guide_screens() {
    const TILE_COLLAPSE: &[&str] = &["color", "palette", "layer", "cel"];
    let shots = [
        shot("colors", colors),
        shot("brush-size", brush_size),
        shot("view", view),
        shot("pencil", pencil),
        shot("eraser", eraser),
        shot("spray", spray),
        shot("blur", blur),
        shot("jumble", jumble),
        shot("bucket", bucket),
        shot("gradient", gradient),
        shot("line", line),
        shot("rectangle", rectangle),
        shot("ellipse", ellipse),
        shot("curve", curve),
        shot("polygon", polygon),
        shot("contour", contour),
        shot("text", text),
        shot("marquee", marquee),
        shot("lasso", lasso),
        shot("polylasso", polylasso),
        shot("wand", wand),
        shot("move", move_tool),
        shot("transform", transform),
        shot("eyedropper", eyedropper),
        shot("slice", slice),
        shot("ink", ink),
        shot("pixel-perfect", pixel_perfect),
        shot("symmetry", symmetry),
        Shot { region: Region::WithTimeline, ..shot("frames", frames) },
        Shot { region: Region::WithTimeline, ..shot("onion", onion) },
        Shot { region: Region::Wide, collapse: TILE_COLLAPSE, ..shot("tilemap", tilemap_scene) },
        Shot { region: Region::Wide, collapse: TILE_COLLAPSE, ..shot("terrain", terrain_scene) },
        shot("iso", iso),
        Shot { region: Region::Whole, ..shot("export-godot", export_godot) },
        Shot { region: Region::Whole, ..shot("live-link", live_link) },
    ];
    let only: Vec<String> = std::env::var("GUIDE_ONLY")
        .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let mut failed = Vec::new();
    for s in &shots {
        if !only.is_empty() && !only.iter().any(|o| o == s.id) {
            continue;
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(s)));
        match result {
            Ok((w, h)) => println!("wrote target/guide/{}.png ({w}x{h})", s.id),
            Err(_) => failed.push(s.id),
        }
        // The core keeps these in globals; put them back for the next scene.
        let mut scratch = Doc::new(1, 1);
        scratch.set_ink(0, false, &[]);
        Doc::set_brush_circle(false);
        Doc::brush_reset();
    }
    assert!(failed.is_empty(), "scenes that panicked: {failed:?}");
}
