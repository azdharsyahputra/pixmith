import AppKit
import Combine
import PixmithCore
import SwiftUI
import UniformTypeIdentifiers

/// Option flags understood by the Rust drawing calls (see pixmith_shapes.h).
private enum Flag {
    static let mirrorX: UInt32 = 1, mirrorY: UInt32 = 2, pixelPerfect: UInt32 = 4, filled: UInt32 = 8, constrain: UInt32 = 16
}

final class DrawingOptions: ObservableObject {
    @Published var secondary: UInt32 = 0xFFFFFFFF
    @Published var filled = false { didSet { refreshCanvas() } }
    @Published var pixelPerfect = false
    @Published var mirrorX = false { didSet { refreshCanvas() } }
    @Published var mirrorY = false { didSet { refreshCanvas() } }
    @Published var contiguous = true
    @Published var sprayDensity = 40.0
    @Published var retouchStrength = 50.0
    @Published var ink = InkMode.simple
    @Published var circleBrush = false { didSet { pixmith_brush_circle(circleBrush) } }
    @Published var customBrush: (w: Int, h: Int)?
    @Published var gradientRadial = false
    @Published var gradientDither = 0
    @Published var fillPattern = 0
    @Published var pressureSize = false
    @Published var pressureAlpha = false
    @Published var textRequest: TextRequest?
    @Published var textString = ""
    @Published var textScale = 2
    @Published var textSystemFont = false
    @Published var textFontName = "System"
    @Published var textFontSize = 12.0
    var textStarted = false
    /// Undo depth right after the text gesture's checkpoint.
    var textDepth = 0
    var flagsMonitor: Any?
    var shape: ShapeSession?
    var shapeObserver: AnyCancellable?

    /// The gesture in progress; not published because the canvas redraws through the model.
    var drag: Drag?

    struct Drag {
        var tool: Tool
        var start: (Int, Int)
        var last: (Int, Int)
        var color: UInt32
        var constrain = false
        var right = false
        /// Gradient: the checkpoint and first preview exist.
        var started = false
        var timer: Timer?
    }

    private func refreshCanvas() { SpriteModel.shared.objectWillChange.send() }
}

extension SpriteModel {
    var symFlags: UInt32 { (opts.mirrorX ? Flag.mirrorX : 0) | (opts.mirrorY ? Flag.mirrorY : 0) }


    // MARK: Mouse

    /// Handles a press for the drawing tools. Returns false for tools it does not own.
    func drawingDown(_ p: (Int, Int), tool: Tool, event e: NSEvent) -> Bool {
        let right = e.type == .rightMouseDown
        let c = right ? opts.secondary : color
        let constrain = e.modifierFlags.contains(.shift)
        if shapeDown(p, tool: tool, event: e) { return true }
        switch tool {
        case .curve, .contour, .shapePolygon, .text:
            return false
        case .picker, .marquee, .move, .wand, .lasso, .polygon, .slice:
            return false
        case .fill:
            beginStroke("Fill")
            applyInk(reverse: right)
            pixmith_fill_pattern(doc, UInt32(layer), UInt32(frame), Int32(p.0), Int32(p.1), c, right ? color : opts.secondary,
                              UInt32(opts.fillPattern), opts.contiguous, symFlags)
            changed()
            return true
        case .gradient:
            break  // the undo step starts with the first drag, so a plain click leaves nothing behind
        case .pencil, .eraser:
            beginStroke(tool == .eraser ? "Erase" : "Draw")
            applyInk(reverse: right)
            let flags = symFlags | (opts.pixelPerfect ? Flag.pixelPerfect : 0)
            let pen = pressureStyle(e, color: tool == .eraser ? 0 : c)
            pixmith_stroke_begin(doc, UInt32(layer), UInt32(frame), Int32(p.0), Int32(p.1),
                              pen?.color ?? (tool == .eraser ? 0 : c), pen?.size ?? UInt32(brushSize), flags)
        case .blur, .jumble:
            beginStroke(tool == .blur ? "Blur" : "Jumble")
            let pen = pressureStyle(e, color: c)
            pixmith_retouch_begin(doc, UInt32(layer), UInt32(frame), Int32(p.0), Int32(p.1), tool == .blur ? 0 : 1,
                               pen?.size ?? UInt32(brushSize), tool == .blur ? UInt32(opts.retouchStrength) : 100,
                               .random(in: 0...UInt32.max), symFlags)
        case .line, .rectangle, .ellipse:
            beginStroke(tool.rawValue.capitalized)
            applyInk(reverse: right)
            shape(tool, from: p, to: p, color: c, constrain: constrain, restore: false)
        case .spray:
            beginStroke("Spray")
            applyInk(reverse: right)
            spray(at: p, color: c)
        }
        var drag = DrawingOptions.Drag(tool: tool, start: p, last: p, color: c, constrain: constrain, right: right)
        if opts.flagsMonitor == nil {
            opts.flagsMonitor = NSEvent.addLocalMonitorForEvents(matching: .flagsChanged) { [weak self] e in
                self?.drawingFlagsChanged(e)
                return e
            }
        }
        if tool == .spray {
            drag.timer = Timer(timeInterval: 0.03, repeats: true) { [weak self] _ in
                guard let self, let d = self.opts.drag else { return }
                // A lost mouse-up (focus change mid-drag) must not leave the spray running.
                guard NSEvent.pressedMouseButtons != 0 else { _ = self.drawingUp(); return }
                self.spray(at: d.last, color: d.color)
            }
            RunLoop.main.add(drag.timer!, forMode: .common)
        }
        opts.drag = drag
        changed()
        return true
    }

    func drawingDrag(_ p: (Int, Int), event e: NSEvent) -> Bool {
        if shapeDrag(p) { return true }
        guard var d = opts.drag else { return false }
        let constrain = e.modifierFlags.contains(.shift)
        guard p != d.last || constrain != d.constrain else { return true }
        switch d.tool {
        case .pencil, .eraser:
            if let pen = pressureStyle(e, color: d.tool == .eraser ? 0 : d.color) {
                pixmith_stroke_style(doc, pen.color, pen.size)
            }
            pixmith_stroke_to(doc, Int32(p.0), Int32(p.1))
        case .blur, .jumble:
            if let pen = pressureStyle(e, color: d.color) { pixmith_retouch_size(doc, pen.size) }
            pixmith_retouch_to(doc, Int32(p.0), Int32(p.1))
        case .line, .rectangle, .ellipse:
            shape(d.tool, from: d.start, to: p, color: d.color, constrain: constrain, restore: true)
        case .spray:
            spray(at: p, color: d.color)
        case .gradient:
            gradientDrag(&d, to: p, constrain: constrain)
        default:
            break
        }
        d.last = p
        d.constrain = constrain
        opts.drag = d
        changed()
        return true
    }

    func drawingUp() -> Bool {
        if shapeUp() { return true }
        guard let d = opts.drag else { return false }
        d.timer?.invalidate()
        pixmith_stroke_end()
        opts.drag = nil
        changed()
        return true
    }

    func shape(_ tool: Tool, from a: (Int, Int), to b: (Int, Int), color: UInt32, constrain: Bool, restore: Bool) {
        let kind: UInt32 = tool == .line ? 0 : tool == .rectangle ? 1 : 2
        let flags = symFlags | (opts.filled ? Flag.filled : 0) | (constrain ? Flag.constrain : 0)
        pixmith_shape(doc, kind, UInt32(layer), UInt32(frame), Int32(a.0), Int32(a.1), Int32(b.0), Int32(b.1),
                   color, UInt32(brushSize), flags, restore)
    }

    private func spray(at p: (Int, Int), color: UInt32) {
        pixmith_spray(doc, UInt32(layer), UInt32(frame), Int32(p.0), Int32(p.1), color,
                   UInt32(max(3, brushSize * 3)), UInt32(opts.sprayDensity), .random(in: 0...UInt32.max), symFlags)
    }

    // MARK: Colors and keys

    func swapColors() {
        (color, opts.secondary) = (opts.secondary, color)
    }

    func drawMirrorGuides(_ ctx: CGContext, size: CGSize) {
        guard opts.mirrorX || opts.mirrorY else { return }
        ctx.saveGState()
        ctx.setStrokeColor(NSColor.systemPink.withAlphaComponent(0.8).cgColor)
        ctx.setLineDash(phase: 0, lengths: [4, 3])
        ctx.setLineWidth(1)
        if opts.mirrorX {
            ctx.move(to: CGPoint(x: size.width / 2, y: 0))
            ctx.addLine(to: CGPoint(x: size.width / 2, y: size.height))
        }
        if opts.mirrorY {
            ctx.move(to: CGPoint(x: 0, y: size.height / 2))
            ctx.addLine(to: CGPoint(x: size.width, y: size.height / 2))
        }
        ctx.strokePath()
        ctx.restoreGState()
    }

    // MARK: Palette

    func removePaletteColor(at i: Int) {
        if pixmith_palette_remove(doc, UInt32(i)) {
            labelLastStep("Remove Palette Color")
            changed()
        }
    }

    func sortPalette(byHue: Bool) {
        pixmith_palette_sort(doc, byHue ? 0 : 1)
        labelLastStep(byHue ? "Sort Palette by Hue" : "Sort Palette by Luminance")
        changed()
    }

    func loadPalette() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = Self.paletteTypes
        guard panel.runModal() == .OK, let url = panel.url else { return }
        if url.path.withCString({ pixmith_palette_load(doc, $0) }) {
            labelLastStep("Load Palette")
            changed()
        } else {
            errorMessage = lastError
        }
    }

    func savePalette() {
        let panel = NSSavePanel()
        panel.allowedContentTypes = Self.paletteTypes
        panel.nameFieldStringValue = (fileURL?.deletingPathExtension().lastPathComponent ?? "palette") + ".gpl"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        if !url.path.withCString({ pixmith_palette_save(doc, $0) }) { errorMessage = lastError }
    }

    static let paletteTypes = ["gpl", "hex"].compactMap { UTType(filenameExtension: $0) }

    @ViewBuilder
    func paletteEntryMenu(_ i: Int, _ c: UInt32) -> some View {
        Button("Set as Primary") { self.color = c }
        Button("Set as Secondary") { self.opts.secondary = c }
        Divider()
        Button("Remove from Palette") { self.removePaletteColor(at: i) }
    }
}

// MARK: Views

/// The active tool's options, in small groups. Only what the tool uses is shown; the symmetry toggles stay put at the
/// right so nothing moves when the tool changes.
struct ToolOptionsBar: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject var opts: DrawingOptions

    @ObservedObject private var terrain = AutotileState.shared
    @ObservedObject private var tiles = TilemapState.shared

    init(model: SpriteModel) {
        self.model = model
        opts = model.opts
    }

    /// The terrain brush and tile stamp replace the drawing tool's options while they are on.
    private enum Mode { case tool, terrain, stamp }
    private var mode: Mode {
        guard model.isTilemapLayer else { return .tool }
        if terrain.brush && model.terrainMode >= 0 { return .terrain }
        return tiles.stamp ? .stamp : .tool
    }

    var body: some View {
        HStack(spacing: 6) {
            Text(mode == .terrain ? "Terrain Brush" : mode == .stamp ? "Tile Stamp" : model.tool.dsTitle)
                .font(DS.Font.bodyLarge.weight(.semibold)).lineLimit(1).fixedSize()
            Group {
                switch mode {
                case .terrain: TerrainOptions(model: model)
                case .stamp:
                    DSGroupDivider()
                    HintLabel(keys: ["⇧"], text: "erase")
                    HintLabel(keys: ["⌥"], text: "pick")
                case .tool: options
                }
            }
            .fixedSize()
            Spacer(minLength: DS.Space.sm)
            DSChip(symbol: "flip.horizontal", help: "Mirror left/right across the canvas center", isOn: $opts.mirrorX)
            DSChip(symbol: "arrow.up.and.down.righttriangle.up.righttriangle.down",
                   help: "Mirror top/bottom across the canvas center", isOn: $opts.mirrorY)
        }
    }

    @ViewBuilder private var options: some View {
        switch model.tool {
        case .pencil:
            group { BrushOptionsView(model: model) }
            group { InkOptionsView(model: model, brush: false) }
            group {
                DSChip(symbol: "stairs", help: "Remove L-shaped corners while drawing", isOn: $opts.pixelPerfect)
                PressureOptionsView(model: model)
            }
        case .eraser:
            group { BrushOptionsView(model: model) }
            group { PressureOptionsView(model: model) }
        case .spray:
            group { BrushOptionsView(model: model) }
            group {
                DSSlider("Density", value: $opts.sprayDensity, range: 5...100, unit: "%").frame(width: 150)
            }
            group { InkOptionsView(model: model, brush: false) }
        case .blur:
            group { BrushOptionsView(model: model) }
            group { DSSlider("Strength", value: $opts.retouchStrength, range: 5...100, unit: "%").frame(width: 150) }
            group { DSChip(symbol: "arrow.up.left.and.arrow.down.right", help: "Scale the brush size by pen pressure (tablet only)", isOn: $opts.pressureSize) }
        case .jumble:
            group { BrushOptionsView(model: model) }
            group { HintLabel(text: "swaps nearby pixels") }
            group { DSChip(symbol: "arrow.up.left.and.arrow.down.right", help: "Scale the brush size by pen pressure (tablet only)", isOn: $opts.pressureSize) }
        case .fill:
            group {
                DSChip("Contiguous", help: "Off: replace every matching pixel in the cel", isOn: $opts.contiguous)
                FillPatternPicker(model: model)
            }
        case .gradient:
            group { GradientOptionsView(model: model) }
        case .line:
            group { BrushOptionsView(model: model) }
            group { InkOptionsView(model: model, brush: false) }
            group { HintLabel(keys: ["⇧"], text: "45° angles") }
        case .rectangle, .ellipse:
            group { BrushOptionsView(model: model) }
            group {
                DSChip(symbol: "square.fill", help: "Filled shape (off: outline). Hold Shift for squares and circles", isOn: $opts.filled)
                InkOptionsView(model: model, brush: false)
            }
            group { HintLabel(keys: ["⇧"], text: "constrain") }
        case .shapePolygon, .contour:
            group { BrushOptionsView(model: model) }
            group {
                DSChip(symbol: "square.fill", help: "Filled shape (off: outline)", isOn: $opts.filled)
                InkOptionsView(model: model, brush: false)
            }
        case .curve:
            group { BrushOptionsView(model: model) }
            group { InkOptionsView(model: model, brush: false) }
            group { HintLabel(keys: ["↩"], text: "finish") }
        case .text:
            group { HintLabel(text: "Click where the text should start") }
        case .wand:
            group { SelectionOptionsView(model: model) }
            group { HintLabel(keys: ["⇧", "⌥"], text: "add / subtract") }
        case .marquee, .lasso, .polygon:
            group { HintLabel(keys: ["⇧", "⌥"], text: "add / subtract") }
        case .move:
            group { HintLabel(keys: ["←", "↑"], text: "nudge (⇧ moves 10 px)") }
        case .picker:
            group { HintLabel(keys: ["⌥"], text: "picks from any tool") }
        case .slice:
            group { HintLabel(text: "Drag to create a slice; double-click to edit") }
        }
    }

    /// Items of one option group, separated from the previous one by a hairline.
    private func group<C: View>(@ViewBuilder _ content: () -> C) -> some View {
        HStack(spacing: 6) {
            DSGroupDivider()
            content()
        }
    }
}

struct DrawingCommands: Commands {
    @ObservedObject var model: SpriteModel
    @ObservedObject var opts: DrawingOptions

    init(model: SpriteModel) {
        self.model = model
        opts = model.opts
    }

    var body: some Commands {
        CommandMenu("Drawing") {
            Toggle("Filled Shapes", isOn: $opts.filled)
            Toggle("Pixel-perfect Pencil", isOn: $opts.pixelPerfect)
            Toggle("Contiguous Fill", isOn: $opts.contiguous)
            Divider()
            Toggle("Mirror Horizontally", isOn: $opts.mirrorX)
            Toggle("Mirror Vertically", isOn: $opts.mirrorY)
            Divider()
            Button("Swap Colors") { model.swapColors() }
            Divider()
            InkMenuItems(model: model)
        }
        CommandMenu("Palette") {
            Button("Load Palette…") { model.loadPalette() }
            Button("Save Palette…") { model.savePalette() }
            Divider()
            Button("Remove Primary Color from Palette") {
                if let i = model.palette.firstIndex(of: model.color) { model.removePaletteColor(at: i) }
            }
            Button("Sort by Hue") { model.sortPalette(byHue: true) }
            Button("Sort by Luminance") { model.sortPalette(byHue: false) }
            Divider()
            PalettePresetMenu(model: model)
        }
    }
}
