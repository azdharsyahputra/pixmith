import AppKit
import SwiftUI
@testable import PixmithKit

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) { Prefs.applyAppearance() }

    func application(_ application: NSApplication, open urls: [URL]) {
        urls.forEach { SpriteModel.shared.requestOpen($0) }
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        SpriteModel.shared.terminateReply()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}

@main
struct PixmithApp: App {
    @NSApplicationDelegateAdaptor private var delegate: AppDelegate
    @ObservedObject private var model = SpriteModel.shared

    var body: some Scene {
        Window("Pixmith", id: "main") {
            ContentView(model: model).frame(minWidth: 860, minHeight: 520)
        }
        .commands {
            PixmithCommands(model: model)
            FileCommands(model: model)
            TabCommands(model: model)
            ScriptCommands(model: model)
            GodotCommands(model: model)
            FXCommands()
            LayerCommands(model: model)
            TilemapCommands(model: model)
            ViewTweenCommands(model: model)
            HelpCommands()
        }
        Window("Keyboard Shortcuts", id: "shortcuts") { ShortcutsHelpView() }
        Settings { PreferencesView() }
    }
}

struct PixmithCommands: Commands {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var sheet = SheetState.shared

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("New Sprite…") { model.requestNewSprite() }.keyboardShortcut("n")
            Button("Open…") { model.requestOpen() }.keyboardShortcut("o")
        }
        CommandGroup(replacing: .saveItem) {
            Button("Save") { model.save() }.keyboardShortcut("s")
            Button("Save As…") { model.saveAs() }.keyboardShortcut("s", modifiers: [.command, .shift])
            Divider()
            Button("Export PNG…") { model.exportPNG(sheet: false) }.keyboardShortcut("e", modifiers: [.command, .shift])
            Button("Export…") { model.showExportOptions() }.keyboardShortcut("e", modifiers: [.command, .option])
            Button("Export Sprite Sheet…") { model.exportSpriteSheet() }
        }
        CommandGroup(replacing: .undoRedo) {
            Button("Undo") { model.undo() }.keyboardShortcut("z")
            Button("Redo") { model.redo() }.keyboardShortcut("z", modifiers: [.command, .shift])
        }
        SelectionCommands(model: model)
        DrawingCommands(model: model)
        CommandMenu("Sprite") {
            Button("New Layer") { model.addLayer() }.keyboardShortcut("n", modifiers: [.command, .shift]).disabled(sheet.isOpen)
            Button("Delete Layer") { model.removeLayer() }.disabled(sheet.isOpen)
            Divider()
            Button("New Frame") { model.addFrame(duplicate: false) }.keyboardShortcut("n", modifiers: [.command, .option]).disabled(sheet.isOpen)
            Button("Duplicate Frame") { model.addFrame(duplicate: true) }.keyboardShortcut("d", modifiers: [.command, .shift]).disabled(sheet.isOpen)
            Button("Delete Frame") { model.removeFrame() }.disabled(sheet.isOpen)
            SpriteSelectionMenuItems(model: model)
            SlicesMenuItems(model: model)
            Divider()
            ColorModeMenu(model: model)
            Divider()
            Button("Zoom In") { model.zoom = min(64, model.zoom * 2) }.keyboardShortcut("=")
            Button("Zoom Out") { model.zoom = max(1, model.zoom / 2) }.keyboardShortcut("-")
            Toggle("Onion Skin", isOn: $model.onionSkin)
            Toggle("Grid", isOn: $model.showGrid).keyboardShortcut("'")
        }
    }
}

/// Both Godot export menus as one item: SwiftUI's commands builder takes at most ten.
struct GodotCommands: Commands {
    @ObservedObject var model: SpriteModel

    var body: some Commands {
        GodotExportCommands(model: model)
        GodotTilesetCommands(model: model)
    }
}
