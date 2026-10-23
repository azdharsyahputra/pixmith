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
    /// The script runs on a snapshot off the main thread behind a progress sheet with Cancel, so the UI stays
    /// responsive and the document is not touched while it runs; a successful result is applied as one undo step.
    func runScript(at url: URL) {
        guard !MainWindow.hasSheet else { return }
        guard let source = try? String(contentsOf: url, encoding: .utf8) else {
            errorMessage = "Could not read \(url.lastPathComponent)."
            return
        }
        finishTransients()  // a floating paste or open shape must not be snapshotted half-lifted
        guard let job = pixmith_snapshot(doc) else { return }
        let (target, active) = (doc, (UInt32(layer), UInt32(frame)))
        pixmith_script_cancel(false)
        let started = Background.run(title: "Running \(url.lastPathComponent)…", immediate: true, cancel: { pixmith_script_cancel(true) },
                                     work: { () -> (ok: Bool, messages: String, failure: String) in
            var out: UnsafeMutablePointer<CChar>?
            let ok = source.withCString { pixmith_run_script_at(job, $0, active.0, active.1, &out) }
            let messages = out.map { String(cString: $0) } ?? ""
            if let out { pixmith_string_free(out) }
            return (ok, messages, ok ? "" : String(cString: pixmith_last_error()))
        }, done: { [weak self] result in
            defer { pixmith_free(job) }
            guard let self else { return }
            if result.ok, self.doc == target { _ = pixmith_job_apply(target, job) }
            self.scriptFinished(name: url.lastPathComponent, messages: result.messages, failure: result.ok ? nil : result.failure)
        })
        if !started { pixmith_free(job) }
    }

    private func scriptFinished(name: String, messages raw: String, failure: String?) {
        var lines = raw.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        // The last line may be "\u{1}PIXMITH-SELECT layer=<n> frame=<n>" (1-based): the script moved the selection.
        if let last = lines.last, last.hasPrefix("\u{1}PIXMITH-SELECT") {
            lines.removeLast()
            let pairs = last.split(separator: " ").dropFirst().compactMap { part -> (String, Int)? in
                let kv = part.split(separator: "=")
                return kv.count == 2 ? Int(kv[1]).map { (String(kv[0]), $0) } : nil
            }
            let values = Dictionary(pairs, uniquingKeysWith: { $1 })
            if failure == nil, let l = values["layer"], let f = values["frame"] {
                layer = max(0, min(l - 1, layerCount - 1))
                frame = max(0, min(f - 1, frameCount - 1))
            }
        }
        let messages = lines.joined(separator: "\n")
        clampSelection()  // a script may have removed the current layer or frame
        snapDrawColor()
        revision &+= 1
        if failure == "script cancelled" { return }
        guard !messages.isEmpty || failure != nil else { return }
        let alert = NSAlert()
        alert.messageText = failure == nil ? name : "Script error in \(name)"
        alert.alertStyle = failure == nil ? .informational : .warning
        alert.informativeText = [messages, failure.map { "\($0)\nThe document was not changed." }]
            .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: "\n\n")
        alert.runModal()
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

    var body: some Commands {
        CommandGroup(after: .saveItem) {
            Menu("Scripts") {
                ForEach(library.scripts, id: \.self) { url in
                    Button(url.deletingPathExtension().lastPathComponent) { model.runScript(at: url) }
                }
                if library.scripts.isEmpty {
                    Button("No scripts found") {}.disabled(true)
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
