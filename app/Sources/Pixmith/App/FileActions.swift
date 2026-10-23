import AppKit
import PixmithCore
import SwiftUI
import UniformTypeIdentifiers

// MARK: Recent files

final class RecentFiles: ObservableObject {
    static let shared = RecentFiles()
    private static let key = "recentFiles"
    private static let limit = 10

    @Published private(set) var urls: [URL]

    private init() {
        urls = (UserDefaults.standard.stringArray(forKey: Self.key) ?? []).map { URL(fileURLWithPath: $0) }
    }

    /// Recent files that still exist.
    var existing: [URL] { urls.filter { FileManager.default.fileExists(atPath: $0.path) } }

    func add(_ url: URL) {
        urls = Array(([url] + urls.filter { $0 != url }).prefix(Self.limit))
        UserDefaults.standard.set(urls.map(\.path), forKey: Self.key)
        NSDocumentController.shared.noteNewRecentDocumentURL(url)
    }

    func clear() {
        urls = []
        UserDefaults.standard.removeObject(forKey: Self.key)
        NSDocumentController.shared.clearRecentDocuments(nil)
    }
}

// MARK: Dirty tracking and confirmation

extension SpriteModel {
    /// Compares with the saved undo position, so undoing back to the saved state is clean again.
    var isDirty: Bool { pixmith_is_dirty(doc) }

    var windowTitle: String {
        documentName + (isDirty ? " — Edited" : "")
    }

    /// Asks to save unsaved changes. Returns false if the user cancelled.
    func confirmDiscard() -> Bool {
        guard isDirty else { return true }
        stop()
        let alert = NSAlert()
        alert.messageText = "Do you want to save the changes made to “\(documentName)”?"
        alert.informativeText = "Your changes will be lost if you don’t save them."
        alert.addButton(withTitle: "Save")
        alert.addButton(withTitle: "Don’t Save")
        alert.addButton(withTitle: "Cancel")
        switch Self.alertRunner(alert) {
        case .alertFirstButtonReturn:
            save()
            return !isDirty
        case .alertSecondButtonReturn:
            // The recovery copy goes only once the whole close or quit succeeds (a later Cancel keeps it).
            return true
        default:
            return false
        }
    }

    // New and opened documents get their own tab, so none of these has anything to discard.
    func requestNewSprite() {
        guard !MainWindow.hasSheet else { return }
        showNewSheet = true
    }

    func requestOpen() {
        guard !MainWindow.hasSheet else { return }
        open()
    }

    func requestOpen(_ url: URL) {
        guard !MainWindow.hasSheet else { return }
        open(url)
    }

    func terminateReply() -> NSApplication.TerminateReply {
        guard WindowCloseGuard.approved || confirmDiscardAll() else { return .terminateCancel }
        Autosaver.shared.discardAll()
        return .terminateNow
    }

    private var baseName: String { fileURL?.deletingPathExtension().lastPathComponent ?? "Untitled" }

    // MARK: Import

    /// Opens an imported sprite in a new tab, which counts as unsaved.
    func adopt(_ imported: OpaquePointer?) {
        guard let imported else {
            errorMessage = lastError
            return
        }
        present(imported, url: nil)
        changed()
    }

    func importPNG(_ url: URL) {
        adopt(url.path.withCString { pixmith_import_png($0) })
        RecentFiles.shared.add(url)
    }

    private func pickPNG(message: String, accessory: NSView? = nil) -> URL? {
        pickFile([.png], message: message, accessory: accessory)
    }

    private func pickFile(_ types: [UTType], message: String, accessory: NSView? = nil) -> URL? {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = types
        panel.message = message
        panel.accessoryView = accessory
        panel.isAccessoryViewDisclosed = true
        return panel.runModal() == .OK ? panel.url : nil
    }

    func importPNGAsSprite() {
        guard !MainWindow.hasSheet else { return }
        guard let url = pickPNG(message: "Choose a PNG to open as a new sprite.") else { return }
        importPNG(url)
    }

    func importGIFAsSprite() {
        guard !MainWindow.hasSheet else { return }
        guard let url = pickFile([.gif], message: "Choose an animated GIF to open as a new sprite.") else { return }
        importInBackground("Importing GIF…") { url.path.withCString { pixmith_import_gif($0) } }
    }

    func importSheetWithJSON() {
        guard !MainWindow.hasSheet else { return }
        guard let url = pickFile([.json], message: "Choose the sprite sheet's Aseprite JSON. The PNG named in it is loaded from the same folder.")
        else { return }
        importInBackground("Importing sprite sheet…") { url.path.withCString { pixmith_import_sheet_json($0, nil) } }
    }

    func importSheetAsFrames() {
        guard !MainWindow.hasSheet else { return }
        let w = NSTextField(string: "\(width)"), h = NSTextField(string: "\(height)")
        [w, h].forEach { $0.frame.size.width = 70 }
        let view = accessoryView([row("Cell width", w), row("Cell height", h)])
        guard let url = pickPNG(message: "Choose a sprite sheet PNG. It is cut into frames of the given cell size.",
                                accessory: view) else { return }
        let (cw, ch) = (UInt32(clamping: max(w.integerValue, 0)), UInt32(clamping: max(h.integerValue, 0)))
        importInBackground("Importing sprite sheet…") { url.path.withCString { pixmith_import_sheet($0, cw, ch) } }
    }

    func importPNGAsLayer() {
        guard !MainWindow.hasSheet else { return }
        guard let url = pickPNG(message: "Choose a PNG to add as a new layer on the current frame.") else { return }
        stop()
        let index = url.path.withCString { pixmith_import_png_layer(doc, UInt32(layer), UInt32(frame), $0) }
        if index < 0 {
            errorMessage = lastError
        } else {
            layer = Int(index)
            changed()
        }
    }

    // MARK: Export

    private func savePanel(type: UTType, name: String, message: String? = nil, accessory: NSView? = nil) -> URL? {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [type]
        panel.nameFieldStringValue = name
        panel.message = message
        panel.accessoryView = accessory
        return panel.runModal() == .OK ? panel.url : nil
    }

    private static let scales = [1, 2, 4, 8, 16]

    private func scalePopup() -> NSPopUpButton {
        let popup = NSPopUpButton()
        popup.addItems(withTitles: Self.scales.map { "\($0)x" })
        popup.sizeToFit()
        return popup
    }

    func exportGIF() {
        guard !MainWindow.hasSheet else { return }
        stop()
        let scale = scalePopup()
        guard let url = savePanel(type: .gif, name: "\(baseName).gif", accessory: accessoryView([row("Scale", scale)]))
        else { return }
        let factor = UInt32(Self.scales[scale.indexOfSelectedItem])
        exportInBackground("Exporting GIF…") { snap in url.path.withCString { pixmith_export_gif(snap, factor, $0) } }
    }

    func exportAPNG() {
        guard !MainWindow.hasSheet else { return }
        stop()
        let scale = scalePopup()
        guard let url = savePanel(type: .png, name: "\(baseName).png", message: "Saves an animated PNG.",
                                  accessory: accessoryView([row("Scale", scale)])) else { return }
        let factor = UInt32(Self.scales[scale.indexOfSelectedItem])
        exportInBackground("Exporting animated PNG…") { snap in url.path.withCString { pixmith_export_apng(snap, factor, $0) } }
    }

    func exportPNGSequence() {
        guard !MainWindow.hasSheet else { return }
        stop()
        let scale = scalePopup()
        guard let url = savePanel(
            type: .png, name: "\(baseName).png",
            message: "Frames are saved as name-001.png, name-002.png, …", accessory: accessoryView([row("Scale", scale)]))
        else { return }
        let base = url.deletingPathExtension()
        let first = URL(fileURLWithPath: base.path + "-001.png")
        if first != url, FileManager.default.fileExists(atPath: first.path), !confirmReplace(first.deletingLastPathComponent()) {
            return
        }
        let factor = UInt32(Self.scales[scale.indexOfSelectedItem])
        exportInBackground("Exporting PNG sequence…") { snap in base.path.withCString { pixmith_export_png_sequence(snap, factor, $0) } }
    }

    private func confirmReplace(_ folder: URL) -> Bool {
        let alert = NSAlert()
        alert.messageText = "Files with these names already exist in “\(folder.lastPathComponent)”."
        alert.informativeText = "Do you want to replace them?"
        alert.addButton(withTitle: "Replace")
        alert.addButton(withTitle: "Cancel")
        return alert.runModal() == .alertFirstButtonReturn
    }

    func exportSpriteSheet() {
        guard !MainWindow.hasSheet else { return }
        stop()
        let layout = NSPopUpButton()
        layout.addItems(withTitles: ["Horizontal strip", "Vertical strip", "Grid"])
        let columns = NSTextField(string: "\(min(frameCount, 4))")
        let padding = NSTextField(string: "0")
        [columns, padding].forEach { $0.frame.size.width = 60 }
        let scale = scalePopup()
        let json = NSButton(checkboxWithTitle: "Write JSON metadata next to the PNG", target: nil, action: nil)
        json.state = .on
        let view = accessoryView([row("Layout", layout), row("Columns", columns), row("Padding", padding),
                                  row("Scale", scale), json])
        guard let url = savePanel(
            type: .png, name: "\(baseName)-sheet.png",
            message: "Columns apply to the grid layout. Padding is in output pixels.", accessory: view)
        else { return }
        let (layoutKind, cols) = (UInt32(layout.indexOfSelectedItem), UInt32(clamping: max(columns.integerValue, 1)))
        let (pad, factor, withJSON) = (UInt32(clamping: max(padding.integerValue, 0)), UInt32(Self.scales[scale.indexOfSelectedItem]), json.state == .on)
        exportInBackground("Exporting sprite sheet…") { snap in
            url.path.withCString { pixmith_export_sheet_ex(snap, layoutKind, cols, pad, factor, withJSON, $0) }
        }
    }
}

// MARK: Dialog helpers

private func row(_ title: String, _ control: NSView) -> NSView {
    let label = NSTextField(labelWithString: title)
    label.alignment = .right
    label.widthAnchor.constraint(equalToConstant: 80).isActive = true
    let stack = NSStackView(views: [label, control])
    stack.spacing = 8
    return stack
}

private func accessoryView(_ rows: [NSView]) -> NSView {
    let stack = NSStackView(views: rows)
    stack.orientation = .vertical
    stack.alignment = .leading
    stack.spacing = 8
    stack.edgeInsets = NSEdgeInsets(top: 8, left: 12, bottom: 8, right: 12)
    stack.frame.size = stack.fittingSize
    return stack
}

// MARK: Commands

struct FileCommands: Commands {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var recents = RecentFiles.shared

    var body: some Commands {
        CommandGroup(after: .newItem) {
            Menu("Open Recent") {
                ForEach(recents.existing, id: \.self) { url in
                    Button(url.lastPathComponent) {
                        // ⌘-click opens it in the background.
                        if NSEvent.modifierFlags.contains(.command) { model.openInBackground(url) } else { model.requestOpen(url) }
                    }
                }
                Divider()
                Button("Clear Menu") { recents.clear() }.disabled(recents.urls.isEmpty)
            }
        }
        CommandGroup(after: .saveItem) {
            Button("Export Animated GIF…") { model.exportGIF() }
            Button("Export Animated PNG…") { model.exportAPNG() }
            Button("Export PNG Sequence…") { model.exportPNGSequence() }
            Divider()
            Menu("Import") {
                Button("PNG as New Sprite…") { model.importPNGAsSprite() }
                Button("Sprite Sheet as Frames…") { model.importSheetAsFrames() }
                Button("Sprite Sheet with JSON…") { model.importSheetWithJSON() }
                Button("Animated GIF as New Sprite…") { model.importGIFAsSprite() }
                Button("PNG as New Layer…") { model.importPNGAsLayer() }
            }
        }
    }
}

// MARK: Window integration

/// Keeps the window's edited dot in sync, asks before closing, and refreshes the canvas when preferences change.
private struct WindowAccessor: NSViewRepresentable {
    @ObservedObject var model: SpriteModel

    func makeNSView(context: Context) -> NSView { NSView() }

    func updateNSView(_ view: NSView, context: Context) {
        let edited = model.isDirty
        DispatchQueue.main.async {
            guard let window = view.window else { return }
            window.isDocumentEdited = edited
            WindowCloseGuard.shared.install(on: window)
        }
    }
}

/// Adds a close confirmation to the SwiftUI-managed window delegate by forwarding everything else to it.
final class WindowCloseGuard: NSObject, NSWindowDelegate {
    static let shared = WindowCloseGuard()
    /// Set once the user has agreed to close, so quitting does not ask a second time.
    static var approved = false
    private var original: NSWindowDelegate?

    func install(on window: NSWindow) {
        MainWindow.window = window
        guard window.delegate !== self else { return }
        original = window.delegate
        window.delegate = self
    }

    override func responds(to selector: Selector!) -> Bool {
        super.responds(to: selector) || original?.responds(to: selector) == true
    }

    override func forwardingTarget(for selector: Selector!) -> Any? {
        original?.responds(to: selector) == true ? original : nil
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        if original?.windowShouldClose?(sender) == false { return false }
        guard SpriteModel.shared.confirmDiscardAll() else { return false }
        Self.approved = true
        return true
    }
}

private struct FileHandling: ViewModifier {
    @ObservedObject var model: SpriteModel

    func body(content: Content) -> some View {
        content
            .background(WindowAccessor(model: model))
            .onAppear { Autosaver.shared.start(model) }
            .onReceive(NotificationCenter.default.publisher(for: UserDefaults.didChangeNotification)) { _ in
                model.objectWillChange.send()
            }
    }
}

extension View {
    func pixmithFileHandling(_ model: SpriteModel) -> some View { modifier(FileHandling(model: model)) }
}

// MARK: Status info (shown as chips over the canvas)

final class StatusInfo: ObservableObject {
    static let shared = StatusInfo()
    @Published var cursor: (x: Int, y: Int)?
}

// MARK: Autosave and crash recovery

enum AppSupport {
    /// ~/Library/Application Support, or `PIXMITH_SUPPORT_DIR` when set, so test and QA runs keep their autosave
    /// copies and scripts out of the real one.
    static let directory: URL = ProcessInfo.processInfo.environment["PIXMITH_SUPPORT_DIR"].map { URL(fileURLWithPath: $0, isDirectory: true) }
        ?? FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
}

/// While a document has unsaved changes, copies it to ~/Library/Application Support/Pixmith/Recovery every
/// minute (`PIXMITH_AUTOSAVE_INTERVAL` overrides the seconds). Every open tab has its own copy, named by the
/// tab's recovery uuid. A clean exit, a save or closing the tab removes the copy, so anything found there at
/// launch is left over from a crash.
final class Autosaver {
    static let shared = Autosaver()

    private struct Candidate {
        let id: UUID
        let file: URL
        let originalPath: String?
        let name: String
        let date: Date
    }

    private var timer: Timer?
    private var offered = false
    private weak var model: SpriteModel?
    private let queue = DispatchQueue(label: "pixmith.autosave", qos: .utility)
    /// Recovery ids with a write in progress, and whether the user has been told that autosave fails.
    private var inFlight: Set<UUID> = []
    private var warned = false

    private let directory = AppSupport.directory.appendingPathComponent("Pixmith/Recovery", isDirectory: true)

    private func file(_ id: UUID, _ ext: String) -> URL {
        directory.appendingPathComponent(id.uuidString).appendingPathExtension(ext)
    }

    func start(_ model: SpriteModel) {
        guard timer == nil else { return }
        self.model = model
        let seconds = Double(ProcessInfo.processInfo.environment["PIXMITH_AUTOSAVE_INTERVAL"] ?? "") ?? 60
        let timer = Timer(timeInterval: max(seconds, 1), repeats: true) { [weak self] _ in self?.tick() }
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
        DispatchQueue.main.async { self.offerRecovery() }
    }

    func tick() {
        guard let model else { return }
        let fm = FileManager.default
        for (index, slot) in model.slots.enumerated() {
            guard pixmith_is_dirty(slot.doc) else {
                discard(slot)
                continue
            }
            // The active document's revision lives in the model; background ones stored theirs when they left.
            let revision = index == model.activeSlot ? model.revision : slot.revision
            guard slot.lastAutosavedRevision != revision else { continue }
            guard !inFlight.contains(slot.recoveryID) else { continue }
            do { try fm.createDirectory(at: directory, withIntermediateDirectories: true) } catch {
                return warn(error.localizedDescription)
            }
            // The copy is written on a background queue from a snapshot, so a big document never stalls the UI.
            // pixmith_save writes through a temporary file, so a crash mid-write keeps the previous copy.
            guard let snapshot = pixmith_snapshot(slot.doc) else { continue }
            let id = slot.recoveryID
            let (asepriteFile, metaFile) = (file(id, "aseprite"), file(id, "json"))
            let meta: [String: Any] = [
                "path": slot.fileURL?.path ?? NSNull(),
                "name": model.name(of: slot),
                "date": ISO8601DateFormatter().string(from: Date()),
            ]
            inFlight.insert(id)
            let handoff = SendableDoc(doc: snapshot)
            queue.async {
                let ok = asepriteFile.path.withCString { pixmith_save(handoff.doc, $0) }
                let failure = ok ? "" : String(cString: pixmith_last_error())
                pixmith_free(handoff.doc)
                if ok, let data = try? JSONSerialization.data(withJSONObject: meta) { try? data.write(to: metaFile, options: .atomic) }
                DispatchQueue.main.async { [weak self, weak slot] in
                    guard let self else { return }
                    self.inFlight.remove(id)
                    // The tab was closed, replaced or saved while the copy was being written: it is stale now.
                    guard let slot, !slot.closed, slot.recoveryID == id, pixmith_is_dirty(slot.doc) else { return self.remove(id) }
                    if ok {
                        slot.lastAutosavedRevision = revision
                        self.warned = false
                    } else {
                        self.warn(failure)
                    }
                }
            }
        }
    }

    /// Tells the user once that recovery copies cannot be written, so they do not rely on them.
    private func warn(_ reason: String) {
        guard !warned else { return }
        warned = true
        model?.errorMessage = "Autosave failed: \(reason)\n\nUnsaved changes are not protected against a crash until this is fixed. "
            + "Free some disk space or check the permissions of ~/Library/Application Support/Pixmith, or save your work now."
    }

    /// Removes the recovery copy of the active document.
    func discard() {
        if let slot = (model ?? SpriteModel.shared).activeSlotObject { discard(slot) }
    }

    /// Removes the recovery copy of one tab.
    func discard(_ slot: DocumentSlot) {
        remove(slot.recoveryID)
        slot.lastAutosavedRevision = nil
    }

    /// Clean exit: no tab leaves a copy behind.
    func discardAll() {
        (model ?? SpriteModel.shared).slots.forEach(discard)
    }

    /// The old document is gone (saved, discarded or replaced); its copy goes with it.
    func documentReplaced(_ slot: DocumentSlot?) {
        guard let slot else { return }
        discard(slot)
        slot.recoveryID = UUID()
    }

    private func remove(_ id: UUID) {
        try? FileManager.default.removeItem(at: file(id, "aseprite"))
        try? FileManager.default.removeItem(at: file(id, "json"))
    }

    private func candidates() -> [Candidate] {
        let open = Set((model ?? SpriteModel.shared).slots.map(\.recoveryID))
        let urls = (try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        return urls.compactMap { url in
            guard url.pathExtension == "aseprite", let id = UUID(uuidString: url.deletingPathExtension().lastPathComponent),
                  !open.contains(id) else { return nil }
            let meta = (try? Data(contentsOf: file(id, "json"))).flatMap { try? JSONSerialization.jsonObject(with: $0) } as? [String: Any]
            let date = (try? url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? Date()
            return Candidate(id: id, file: url, originalPath: meta?["path"] as? String,
                             name: meta?["name"] as? String ?? "Untitled", date: date)
        }
        .sorted { $0.date > $1.date }
    }

    /// Offers every leftover copy at launch; restoring opens each one as a tab.
    func offerRecovery() {
        guard !offered, let model else { return }
        offered = true
        let found = candidates()
        guard let newest = found.first else { return }
        let alert = NSAlert()
        alert.icon = NSApp.applicationIconImage
        alert.messageText = found.count == 1
            ? "Pixmith quit unexpectedly. Restore the unsaved changes to “\(newest.name)”?"
            : "Pixmith quit unexpectedly. Restore unsaved work in \(found.count) documents?"
        let lines = found.prefix(8).map { "\($0.name) – \($0.date.formatted(date: .abbreviated, time: .shortened))" }
        alert.informativeText = lines.joined(separator: "\n") + (found.count > 8 ? "\n…" : "")
        alert.addButton(withTitle: found.count > 1 ? "Restore All" : "Restore")
        alert.addButton(withTitle: found.count > 1 ? "Discard All" : "Discard")
        alert.addButton(withTitle: "Not Now")
        switch SpriteModel.alertRunner(alert) {
        case .alertFirstButtonReturn:
            // Oldest first, so the most recent document ends up as the active tab.
            for c in found.reversed() { restore(c, into: model) }
        case .alertSecondButtonReturn: found.forEach { remove($0.id) }
        default: break
        }
    }

    private func restore(_ c: Candidate, into model: SpriteModel) {
        guard let doc = c.file.path.withCString({ pixmith_open($0) }) else {
            model.errorMessage = model.lastError
            return
        }
        var url: URL?
        if let path = c.originalPath, FileManager.default.fileExists(atPath: path) { url = URL(fileURLWithPath: path) }
        model.present(doc, url: url)
        model.changed()
        // Keep using the recovered copy, so there is no unprotected gap until the next autosave.
        if let slot = model.activeSlotObject {
            remove(slot.recoveryID)
            slot.recoveryID = c.id
        }
    }
}
