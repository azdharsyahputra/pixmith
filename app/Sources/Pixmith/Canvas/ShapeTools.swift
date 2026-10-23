import AppKit
import Combine
import CoreText
import PixmithCore
import SwiftUI

/// An in-progress curve, polygon or contour. The preview is drawn into the document and redrawn from the
/// checkpoint taken when the gesture began, so the whole shape stays a single undo step.
struct ShapeSession {
    var tool: Tool
    var doc: OpaquePointer
    /// Curve: start, end, first and second control point. Polygon and contour: the vertices.
    var points: [(Int, Int)]
    var color: UInt32
    var cursor: (Int, Int)
    /// Curve only: 0 dragging the line, 1 placing the first control point, 2 the second.
    var stage = 0
    var dragging = false
    /// Undo depth right after the gesture's checkpoint; a different depth means the history moved on.
    var depth = 0
}

struct TextRequest: Identifiable {
    let id = UUID()
    var at: (Int, Int)
    var color: UInt32
}

extension SpriteModel {
    // MARK: Multi-point shapes

    private func validSession() -> ShapeSession? {
        guard let s = opts.shape, s.doc == doc, historyCounts.undo == s.depth else {
            opts.shape = nil
            return nil
        }
        return s
    }

    var drawingSessionOpen: Bool { validSession() != nil }

    /// Starts or advances a curve, polygon, contour or text gesture. False for other tools.
    func shapeDown(_ p: (Int, Int), tool: Tool, event e: NSEvent) -> Bool {
        let right = e.type == .rightMouseDown
        let c = right ? opts.secondary : color
        switch tool {
        case .text:
            beginStroke("Text")
            applyInk(reverse: right)
            opts.textStarted = false
            opts.textDepth = historyCounts.undo
            opts.textRequest = TextRequest(at: p, color: c)
            return true
        case .contour:
            beginShape(ShapeSession(tool: tool, doc: doc, points: [p], color: c, cursor: p, dragging: true), right: right)
        case .curve:
            if var s = validSession(), s.tool == .curve, s.stage > 0 {
                s.points[s.stage + 1] = p
                s.dragging = true
                opts.shape = s
            } else {
                beginShape(ShapeSession(tool: tool, doc: doc, points: Array(repeating: p, count: 4), color: c, cursor: p, dragging: true), right: right)
            }
        case .shapePolygon:
            if var s = validSession(), s.tool == .shapePolygon {
                if e.clickCount >= 2 { finishShape(); return true }
                if let last = s.points.last, last != p { s.points.append(p) }
                s.cursor = p
                opts.shape = s
            } else {
                beginShape(ShapeSession(tool: tool, doc: doc, points: [p], color: c, cursor: p), right: right)
            }
        default:
            return false
        }
        drawShape()
        return true
    }

    private func beginShape(_ s: ShapeSession, right: Bool) {
        beginStroke(s.tool == .curve ? "Curve" : s.tool == .contour ? "Contour" : "Polygon")
        applyInk(reverse: right)
        var s = s
        s.depth = historyCounts.undo
        opts.shape = s
        if opts.shapeObserver == nil {
            opts.shapeObserver = $tool.sink { [weak self] new in
                if let s = self?.opts.shape, s.tool != new { self?.finishShape() }
            }
        }
    }

    func shapeDrag(_ p: (Int, Int)) -> Bool {
        guard var s = validSession() else { return false }
        switch s.tool {
        case .contour:
            guard s.points.last != nil, p != s.points.last! else { return true }
            s.points.append(p)
        case .curve:
            guard s.dragging else { return true }
            s.points[s.stage == 0 ? 1 : s.stage + 1] = p
        default:
            s.cursor = p
        }
        s.cursor = p
        opts.shape = s
        drawShape()
        return true
    }

    func shapeUp() -> Bool {
        guard var s = validSession() else { return false }
        switch s.tool {
        case .contour:
            finishShape()
        case .curve:
            s.dragging = false
            if s.stage == 0 {
                if s.points[0] == s.points[1] { opts.shape = s; return finishShape() }
                s.stage = 1
            } else if s.stage == 1 {
                s.stage = 2
            } else {
                opts.shape = s
                return finishShape()
            }
            opts.shape = s
            drawShape()
        default:
            break
        }
        return true
    }

    func shapeMoved(_ p: (Int, Int)) {
        guard var s = validSession(), !s.dragging, s.cursor != p else { return }
        s.cursor = p
        opts.shape = s
        if s.tool == .shapePolygon || (s.tool == .curve && s.stage > 0) { drawShape() }
    }

    /// Redraws the shape from the checkpoint, including the cursor as the next vertex or control point.
    private func drawShape(final: Bool = false) {
        guard let s = validSession() else { return }
        let flags = symFlags | (opts.filled ? 8 : 0)
        switch s.tool {
        case .curve:
            let (a, b) = (s.points[0], s.points[1])
            let (c1, c2): ((Int, Int), (Int, Int))
            switch s.stage {
            case 0: (c1, c2) = (a, b)
            case 1: (c1, c2) = s.dragging || final ? (s.points[2], s.points[2]) : (s.cursor, s.cursor)
            default: (c1, c2) = (s.points[2], s.dragging || final ? s.points[3] : s.cursor)
            }
            let v: [Int32] = [a.0, a.1, c1.0, c1.1, c2.0, c2.1, b.0, b.1].map(Int32.init)
            v.withUnsafeBufferPointer {
                pixmith_curve(doc, UInt32(layer), UInt32(frame), $0.baseAddress, s.color, UInt32(brushSize), flags, true)
            }
        default:
            var pts = s.points
            var flags = flags
            if !final, s.tool == .shapePolygon, pts.last! != s.cursor { pts.append(s.cursor) }
            // While dragging a contour only the path is shown; releasing closes and fills it.
            if !final, s.tool == .contour { flags |= 32 }
            let v: [Int32] = pts.flatMap { [Int32($0.0), Int32($0.1)] }
            v.withUnsafeBufferPointer {
                pixmith_polygon(doc, UInt32(layer), UInt32(frame), $0.baseAddress, pts.count, s.color, UInt32(brushSize), flags, true)
            }
        }
        changed()
    }

    /// Commits the shape as last drawn (a polygon without its cursor vertex).
    @discardableResult
    func finishShape() -> Bool {
        guard let s = validSession() else { return true }
        if s.tool == .shapePolygon || s.tool == .contour { drawShape(final: true) }
        opts.shape = nil
        changed()
        return true
    }

    func cancelShape() {
        guard validSession() != nil else { return }
        opts.shape = nil
        if pixmith_rollback(doc) { changed() }
    }

    /// Enter commits and Escape cancels an open shape.
    func handleShapeKey(_ e: NSEvent) -> Bool {
        guard validSession() != nil else { return false }
        switch e.keyCode {
        case 36, 76: finishShape()
        case 53: cancelShape()
        default: return false
        }
        return true
    }

    // MARK: Text

    /// Draws the pending text at its click position, replacing the previous preview.
    func renderText() {
        guard let r = opts.textRequest else { return }
        // A command run under the sheet moved the history on: the preview can no longer be rewound safely.
        guard historyCounts.undo == opts.textDepth else {
            opts.textRequest = nil
            return
        }
        let restore = opts.textStarted
        opts.textStarted = true
        let text = opts.textString
        let name = opts.textFontName == "System" ? nil : opts.textFontName
        if opts.textSystemFont, !text.isEmpty, let m = Self.rasterize(text, fontName: name, size: CGFloat(opts.textFontSize)) {
            m.mask.withUnsafeBufferPointer {
                pixmith_stamp_mask(doc, UInt32(layer), UInt32(frame), Int32(r.at.0), Int32(r.at.1), UInt32(m.w), UInt32(m.h),
                                $0.baseAddress, r.color, symFlags, restore)
            }
        } else {
            let t = opts.textSystemFont ? "" : text
            pixmith_text(doc, UInt32(layer), UInt32(frame), Int32(r.at.0), Int32(r.at.1), t, UInt32(opts.textScale),
                      r.color, symFlags, restore)
        }
        changed()
    }

    func placeText() {
        if opts.textString.isEmpty { return cancelText() }
        opts.textRequest = nil
    }

    func cancelText() {
        opts.textRequest = nil
        if historyCounts.undo == opts.textDepth, pixmith_rollback(doc) { changed() }
    }

    /// Rasterizes text with a system font and no antialiasing into a 0/255 coverage mask.
    static func rasterize(_ text: String, fontName: String?, size: CGFloat) -> (w: Int, h: Int, mask: [UInt8])? {
        let font = fontName.flatMap { NSFont(name: $0, size: size) } ?? NSFont.systemFont(ofSize: size)
        let lines = text.components(separatedBy: "\n").map {
            CTLineCreateWithAttributedString(NSAttributedString(string: $0, attributes: [.font: font, .foregroundColor: NSColor.white]))
        }
        let lineHeight = Int(ceil(font.ascender - font.descender + font.leading))
        let width = Int(ceil(lines.map { CTLineGetTypographicBounds($0, nil, nil, nil) }.max() ?? 0))
        let height = lineHeight * lines.count
        guard width > 0, height > 0, width * height <= 1 << 24,
              let ctx = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width,
                                  space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue),
              let data = ctx.data
        else { return nil }
        ctx.setShouldAntialias(false)
        ctx.setShouldSmoothFonts(false)
        ctx.setFillColor(gray: 1, alpha: 1)
        for (i, line) in lines.enumerated() {
            ctx.textPosition = CGPoint(x: 0, y: CGFloat(height - (i + 1) * lineHeight) - font.descender)
            CTLineDraw(line, ctx)
        }
        let mask = Array(UnsafeBufferPointer(start: data.assumingMemoryBound(to: UInt8.self), count: width * height))
        return (width, height, mask)
    }

    // MARK: Pen pressure

    /// Color and brush size adjusted by pen pressure, or nil for mouse input and when pressure is off.
    func pressureStyle(_ e: NSEvent, color: UInt32, size: Int? = nil) -> (color: UInt32, size: UInt32)? {
        guard e.subtype == .tabletPoint, opts.pressureSize || opts.pressureAlpha else { return nil }
        let pressure = Double(e.pressure).clamped(0.02, 1)
        let base = size ?? brushSize
        let sz = opts.pressureSize ? max(1, Int((Double(base) * pressure).rounded())) : base
        var c = color
        if opts.pressureAlpha, color & 0xFF != 0 {
            c = color & ~0xFF | UInt32(max(1, (Double(color & 0xFF) * pressure).rounded()))
        }
        return (c, UInt32(sz))
    }
}

private extension Double {
    func clamped(_ lo: Double, _ hi: Double) -> Double { Swift.min(Swift.max(self, lo), hi) }
}

// MARK: Views

struct PressureOptionsView: View {
    @ObservedObject var opts: DrawingOptions

    init(model: SpriteModel) { opts = model.opts }

    var body: some View {
        DSChip(symbol: "arrow.up.left.and.arrow.down.right", help: "Scale the brush size by pen pressure (tablet only)",
               isOn: $opts.pressureSize)
        DSChip(symbol: "circle.lefthalf.filled", help: "Scale the color's opacity by pen pressure (tablet only)",
               isOn: $opts.pressureAlpha)
    }
}

struct TextSheet: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject var opts: DrawingOptions

    private static let families = ["System"] + ["Helvetica", "Menlo", "Courier", "Times New Roman", "Georgia", "Arial", "Chalkboard SE"]
        .filter { NSFont(name: $0, size: 12) != nil }

    var body: some View {
        DSDialog(title: "Text", subtitle: "Drawn in the primary color at the clicked position (right-click: secondary).",
                 primary: "Place", width: 380, onPrimary: { model.placeText() }, onCancel: { model.cancelText() }) {
            TextEditor(text: $opts.textString)
                .font(.system(size: 13, design: .monospaced))
                .scrollContentBackground(.hidden)
                .padding(DS.Space.xs)
                .frame(height: 90)
                .dsFieldBackground()
            Toggle("Use system font (rasterized without antialiasing)", isOn: $opts.textSystemFont)
                .toggleStyle(.checkbox)
            if opts.textSystemFont {
                DSFormRow(label: "Font") {
                    DSPopup(selection: $opts.textFontName, options: Self.families.map { ($0, $0) }, fill: true)
                }
                DSFormRow(label: "Size") {
                    DSNumberField("Size", value: Binding(get: { Int(opts.textFontSize) }, set: { opts.textFontSize = Double($0) }),
                                  range: 6...72, unit: "pt", width: 120)
                }
            } else {
                DSFormRow(label: "Pixel scale") {
                    DSNumberField("Scale", value: $opts.textScale, range: 1...16, unit: "x", width: 120)
                }
            }
        }
        .onAppear { model.renderText() }
        .onChange(of: opts.textString) { _ in model.renderText() }
        .onChange(of: opts.textScale) { _ in model.renderText() }
        .onChange(of: opts.textSystemFont) { _ in model.renderText() }
        .onChange(of: opts.textFontName) { _ in model.renderText() }
        .onChange(of: opts.textFontSize) { _ in model.renderText() }
    }
}

private struct TextSheetModifier: ViewModifier {
    @ObservedObject var model: SpriteModel
    @ObservedObject var opts: DrawingOptions

    func body(content: Content) -> some View {
        content.sheet(item: $opts.textRequest) { _ in TextSheet(model: model, opts: opts) }
    }
}

extension View {
    func textSheet(model: SpriteModel) -> some View {
        modifier(TextSheetModifier(model: model, opts: model.opts))
    }
}
