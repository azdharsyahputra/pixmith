//! Home, the project view, the project switcher and the `.pxmt` file flow, driven headlessly (no GPU needed).

mod common;

use common::{harness, loaded, make_project, scratch, settle};
use eframe::egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use pixmith_desktop::PixmithApp;
use pixmith_desktop::doc::Doc;
use pixmith_desktop::projects as calls;
use pixmith_desktop::ui::projects::Screen;
use std::path::Path;

fn names(app: &PixmithApp, folder: &Path) -> Vec<String> {
    app.projects.scans.project(folder).map(|p| p.canvases.iter().map(|c| c.file_name()).collect()).unwrap_or_default()
}

/// Runs frames until the project view lists `expected` (the scans run in the background).
fn wait_for_names(h: &mut egui_kittest::Harness<'static, PixmithApp>, folder: &Path, expected: &[&str]) {
    let folder = folder.to_path_buf();
    let want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    settle(h, |app| loaded(app) && names(app, &folder) == want);
}

fn magic(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap().into_iter().take(8).collect()
}

fn is_aseprite(path: &Path) -> bool {
    let b = std::fs::read(path).unwrap();
    b.len() > 6 && u16::from_le_bytes([b[4], b[5]]) == 0xA5E0
}

// --- Home ------------------------------------------------------------------------------------------------

#[test]
fn home_lists_recent_projects_and_opens_one() {
    let root = scratch("home");
    let forest = make_project(&root, "Forest", &[("hero", 16, 16), ("tree", 16, 24)]);
    let dungeon = make_project(&root, "Dungeon", &[("wall", 16, 16)]);
    let mut app = PixmithApp::default();
    app.projects.remember_project(&dungeon);
    app.projects.remember_project(&forest);
    app.menus.recents = vec![forest.join("hero.pxmt")];
    app.go_home();
    let mut h = harness(app);
    settle(&mut h, loaded);

    for label in ["New Project", "Recent", "Projects", "Open Project\u{2026}"] {
        assert!(h.query_all_by_label(label).next().is_some(), "{label} is missing");
    }
    assert!(h.query_by_label("hero").is_some(), "the recent canvas card");
    h.query_all_by_label("Dungeon").last().unwrap().click();
    h.run();
    assert_eq!(h.state().projects.screen, Screen::Project(dungeon.clone()));
    settle(&mut h, loaded);
    assert!(h.query_by_label("wall").is_some());
    assert!(h.query_by_label("New Canvas").is_some());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn home_search_filters_and_sort_orders_projects() {
    let root = scratch("home-search");
    let a = make_project(&root, "Alpha", &[("a", 8, 8)]);
    let b = make_project(&root, "Beta", &[("b", 8, 8)]);
    let mut app = PixmithApp::default();
    app.projects.remember_project(&a);
    app.projects.remember_project(&b);
    app.go_home();
    let mut h = harness(app);
    settle(&mut h, loaded);
    // Each project is in the sidebar list and in the grid.
    let count = |h: &egui_kittest::Harness<'static, PixmithApp>, l: &str| h.query_all_by_label(l).count();
    assert_eq!((count(&h, "Alpha"), count(&h, "Beta")), (2, 2));
    h.state_mut().projects.search = "alp".into();
    h.run();
    assert_eq!((count(&h, "Alpha"), count(&h, "Beta")), (2, 1), "Beta leaves the grid, not the sidebar");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn opening_a_document_skips_home_and_a_folder_opens_a_project() {
    let root = scratch("args");
    let project = make_project(&root, "P", &[("one", 8, 8)]);
    let mut app = PixmithApp::default();
    app.go_home();
    app.open_args([project.join("one.pxmt")]);
    assert_eq!(app.projects.screen, Screen::Editor);
    assert_eq!(app.tab().doc.path.as_deref(), Some(project.join("one.pxmt").as_path()));
    app.go_home();
    assert_eq!(app.projects.screen, Screen::Home);
    app.open_args([project.clone()]);
    assert_eq!(app.projects.screen, Screen::Project(project.clone()));
    assert_eq!(app.projects.recent[0].path, project);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn file_home_and_its_shortcut_go_home_and_the_tabs_stay() {
    let root = scratch("shortcut");
    let project = make_project(&root, "P", &[("one", 8, 8)]);
    let mut app = PixmithApp::default();
    app.open_canvas(&project.join("one.pxmt"));
    let mut h = harness(app);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::H);
    h.run();
    assert_eq!(h.state().projects.screen, Screen::Home);
    assert_eq!(h.state().tabs.len(), 1, "the document stays open");
    // The document's tab stays on the title row; showing the editor again goes back to it.
    h.state_mut().show_editor();
    h.run();
    assert_eq!(h.state().tab().doc.path.as_deref(), Some(project.join("one.pxmt").as_path()));
    let _ = std::fs::remove_dir_all(root);
}

// --- Project view ------------------------------------------------------------------------------------------

#[test]
fn the_project_view_shows_canvases_and_opens_one_with_enter_after_arrow_keys() {
    let root = scratch("view");
    let project = make_project(&root, "P", &[("a", 8, 8), ("b", 16, 8), ("c", 8, 16)]);
    let mut app = PixmithApp::default();
    app.open_project(&project);
    let mut h = harness(app);
    wait_for_names(&mut h, &project, &["a.pxmt", "b.pxmt", "c.pxmt"]);
    assert!(h.query_by_label("3 canvases").is_some() || h.query_by_label_contains("3 canvases").is_some());
    for n in ["a", "b", "c"] {
        assert!(h.query_by_label(n).is_some(), "card {n}");
    }
    h.key_press(Key::ArrowRight);
    h.run();
    assert_eq!(
        h.state().projects.view.selected.as_deref(),
        Some(project.join("a.pxmt").as_path()),
        "the first arrow selects the first"
    );
    h.key_press(Key::ArrowRight);
    h.run();
    assert_eq!(h.state().projects.view.selected.as_deref(), Some(project.join("b.pxmt").as_path()));
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().projects.screen, Screen::Editor);
    assert_eq!(h.state().tab().doc.path.as_deref(), Some(project.join("b.pxmt").as_path()));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn clicking_a_card_selects_it_and_return_opens_it() {
    let root = scratch("click");
    let project = make_project(&root, "P", &[("a", 8, 8), ("b", 8, 8)]);
    let mut app = PixmithApp::default();
    app.open_project(&project);
    let mut h = harness(app);
    wait_for_names(&mut h, &project, &["a.pxmt", "b.pxmt"]);
    h.get_by_label("b").click();
    h.run();
    assert_eq!(h.state().projects.view.selected.as_deref(), Some(project.join("b.pxmt").as_path()));
    assert_eq!(h.state().projects.screen, Screen::Project(project.clone()), "one click only selects");
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().projects.screen, Screen::Editor);
    assert_eq!(h.state().tab().doc.path.as_deref(), Some(project.join("b.pxmt").as_path()));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn delete_key_trashes_the_selection_and_undo_puts_it_back_in_place() {
    let root = scratch("trash");
    let project = make_project(&root, "P", &[("a", 8, 8), ("b", 8, 8), ("c", 8, 8)]);
    let mut app = PixmithApp::default();
    app.open_project(&project);
    let mut h = harness(app);
    wait_for_names(&mut h, &project, &["a.pxmt", "b.pxmt", "c.pxmt"]);
    h.state_mut().projects.view.selected = Some(project.join("b.pxmt"));
    h.key_press(Key::Delete);
    h.run_ok();
    wait_for_names(&mut h, &project, &["a.pxmt", "c.pxmt"]);
    assert!(!project.join("b.pxmt").exists());
    // The next card is selected once the rescan that follows the trash has landed.
    let next = project.join("c.pxmt");
    settle(&mut h, |app| app.projects.view.selected.as_deref() == Some(next.as_path()));
    assert_eq!(
        h.state().projects.view.selected.as_deref(),
        Some(project.join("c.pxmt").as_path()),
        "the next card is selected"
    );
    assert!(h.state().projects.view.toast.is_some());
    assert!(h.query_by_label_contains("Moved").is_some());
    h.get_by_label("Undo").click();
    h.run_ok();
    wait_for_names(&mut h, &project, &["a.pxmt", "b.pxmt", "c.pxmt"]);
    assert!(project.join("b.pxmt").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn canvas_operations_rename_duplicate_trash_restore_and_reorder() {
    let root = scratch("ops");
    let project = make_project(&root, "P", &[("a", 8, 8), ("b", 8, 8), ("c", 8, 8)]);
    let mut app = PixmithApp::default();
    app.open_project(&project);
    let mut h = harness(app);
    wait_for_names(&mut h, &project, &["a.pxmt", "b.pxmt", "c.pxmt"]);

    h.state_mut().rename_canvas(&project.join("a.pxmt"), "alpha");
    wait_for_names(&mut h, &project, &["alpha.pxmt", "b.pxmt", "c.pxmt"]);
    h.state_mut().rename_canvas(&project.join("b.pxmt"), "alpha");
    assert!(h.state().error.is_some(), "an existing name is refused");
    h.state_mut().error = None;

    h.state_mut().duplicate_canvas(&project.join("alpha.pxmt"));
    wait_for_names(&mut h, &project, &["alpha.pxmt", "alpha copy.pxmt", "b.pxmt", "c.pxmt"]);

    h.state_mut().trash_canvas(&project.join("b.pxmt"));
    wait_for_names(&mut h, &project, &["alpha.pxmt", "alpha copy.pxmt", "c.pxmt"]);
    assert_eq!(h.state().projects.view.trash.len(), 1);
    let trashed = h.state().projects.view.trash[0].0.clone();
    h.state_mut().restore_canvas(&trashed, Some(2));
    wait_for_names(&mut h, &project, &["alpha.pxmt", "alpha copy.pxmt", "b.pxmt", "c.pxmt"]);

    // Dragging c to the front saves the order, and it survives a new scan.
    h.state_mut().projects.view.drag = None;
    let order = ["c.pxmt", "alpha.pxmt", "alpha copy.pxmt", "b.pxmt"].map(String::from);
    calls::set_order(&project, &order).unwrap();
    assert_eq!(
        calls::scan_project(&project).unwrap().canvases.iter().map(|c| c.file_name()).collect::<Vec<_>>(),
        order
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn renaming_an_open_canvas_follows_the_tab_and_deleting_it_is_refused() {
    let root = scratch("open-ops");
    let project = make_project(&root, "P", &[("a", 8, 8)]);
    let mut app = PixmithApp::default();
    app.open_canvas(&project.join("a.pxmt"));
    app.rename_canvas(&project.join("a.pxmt"), "renamed");
    assert_eq!(app.tab().doc.path.as_deref(), Some(project.join("renamed.pxmt").as_path()));
    app.trash_canvas(&project.join("renamed.pxmt"));
    assert!(app.error.as_deref().is_some_and(|e| e.contains("open in a tab")));
    assert!(project.join("renamed.pxmt").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_new_canvas_is_created_and_opened_in_a_tab() {
    let root = scratch("new-canvas");
    let project = make_project(&root, "P", &[("a", 8, 8)]);
    let mut app = PixmithApp::default();
    app.open_project(&project);
    app.create_canvas(&project, "Sprite", 20, 10);
    assert_eq!(app.projects.screen, Screen::Editor);
    let tab = app.tab();
    assert_eq!(tab.doc.path.as_deref(), Some(project.join("Sprite.pxmt").as_path()));
    assert_eq!((tab.doc.width(), tab.doc.height()), (20, 10));
    assert_eq!(app.files.new_size, (20, 10));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn hundreds_of_canvases_only_load_what_is_visible() {
    let root = scratch("many");
    let project = root.join("Big");
    calls::create_project(&project, "Big").unwrap();
    for i in 0..400 {
        calls::new_canvas(&project, &format!("canvas {i:03}"), 4, 4).unwrap();
    }
    let mut app = PixmithApp::default();
    app.open_project(&project);
    let mut h = harness(app);
    settle(&mut h, |app| loaded(app) && app.projects.scans.project(&project).is_some_and(|p| p.canvases.len() == 400));
    h.run_ok();
    let requested = h.state().projects.thumbs.len();
    assert!(requested > 0, "the first rows load");
    assert!(requested < 60, "only visible rows are laid out, {requested} thumbnails were requested");
    let _ = std::fs::remove_dir_all(root);
}

// --- The project switcher -----------------------------------------------------------------------------------

#[test]
fn the_switcher_lists_the_projects_canvases_and_opens_another() {
    let root = scratch("switcher");
    let project = make_project(&root, "Forest", &[("hero", 8, 8), ("tree", 8, 8)]);
    let mut app = PixmithApp::default();
    app.open_canvas(&project.join("hero.pxmt"));
    let mut h = harness(app);
    settle(&mut h, |app| app.projects.scans.project(&project).is_some());
    h.get_by_label("Forest").click();
    h.run_ok();
    settle(&mut h, loaded);
    assert!(h.query_by_label("tree").is_some(), "the menu lists the other canvas");
    assert!(h.query_by_label("Back to Home").is_some());
    h.get_by_label("tree").click();
    h.run_ok();
    assert_eq!(h.state().tab().doc.path.as_deref(), Some(project.join("tree.pxmt").as_path()));
    assert_eq!(h.state().tabs.len(), 2);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn back_to_home_in_the_switcher_menu_goes_home() {
    let root = scratch("switcher-home");
    let project = make_project(&root, "Forest", &[("hero", 8, 8)]);
    let mut app = PixmithApp::default();
    app.open_canvas(&project.join("hero.pxmt"));
    let mut h = harness(app);
    settle(&mut h, |app| app.projects.scans.project(&project).is_some());
    h.get_by_label("Forest").click();
    h.run_ok();
    h.get_by_label("Back to Home").click();
    h.run_ok();
    assert_eq!(h.state().projects.screen, Screen::Home);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_document_outside_a_project_gets_a_plain_home_button() {
    let mut h = harness(PixmithApp::default());
    h.get_by_label("Home").click();
    h.run_ok();
    assert_eq!(h.state().projects.screen, Screen::Home);
}

// --- .pxmt and .aseprite -------------------------------------------------------------------------------------

#[test]
fn pxmt_is_the_native_format_and_aseprite_stays_importable_and_exportable() {
    let dir = scratch("formats");
    let mut doc = common::art(1, 12, 9);
    let pxmt = dir.join("art.pxmt");
    doc.save(&pxmt).unwrap();
    assert!(!is_aseprite(&pxmt), "a .pxmt file is not an Aseprite file");
    assert_eq!(&magic(&pxmt)[..4], b"PXMT", "{:?}", magic(&pxmt));

    let ase = dir.join("art.aseprite");
    doc.save_aseprite(&ase).unwrap();
    assert!(is_aseprite(&ase));
    assert_eq!(doc.path.as_deref(), Some(pxmt.as_path()), "exporting leaves the document's own path");
    // The .pxmt extension decides, not the name of the export.
    let odd = dir.join("odd.pxmt");
    doc.save_aseprite(&odd).unwrap();
    assert!(is_aseprite(&odd));

    for path in [&pxmt, &ase, &odd] {
        let d = Doc::open(path).unwrap();
        assert_eq!((d.width(), d.height()), (12, 9), "{}", path.display());
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn saving_an_opened_aseprite_writes_back_to_that_aseprite() {
    let dir = scratch("keep-format");
    let ase = dir.join("old.aseprite");
    common::art(2, 10, 10).save_aseprite(&ase).unwrap();
    let mut app = PixmithApp::default();
    app.open_path(&ase);
    app.tab_mut().doc.touch();
    assert!(app.save());
    assert!(is_aseprite(&ase), "still an Aseprite file");
    assert_eq!(app.tab().doc.path.as_deref(), Some(ase.as_path()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn renaming_a_tab_keeps_the_extension() {
    let dir = scratch("rename-ext");
    let mut app = PixmithApp::default();
    app.tab_mut().doc.save(&dir.join("first.pxmt")).unwrap();
    app.rename_document(0, "second.pxmt");
    assert_eq!(app.tab_name(0), "second.pxmt");
    app.rename_document(0, "third");
    assert_eq!(app.tab_name(0), "third.pxmt");
    assert!(dir.join("third.pxmt").exists() && !dir.join("first.pxmt").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn import_into_a_project_converts_to_pxmt() {
    let dir = scratch("import");
    let project = make_project(&dir, "P", &[]);
    let ase = dir.join("old.aseprite");
    common::art(3, 14, 6).save_aseprite(&ase).unwrap();
    let png = dir.join("pic.png");
    Doc::new(5, 5).export_png(0, 1, &png).unwrap();
    let a = calls::import_into_project(&project, &ase).unwrap();
    let b = calls::import_into_project(&project, &png).unwrap();
    assert!(!is_aseprite(&a) && a.extension().unwrap() == "pxmt");
    assert!(b.extension().unwrap() == "pxmt");
    let p = calls::scan_project(&project).unwrap();
    assert_eq!(p.canvases.len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}
