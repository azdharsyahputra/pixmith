//! Drives the menu bar, the filter sheets, the History panel, Preferences and the single-key shortcuts
//! headlessly with synthetic input (no GPU needed).

use eframe::egui::{self, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pixmith_desktop::app::Tool;
use pixmith_desktop::ui::menus::{Action, FilterKind, MenusState, Prefs, PrefsTab};
use pixmith_desktop::{PixmithApp, theme};
use std::collections::HashMap;

fn harness(setup: impl FnOnce(&mut PixmithApp)) -> Harness<'static, PixmithApp> {
    let mut app = PixmithApp::default();
    setup(&mut app);
    let mut installed = false;
    let mut h = Harness::builder().with_size([1280.0, 800.0]).build_ui_state(
        move |ui, app: &mut PixmithApp| {
            if !installed {
                theme::install(ui.ctx());
                theme::apply(ui.ctx(), app.dark);
                installed = true;
                ui.ctx().request_discard("fonts installed");
                return;
            }
            let full = ui.ctx().content_rect();
            let mut root = ui.new_child(egui::UiBuilder::new().max_rect(full));
            root.set_clip_rect(full);
            app.ui(&mut root);
        },
        app,
    );
    h.run();
    h
}

/// A sprite with one red pixel at (1, 1) in the first tab.
fn red_dot(app: &mut PixmithApp) {
    let tab = app.tab_mut();
    tab.doc.begin_edit("Pencil");
    tab.doc.stroke_begin(0, 0, 1, 1, 0xFF0000FF, 1, 0);
    tab.doc.stroke_end();
    tab.doc.touch();
    tab.changed();
}

fn pixel(h: &Harness<'static, PixmithApp>, x: i32, y: i32) -> u32 {
    let doc = &h.state().tab().doc;
    let mut buf = Vec::new();
    assert!(doc.render(0, &mut buf));
    pixmith_desktop::doc::Doc::pixel_of(&buf, doc.width(), x, y).unwrap()
}

#[test]
fn the_menu_bar_has_every_menu() {
    let h = harness(|_| {});
    for name in
        ["File", "Edit", "Select", "Sprite", "Layer", "Frame", "Drawing", "Palette", "Tilemap", "FX", "View", "Help"]
    {
        assert!(h.query_by_label(name).is_some(), "menu {name} is missing");
    }
}

#[test]
fn edit_undo_runs_from_the_menu() {
    let mut h = harness(red_dot);
    assert_eq!(pixel(&h, 1, 1), 0xFF0000FF);
    h.get_by_label("Edit").click();
    h.run();
    h.get_by_label_contains("Undo").click();
    h.run();
    assert_eq!(pixel(&h, 1, 1), 0x00000000);
    assert_eq!(h.state().tab().doc.history_counts().1, 1);
}

#[test]
fn layer_and_frame_menus_edit_the_sprite() {
    let mut h = harness(|_| {});
    let layers = h.state().tab().doc.layer_count();
    h.get_by_label("Layer").click();
    h.run();
    h.get_by_label_contains("New Layer").click();
    h.run();
    assert_eq!(h.state().tab().doc.layer_count(), layers + 1);
    h.get_by_label("Frame").click();
    h.run();
    h.get_by_label_contains("New Frame").click();
    h.run();
    assert_eq!(h.state().tab().doc.frame_count(), 2);
    assert_eq!(h.state().tab().frame, 1);
}

#[test]
fn menu_shortcuts_run_their_commands() {
    let mut h = harness(|_| {});
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::N);
    h.run();
    assert_eq!(h.state().tab().doc.layer_count(), 2);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::N);
    h.run();
    assert_eq!(h.state().tab().doc.frame_count(), 2);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::Y);
    h.run();
    assert!(h.state().menus.history_open);
}

#[test]
fn drawing_menu_toggles_the_tool_options() {
    let mut h = harness(|_| {});
    assert!(!h.state().canvas.opts.mirror_x);
    h.get_by_label("Drawing").click();
    h.run();
    h.get_by_label_contains("Mirror Horizontally").click();
    h.run();
    assert!(h.state().canvas.opts.mirror_x);
}

#[test]
fn a_filter_sheet_previews_then_applies_one_undo_step() {
    let mut h = harness(red_dot);
    let before = h.state().tab().doc.history_counts().0;
    {
        let app = h.state_mut();
        let PixmithApp { menus, tabs, active, primary, .. } = app;
        menus.open_filter(&mut tabs[*active], FilterKind::Invert, *primary);
    }
    h.run();
    assert!(h.state().canvas.preview.is_some(), "the canvas shows the preview while the sheet is open");
    assert_eq!(pixel(&h, 1, 1), 0xFF0000FF, "the document is untouched until Apply");
    h.key_press(Key::Enter);
    h.run();
    assert!(h.state().canvas.preview.is_none());
    assert!(h.state().menus.filter.is_none());
    assert_eq!(pixel(&h, 1, 1), 0x00FFFFFF);
    let doc = &h.state().tab().doc;
    assert_eq!(doc.history_counts().0, before + 1);
    assert_eq!(doc.history_label(before), "Filter: Invert");
}

#[test]
fn remove_background_sheet_clears_the_paper_and_is_one_undo_step() {
    let mut h = harness(|app| {
        let tab = app.tab_mut();
        // White paper everywhere, a red dot in the middle.
        tab.doc.begin_edit("Paper");
        tab.doc.stroke_begin(0, 0, 16, 16, 0xFFFFFFFF, 64, 0);
        tab.doc.stroke_end();
        tab.doc.begin_edit("Dot");
        tab.doc.stroke_begin(0, 0, 16, 16, 0xFF0000FF, 1, 0);
        tab.doc.stroke_end();
        tab.doc.touch();
        tab.changed();
    });
    assert_eq!(pixel(&h, 1, 1), 0xFFFFFFFF);
    let before = h.state().tab().doc.history_counts().0;
    h.get_by_label("FX").click();
    h.run();
    h.get_by_label_contains("Remove Background").click();
    h.run();
    assert!(h.state().menus.filter.is_some());
    assert!(h.query_by_label_contains("Also remove the drop shadow").is_some());
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(pixel(&h, 1, 1) & 0xFF, 0, "the paper is transparent");
    assert_eq!(pixel(&h, 16, 16), 0xFF0000FF, "the dot stays");
    assert_eq!(h.state().tab().doc.history_counts().0, before + 1);
}

#[test]
fn trace_pixel_art_dialog_asks_for_its_options_and_cancels() {
    use pixmith_desktop::ui::menus::hooks::TraceImport;
    let mut h = harness(|app| app.files.trace_import = Some(TraceImport::new("hero.png".into())));
    assert!(h.query_by_label_contains("Remove the drop shadow").is_some());
    assert!(h.query_by_label_contains("Close gaps in the outline").is_some());
    assert!(h.query_by_label_contains("hero.png").is_some());
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().files.trace_import.is_none());
}

#[test]
fn cancelling_a_filter_leaves_the_sprite_alone() {
    let mut h = harness(red_dot);
    h.get_by_label("FX").click();
    h.run();
    h.get_by_label_contains("Invert").click();
    h.run();
    assert!(h.state().menus.filter.is_some());
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().menus.filter.is_none() && h.state().canvas.preview.is_none());
    assert_eq!(pixel(&h, 1, 1), 0xFF0000FF);
}

#[test]
fn scale_sprite_sheet_applies() {
    let mut h = harness(|_| {});
    h.get_by_label("FX").click();
    h.run();
    h.get_by_label_contains("Scale Sprite").click();
    h.run();
    h.key_press(Key::Enter);
    h.run();
    // Percent mode starts at 200%.
    assert_eq!((h.state().tab().doc.width(), h.state().tab().doc.height()), (64, 64));
}

#[test]
fn canvas_size_sheet_resizes() {
    let mut h = harness(|_| {});
    h.get_by_label("Sprite").click();
    h.run();
    h.get_by_label_contains("Canvas Size").click();
    h.run();
    h.state_mut().menus.canvas_size.as_mut().expect("the sheet is open").width = 40;
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().tab().doc.width(), 40);
}

#[test]
fn history_panel_jumps_to_a_step() {
    let mut h = harness(|app| {
        red_dot(app);
        app.menus.history_open = true;
    });
    h.run();
    assert_eq!(h.state().tab().doc.history_counts().0, 1);
    h.get_by_label("Original").click();
    h.run();
    assert_eq!(h.state().tab().doc.history_counts(), (0, 1));
    assert_eq!(pixel(&h, 1, 1), 0x00000000);
}

#[test]
fn single_keys_switch_tools_and_can_be_rebound() {
    let mut h = harness(|_| {});
    h.key_press(Key::E);
    h.run();
    assert_eq!(h.state().tool, Tool::Eraser);
    // Rebind the pencil to J, the way the Keyboard tab does.
    h.state_mut().menus.shortcuts.recording = Some(Action::Tool(Tool::Pencil));
    h.run();
    h.key_press(Key::J);
    h.run();
    assert!(h.state().menus.shortcuts.recording.is_none());
    assert_eq!(h.state().menus.shortcuts.label(Action::Tool(Tool::Pencil)), "J");
    h.key_press(Key::J);
    h.run();
    assert_eq!(h.state().tool, Tool::Pencil);
    h.key_press(Key::B);
    h.run();
    assert_eq!(h.state().tool, Tool::Pencil, "B no longer selects a tool, so the tool stays");
}

#[test]
fn swap_colors_and_brush_size_keys() {
    let mut h = harness(|app| {
        app.primary = 0x112233FF;
        app.secondary = 0x445566FF;
    });
    h.key_press(Key::X);
    h.run();
    assert_eq!((h.state().primary, h.state().secondary), (0x445566FF, 0x112233FF));
    h.key_press(Key::CloseBracket);
    h.run();
    assert_eq!(h.state().brush_size, 2);
    h.key_press(Key::OpenBracket);
    h.run();
    assert_eq!(h.state().brush_size, 1);
}

#[test]
fn keys_do_nothing_while_a_dialog_is_open() {
    let mut h = harness(|app| app.error = Some("Oops".into()));
    h.key_press(Key::E);
    h.run();
    assert_eq!(h.state().tool, Tool::Pencil);
}

#[test]
fn preferences_window_has_the_keyboard_tab() {
    let mut h = harness(|app| app.menus.prefs_open = true);
    h.run();
    h.state_mut().menus.prefs_tab = PrefsTab::Keyboard;
    h.run();
    assert!(h.query_by_label("Restore Default Shortcuts").is_some());
    assert!(h.query_by_label("Ellipse").is_some());
    h.state_mut().menus.prefs_tab = PrefsTab::General;
    h.run();
    assert!(h.query_by_label("Restore Defaults").is_some());
}

#[test]
fn the_shortcuts_window_lists_menu_shortcuts() {
    let mut h = harness(|app| app.menus.help_open = true);
    h.run();
    assert!(h.query_by_label("Tools").is_some());
    assert!(h.query_by_label("Other keys").is_some());
    assert!(h.query_by_label("File menu").is_some());
}

/// A map standing in for eframe's storage.
#[derive(Default)]
struct Store(HashMap<String, String>);

impl eframe::Storage for Store {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.to_string(), value);
    }
    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }
    fn flush(&mut self) {}
}

#[test]
fn preferences_shortcuts_and_recents_persist() {
    let mut s = MenusState::default();
    s.prefs = Prefs { new_width: 64, default_zoom: 8, appearance: 2, ..Default::default() };
    s.recents = vec!["/tmp/a.aseprite".into(), "/tmp/b.aseprite".into()];
    let mut store = Store::default();
    s.save(&mut store);
    let back = MenusState::load(Some(&store));
    assert_eq!(back.prefs, s.prefs);
    assert_eq!(back.recents, s.recents);
    assert_eq!(back.shortcuts.label(Action::Tool(Tool::Pencil)), "B");
    assert_eq!(MenusState::load(None).prefs, Prefs::default());
}

#[test]
fn recently_opened_files_are_remembered() {
    let dir = std::env::temp_dir().join(format!("pixmith-menus-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("recent.aseprite");
    let mut h = harness(|_| {});
    h.state_mut().tab_mut().doc.save(&path).unwrap();
    h.run();
    assert_eq!(h.state().menus.recents_existing(), vec![path.clone()]);
    h.get_by_label("File").click();
    h.run();
    h.get_by_label_contains("Open Recent").click();
    h.run();
    assert!(h.query_by_label_contains("recent.aseprite").is_some());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn edit_warp_opens_handles_and_apply_is_one_undo_step() {
    let mut h = harness(red_dot);
    h.get_by_label("Edit").click();
    h.run();
    h.get_by_label_contains("Warp").click();
    h.run();
    assert!(h.state().selection.warp.is_some(), "the menu starts a warp session");
    // The canvas already shows the (identity) preview; move every handle one pixel right, as a drag would.
    h.state_mut().selection.warp.as_mut().unwrap().warp.offsets.fill((1.0, 0.0));
    let before = h.state().tab().doc.history_counts().0;
    assert!(h.state().canvas.preview.is_some(), "the canvas shows the preview while the warp is open");
    h.key_press(Key::Enter);
    h.run();
    assert!(h.state().canvas.preview.is_none());
    assert!(h.state().selection.warp.is_none());
    assert_eq!(pixel(&h, 2, 1), 0xFF0000FF);
    assert_eq!(pixel(&h, 1, 1), 0);
    assert_eq!(h.state().tab().doc.history_counts().0, before + 1);
}

#[test]
fn edit_warp_cancel_changes_nothing() {
    let mut h = harness(red_dot);
    h.get_by_label("Edit").click();
    h.run();
    h.get_by_label_contains("Warp").click();
    h.run();
    h.state_mut().selection.warp.as_mut().unwrap().warp.offsets.fill((3.0, 0.0));
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().canvas.preview.is_none());
    assert!(h.state().selection.warp.is_none());
    assert_eq!(pixel(&h, 1, 1), 0xFF0000FF);
}
