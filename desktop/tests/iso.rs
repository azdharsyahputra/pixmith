//! The isometric grid: its settings, and Select > Isometric Tile on a real document.

mod common;

use pixmith_desktop::app::PixmithApp;
use pixmith_desktop::doc::Doc;
use pixmith_desktop::ui::iso::{self, Iso};
use pixmith_desktop::ui::selection::{self, SelMode};
use pixmith_desktop::ui::tools::view::{GridType, ViewSettings};

/// The selected pixels of `doc` as `(x, y)`, row by row.
fn selected(doc: &Doc) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for y in 0..doc.height() as i32 {
        for x in 0..doc.width() as i32 {
            if doc.sel_contains(x, y) {
                out.push((x, y));
            }
        }
    }
    out
}

#[test]
fn iso_grid_settings_round_trip_and_persist() {
    let mut s = ViewSettings::default();
    assert_eq!(
        (s.custom_grid_type, s.iso_grid_w, s.iso_grid_h, s.iso_grid_x, s.iso_grid_y),
        (GridType::Rectangular, 32, 16, 0, 0)
    );
    s.custom_grid = true;
    s.custom_grid_type = GridType::Isometric;
    s.iso_grid_w = 104;
    s.iso_grid_h = 56;
    s.iso_grid_x = -6;
    s.iso_grid_y = 9;
    // Through the file the app keeps next to its other per-user data.
    let dir = common::scratch("iso-settings");
    let path = dir.join("view.txt");
    std::fs::write(&path, s.to_text()).unwrap();
    let mut back = ViewSettings::default();
    back.apply_text(&std::fs::read_to_string(&path).unwrap());
    assert_eq!(back, s);
    assert_eq!(back.iso(), Iso::new(104, 56, -6, 9));
    // The rectangular grid's own settings are untouched.
    assert_eq!((back.custom_grid_w, back.custom_grid_h), (16, 16));
}

#[test]
fn sizes_are_even_and_at_least_four() {
    let mut s = ViewSettings::default();
    s.apply_text("isoGridW=3\nisoGridH=15");
    assert_eq!((s.iso_grid_w, s.iso_grid_h), (4, 16));
    assert_eq!(Iso::new(7, 100, 0, 0), Iso::new(8, 100, 0, 0));
    assert_eq!(iso::clamp_side(0), 4);
}

#[test]
fn a_32_by_16_cell_selects_an_exact_diamond() {
    let mut doc = Doc::new(96, 64);
    let iso = Iso::new(32, 16, 0, 0);
    // Any pixel of the cell picks it; (20, 9) is inside the cell at the origin.
    assert!(selection::select_iso_cell(&mut doc, &iso, (20, 9), SelMode::Replace));
    let px = selected(&doc);
    assert_eq!(px.len(), 32 * 16 / 2, "half of the bounding box");
    // Rows are at most 30 wide (x 1 to 30) and 16 tall.
    assert_eq!(doc.sel_bounds(), Some([1, 0, 30, 16]));
    let mut widths = Vec::new();
    for y in 0..16 {
        let row: Vec<i32> = px.iter().filter(|p| p.1 == y).map(|p| p.0).collect();
        // One unbroken run, centered on the tile.
        let (l, r) = (*row.first().unwrap(), *row.last().unwrap());
        assert_eq!(row.len() as i32, r - l + 1, "row {y} is one run");
        assert_eq!(l + r + 1, 32, "row {y} is centered");
        widths.push(row.len());
    }
    assert_eq!(widths, [2, 6, 10, 14, 18, 22, 26, 30, 30, 26, 22, 18, 14, 10, 6, 2]);
    assert!(doc.sel_contains(15, 0) && doc.sel_contains(16, 0) && !doc.sel_contains(14, 0));
    assert!(doc.sel_contains(1, 7) && doc.sel_contains(30, 7) && !doc.sel_contains(0, 7) && !doc.sel_contains(31, 7));
    // It is one undo step.
    assert!(doc.undo());
    assert!(!doc.sel_active());
}

#[test]
fn the_selected_cell_matches_the_grid_for_any_cell_size_and_offset() {
    for (w, h, ox, oy) in [(32, 16, 0, 0), (32, 16, 5, -3), (104, 56, 0, 0), (4, 4, 1, 1), (8, 16, 0, 0), (10, 4, 2, 0)]
    {
        let iso = Iso::new(w, h, ox, oy);
        for cell in [(1, 1), (2, 1), (1, 2), (2, 0)] {
            let mut doc = Doc::new(220, 160);
            let rows = iso.cell_rows(cell);
            let (y, l, _) = rows[rows.len() / 2];
            assert!(selection::select_iso_cell(&mut doc, &iso, (l, y), SelMode::Replace));
            let mut want = Vec::new();
            for (y, l, r) in &rows {
                for x in *l..=*r {
                    if (0..220).contains(&x) && (0..160).contains(y) {
                        want.push((x, *y));
                    }
                }
            }
            want.sort_by_key(|p| (p.1, p.0));
            assert_eq!(selected(&doc), want, "{w}x{h} at {ox},{oy}, cell {cell:?}");
        }
    }
}

#[test]
fn add_and_subtract_modes_combine_cells() {
    let mut doc = Doc::new(96, 64);
    let iso = Iso::new(32, 16, 0, 0);
    selection::select_iso_cell(&mut doc, &iso, (16, 8), SelMode::Replace);
    selection::select_iso_cell(&mut doc, &iso, (48, 8), SelMode::Add);
    assert_eq!(selected(&doc).len(), 2 * 256);
    selection::select_iso_cell(&mut doc, &iso, (16, 8), SelMode::Subtract);
    assert_eq!(selected(&doc).len(), 256);
    assert!(doc.sel_contains(48, 8) && !doc.sel_contains(16, 8));
}

#[test]
fn the_menu_command_selects_under_the_cursor_then_the_last_click() {
    let mut app = PixmithApp::default();
    app.tabs[0].doc = Doc::new(96, 64);
    app.canvas.tools.settings.iso_grid_w = 32;
    app.canvas.tools.settings.iso_grid_h = 16;
    app.canvas.cursor = Some((48, 8));
    selection::select_iso_tile(&mut app);
    let doc = &app.tab().doc;
    assert_eq!(doc.sel_bounds(), Some([33, 0, 30, 16]), "the cell centered on (48, 8)");
    assert!(doc.sel_contains(48, 8) && !doc.sel_contains(16, 8));
    // With the pointer off the canvas and no click yet, the command waits for a click instead.
    selection::deselect(&mut app);
    app.canvas.cursor = None;
    selection::select_iso_tile(&mut app);
    assert!(!app.tab().doc.sel_active());
}
