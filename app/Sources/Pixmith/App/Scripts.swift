import AppKit
import PixmithCore
import SwiftUI
import UniformTypeIdentifiers

/// The user's script folder and the list of scripts found in it.
final class ScriptLibrary: ObservableObject {
    static let shared = ScriptLibrary()

    @Published private(set) var scripts: [URL] = []

    static var folder: URL {
        AppSupport.directory.appendingPathComponent("Pixmith/Scripts", isDirectory: true)
    }

    private init() { rescan() }

    func rescan() {
        let files = (try? FileManager.default.contentsOfDirectory(
            at: Self.folder, includingPropertiesForKeys: nil, options: [.skipsHiddenFiles])) ?? []
        scripts = files.filter { $0.pathExtension.lowercased() == "lua" }
            .sorted { $0.lastPathComponent.localizedStandardCompare($1.lastPathComponent) == .orderedAscending }
    }

    func openFolder() {
        try? FileManager.default.createDirectory(at: Self.folder, withIntermediateDirectories: true)
        NSWorkspace.shared.open(Self.folder)
    }

    /// Copies the example scripts that ship with the app into the folder (existing files are kept).
    func installExamples() {
        try? FileManager.default.createDirectory(at: Self.folder, withIntermediateDirectories: true)
        for i in 0..<pixmith_example_script_count() {
            guard let n = pixmith_example_script_name(i), let s = pixmith_example_script_source(i) else { continue }
            defer { pixmith_string_free(n); pixmith_string_free(s) }
            let url = Self.folder.appendingPathComponent(String(cString: n))
            if !FileManager.default.fileExists(atPath: url.path) {
                try? String(cString: s).write(to: url, atomically: true, encoding: .utf8)
            }
        }
        rescan()
    }
}

extension SpriteModel {
    /// Starts the script as a session (docs/LUA_DIALOG.md): it runs on a core thread against a snapshot, may show
    /// dialogs, and its result is applied as one undo step when it finishes. An error leaves the document untouched.
    func runScript(at url: URL) {
        guard !MainWindow.hasSheet else { return }
        guard let source = try? String(contentsOf: url, encoding: .utf8) else {
            errorMessage = "Could not read \(url.lastPathComponent)."
            return
        }
        // The full path is the script's identity for "Always Allow for This Script" (two scripts may share a file name).
        ScriptSessions.shared.start(source: source, name: url.standardizedFileURL.path)
    }

    func runScriptFromPanel() {
        let panel = NSOpenPanel()
        if let lua = UTType(filenameExtension: "lua") { panel.allowedContentTypes = [lua, .plainText] }
        panel.message = "Choose a Lua script to run"
        if panel.runModal() == .OK, let url = panel.url { runScript(at: url) }
    }
}

struct ScriptCommands: Commands {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var library = ScriptLibrary.shared
    @ObservedObject private var sheet = SheetState.shared
    @ObservedObject private var sessions = ScriptSessions.shared

    var body: some Commands {
        CommandGroup(after: .saveItem) {
            Menu("Scripts") {
                ForEach(library.scripts, id: \.self) { url in
                    Button(url.deletingPathExtension().lastPathComponent) { model.runScript(at: url) }
                }
                if library.scripts.isEmpty {
                    Button("No scripts found") {}.disabled(true)
                }
                if !sessions.commands.isEmpty {
                    Divider()
                    // Commands that plugins (including those of extensions) registered with plugin:newCommand.
                    ForEach(sessions.commands) { c in
                        Button {
                            sessions.invoke(c)
                        } label: {
                            if c.checked { Label(c.title, systemImage: "checkmark") } else { Text(c.title) }
                        }
                        .disabled(!c.enabled)
                    }
                }
                Divider()
                Button("Run Script…") { model.runScriptFromPanel() }
                Button("Open Scripts Folder") { library.openFolder() }
                Button("Install Example Scripts") { library.installExamples() }
                Button("Rescan") { library.rescan() }
            }
            .disabled(sheet.isOpen)
        }
    }
}
