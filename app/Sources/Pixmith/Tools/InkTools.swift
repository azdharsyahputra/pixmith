import AppKit
import PixmithCore
import SwiftUI

enum InkMode: Int, CaseIterable, Identifiable {
    case simple, alpha, lockAlpha, behind, shading

    var id: Int { rawValue }

    var title: String {
        switch self {
        case .simple: "Simple"
        case .alpha: "Alpha Compositing"
        case .lockAlpha: "Lock Alpha"
        case .behind: "Behind"
        case .shading: "Shading"
        }
    }
}

extension SpriteModel {
    /// Sends the ink mode to the core. Shading steps through the palette between the two colors,
    /// toward the secondary color, or back toward the primary when drawing with the right button.
    func applyInk(reverse: Bool) {
        let ramp = opts.ink == .shading ? shadingRamp() : []
        ramp.withUnsafeBufferPointer {
            pixmith_ink_set(UInt32(opts.ink.rawValue), reverse, $0.baseAddress, $0.count)
        }
    }

    private func shadingRamp() -> [UInt32] {
        let pal = palette
        if let a = pal.firstIndex(of: color), let b = pal.firstIndex(of: opts.secondary), a != b {
            return a < b ? Array(pal[a...b]) : Array(pal[b...a].reversed())
        }
        return [color, opts.secondary]
    }

    func gradient(from a: (Int, Int), to b: (Int, Int), right: Bool, restore: Bool) {
        let (c0, c1) = right ? (opts.secondary, color) : (color, opts.secondary)
        pixmith_gradient(doc, UInt32(layer), UInt32(frame), Int32(a.0), Int32(a.1), Int32(b.0), Int32(b.1),
                      c0, c1, opts.gradientRadial ? 1 : 0, UInt32(opts.gradientDither), restore)
    }

    /// Snaps a gradient end point to the nearest multiple of 45 degrees from the start.
    private func snapped(_ a: (Int, Int), _ b: (Int, Int)) -> (Int, Int) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1)
        let (ax, ay) = (abs(dx), abs(dy))
        if ax > 2 * ay { return (b.0, a.1) }
        if ay > 2 * ax { return (a.0, b.1) }
        let m = max(ax, ay)
        return (a.0 + m * (dx < 0 ? -1 : 1), a.1 + m * (dy < 0 ? -1 : 1))
    }

    /// Updates the gradient while dragging. The checkpoint is taken on the first real drag.
    func gradientDrag(_ d: inout DrawingOptions.Drag, to p: (Int, Int), constrain: Bool) {
        if !d.started {
            beginStroke("Gradient")
            applyInk(reverse: d.right)
        }
        gradient(from: d.start, to: constrain ? snapped(d.start, p) : p, right: d.right, restore: d.started)
        d.started = true
    }

    /// Shift pressed or released mid-drag: redraw the preview with the new constraint right away.
    func drawingFlagsChanged(_ e: NSEvent) {
        guard var d = opts.drag else { return }
        let constrain = e.modifierFlags.contains(.shift)
        guard constrain != d.constrain else { return }
        switch d.tool {
        case .line, .rectangle, .ellipse:
            shape(d.tool, from: d.start, to: d.last, color: d.color, constrain: constrain, restore: true)
        case .gradient where d.started:
            gradientDrag(&d, to: d.last, constrain: constrain)
        default:
            return
        }
        d.constrain = constrain
        opts.drag = d
        changed()
    }

    func brushFromSelection() {
        guard pixmith_brush_from_selection(doc, UInt32(layer), UInt32(frame)) else {
            errorMessage = "Select some visible pixels on the current layer to use as a brush."
            return
        }
        refreshBrushInfo()
        tool = .pencil
    }

    func resetBrush() {
        pixmith_brush_reset()
        refreshBrushInfo()
    }

    func refreshBrushInfo() {
        var info: [Int32] = [0, 0, 0]
        pixmith_brush_info(&info)
        opts.customBrush = info[0] != 0 ? (Int(info[1]), Int(info[2])) : nil
    }
}

/// Brush size and shape (or the custom brush with its reset button).
struct BrushOptionsView: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject var opts: DrawingOptions

    init(model: SpriteModel) {
        self.model = model
        opts = model.opts
    }

    var body: some View {
        HStack(spacing: DS.Space.sm) {
            DSNumberField("Size", value: $model.brushSize, range: 1...64, unit: "px", width: 90,
                          help: "Brush size ([ and ] change it). Drag the label to scrub.")
            if let b = opts.customBrush {
                HStack(spacing: DS.Space.xs) {
                    Text("Brush \(b.w)×\(b.h)").font(DS.Font.caption).foregroundStyle(DSColor.textSecondary)
                    Button("Reset") { model.resetBrush() }.buttonStyle(DSBarButtonStyle())
                }
            } else {
                DSSegmented(selection: $opts.circleBrush, options: [
                    .init(false, symbol: "square.fill", help: "Square brush"),
                    .init(true, symbol: "circle.fill", help: "Round brush"),
                ])
                .help("Brush shape for sizes above 1px")
            }
        }
    }
}

struct InkOptionsView: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject var opts: DrawingOptions
    let brush: Bool

    init(model: SpriteModel, brush: Bool = true) {
        self.model = model
        opts = model.opts
        self.brush = brush
    }

    var body: some View {
        HStack(spacing: DS.Space.sm) {
            if brush { BrushOptionsView(model: model) }
            DSPopup(selection: $opts.ink, options: InkMode.allCases.map { ($0, $0.title) }, width: 120,
                    help: "Ink mode. Shading steps along the palette from the primary toward the secondary color; right-click steps back.")
        }
    }
}

struct GradientOptionsView: View {
    @ObservedObject var opts: DrawingOptions

    init(model: SpriteModel) { opts = model.opts }

    var body: some View {
        HStack(spacing: DS.Space.sm) {
            DSSegmented(selection: $opts.gradientRadial, options: [
                .init(false, title: "Linear", help: "Linear gradient"),
                .init(true, title: "Radial", help: "Radial gradient"),
            ])
            DSPopup(selection: $opts.gradientDither, options: [(0, "Smooth"), (2, "Dither 2×2"), (4, "Dither 4×4"), (8, "Dither 8×8")],
                    width: 110, help: "Dithering")
            HintLabel(text: "primary → secondary")
        }
    }
}

struct FillPatternPicker: View {
    @ObservedObject var opts: DrawingOptions

    init(model: SpriteModel) { opts = model.opts }

    var body: some View {
        DSPopup(selection: $opts.fillPattern,
                options: [(0, "Solid"), (1, "Checker"), (2, "25%"), (3, "50%"), (4, "75%")], width: 92,
                help: "Dither the fill between the primary and secondary colors")
    }
}

/// Ink and brush entries of the Drawing menu.
struct InkMenuItems: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject var opts: DrawingOptions

    init(model: SpriteModel) {
        self.model = model
        opts = model.opts
    }

    var body: some View {
        Picker("Ink Mode", selection: $opts.ink) {
            ForEach(InkMode.allCases) { Text($0.title).tag($0) }
        }
        Toggle("Round Brush", isOn: $opts.circleBrush)
        Button("Brush from Selection") { model.brushFromSelection() }.keyboardShortcut("b")
        Button("Reset Brush") { model.resetBrush() }.disabled(opts.customBrush == nil)
        Divider()
        Button("Gradient Tool") { model.tool = .gradient }
    }
}
