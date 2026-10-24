//! Screenshots of a Lua dialog that uses every widget, modal and as a tool window, next to the macOS ones in
//! `docs/design/screens/` (`lua-dialog-*@2x.png`). Needs a GPU (wgpu), so they are ignored by default:
//!
//! ```sh
//! cargo test --test lua_dialog_screens -- --ignored    # writes target/screens/lua-dialog-<name>-<theme>.png
//! ```

use eframe::egui;
use egui_kittest::Harness;
use pixmith_desktop::script_ui::FakeHost;
use pixmith_desktop::{PixmithApp, theme};

/// A dialog with one of everything: the same JSON the core produces for `scripts/lua-examples/dialog-gallery.lua`.
pub fn gallery(modal: bool) -> String {
    format!(
        r##"{{"type":"dialog.open","session":3,"dialog":{{"id":"s3.d1","title":"Palette ramp","modal":{modal},"notitlebar":false,"rows":[
  {{"widgets":[{{"id":"info","kind":"label","text":"Build a ramp between two colors and name it.","enabled":true,"visible":true}}]}},
  {{"widgets":[{{"id":"name","kind":"entry","label":"Name","text":"Sunset","enabled":true,"visible":true,"hexpand":true}}]}},
  {{"widgets":[{{"id":"from","kind":"color","label":"From","color":488714575,"enabled":true,"visible":true,"hexpand":true}},
              {{"id":"to","kind":"color","label":"To","color":4289790975,"enabled":true,"visible":true,"hexpand":true}}]}},
  {{"widgets":[{{"id":"steps","kind":"slider","label":"Steps","min":2,"max":32,"value":12,"enabled":true,"visible":true,"hexpand":true}}]}},
  {{"widgets":[{{"id":"gamma","kind":"number","label":"Gamma","value":2.2,"decimals":1,"enabled":true,"visible":true,"hexpand":true}}]}},
  {{"widgets":[{{"id":"mode","kind":"combobox","label":"Blend","options":["Nearest","Linear","Cubic"],"option":"Linear","enabled":true,"visible":true,"hexpand":true}}]}},
  {{"widgets":[{{"id":"out","kind":"file","label":"Save to","filename":"/Users/me/ramps/sunset.gpl","open":false,"save":true,"filetypes":["gpl"],"title":"Save palette","enabled":true,"visible":true,"hexpand":true}}]}},
  {{"widgets":[{{"id":"pal","kind":"shades","label":"Preview","colors":[488714575,2132288255,4289790975,4293721855,4294960383],"enabled":true,"visible":true}}]}},
  {{"widgets":[{{"id":"_w9","kind":"separator","text":"Options"}}]}},
  {{"widgets":[{{"id":"dither","kind":"check","text":"Dither","selected":true,"enabled":true,"visible":true}},
              {{"id":"loop","kind":"check","text":"Wrap around","selected":false,"enabled":true,"visible":true}}]}},
  {{"widgets":[{{"id":"r1","kind":"radio","text":"RGB","selected":true,"group":1,"enabled":true,"visible":true}},
              {{"id":"r2","kind":"radio","text":"HSV","selected":false,"group":1,"enabled":true,"visible":true}},
              {{"id":"r3","kind":"radio","text":"Disabled","selected":false,"group":1,"enabled":false,"visible":true}}]}},
  {{"widgets":[{{"id":"_w12","kind":"separator","text":""}}]}},
  {{"widgets":[{{"id":"ok","kind":"button","text":"&OK","focus":true,"enabled":true,"visible":true}},
              {{"id":"cancel","kind":"button","text":"Cancel","enabled":true,"visible":true}},
              {{"id":"help","kind":"button","text":"Help","enabled":false,"visible":true}}]}}
]}}}}"##
    )
}

fn render(name: &str, modal: bool) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/screens");
    std::fs::create_dir_all(&dir).unwrap();
    for dark in [true, false] {
        let host = FakeHost::new();
        let mut app = PixmithApp { dark, ..Default::default() };
        app.gamedev.lua.set_host(Box::new(host.clone()));
        host.push(r#"{"type":"state","session":3,"state":"running"}"#);
        host.push(&gallery(modal));
        host.push(&format!(r#"{{"type":"state","session":3,"state":"{}"}}"#, if modal { "waiting" } else { "idle" }));
        let mut installed = false;
        let mut harness = Harness::builder().with_size([1280.0, 800.0]).with_pixels_per_point(2.0).build_ui_state(
            |ui, app: &mut PixmithApp| {
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
        for _ in 0..8 {
            harness.step();
        }
        let image = harness.render().expect("render");
        let path = dir.join(format!("lua-dialog-{name}-{}.png", if dark { "dark" } else { "light" }));
        image.save(&path).unwrap();
        println!("wrote {}", path.display());
    }
}

#[test]
#[ignore = "needs a GPU"]
fn modal_dialog_with_every_widget() {
    render("modal", true);
}

#[test]
#[ignore = "needs a GPU"]
fn tool_window_with_every_widget() {
    render("window", false);
}
