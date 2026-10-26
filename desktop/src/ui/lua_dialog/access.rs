//! The "ask first" dialog for scripts that touch files or run commands (`docs/LUA_HOST_API.md` section 1).
//!
//! The script's thread waits in `calls::script_access` until this window is answered: Allow Once, Always Allow for
//! This Script, or Deny (Escape, too).

use super::{LuaState, short_name};
use crate::app::PixmithApp;
use crate::calls::script_access::{self, Pending};
use crate::theme::{Colors, font, ui_font};
use crate::ui::widgets;
use eframe::egui;
use pixmith_core::scripting::access::Permission;

/// Which requests of scripts an app answers with a dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    /// None: the core denies them (no handler) or another app answers them.
    Off,
    /// Those of scripts this app runs (tests that run several apps in one process).
    Sessions,
    /// All of them (the real app: one window per process).
    All,
}

impl LuaState {
    /// Takes the queued requests this app answers.
    pub(super) fn take_requests(&self) -> Vec<Pending> {
        match self.permissions {
            Mode::Off => Vec::new(),
            Mode::All => script_access::take(|_| true),
            Mode::Sessions => {
                let names: Vec<&str> = self.sessions.values().map(|s| s.name.as_str()).collect();
                script_access::take(|r| names.contains(&r.script_name.as_str()))
            }
        }
    }
}

/// Draws the question on top of everything and hands the answer to the script. Call every frame.
pub(super) fn show(app: &mut PixmithApp, ctx: &egui::Context) {
    let taken = app.gamedev.lua.take_requests();
    app.gamedev.lua.asking.extend(taken);
    let Some(front) = app.gamedev.lua.asking.first() else { return };
    let (script, path, access) =
        (short_name(&front.request.script_name), front.request.path.clone(), front.request.access);
    let c = Colors::of(ctx);
    let title = if access == pixmith_core::scripting::access::Access::Execute {
        "Allow this script to run a command?"
    } else {
        "Allow this script to use a file?"
    };
    let question = format!("\u{201c}{script}\u{201d} wants to {} {path}", script_access::verb(access));
    let answer = widgets::dialog(ctx, "script_permission", 500.0, title, None, |ui| {
        ui.add(egui::Label::new(egui::RichText::new(&question).font(ui_font(font::BODY_LARGE)).color(c.text)).wrap());
        widgets::note(ui, script_access::always_note(access));
        ui.add_space(crate::theme::space::XS);
        let mut answer = None;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let once = widgets::primary_button(ui, "Allow Once", true);
                once.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Allow Once"));
                if once.clicked() {
                    answer = Some(Permission::AllowOnce);
                }
                let always = widgets::secondary_button(ui, "Always Allow for This Script", false);
                always.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Always Allow for This Script")
                });
                if always.clicked() {
                    answer = Some(Permission::AllowAlwaysForScript);
                }
                let deny = widgets::secondary_button(ui, "Deny", false);
                deny.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Deny"));
                if deny.clicked() {
                    answer = Some(Permission::Deny);
                }
            });
        });
        // Return does nothing here: a security question gets a deliberate click. Escape denies.
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            answer = Some(Permission::Deny);
        }
        answer
    });
    if let Some(permission) = answer {
        app.gamedev.lua.asking.remove(0).answer(permission);
        ctx.request_repaint();
    }
}
