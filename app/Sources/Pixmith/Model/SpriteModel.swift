import AppKit
import PixmithCore
import UniformTypeIdentifiers

enum Tool: String, CaseIterable {
    case pencil, eraser, fill, picker, line, rectangle, ellipse, spray
    case marquee
    case move
    case wand
    case lasso
    case polygon
    case gradient
    case slice
    case curve, contour, text
    case shapePolygon = "polygon (draw)"

    var icon: String {
        switch self {
        case .pencil: "pencil"
        case .eraser: "eraser"
        case .fill: "drop.fill"
        case .picker: "eyedropper"
        case .marquee: "rectangle.dashed"
        case .move: "arrow.up.and.down.and.arrow.left.and.right"
        case .line: "line.diagonal"
        case .rectangle: "rectangle"
        case .ellipse: "circle"
        case .spray: "circle.dotted"
        case .wand: "wand.and.stars"
        case .lasso: "lasso"
        case .polygon: "pentagon"
        case .gradient: "rectangle.lefthalf.filled"
        case .slice: "crop"
        case .curve: "point.topleft.down.curvedto.point.bottomright.up"
        case .contour: "scribble"
        case .shapePolygon: "hexagon"
        case .text: "textformat"
        }
    }
}

/// Swift-side state around the Rust document. Pixel data lives in Rust.
final class SpriteModel: ObservableObject {
    static let shared = SpriteModel()

    var doc: OpaquePointer
    @Published var revision = 0
    @Published var savedRevision = 0
    @Published var layer = 0
    @Published var frame = 0
    @Published var tool: Tool = .pencil
    @Published var color: UInt32 = 0x000000FF { didSet { snapDrawColor() } }
    @Published var brushSize = 1
    @Published var zoom: CGFloat = 16
    @Published var onionSkin = false
    @Published var showGrid = true
    @Published var isPlaying = false
    @Published var fileURL: URL? {
        didSet {
            if let fileURL, !tabSwitching { RecentFiles.shared.add(fileURL) }
            activeSlotObject?.fileURL = fileURL
        }
    }
    /// Open documents (see Documents.swift); `activeSlot` is the one the model currently shows.
    @Published var slots: [DocumentSlot] = []
    @Published var activeSlot = 0
    /// True while `activate` loads another document's fields, which must not count as opening the file.
    var tabSwitching = false
    /// Canvas scroll origin to restore once the canvas view has resized for the newly active document.
    var pendingScroll: CGPoint?
    @Published var errorMessage: String?
    @Published var showNewSheet = false
    let opts = DrawingOptions()
    @Published var renamingLayer: Int?
    @Published var toolSheet: ToolSheet?
    @Published var loopTags = true
    var playCursor = PlaybackCursor()
    var composite: [UInt8] = []
    private var playTask: Task<Void, Never>?

    private init() {
        doc = pixmith_new(32, 32)
        slots = [DocumentSlot(doc: doc)]
        installShortcuts()
        installSelectionShortcuts()
        installTabShortcuts()
    }

    // MARK: Queries

    var width: Int { Int(pixmith_width(doc)) }
    var height: Int { Int(pixmith_height(doc)) }
    var layerCount: Int { Int(pixmith_layer_count(doc)) }
    var frameCount: Int { Int(pixmith_frame_count(doc)) }

    func layerName(_ i: Int) -> String {
        guard let p = pixmith_layer_name(doc, UInt32(i)) else { return "" }
        defer { pixmith_string_free(p) }
        return String(cString: p)
    }

    func isVisible(_ i: Int) -> Bool { pixmith_layer_visible(doc, UInt32(i)) }
    func isGroup(_ i: Int) -> Bool { pixmith_layer_is_group(doc, UInt32(i)) }
    func level(_ i: Int) -> Int { Int(pixmith_layer_level(doc, UInt32(i))) }
    func duration(_ f: Int) -> Int { Int(pixmith_frame_duration(doc, UInt32(f))) }

    var palette: [UInt32] {
        (0..<pixmith_palette_count(doc)).map { pixmith_palette_color(doc, $0) }
    }

    var layerOpacity: Double {
        get { Double(pixmith_layer_opacity(doc, UInt32(layer))) }
        set { pixmith_set_layer_opacity(doc, UInt32(layer), UInt8(newValue.rounded())); changed() }
    }

    /// Renders a frame; the current frame's pixels are kept for the eyedropper.
    func image(frame f: Int) -> CGImage? {
        if f == frame, let preview = FilterState.shared.preview { return preview }
        var buf = [UInt8](repeating: 0, count: width * height * 4)
        guard pixmith_render(doc, UInt32(f), &buf, buf.count) else { return nil }
        if f == frame { composite = buf }
        guard let provider = CGDataProvider(data: Data(buf) as CFData) else { return nil }
        return CGImage(
            width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue),
            provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
    }

    // MARK: Editing

    func changed() {
        pixmith_touch(doc)
        ChangeCoalescer.shared.noteChange(self)
    }

    func beginStroke(_ label: String = "Draw") {
        stop()
        pixmith_begin_stroke(doc)
        labelLastStep(label)
    }

    func line(from a: (Int, Int), to b: (Int, Int), erase: Bool) {
        pixmith_draw_line(doc, UInt32(layer), UInt32(frame), Int32(a.0), Int32(a.1), Int32(b.0), Int32(b.1),
                       erase ? 0 : color, UInt32(brushSize))
        changed()
    }

    func fill(at p: (Int, Int)) {
        beginStroke("Fill")
        pixmith_fill(doc, UInt32(layer), UInt32(frame), Int32(p.0), Int32(p.1), color)
        changed()
    }

    func pick(at p: (Int, Int)) {
        guard p.0 >= 0, p.1 >= 0, p.0 < width, p.1 < height, let c = RenderCache.shared.pixel(self, x: p.0, y: p.1) else { return }
        color = c
    }

    func setVisible(_ i: Int, _ v: Bool) {
        pixmith_set_layer_visible(doc, UInt32(i), v)
        changed()
    }

    func rename(_ i: Int, to name: String) {
        name.withCString { pixmith_rename_layer(doc, UInt32(i), $0) }
        labelLastStep("Rename Layer")
        changed()
    }

    func addLayer() {
        layer = Int(pixmith_add_layer(doc, UInt32(layer)))
        labelLastStep("Add Layer")
        changed()
    }

    func removeLayer() {
        guard pixmith_remove_layer(doc, UInt32(layer)) else { return }
        labelLastStep("Delete Layer")
        clampSelection()
        changed()
    }

    func addFrame(duplicate: Bool) {
        stop()
        frame = Int(pixmith_add_frame(doc, UInt32(frame), duplicate))
        labelLastStep(duplicate ? "Duplicate Frame" : "New Frame")
        changed()
    }

    func removeFrame() {
        stop()
        guard pixmith_remove_frame(doc, UInt32(frame)) else { return }
        labelLastStep("Delete Frame")
        clampSelection()
        changed()
    }

    func setDuration(_ ms: Int) {
        pixmith_set_frame_duration(doc, UInt32(frame), UInt16(clamping: ms))
        changed()
    }

    func addColorToPalette() {
        pixmith_palette_add(doc, color)
        changed()
    }

    func undo() {
        guard !MainWindow.hasSheet else { return }
        opts.shape = nil
        if pixmith_undo(doc) { clampSelection(); revision &+= 1 }
    }

    func redo() {
        guard !MainWindow.hasSheet else { return }
        opts.shape = nil
        if pixmith_redo(doc) { clampSelection(); revision &+= 1 }
    }

    func step(_ delta: Int) {
        frame = (frame + delta + frameCount) % frameCount
    }

    func clampSelection() {
        frame = min(frame, frameCount - 1)
        layer = min(layer, layerCount - 1)
        if isGroup(layer), let i = (0..<layerCount).reversed().first(where: { !isGroup($0) }) {
            layer = i
        }
    }

    // MARK: Playback

    func togglePlay() { isPlaying ? stop() : play() }

    func play() {
        guard frameCount > 1 else { return }
        isPlaying = true
        playCursor = PlaybackCursor()
        playTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                guard let self, self.isPlaying else { return }
                try? await Task.sleep(nanoseconds: UInt64(max(self.duration(self.frame), 10)) * 1_000_000)
                guard self.isPlaying, !Task.isCancelled else { return }
                self.advance()
            }
        }
    }

    func stop() {
        isPlaying = false
        playTask?.cancel()
    }

    // MARK: Files

    func replace(with newDoc: OpaquePointer) {
        stop()
        resetTransients()
        Autosaver.shared.documentReplaced(activeSlotObject)
        pixmith_free(doc)
        doc = newDoc
        activeSlotObject?.doc = newDoc
        frame = 0
        layer = layerCount - 1
        clampSelection()
        snapDrawColor()
        zoom = Prefs.initialZoom(width: width, height: height)
        revision &+= 1
        savedRevision = revision
    }

    var lastError: String { String(cString: pixmith_last_error()) }

    func newSprite(width w: Int, height h: Int) {
        present(pixmith_new(UInt32(clamping: w), UInt32(clamping: h)), url: nil)
    }

    func open() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = Self.aseTypes + [.png]
        if panel.runModal() == .OK, let url = panel.url { open(url) }
    }

    func open(_ url: URL) {
        if url.pathExtension.lowercased() == "png" { return importPNG(url) }
        let key = url.resolvingSymlinksInPath()
        if let i = slots.firstIndex(where: { $0.fileURL?.resolvingSymlinksInPath() == key }) { return activate(i) }
        guard let d = url.path.withCString({ pixmith_open($0) }) else {
            errorMessage = lastError
            return
        }
        present(d, url: url)
        NSDocumentController.shared.noteNewRecentDocumentURL(url)
    }

    func save() {
        guard !MainWindow.hasSheet else { return }
        if let fileURL { write(to: fileURL) } else { saveAs() }
    }

    func saveAs() {
        guard !MainWindow.hasSheet else { return }
        let panel = NSSavePanel()
        panel.allowedContentTypes = Self.aseTypes
        panel.nameFieldStringValue = fileURL?.lastPathComponent ?? "Untitled.aseprite"
        if panel.runModal() == .OK, let url = panel.url { write(to: url) }
    }

    private func write(to url: URL) {
        let key = url.resolvingSymlinksInPath()
        if let other = slots.first(where: { $0 !== activeSlotObject && $0.fileURL?.resolvingSymlinksInPath() == key }) {
            errorMessage = "“\(name(of: other))” is already open in another tab. Close that tab first."
            return
        }
        finishShape()  // an open polygon or curve is part of the document being saved
        if url.path.withCString({ pixmith_save(doc, $0) }) {
            fileURL = url
            pixmith_mark_saved(doc)
            savedRevision = revision
            Autosaver.shared.discard()
        } else {
            errorMessage = lastError
        }
    }

    func exportPNG(sheet: Bool) {
        guard !MainWindow.hasSheet else { return }
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.png]
        let base = fileURL?.deletingPathExtension().lastPathComponent ?? "Untitled"
        panel.nameFieldStringValue = sheet ? "\(base)-sheet.png" : "\(base).png"
        let scales = [1, 2, 4, 8, 16]
        let popup = NSPopUpButton()
        popup.addItems(withTitles: scales.map { "Scale \($0)x" })
        popup.sizeToFit()
        panel.accessoryView = popup
        guard panel.runModal() == .OK, let url = panel.url else { return }
        let scale = UInt32(scales[popup.indexOfSelectedItem])
        let ok = url.path.withCString {
            sheet ? pixmith_export_sheet(doc, scale, $0) : pixmith_export_png(doc, UInt32(frame), scale, $0)
        }
        if !ok { errorMessage = lastError }
    }

    static let aseTypes = ["aseprite", "ase"].compactMap { UTType(filenameExtension: $0) }

    // MARK: Keyboard

    /// Single-key shortcuts, skipped while a text field is being edited.
    private func installShortcuts() {
        NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] e in
            guard let self,
                  e.modifierFlags.intersection([.command, .control, .option]).isEmpty,
                  !(NSApp.keyWindow?.firstResponder is NSText),
                  NSApp.keyWindow?.attachedSheet == nil
            else { return e }
            // Rebinding a key in Preferences records it; every other key works only in the document window.
            if Shortcuts.shared.recording != nil { return Shortcuts.shared.handle(e, model: self) ? nil : e }
            guard MainWindow.isKey else { return e }
            if handleShapeKey(e) { return nil }
            return Shortcuts.shared.handle(e, model: self) ? nil : e
        }
    }
}

extension UInt32 {
    var cgColor: CGColor {
        CGColor(srgbRed: CGFloat(self >> 24 & 0xFF) / 255, green: CGFloat(self >> 16 & 0xFF) / 255,
                blue: CGFloat(self >> 8 & 0xFF) / 255, alpha: CGFloat(self & 0xFF) / 255)
    }

    init(_ color: CGColor) {
        let c = NSColor(cgColor: color)?.usingColorSpace(.sRGB) ?? .black
        let parts = [c.redComponent, c.greenComponent, c.blueComponent, c.alphaComponent]
        self = parts.reduce(0) { $0 << 8 | UInt32(($1 * 255).rounded().clamped(0, 255)) }
    }
}

extension CGFloat {
    func clamped(_ lo: CGFloat, _ hi: CGFloat) -> CGFloat { Swift.min(Swift.max(self, lo), hi) }
}
