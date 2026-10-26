import AppKit
import PixmithCore
import Combine
import SwiftUI

/// Mouse-drag state for the selection tools (the model itself cannot hold extension state).
private final class SelectionDrag {
    static let shared = SelectionDrag()
    var active = false
    var tool: Tool = .marquee
    var start = (0, 0)
    var moved = false
    var mode: UInt32 = 0
    var points: [Int32] = []
    var polygonOpen = false
    var selectByColorArmed = false
    var observer: AnyCancellable?
}

/// Options of the magic wand, shown in the tool panel.
final class SelectionOptions: ObservableObject {
    static let shared = SelectionOptions()
    @Published var tolerance = 0
    @Published var contiguous = true
}

extension Tool {
    var isSelection: Bool { [.marquee, .wand, .lasso, .polygon].contains(self) }
}

/// Cached outline segments plus the marching-ants animation.
final class SelectionOutline {
    static let shared = SelectionOutline()
    private var revision = -1
    private(set) var segments: [Int32] = []
    private var phase: CGFloat = 0
    private(set) weak var canvas: NSView?
    private var timer: Timer?

    func update(for m: SpriteModel, canvas: NSView) -> (segments: [Int32], phase: CGFloat) {
        if revision != m.revision {
            revision = m.revision
            let n = pixmith_sel_outline(m.doc, nil, 0)
            segments = [Int32](repeating: 0, count: n * 4)
            if n > 0 { _ = pixmith_sel_outline(m.doc, &segments, n) }
        }
        self.canvas = canvas
        if timer == nil, !segments.isEmpty {
            timer = Timer.scheduledTimer(withTimeInterval: 0.12, repeats: true) { [weak self] _ in
                guard let self else { return }
                if self.segments.isEmpty {
                    self.timer?.invalidate()
                    self.timer = nil
                    return
                }
                self.phase = (self.phase + 1).truncatingRemainder(dividingBy: 8)
                self.canvas?.needsDisplay = true
            }
        }
        return (segments, phase)
    }
}

extension CanvasNSView {
    func drawSelectionOutline(_ ctx: CGContext, _ m: SpriteModel) {
        let (segs, phase) = SelectionOutline.shared.update(for: m, canvas: self)
        guard !segs.isEmpty else { return }
        let z = m.zoom
        let path = CGMutablePath()
        for i in stride(from: 0, to: segs.count, by: 4) {
            path.move(to: CGPoint(x: CGFloat(segs[i]) * z, y: CGFloat(segs[i + 1]) * z))
            path.addLine(to: CGPoint(x: CGFloat(segs[i + 2]) * z, y: CGFloat(segs[i + 3]) * z))
        }
        ctx.saveGState()
        ctx.setLineWidth(1)
        ctx.setStrokeColor(NSColor.white.cgColor)
        ctx.addPath(path)
        ctx.strokePath()
        ctx.setStrokeColor(NSColor.black.cgColor)
        ctx.setLineDash(phase: phase, lengths: [4, 4])
        ctx.addPath(path)
        ctx.strokePath()
        ctx.restoreGState()
    }
}

extension SpriteModel {
    var hasSelection: Bool { pixmith_sel_active(doc) }
    var selectionDragging: Bool { SelectionDrag.shared.active }

    // MARK: Selection

    func selectAll() {
        pixmith_sel_all(doc)
        labelLastStep("Select All")
        changed()
    }

    func deselect() {
        pixmith_sel_clear(doc)
        labelLastStep("Deselect")
        changed()
    }

    func invertSelection() {
        pixmith_sel_invert(doc)
        labelLastStep("Invert Selection")
        changed()
    }

    // MARK: Mouse

    /// Shift adds to the selection, option subtracts from it.
    private func selectionMode(_ e: NSEvent) -> UInt32 {
        e.modifierFlags.contains(.shift) ? 1 : e.modifierFlags.contains(.option) ? 2 : 0
    }

    func selectionMouseDown(_ p: (Int, Int), tool: Tool, event e: NSEvent) {
        stop()
        let d = SelectionDrag.shared
        switch tool {
        case .wand:
            let o = SelectionOptions.shared
            if pixmith_sel_wand(doc, UInt32(layer), UInt32(frame), Int32(p.0), Int32(p.1), UInt32(o.tolerance),
                             o.contiguous, selectionMode(e)) { changed() }
        case .polygon:
            if e.clickCount >= 2 { return finishPolygon() }
            if !d.polygonOpen {
                pixmith_sel_begin(doc, selectionMode(e))
                d.polygonOpen = true
                d.points = []
            }
            d.points += [Int32(p.0), Int32(p.1)]
            pixmith_sel_poly(doc, d.points, d.points.count / 2)
            changed()
        case .move:
            if pixmith_float_active(doc), !pixmith_sel_contains(doc, Int32(p.0), Int32(p.1)) {
                pixmith_float_commit(doc)
                return
            }
            d.tool = tool
            d.start = p
            d.active = pixmith_sel_move_begin(doc, UInt32(layer), UInt32(frame))
            if d.active { labelLastStep("Move") }
        default:
            d.tool = tool
            d.start = p
            d.moved = false
            d.mode = selectionMode(e)
            d.points = [Int32(p.0), Int32(p.1)]
            d.active = true
        }
    }

    func selectionMouseDragged(_ p: (Int, Int)) {
        let d = SelectionDrag.shared
        guard d.active, p != d.start || d.moved else { return }
        if d.tool == .move {
            pixmith_sel_move_to(doc, Int32(p.0 - d.start.0), Int32(p.1 - d.start.1))
        } else {
            if !d.moved { pixmith_sel_begin(doc, d.mode) }
            if d.tool == .lasso {
                if d.points.suffix(2) != [Int32(p.0), Int32(p.1)] { d.points += [Int32(p.0), Int32(p.1)] }
                pixmith_sel_poly(doc, d.points, d.points.count / 2)
            } else {
                pixmith_sel_rect(doc, Int32(d.start.0), Int32(d.start.1), Int32(p.0), Int32(p.1))
            }
        }
        d.moved = true
        changed()
    }

    func selectionMouseUp() {
        let d = SelectionDrag.shared
        guard d.active else { return }
        d.active = false
        if d.tool == .move {
            pixmith_sel_move_end(doc)
        } else if d.moved {
            pixmith_sel_end(doc)
        } else if d.mode == 0 {
            pixmith_sel_clear(doc)
        }
        changed()
    }

    func finishPolygon() {
        let d = SelectionDrag.shared
        guard d.polygonOpen else { return }
        d.polygonOpen = false
        d.points = []
        pixmith_sel_end(doc)
        changed()
    }

    private func cancelPolygon() {
        let d = SelectionDrag.shared
        guard d.polygonOpen else { return }
        finishPolygon()
        undo()
    }

    func nudge(dx: Int, dy: Int) {
        stop()
        if pixmith_sel_nudge(doc, UInt32(layer), UInt32(frame), Int32(dx), Int32(dy)) { labelLastStep("Nudge"); changed() }
    }

    // MARK: Select by color and modify

    /// Selects every pixel of the current cel matching the color under the cursor; when the cursor is
    /// not over the canvas (menu click), the next click on the canvas picks the color instead.
    func selectByColor() {
        if let canvas = SelectionOutline.shared.canvas, let w = canvas.window {
            let pt = canvas.convert(w.mouseLocationOutsideOfEventStream, from: nil)
            if canvas.bounds.contains(pt) {
                return selectByColor(at: (Int((pt.x / zoom).rounded(.down)), Int((pt.y / zoom).rounded(.down))))
            }
        }
        SelectionDrag.shared.selectByColorArmed = true
    }

    private func selectByColor(at p: (Int, Int)) {
        if pixmith_sel_wand(doc, UInt32(layer), UInt32(frame), Int32(p.0), Int32(p.1),
                         UInt32(SelectionOptions.shared.tolerance), false, 0) { changed() }
    }

    /// Consumes the click that completes a menu-initiated "Select by Color".
    func consumeSelectByColorClick(_ p: (Int, Int)) -> Bool {
        let d = SelectionDrag.shared
        guard d.selectByColorArmed else { return false }
        d.selectByColorArmed = false
        selectByColor(at: p)
        return true
    }

    func modifySelection(_ kind: UInt32) {
        let titles = ["Expand Selection", "Contract Selection", "Border Selection"]
        guard hasSelection, let n = promptNumber(titles[Int(kind)], "Pixels", initial: 1) else { return }
        if pixmith_sel_modify(doc, kind, UInt32(n)) { labelLastStep(titles[Int(kind)]); changed() }
    }

    private func promptNumber(_ title: String, _ label: String, initial: Int) -> Int? {
        let alert = NSAlert()
        alert.messageText = title
        let field = NSTextField(string: "\(initial)")
        field.frame = NSRect(x: 0, y: 0, width: 120, height: 22)
        alert.informativeText = label
        alert.accessoryView = field
        alert.addButton(withTitle: "OK")
        alert.addButton(withTitle: "Cancel")
        alert.window.initialFirstResponder = field
        guard alert.runModal() == .alertFirstButtonReturn else { return nil }
        return min(max(field.integerValue, 0), 1024)
    }

    // MARK: Edit

    func deleteSelection() {
        if TimelineSelection.shared.focused { return clearSelectedCels() }
        stop()
        if pixmith_sel_delete(doc, UInt32(layer), UInt32(frame)) { labelLastStep("Delete"); changed() }
    }

    func transform(_ op: UInt32) {
        stop()
        if pixmith_sel_transform(doc, UInt32(layer), UInt32(frame), op) { labelLastStep("Transform"); changed() }
    }

    // MARK: Clipboard

    private static var copyOrigin: (changeCount: Int, x: Int, y: Int)?

    func copySelection(cut: Bool) {
        if TimelineSelection.shared.focused { return copyCels(cut: cut) }
        var b = [Int32](repeating: 0, count: 4)
        pixmith_sel_copy_bounds(doc, &b)
        let (w, h) = (Int(b[2]), Int(b[3]))
        var buf = [UInt8](repeating: 0, count: w * h * 4)
        guard w > 0, h > 0, pixmith_sel_copy(doc, UInt32(layer), UInt32(frame), &buf, buf.count),
              let provider = CGDataProvider(data: Data(buf) as CFData),
              let image = CGImage(
                width: w, height: h, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: w * 4,
                space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue),
                provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
        else { return }
        let rep = NSBitmapImageRep(cgImage: image)
        let pb = NSPasteboard.general
        pb.clearContents()
        if let png = rep.representation(using: .png, properties: [:]) { pb.setData(png, forType: .png) }
        if let tiff = rep.tiffRepresentation { pb.setData(tiff, forType: .tiff) }
        Self.copyOrigin = (pb.changeCount, Int(b[0]), Int(b[1]))
        if cut { deleteSelection() }
    }

    var canPaste: Bool {
        let pb = NSPasteboard.general
        return pb.data(forType: .png) != nil || pb.data(forType: .tiff) != nil
    }

    func pasteFromPasteboard() {
        if TimelineSelection.shared.focused, pixmith_cels_has_clip() { return pasteCels() }
        let pb = NSPasteboard.general
        guard let data = pb.data(forType: .png) ?? pb.data(forType: .tiff),
              let cg = NSBitmapImageRep(data: data)?.cgImage else { return }
        let (w, h) = (cg.width, cg.height)
        var buf = [UInt8](repeating: 0, count: w * h * 4)
        let drawn = buf.withUnsafeMutableBytes { raw -> Bool in
            guard let ctx = CGContext(
                data: raw.baseAddress, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4,
                space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
            ctx.draw(cg, in: CGRect(x: 0, y: 0, width: w, height: h))
            return true
        }
        guard drawn else { return }
        for i in stride(from: 0, to: buf.count, by: 4) {
            let a = Int(buf[i + 3])
            if a > 0, a < 255 {
                for c in 0..<3 { buf[i + c] = UInt8(min(255, (Int(buf[i + c]) * 255 + a / 2) / a)) }
            }
        }
        var origin = (x: max(0, (width - w) / 2), y: max(0, (height - h) / 2))
        if let o = Self.copyOrigin, o.changeCount == pb.changeCount { origin = (o.x, o.y) }
        stop()
        tool = .move
        if pixmith_sel_paste(doc, UInt32(layer), UInt32(frame), UInt32(w), UInt32(h), buf, Int32(origin.x), Int32(origin.y)) {
            labelLastStep("Paste")
            changed()
        }
    }

    // MARK: Canvas size

    func cropToSelection() {
        stop()
        if pixmith_crop_to_selection(doc) { labelLastStep("Crop"); changed() }
    }

    func showCanvasSizeDialog() {
        let alert = NSAlert()
        alert.messageText = "Canvas Size"
        alert.informativeText = "Resizes every frame and layer. Anchor decides where existing pixels stay."
        let wField = NSTextField(string: "\(width)")
        let hField = NSTextField(string: "\(height)")
        let anchor = NSPopUpButton()
        let names = ["Top Left", "Top", "Top Right", "Left", "Center", "Right", "Bottom Left", "Bottom", "Bottom Right"]
        anchor.addItems(withTitles: names)
        anchor.selectItem(at: 4)
        let rows = [("Width", wField), ("Height", hField), ("Anchor", anchor)].map { label, control -> NSStackView in
            let text = NSTextField(labelWithString: label)
            text.widthAnchor.constraint(equalToConstant: 56).isActive = true
            control.widthAnchor.constraint(equalToConstant: 140).isActive = true
            return NSStackView(views: [text, control])
        }
        let stack = NSStackView(views: rows)
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.frame = NSRect(x: 0, y: 0, width: 210, height: 90)
        alert.accessoryView = stack
        alert.addButton(withTitle: "Resize")
        alert.addButton(withTitle: "Cancel")
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        let nw = min(max(wField.integerValue, 1), 16384)
        let nh = min(max(hField.integerValue, 1), 16384)
        let i = anchor.indexOfSelectedItem
        let (ax, ay) = (i % 3, i / 3)
        stop()
        if pixmith_resize_canvas(doc, UInt32(nw), UInt32(nh), Int32((nw - width) * ax / 2), Int32((nh - height) * ay / 2)) {
            labelLastStep("Canvas Size")
            changed()
        }
    }

    // MARK: Keyboard

    /// Arrow-key nudging, Delete, Enter and Escape, skipped while a text field is being edited.
    /// Also commits a floating paste or open polygon when the tool changes.
    func installSelectionShortcuts() {
        SelectionDrag.shared.observer = $tool.dropFirst().sink { [weak self] _ in
            self?.finishPolygon()
            TimelineSelection.shared.focused = false
            if let self { pixmith_float_commit(doc); pixmith_xf_commit(doc) }
        }
        NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] e in
            guard let self,
                  e.modifierFlags.intersection([.command, .control, .option]).isEmpty,
                  !(NSApp.keyWindow?.firstResponder is NSText),
                  NSApp.keyWindow?.attachedSheet == nil,
                  !drawingSessionOpen,
                  Shortcuts.shared.recording == nil,
                  MainWindow.isKey
            else { return e }
            let step = e.modifierFlags.contains(.shift) ? 10 : 1
            if transformActive {
                switch e.keyCode {
                case 36, 76: commitTransform()
                case 53: cancelTransform()
                case 123: nudgeTransform(dx: -Double(step), dy: 0)
                case 124: nudgeTransform(dx: Double(step), dy: 0)
                case 125: nudgeTransform(dx: 0, dy: Double(step))
                case 126: nudgeTransform(dx: 0, dy: -Double(step))
                default: return e
                }
                return nil
            }
            switch e.keyCode {
            case 123 where tool == .move || hasSelection: nudge(dx: -step, dy: 0)
            case 124 where tool == .move || hasSelection: nudge(dx: step, dy: 0)
            case 125 where tool == .move || hasSelection: nudge(dx: 0, dy: step)
            case 126 where tool == .move || hasSelection: nudge(dx: 0, dy: -step)
            case 51, 117: if tool == .slice { deleteSelectedSlice() } else { deleteSelection() }
            case 36, 76:
                guard SelectionDrag.shared.polygonOpen || pixmith_float_active(doc) else { return e }
                finishPolygon()
                pixmith_float_commit(doc)
            case 53:
                if SelectionDrag.shared.polygonOpen {
                    cancelPolygon()
                } else if pixmith_float_active(doc) {
                    if pixmith_float_cancel(doc) { changed() }
                } else if hasSelection {
                    deselect()
                } else {
                    return e
                }
            default: return e
            }
            return nil
        }
    }
}

/// Runs `action` on the canvas, or forwards `selector` to the focused text field.
private func editAction(_ selector: Selector, _ action: () -> Void) {
    if NSApp.keyWindow?.firstResponder is NSText {
        NSApp.sendAction(selector, to: nil, from: nil)
    } else if NSApp.keyWindow?.attachedSheet == nil {
        action()
    }
}

struct SelectionCommands: Commands {
    @ObservedObject var model: SpriteModel

    var body: some Commands {
        CommandGroup(replacing: .pasteboard) {
            Button("Cut") { editAction(#selector(NSText.cut(_:))) { model.copySelection(cut: true) } }
                .keyboardShortcut("x")
            Button("Copy") { editAction(#selector(NSText.copy(_:))) { model.copySelection(cut: false) } }
                .keyboardShortcut("c")
            Button("Paste") { editAction(#selector(NSText.paste(_:))) { model.pasteFromPasteboard() } }
                .keyboardShortcut("v")
            Button("Delete") { editAction(#selector(NSText.delete(_:))) { model.deleteSelection() } }
            Divider()
            Button("Free Transform") { model.startTransform() }
                .keyboardShortcut("t")
            Button("Flip Horizontal") { model.transform(0) }
            Button("Flip Vertical") { model.transform(1) }
            Button("Rotate 90° Clockwise") { model.transform(2) }
            Button("Rotate 90° Counterclockwise") { model.transform(3) }
        }
        CommandMenu("Select") {
            Button("Select All") { editAction(#selector(NSText.selectAll(_:))) { model.selectAll() } }
                .keyboardShortcut("a")
            Button("Deselect") { model.deselect() }.keyboardShortcut("d")
            Button("Invert Selection") { model.invertSelection() }.keyboardShortcut("i", modifiers: [.command, .shift])
            Divider()
            Menu("Modify") {
                Button("Expand…") { model.modifySelection(0) }
                Button("Contract…") { model.modifySelection(1) }
                Button("Border…") { model.modifySelection(2) }
            }
            Button("Select by Color") { model.selectByColor() }.keyboardShortcut("b", modifiers: [.command, .option])
            Button("Isometric Tile") { model.selectIsometricTile() }
        }
    }
}

/// Magic wand options, shown in the context bar while the wand is active.
struct SelectionOptionsView: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var opts = SelectionOptions.shared

    var body: some View {
        if model.tool == .wand {
            HStack(spacing: DS.Space.sm) {
                DSNumberField("Tolerance", value: $opts.tolerance, range: 0...255, width: 128,
                              help: "Magic wand tolerance (0-255). Drag the label to scrub.")
                DSChip(opts.contiguous ? "Contiguous" : "Global",
                       symbol: opts.contiguous ? "link" : "globe",
                       help: opts.contiguous ? "Contiguous (click for global)" : "Global (click for contiguous)",
                       isOn: Binding(get: { opts.contiguous }, set: { opts.contiguous = $0 }))
            }
        }
    }
}

struct SpriteSelectionMenuItems: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        Divider()
        Button("Canvas Size…") { model.showCanvasSizeDialog() }
        Button("Crop to Selection") { model.cropToSelection() }.disabled(!model.hasSelection)
        Divider()
    }
}

extension SpriteModel {
    /// Tab switch or document replacement: forget the mouse-drag state of the document being left.
    func resetSelectionTransients() {
        let d = SelectionDrag.shared
        d.active = false
        d.moved = false
        d.polygonOpen = false
        d.points = []
        d.selectByColorArmed = false
    }
}
