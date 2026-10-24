//! Screenshot scenarios for Home, the project view and the project switcher, written to
//! `target/screens/<name>-<dark|light>.png` at the macOS screenshot size (1280 x 800 points, 2x):
//!
//! ```sh
//! cargo test --test projects_screens -- --ignored
//! ```

mod common;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pixmith_desktop::PixmithApp;
use std::path::{Path, PathBuf};

/// Sample projects under `root`; returns their folders: Forest Quest (14 canvases), Dungeon Tiles, Hero
/// Animations, Space Jam, UI Kit and Pixel Pets.
fn demo_projects(root: &Path) -> Vec<PathBuf> {
    let forest = [
        ("hero", 32, 32),
        ("hero-run", 48, 32),
        ("oak tree", 32, 48),
        ("pine tree", 32, 48),
        ("campfire", 24, 24),
        ("mushrooms", 16, 16),
        ("fox", 32, 24),
        ("river tiles", 64, 64),
        ("moon", 32, 32),
        ("owl", 24, 32),
        ("bridge", 64, 32),
        ("treasure chest", 24, 24),
        ("title screen", 96, 64),
        ("cloud", 32, 16),
    ];
    let dungeon = [
        ("floor", 16, 16),
        ("wall", 16, 16),
        ("door", 16, 32),
        ("torch", 8, 16),
        ("tileset", 128, 128),
        ("skeleton", 32, 32),
    ];
    let heroes = [("idle", 32, 32), ("walk", 32, 32), ("attack", 48, 32), ("jump", 32, 32)];
    let space = [
        ("ship", 32, 32),
        ("asteroid", 24, 24),
        ("laser", 8, 16),
        ("boss", 64, 64),
        ("stars", 96, 64),
        ("planet", 48, 48),
        ("explosion", 32, 32),
        ("shield", 24, 24),
        ("coin", 16, 16),
        ("hud", 96, 32),
        ("logo", 80, 32),
        ("nebula", 96, 64),
    ];
    let kit = [("buttons", 64, 32), ("icons", 64, 64), ("frames", 96, 96)];
    let pets = [("cat", 32, 32), ("dog", 32, 32)];
    vec![
        common::make_project(root, "Forest Quest", &forest),
        common::make_project(root, "Dungeon Tiles", &dungeon),
        common::make_project(root, "Hero Animations", &heroes),
        common::make_project(root, "Space Jam", &space),
        common::make_project(root, "UI Kit", &kit),
        common::make_project(root, "Pixel Pets", &pets),
    ]
}

/// Renders a projects scene at the macOS screenshot size once the background loads are done.
fn render_projects(
    name: &str,
    mut setup: impl FnMut(&mut PixmithApp, &[PathBuf]),
    mut interact: impl FnMut(&mut Harness<'static, PixmithApp>),
) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/screens");
    std::fs::create_dir_all(&dir).unwrap();
    let root = common::scratch(&format!("screens-{name}"));
    let folders = demo_projects(&root);
    for dark in [true, false] {
        let mut app = PixmithApp { dark, ..Default::default() };
        setup(&mut app, &folders);
        let mut h = common::harness_sized(app, [1280.0, 800.0], 2.0);
        common::settle(&mut h, common::loaded);
        interact(&mut h);
        common::settle(&mut h, common::loaded);
        h.run_ok();
        let image = h.render().expect("render");
        let path = dir.join(format!("{name}-{}.png", if dark { "dark" } else { "light" }));
        image.save(&path).unwrap();
        println!("wrote {}", path.display());
    }
    let _ = std::fs::remove_dir_all(root);
}

fn remember_all(app: &mut PixmithApp, folders: &[PathBuf]) {
    // Oldest first, so the first project ends up most recent.
    for f in folders.iter().rev() {
        app.projects.remember_project(f);
    }
    app.menus.recents = [
        folders[0].join("hero.pxmt"),
        folders[3].join("ship.pxmt"),
        folders[0].join("oak tree.pxmt"),
        folders[1].join("tileset.pxmt"),
        folders[2].join("walk.pxmt"),
        folders[4].join("buttons.pxmt"),
        folders[0].join("fox.pxmt"),
    ]
    .to_vec();
}

/// The Home screen: Recent canvases strip and the projects grid.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn home() {
    render_projects(
        "home",
        |app, folders| {
            remember_all(app, folders);
            app.go_home();
        },
        |_| {},
    );
}

/// Home without any project yet.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn home_empty() {
    render_projects("home-empty", |app, _| app.go_home(), |_| {});
}

/// The New Project sheet over Home.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn new_project_sheet() {
    render_projects(
        "new-project",
        |app, folders| {
            remember_all(app, folders);
            app.go_home();
            app.projects.new_project = Some(pixmith_desktop::ui::projects::sheets::NewProject::new(&app.projects));
        },
        |_| {},
    );
}

/// A project with 14 canvases, one selected.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn project() {
    render_projects(
        "project",
        |app, folders| {
            remember_all(app, folders);
            app.open_project(&folders[0]);
            app.projects.view.selected = Some(folders[0].join("oak tree.pxmt"));
        },
        |_| {},
    );
}

/// A canvas deleted: the Undo toast and the Recently deleted list.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn project_deleted() {
    render_projects(
        "project-deleted",
        |app, folders| {
            remember_all(app, folders);
            app.open_project(&folders[1]);
        },
        |h| {
            let state = h.state();
            let folder = state.projects.folder().map(Path::to_path_buf).unwrap();
            let doomed = state.projects.scans.project(&folder).map(|p| p.canvases[3].path.clone()).unwrap();
            h.state_mut().trash_canvas(&doomed);
            h.state_mut().projects.view.trash_open = true;
            h.step();
            h.step();
        },
    );
}

/// An empty project.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn project_empty() {
    render_projects(
        "project-empty",
        |app, folders| {
            let empty = folders[0].parent().unwrap().join("Fresh Project");
            pixmith_desktop::projects::create_project(&empty, "Fresh Project").unwrap();
            app.open_project(&empty);
        },
        |_| {},
    );
}

/// The editor with a canvas of a project open and the project switcher menu showing its canvases.
#[test]
#[ignore = "needs a GPU; run with --ignored"]
fn project_switcher() {
    render_projects(
        "switcher",
        |app, folders| {
            remember_all(app, folders);
            app.open_canvas(&folders[0].join("hero.pxmt"));
        },
        |h| {
            h.get_by_label("Forest Quest").click();
            h.run();
            common::settle(h, common::loaded);
        },
    );
}
