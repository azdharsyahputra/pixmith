//! Renaming a document from its tab (double-click), like `renameDocument` in the macOS app.

use pixmith_desktop::PixmithApp;

#[test]
fn unsaved_documents_keep_the_name_for_the_tab() {
    let mut app = PixmithApp::default();
    app.rename_document(0, "  hero.aseprite ");
    assert_eq!(app.tab_name(0), "hero");
    app.rename_document(0, "   ");
    assert_eq!(app.tab_name(0), "hero", "an empty name is ignored");
}

#[test]
fn saved_documents_are_renamed_on_disk() {
    let dir = std::env::temp_dir().join(format!("pixmith-rename-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let old = dir.join("rename-me.aseprite");
    let mut app = PixmithApp::default();
    app.tab_mut().doc.save(&old).unwrap();

    app.rename_document(0, "walk-cycle");
    let new = dir.join("walk-cycle.aseprite");
    assert_eq!(app.tab().doc.path.as_deref(), Some(new.as_path()));
    assert_eq!(app.tab_name(0), "walk-cycle.aseprite");
    assert!(new.exists() && !old.exists());

    std::fs::write(dir.join("taken.aseprite"), b"").unwrap();
    app.rename_document(0, "taken");
    assert_eq!(app.tab().doc.path.as_deref(), Some(new.as_path()), "an existing file is not overwritten");
    assert!(app.error.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}
