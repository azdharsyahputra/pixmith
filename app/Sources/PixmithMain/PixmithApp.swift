import AppKit
import SwiftUI
@testable import PixmithKit

final class AppDelegate: NSObject, NSApplicationDelegate {
    /// Files that arrived before the window existed.
    private var pendingOpens: [URL] = []
    private var launched = false

    func applicationWillFinishLaunching(_ notification: Notification) {
        // Files from Finder, the Dock or `open` arrive as an Open Documents event. SwiftUI's own handler looks for a
        // scene to take the file, finds none for the single main Window, and closes that window, which quits the app.
        // Taking the event here first keeps the window and opens the file in a tab.
        NSAppleEventManager.shared().setEventHandler(
            self, andSelector: #selector(handleOpenDocuments(_:withReply:)),
            forEventClass: AEEventClass(kCoreEventClass), andEventID: AEEventID(kAEOpenDocuments))
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        Prefs.applyAppearance()
        LiveServer.shared.applyPreference()
        // Scripts that touch files or run commands ask first.
        ScriptPermissions.install()
        // Scripts that enabled extensions contribute load as plugins.
        PluginHost.shared.syncExtensions()
        launched = true
        let urls = pendingOpens
        pendingOpens = []
        if !urls.isEmpty { DispatchQueue.main.async { self.open(urls) } }
    }

    @objc private func handleOpenDocuments(_ event: NSAppleEventDescriptor, withReply reply: NSAppleEventDescriptor) {
        guard let list = event.paramDescriptor(forKeyword: keyDirectObject) else { return }
        let urls = list.numberOfItems == 0
            ? [list.fileURLValue].compactMap { $0 }
            : (1...list.numberOfItems).compactMap { list.atIndex($0)?.fileURLValue }
        if launched { open(urls) } else { pendingOpens += urls }
    }

    func applicationWillTerminate(_ notification: Notification) {
        ScriptSessions.shared.stopAll()
        LiveServer.shared.stop()
    }

    /// Files and project folders from Finder, the Dock or `open`; opening one skips the Home screen.
    func application(_ application: NSApplication, open urls: [URL]) {
        open(urls)
    }

    private func open(_ urls: [URL]) {
        NSApp.activate(ignoringOtherApps: true)
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
    @ObservedObject private var route = AppRoute.shared

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("New Sprite…") { model.requestNewSprite() }.keyboardShortcut("n")
            Button("New Project…") { route.requestNewProject() }
            Button("Open…") { model.requestOpen() }.keyboardShortcut("o")
            Button("Open Project…") { route.openProjectPanel() }.keyboardShortcut("o", modifiers: [.command, .shift])
            Button("Home") { route.showHome(model) }.keyboardShortcut("h", modifiers: [.command, .shift])
                .disabled(route.screen == .home)
        }
        CommandGroup(replacing: .saveItem) {
            Group {
                Button("Save") { model.save() }.keyboardShortcut("s")
                Button("Save As…") { model.saveAs() }.keyboardShortcut("s", modifiers: [.command, .shift])
                Divider()
                Button("Export PNG…") { model.exportPNG(sheet: false) }.keyboardShortcut("e", modifiers: [.command, .shift])
                Button("Export…") { model.showExportOptions() }.keyboardShortcut("e", modifiers: [.command, .option])
                Button("Export Sprite Sheet…") { model.exportSpriteSheet() }
                Button("Export As Aseprite…") { model.exportAseprite() }
            }
            .disabled(!route.isEditor)
        }
        CommandGroup(replacing: .undoRedo) {
            Button("Undo") { ScriptSessions.shared.command("Undo") { model.undo() } }.keyboardShortcut("z")
            Button("Redo") { ScriptSessions.shared.command("Redo") { model.redo() } }.keyboardShortcut("z", modifiers: [.command, .shift])
        }
        SelectionCommands(model: model)
        DrawingCommands(model: model)
        CommandMenu("Sprite") {
            Button("New Layer") { ScriptSessions.shared.command("New Layer") { model.addLayer() } }.keyboardShortcut("n", modifiers: [.command, .shift]).disabled(sheet.isOpen)
            Button("Delete Layer") { ScriptSessions.shared.command("Delete Layer") { model.removeLayer() } }.disabled(sheet.isOpen)
            Divider()
            Button("New Frame") { ScriptSessions.shared.command("New Frame") { model.addFrame(duplicate: false) } }.keyboardShortcut("n", modifiers: [.command, .option]).disabled(sheet.isOpen)
            Button("Duplicate Frame") { ScriptSessions.shared.command("Duplicate Frame") { model.addFrame(duplicate: true) } }.keyboardShortcut("d", modifiers: [.command, .shift]).disabled(sheet.isOpen)
            Button("Delete Frame") { ScriptSessions.shared.command("Delete Frame") { model.removeFrame() } }.disabled(sheet.isOpen)
            SpriteDataSpriteMenuItems(model: model).disabled(sheet.isOpen)
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
