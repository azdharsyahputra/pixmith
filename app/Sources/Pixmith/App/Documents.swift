import AppKit
import PixmithCore
import SwiftUI
import UniformTypeIdentifiers

// Multiple open documents (tabs).
//
// `SpriteModel.shared` stays the one object every view and command talks to; it always represents the *active*
// document. Each open document has a `DocumentSlot` that stores the per-document fields while it is in the
// background. `activate` stashes the live fields into the current slot and loads the target slot's fields.

/// Everything that belongs to one open document.
final class DocumentSlot: Identifiable {
    let id = UUID()
    var doc: OpaquePointer
    var fileURL: URL?
    var layer = 0
    var frame = 0
    var zoom: CGFloat = 16
    /// Canvas scroll origin (clip view bounds origin) while the slot is in the background.
    var scroll: CGPoint?
    /// `SpriteModel.revision` as of the last stash. Revisions are one increasing sequence for all documents.
    var revision = 0
    var savedRevision = 0
    /// Names the recovery files written by `Autosaver` (`<uuid>.pxmt` and `<uuid>.json`).
    var recoveryID = UUID()
    /// Revision whose content is in the recovery file, if any.
    var lastAutosavedRevision: Int?
    /// "Untitled N" for documents that have no file.
    var untitledNumber = 1
    /// A name given to a document that has no file yet (double-click its tab); the Save panel starts with it.
    var customName: String?
    /// The image, GIF or sheet this document was imported from: Save As starts in its folder with its name.
    var importSource: URL?
    /// Set when the tab is closed, just before its document is freed; a closed slot never touches `doc`.
    var closed = false
    /// The reference image and its placement belong to the document.
    var reference = ReferenceState()
    /// Pixmith Live (Live.swift): the id clients know this tab by, its revision, and what it looked like when the
    /// revision was last updated.
    var liveID: String?
    var liveRevision = 1
    var liveSeen: LiveStamp?
    var liveDocPtr: OpaquePointer?
    /// Created or opened by a Live request (`documents.new`, `documents.open`): never reused as a blank tab by a
    /// later request, so two quick `documents.new` give two documents.
    var liveCreated = false

    init(doc: OpaquePointer, fileURL: URL? = nil) {
        self.doc = doc
        self.fileURL = fileURL
    }
}

/// Per-document reference image settings (the rest of `ViewSettings` is app-wide).
struct ReferenceState {
    var image: CGImage?
    var name = ""
    var show = true
    var opacity = 0.5
    var placement: ReferencePlacement = .behind
    var x = 0, y = 0
    var scale = 100.0

    init() {}

    init(_ v: ViewSettings) {
        image = v.referenceImage; name = v.referenceName; show = v.showReference; opacity = v.referenceOpacity
        placement = v.referencePlacement; x = v.referenceX; y = v.referenceY; scale = v.referenceScale
    }

    func apply(to v: ViewSettings) {
        v.referenceImage = image; v.referenceName = name; v.showReference = show; v.referenceOpacity = opacity
        v.referencePlacement = placement; v.referenceX = x; v.referenceY = y; v.referenceScale = scale
    }
}

extension SpriteModel {
    // MARK: Queries

    var activeSlotObject: DocumentSlot? { slots.indices.contains(activeSlot) ? slots[activeSlot] : nil }
    var activeSlotID: UUID? { activeSlotObject?.id }

    func isDirty(_ slot: DocumentSlot) -> Bool { !slot.closed && pixmith_is_dirty(slot.doc) }

    func name(of slot: DocumentSlot) -> String {
        if let url = slot.fileURL { return url.lastPathComponent }
        if let custom = slot.customName { return custom }
        return slots.count == 1 && slot.untitledNumber == 1 ? "Untitled" : "Untitled \(slot.untitledNumber)"
    }

    /// Name of the active document, for the window title and dialogs.
    var documentName: String { activeSlotObject.map(name(of:)) ?? "Untitled" }

    /// Renames a document from its tab. A saved document's file is renamed on disk (keeping its extension);
    /// one without a file keeps the name for its tab and as the Save panel's suggestion.
    func renameDocument(_ slot: DocumentSlot, to newName: String) {
        var base = newName.trimmingCharacters(in: .whitespacesAndNewlines)
        for ext in [".pxmt", ".aseprite", ".ase"] where base.lowercased().hasSuffix(ext) { base.removeLast(ext.count) }
        guard !base.isEmpty, !base.contains("/"), !base.contains(":") else { return }
        guard let url = slot.fileURL else {
            slot.customName = base
            objectWillChange.send()
            return
        }
        let target = url.deletingLastPathComponent().appendingPathComponent(base).appendingPathExtension(url.pathExtension)
        guard target != url else { return }
        if FileManager.default.fileExists(atPath: target.path), target.path.lowercased() != url.path.lowercased() {
            errorMessage = "“\(target.lastPathComponent)” already exists in that folder."
            return
        }
        do {
            try FileManager.default.moveItem(at: url, to: target)
        } catch {
            errorMessage = "Could not rename “\(url.lastPathComponent)”: \(error.localizedDescription)"
            return
        }
        if slot === activeSlotObject { fileURL = target } else { slot.fileURL = target }
        objectWillChange.send()
    }

    /// The tab that shows this file, if one is open.
    func slot(for url: URL) -> DocumentSlot? {
        let key = url.resolvingSymlinksInPath()
        return slots.first { $0.fileURL?.resolvingSymlinksInPath() == key }
    }

    /// A project action renamed (`to` is the new URL) or deleted (`to` is nil) the file behind a tab. The tab follows
    /// a rename; after a delete it keeps its pixels as a document without a file, so Save asks where to put it.
    func canvasFileMoved(from old: URL, to new: URL?) {
        guard let slot = slot(for: old) else { return }
        if slot === activeSlotObject {
            if new == nil { slot.customName = old.deletingPathExtension().lastPathComponent }
            fileURL = new
        } else {
            if new == nil { slot.customName = old.deletingPathExtension().lastPathComponent }
            slot.fileURL = new
        }
        objectWillChange.send()
    }

    func nextUntitledNumber(excluding slot: DocumentSlot? = nil) -> Int {
        let used = Set(slots.filter { $0 !== slot && $0.fileURL == nil }.map(\.untitledNumber))
        return (1...).first { !used.contains($0) }!
    }

    /// An untitled document that was never edited (the one a fresh launch starts with). Opening something
    /// replaces it instead of adding a tab, like a browser's blank tab.
    var activeIsPristine: Bool {
        fileURL == nil && !isDirty && historyCounts == (0, 0)
    }

    // MARK: Opening

    /// Shows a document: replaces the active tab if it is a pristine untitled one, else opens a new tab.
    func present(_ d: OpaquePointer, url: URL?) {
        AppRoute.shared.enterEditor()
        if activeIsPristine {
            replace(with: d)
            fileURL = url
            activeSlotObject?.untitledNumber = nextUntitledNumber(excluding: activeSlotObject)
        } else {
            newTab(doc: d, url: url)
        }
    }

    /// Adds a tab holding `doc` (which the model takes ownership of) and makes it the active one.
    @discardableResult
    func newTab(doc d: OpaquePointer, url: URL?) -> Int {
        AppRoute.shared.enterEditor()
        finishTransients()
        stash(activeSlot)
        let slot = DocumentSlot(doc: d, fileURL: url)
        slot.untitledNumber = nextUntitledNumber()
        slot.layer = max(0, Int(pixmith_layer_count(d)) - 1)
        slot.zoom = Prefs.initialZoom(width: Int(pixmith_width(d)), height: Int(pixmith_height(d)))
        slots.append(slot)
        load(slots.count - 1, fresh: true)
        if let url { RecentFiles.shared.add(url) }
        return slots.count - 1
    }

    // MARK: Switching

    func activate(_ index: Int) {
        guard slots.indices.contains(index), index != activeSlot else { return }
        finishTransients()
        stash(activeSlot)
        load(index)
    }

    func nextTab() { if slots.count > 1 { activate((activeSlot + 1) % slots.count) } }
    func previousTab() { if slots.count > 1 { activate((activeSlot - 1 + slots.count) % slots.count) } }

    /// Moves `slot` to `index` (drag to reorder). The active document stays active.
    func moveTab(_ slot: DocumentSlot, to index: Int) {
        guard let from = slots.firstIndex(where: { $0 === slot }), slots.indices.contains(index), from != index else { return }
        let active = activeSlotObject
        slots.insert(slots.remove(at: from), at: index)
        if let active, let i = slots.firstIndex(where: { $0 === active }) { activeSlot = i }
    }

    /// Writes the live per-document fields into the slot at `index`.
    private func stash(_ index: Int) {
        guard slots.indices.contains(index) else { return }
        let s = slots[index]
        s.doc = doc
        s.fileURL = fileURL
        s.layer = layer
        s.frame = frame
        s.zoom = zoom
        s.savedRevision = savedRevision
        s.revision = revision
        s.reference = ReferenceState(ViewSettings.shared)
        s.scroll = SelectionOutline.shared.canvas?.enclosingScrollView?.contentView.bounds.origin
    }

    /// Makes the slot at `index` the live document. Revisions keep increasing across documents, so caches keyed
    /// by revision (render cache, selection outline, timeline) never see the same number for different content.
    private func load(_ index: Int, fresh: Bool = false) {
        let s = slots[index]
        tabSwitching = true
        defer { tabSwitching = false }
        let unchanged = s.lastAutosavedRevision == s.revision
        activeSlot = index
        doc = s.doc
        fileURL = s.fileURL
        frame = s.frame
        layer = s.layer
        zoom = s.zoom
        savedRevision = s.savedRevision
        composite = []
        playCursor = PlaybackCursor()
        clampSelection()
        s.reference.apply(to: ViewSettings.shared)
        HistoryStamp.value &+= 1  // step labels differ per document even when the counts match
        revision &+= 1
        s.revision = revision
        if unchanged { s.lastAutosavedRevision = revision }
        if !fresh { pendingScroll = s.scroll }
        StatusInfo.shared.cursor = nil
        followActiveDocument()
    }

    /// Auxiliary panels read the shared model, so they follow the switch; the preview keeps its own playback state.
    private func followActiveDocument() {
        let preview = PreviewController.shared
        guard preview.state.active else { return }
        preview.state.frame = 0
        preview.state.setActive(true, model: self)
        if preview.isVisible { preview.fitWindow(to: preview.state.zoom, model: self) }
    }

    // MARK: Transient interaction state

    /// Finishes everything in flight on the active document so nothing half-done is left behind or leaks into
    /// another document: a floating paste, a free transform and an open shape or text are committed.
    func finishTransients() {
        stop()
        let wasClean = !isDirty
        defer {
            // Committing what the saved copy already shows (a floating paste or transform saved while still open)
            // changes no pixels, so a clean document stays clean and is not asked about.
            if wasClean, isDirty {
                pixmith_mark_saved(doc)
                savedRevision = revision
            }
        }
        if opts.textRequest != nil { placeText() }
        finishShape()
        finishPolygon()
        if pixmith_float_active(doc) {
            pixmith_float_commit(doc)
            changed()
        }
        commitTransform()
        resetTransients()
    }

    /// Drops transient state that belongs to the active document without touching its pixels. Callers commit
    /// first (`finishTransients`) or are about to discard the document.
    func resetTransients() {
        stop()
        // Open filter sheet: cancel its preview.
        let filters = FilterState.shared
        if filters.active != nil || filters.preview != nil {
            filters.setPreview(nil)
            filters.active = nil
        }
        filters.region = (0, 0, 0, 0)
        // Gestures.
        opts.drag?.timer?.invalidate()
        opts.drag = nil
        opts.shape = nil
        opts.textRequest = nil
        pixmith_stroke_end_doc(doc)
        for _ in 0..<16 { pixmith_end_transaction(doc) }  // no-op unless a transaction was left open
        resetSelectionTransients()
        resetTransformTransients()
        resetSliceTransients()
        OpacityDrag.shared.active = false
        OpacityDrag.shared.checkpointed = false
        LayerDragState.source = nil
        let timeline = TimelineSelection.shared
        timeline.range = nil
        timeline.anchor = nil
        timeline.focused = false
        let tiles = TilemapState.shared
        tiles.stamp = false
        tiles.selected = 1
        tiles.flipX = false
        tiles.flipY = false
        tiles.flipD = false
        tiles.sheet = nil
        // Sheets that refer to layers or cels of the document being left.
        renamingLayer = nil
        toolSheet = nil
    }

    // MARK: Closing

    /// Closes a tab, asking about unsaved changes first. Returns false if the user cancelled.
    /// `discardChanges` closes a document with unsaved changes without asking (Live mode, after the client said so).
    @discardableResult
    func closeTab(_ index: Int, discardChanges: Bool = false) -> Bool {
        guard slots.indices.contains(index) else { return false }
        let slot = slots[index]
        let previous = index == activeSlot ? nil : activeSlotObject
        if index == activeSlot {
            finishTransients()
        } else if isDirty(slot) {
            activate(index)  // show the tab the question is about
        }
        if isDirty(slot), !discardChanges {
            guard confirmDiscard() else { return false }
        }
        guard let index = slots.firstIndex(where: { $0 === slot }) else { return false }
        Autosaver.shared.discard(slot)

        if slots.count == 1 {
            if activeIsPristine { return true }
            // The last tab becomes a fresh untitled document; the window stays open.
            replace(with: pixmith_new(UInt32(Prefs.newWidth), UInt32(Prefs.newHeight)))
            fileURL = nil
            slot.untitledNumber = 1
            slot.importSource = nil
            return true
        }
        if index == activeSlot {
            resetTransients()
            slots.remove(at: index)
            // Back to the tab that was showing before the question, else the neighbor.
            load(previous.flatMap { p in slots.firstIndex { $0 === p } } ?? min(index, slots.count - 1))
        } else {
            slots.remove(at: index)
            if index < activeSlot { activeSlot -= 1 }
        }
        // `Autosaver`, the render cache and every panel read the live document through the model, and stroke state
        // is dropped by `pixmith_free`, so nothing else can still use this pointer.
        // The tab view may still evaluate once with this slot; it must not read the document any more.
        slot.closed = true
        let dead = slot.doc
        DispatchQueue.main.async { pixmith_free(dead) }
        return true
    }

    @discardableResult
    func closeActiveTab() -> Bool { closeTab(activeSlot) }

    func closeTab(_ slot: DocumentSlot) {
        if let i = slots.firstIndex(where: { $0 === slot }) { closeTab(i) }
    }

    /// Asks about every tab with unsaved changes, showing each one in turn. False if the user cancelled any.
    func confirmDiscardAll() -> Bool {
        finishTransients()
        for slot in slots where isDirty(slot) {
            guard let i = slots.firstIndex(where: { $0 === slot }) else { continue }
            activate(i)
            guard confirmDiscard() else { return false }
        }
        return true
    }

    /// False while a sheet or modal is up, when tab commands must not run.
    var canSwitchTabs: Bool { !MainWindow.hasSheet && NSApp.modalWindow == nil }
}

// MARK: - Alerts

extension SpriteModel {
    /// How alerts are shown; tests replace it with canned answers.
    static var alertRunner: (NSAlert) -> NSApplication.ModalResponse = { $0.runModal() }
}

// MARK: - Menu

struct TabCommands: Commands {
    @ObservedObject var model: SpriteModel

    var body: some Commands {
        CommandGroup(after: .newItem) {
            Button("Close Tab") { closeTabOrWindow() }.keyboardShortcut("w")
        }
        CommandGroup(before: .windowList) {
            // ⌘⇧] and ⌘⇧[ are the alternate shortcuts; `installTabShortcuts` handles them.
            Button("Next Tab") { if model.canSwitchTabs { model.nextTab() } }
                .keyboardShortcut(.tab, modifiers: .control).disabled(model.slots.count < 2)
            Button("Previous Tab") { if model.canSwitchTabs { model.previousTab() } }
                .keyboardShortcut(.tab, modifiers: [.control, .shift]).disabled(model.slots.count < 2)
            Divider()
        }
    }

    /// ⌘W closes the active tab in the editor window and the window itself for the other windows (settings, help).
    private func closeTabOrWindow() {
        if MainWindow.hasSheet { return }
        if let key = NSApp.keyWindow, key !== MainWindow.window {
            key.performClose(nil)
        } else if model.canSwitchTabs {
            model.closeActiveTab()
        }
    }
}

// MARK: - Tab bar

/// Document tabs in the title row. With one document it shows that document as a single chip (name and unsaved dot);
/// with several, a tab per document, a close button on hover and a "+" that starts a new sprite.
struct TabBar: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: DS.Space.xxs) {
                ForEach(model.slots) { slot in
                    TabItem(model: model, slot: slot)
                }
                IconButton("plus", help: "New sprite", shortcut: "⌘N", compact: true) { model.requestNewSprite() }
            }
        }
        .frame(maxWidth: .infinity)
    }
}

private struct TabItem: View {
    @ObservedObject var model: SpriteModel
    let slot: DocumentSlot
    @State private var hovering = false
    @State private var editing = false
    @State private var draft = ""
    @FocusState private var focused: Bool

    private func beginRename() {
        draft = slot.fileURL?.deletingPathExtension().lastPathComponent ?? model.name(of: slot)
        editing = true
        focused = true
    }

    private func commitRename() {
        guard editing else { return }
        editing = false
        model.renameDocument(slot, to: draft)
    }

    var body: some View {
        let active = model.activeSlotID == slot.id
        let dirty = model.isDirty(slot)
        HStack(spacing: DS.Space.sm) {
            if editing {
                TextField("Name", text: $draft)
                    .textFieldStyle(.plain)
                    .font(DS.Font.bodyLarge)
                    .foregroundStyle(DSColor.text)
                    .focused($focused)
                    .onSubmit(commitRename)
                    .onExitCommand { editing = false }
                    .onChange(of: focused) { if !$0 { commitRename() } }
            } else {
                Text(model.name(of: slot))
                    .font(DS.Font.bodyLarge)
                    .foregroundStyle(active ? DSColor.text : DSColor.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            ZStack {
                if hovering {
                    Button { model.closeTab(slot) } label: {
                        Image(systemName: "xmark").font(.system(size: 9, weight: .bold))
                            .foregroundStyle(DSColor.textSecondary)
                            .frame(width: 16, height: 16)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .help("Close tab")
                } else if dirty {
                    Circle().fill(DSColor.textSecondary).frame(width: 6, height: 6)
                }
            }
            .frame(width: 16, height: 16)
        }
        .padding(.leading, 12)
        .padding(.trailing, DS.Space.sm)
        .frame(minWidth: 90, maxWidth: 200, minHeight: DS.Metric.iconButton)
        .background(RoundedRectangle(cornerRadius: DS.Radius.control)
            .fill(active ? DSColor.panel2 : hovering ? DSColor.hover.opacity(0.6) : .clear))
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
        .onTapGesture { if let i = model.slots.firstIndex(where: { $0 === slot }) { model.activate(i) } }
        .simultaneousGesture(TapGesture(count: 2).onEnded { beginRename() })
        .overlay(MiddleClickCatcher { model.closeTab(slot) })
        .contextMenu {
            Button("Rename…") { beginRename() }
            Button("Close Tab") { model.closeTab(slot) }
        }
        .help((slot.fileURL?.path ?? model.name(of: slot)) + " — double-click to rename")
        .accessibilityAddTraits(active ? .isSelected : [])
        .onDrag { NSItemProvider(object: Self.payload(slot) as NSString) }
        .onDrop(of: [.text], delegate: TabDropDelegate(model: model, target: slot))
    }

    /// Only text with this prefix and a known tab id reorders tabs; any other dropped text is ignored.
    static let payloadPrefix = "pixmith-tab:"
    static func payload(_ slot: DocumentSlot) -> String { payloadPrefix + slot.id.uuidString }
}

private struct TabDropDelegate: DropDelegate {
    let model: SpriteModel
    let target: DocumentSlot

    func validateDrop(info: DropInfo) -> Bool { info.hasItemsConforming(to: [.text]) }

    func dropUpdated(info: DropInfo) -> DropProposal? { DropProposal(operation: .move) }

    func performDrop(info: DropInfo) -> Bool {
        guard let provider = info.itemProviders(for: [.text]).first else { return false }
        let model = model, target = target
        _ = provider.loadObject(ofClass: NSString.self) { object, _ in
            guard let text = object as? String, text.hasPrefix(TabItem.payloadPrefix),
                  let id = UUID(uuidString: String(text.dropFirst(TabItem.payloadPrefix.count))) else { return }
            DispatchQueue.main.async {
                guard let source = model.slots.first(where: { $0.id == id }),
                      let to = model.slots.firstIndex(where: { $0 === target }) else { return }
                withAnimation(.easeInOut(duration: 0.12)) { model.moveTab(source, to: to) }
            }
        }
        return true
    }
}

/// Invisible overlay that only takes middle-button events, so left clicks reach the SwiftUI view below.
private struct MiddleClickCatcher: NSViewRepresentable {
    let action: () -> Void

    func makeNSView(context: Context) -> MiddleClickView {
        let v = MiddleClickView()
        v.action = action
        return v
    }

    func updateNSView(_ view: MiddleClickView, context: Context) { view.action = action }
}

private final class MiddleClickView: NSView {
    var action: () -> Void = {}

    override func hitTest(_ point: NSPoint) -> NSView? {
        guard let t = NSApp.currentEvent?.type, t == .otherMouseDown || t == .otherMouseUp else { return nil }
        return super.hitTest(point)
    }

    override func otherMouseDown(with event: NSEvent) {}

    override func otherMouseUp(with event: NSEvent) {
        if event.buttonNumber == 2 { action() }
    }
}

// MARK: - Alternate tab shortcuts

extension SpriteModel {
    /// ⌘⇧] and ⌘⇧[ switch tabs like ⌃Tab and ⌃⇧Tab (a menu item can only show one shortcut).
    func installTabShortcuts() {
        NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] e in
            guard let self, MainWindow.isKey,
                  e.modifierFlags.intersection([.command, .shift, .control, .option]) == [.command, .shift],
                  let ch = e.charactersIgnoringModifiers, ["]", "}", "[", "{"].contains(ch)
            else { return e }
            if canSwitchTabs, slots.count > 1 {
                if ch == "]" || ch == "}" { nextTab() } else { previousTab() }
            }
            return nil
        }
    }
}

// MARK: - Opening files from Finder drags and in the background

extension SpriteModel {
    /// Opens a file as a tab without switching to it (⌘-click in Open Recent). Already open files stay as they are.
    func openInBackground(_ url: URL) {
        guard url.pathExtension.lowercased() != "png", !ImageFormats.isImage(url) else { return requestOpen(url) }
        let key = url.resolvingSymlinksInPath()
        if slots.contains(where: { $0.fileURL?.resolvingSymlinksInPath() == key }) { return }
        guard let d = url.path.withCString({ pixmith_open($0) }) else {
            errorMessage = lastError
            return
        }
        addBackgroundTab(doc: d, url: url)
        RecentFiles.shared.add(url)
    }

    /// Adds a tab holding `d` (which the model takes ownership of) behind the active one, without switching to it.
    @discardableResult
    func addBackgroundTab(doc d: OpaquePointer, url: URL?) -> DocumentSlot {
        let slot = DocumentSlot(doc: d, fileURL: url)
        slot.untitledNumber = nextUntitledNumber()
        slot.layer = max(0, Int(pixmith_layer_count(d)) - 1)
        slot.zoom = Prefs.initialZoom(width: Int(pixmith_width(d)), height: Int(pixmith_height(d)))
        slots.append(slot)
        return slot
    }
}

/// Drop target for files dragged from Finder onto the window: each `.pxmt`, `.aseprite`, `.ase` or `.png` opens as a
/// tab, and a project folder opens as a project.
private struct FileDropDelegate: DropDelegate {
    let model: SpriteModel

    func validateDrop(info: DropInfo) -> Bool { info.hasItemsConforming(to: [.fileURL]) }

    func performDrop(info: DropInfo) -> Bool {
        let model = model
        for provider in info.itemProviders(for: [.fileURL]) {
            _ = provider.loadObject(ofClass: URL.self) { url, _ in
                guard let url else { return }
                let isFolder = (try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true
                guard isFolder && ProjectStore.isProject(url) || ["pxmt", "aseprite", "ase", "png", ExtensionLibrary.packageExtension].contains(url.pathExtension.lowercased())
                        || ImageFormats.isImage(url)
                else { return }
                DispatchQueue.main.async { model.requestOpen(url) }
            }
        }
        return true
    }
}

extension View {
    func pixmithFileDrop(_ model: SpriteModel) -> some View { onDrop(of: [.fileURL], delegate: FileDropDelegate(model: model)) }
}
