import AppKit
import PixmithCore
import SwiftUI

/// Edit > Warp: a puppet / mesh warp of the cel or selection (`pixmith_warp.h`). A grid of handles lies over the
/// selection bounds (or the canvas); dragging a handle moves the content under it, right-clicking puts it back. The
/// canvas shows the core's majority-vote result live, Apply makes it one undo step.
final class WarpSession: ObservableObject {
    static let shared = WarpSession()

    static let maxGrid = 8
    static let maxSamples = 8

    @Published var active = false
    @Published var cols = 3
    @Published var rows = 3
    @Published var samples = 4
    @Published var smooth = false
    /// The area the grid covers: x, y, width, height in canvas pixels.
    var region = (x: 0.0, y: 0.0, w: 0.0, h: 0.0)
    /// One displacement per handle, row by row.
    var offsets: [CGPoint] = []
    /// The handle being dragged: its index, the canvas point of the press and its offset then.
    var grab: (handle: Int, start: CGPoint, offset: CGPoint)?

    /// Grid position of handle `k` (before its displacement).
    func gridPoint(_ k: Int) -> CGPoint {
        let (i, j) = (k % cols, k / cols)
        return CGPoint(x: region.x + region.w * Double(i) / Double(cols - 1),
                       y: region.y + region.h * Double(j) / Double(rows - 1))
    }

    /// Where handle `k` is now: its grid position plus its displacement.
    func handle(_ k: Int) -> CGPoint {
        let g = gridPoint(k), o = offsets[k]
        return CGPoint(x: g.x + o.x, y: g.y + o.y)
    }

    var isIdentity: Bool { offsets.allSatisfy { $0 == .zero } }

    /// Moves to a grid of another size, keeping the deformation (bilinear in the old handle grid).
    func regrid(cols newCols: Int, rows newRows: Int) {
        let (oc, or_, old) = (cols, rows, offsets)
        let at = { (i: Int, j: Int) in old[min(max(j, 0), or_ - 1) * oc + min(max(i, 0), oc - 1)] }
        var next: [CGPoint] = []
        for j in 0..<newRows {
            for i in 0..<newCols {
                let u = Double(i) / Double(newCols - 1) * Double(oc - 1)
                let v = Double(j) / Double(newRows - 1) * Double(or_ - 1)
                let (i0, j0) = (min(Int(u), oc - 2), min(Int(v), or_ - 2))
                let (tx, ty) = (u - Double(i0), v - Double(j0))
                let lerp = { (a: CGPoint, b: CGPoint, t: Double) in
                    CGPoint(x: a.x * (1 - t) + b.x * t, y: a.y * (1 - t) + b.y * t)
                }
                next.append(lerp(lerp(at(i0, j0), at(i0 + 1, j0), tx), lerp(at(i0, j0 + 1), at(i0 + 1, j0 + 1), tx), ty))
            }
        }
        cols = newCols
        rows = newRows
        offsets = next
    }

    func reset() {
        offsets = [CGPoint](repeating: .zero, count: cols * rows)
    }

    /// The flat arrays of the C ABI: `cols * rows` x, y pairs.
    var flatOffsets: [Double] { offsets.flatMap { [Double($0.x), Double($0.y)] } }
}

extension SpriteModel {
    var warpActive: Bool { WarpSession.shared.active }

    func startWarp() {
        let w = WarpSession.shared
        stop()
        guard !w.active, !transformActive else { return }
        var b = [Int32](repeating: 0, count: 4)
        guard pixmith_warp_bounds(doc, &b), b[2] > 0, b[3] > 0 else { return }
        w.region = (Double(b[0]), Double(b[1]), Double(b[2]), Double(b[3]))
        w.cols = 3
        w.rows = 3
        w.reset()
        w.active = true
        updateWarpPreview()
        changed()
    }

    func applyWarp() {
        let w = WarpSession.shared
        guard w.active else { return }
        let offsets = w.flatOffsets
        let applied = offsets.withUnsafeBufferPointer {
            pixmith_warp_apply(doc, UInt32(layer), UInt32(frame), UInt32(w.cols), UInt32(w.rows), $0.baseAddress,
                               UInt32(w.samples), w.smooth, nil, 0)
        }
        closeWarp()
        if applied {
            labelLastStep("Warp")
            revision &+= 1
        }
    }

    func cancelWarp() {
        closeWarp()
    }

    private func closeWarp() {
        let w = WarpSession.shared
        w.active = false
        w.grab = nil
        FilterState.shared.setPreview(nil)
        changed()
    }

    /// Shows the warped frame on the canvas in place of the sprite.
    func updateWarpPreview() {
        let w = WarpSession.shared
        guard w.active else { return }
        let (pw, ph) = (width, height)
        var buf = [UInt8](repeating: 0, count: pw * ph * 4)
        let offsets = w.flatOffsets
        let ok = offsets.withUnsafeBufferPointer {
            pixmith_warp_preview(doc, UInt32(layer), UInt32(frame), UInt32(w.cols), UInt32(w.rows), $0.baseAddress,
                                 UInt32(w.samples), w.smooth, nil, 0, &buf, buf.count)
        }
        FilterState.shared.setPreview(ok ? FilterSheet.image(buf, pw, ph) : nil)
    }

    // MARK: Mouse

    private func warpPoint(_ e: NSEvent) -> CGPoint? {
        guard let canvas = SelectionOutline.shared.canvas else { return nil }
        let p = canvas.convert(e.locationInWindow, from: nil)
        return CGPoint(x: p.x / zoom, y: p.y / zoom)
    }

    /// A press while a warp is open: grabs the handle under the pointer (right-click resets it). The warp owns the
    /// pointer until it ends, so this returns true whenever one is open.
    func warpMouseDown(_ e: NSEvent) -> Bool {
        let w = WarpSession.shared
        guard w.active else { return false }
        guard let p = warpPoint(e) else { return true }
        let near = (0..<w.offsets.count)
            .map { k -> (Int, CGFloat) in
                let h = w.handle(k)
                return (k, hypot(h.x - p.x, h.y - p.y) * zoom)
            }
            .filter { $0.1 <= Self.warpGrabRadius }
            .min { $0.1 < $1.1 }
        guard let (k, _) = near else { return true }
        if e.type == .rightMouseDown || e.modifierFlags.contains(.control) {
            w.offsets[k] = .zero
            updateWarpPreview()
            objectWillChange.send()
        } else {
            w.grab = (k, p, w.offsets[k])
        }
        return true
    }

    func warpMouseDragged(_ e: NSEvent) -> Bool {
        let w = WarpSession.shared
        guard w.active else { return false }
        guard let g = w.grab, let p = warpPoint(e) else { return true }
        w.offsets[g.handle] = CGPoint(x: g.offset.x + p.x - g.start.x, y: g.offset.y + p.y - g.start.y)
        updateWarpPreview()
        return true
    }

    func warpMouseUp() -> Bool {
        let w = WarpSession.shared
        guard w.active else { return false }
        w.grab = nil
        return true
    }

    /// Screen distance within which a click grabs a handle.
    static let warpGrabRadius: CGFloat = 10
}

extension CanvasNSView {
    /// The deformed mesh and its handles.
    func drawWarpOverlay(_ ctx: CGContext, _ m: SpriteModel) {
        let w = WarpSession.shared
        guard w.active, w.offsets.count == w.cols * w.rows else { return }
        let z = m.zoom
        let pts = (0..<w.offsets.count).map { k -> CGPoint in
            let h = w.handle(k)
            return CGPoint(x: h.x * z, y: h.y * z)
        }
        ctx.saveGState()
        for (color, width) in [(NSColor.black.withAlphaComponent(0.6), CGFloat(3)), (NSColor.white, CGFloat(1))] {
            ctx.setStrokeColor(color.cgColor)
            ctx.setLineWidth(width)
            for j in 0..<w.rows {
                for i in 0..<w.cols {
                    if i + 1 < w.cols { ctx.addLines(between: [pts[j * w.cols + i], pts[j * w.cols + i + 1]]) }
                    if j + 1 < w.rows { ctx.addLines(between: [pts[j * w.cols + i], pts[(j + 1) * w.cols + i]]) }
                }
            }
            ctx.strokePath()
        }
        ctx.setLineWidth(1)
        for (k, p) in pts.enumerated() {
            let r = CGRect(x: p.x - 4, y: p.y - 4, width: 8, height: 8)
            ctx.setFillColor((w.offsets[k] == .zero ? NSColor.white : NSColor.controlAccentColor).cgColor)
            ctx.fill(r)
            ctx.setStrokeColor(NSColor.black.cgColor)
            ctx.stroke(r)
        }
        ctx.restoreGState()
    }
}

/// Options of the open warp: a strip under the context bar.
struct WarpBar: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var warp = WarpSession.shared

    private func intField(_ title: String, _ value: Int, range: ClosedRange<Int>, help: String,
                          set: @escaping (Int) -> Void) -> some View {
        DSNumberField(title, value: Binding(get: { Double(value) }, set: { set(Int($0.rounded())) }),
                      range: Double(range.lowerBound)...Double(range.upperBound), step: 0.2, width: 78, help: help)
    }

    private func regrid(cols: Int, rows: Int) {
        warp.regrid(cols: cols, rows: rows)
        model.updateWarpPreview()
        model.objectWillChange.send()
    }

    var body: some View {
        if warp.active {
            DSToolbar {
                HStack(spacing: DS.Space.xs + 2) {
                    Image(systemName: "wand.and.rays").font(.system(size: 12)).foregroundStyle(DSColor.accent)
                    Text("Warp").font(DS.Font.bodyLarge.weight(.semibold))
                }
                DSGroupDivider()
                intField("Cols", warp.cols, range: 2...WarpSession.maxGrid, help: "Handles across") {
                    regrid(cols: $0, rows: warp.rows)
                }
                intField("Rows", warp.rows, range: 2...WarpSession.maxGrid, help: "Handles down") {
                    regrid(cols: warp.cols, rows: $0)
                }
                intField("Samples", warp.samples, range: 1...WarpSession.maxSamples,
                         help: "Sample points per output pixel along each axis; the most frequent source color wins") {
                    warp.samples = $0
                    model.updateWarpPreview()
                }
                DSChip("Smooth", help: "Smooth spline between handles instead of straight (bilinear) interpolation",
                       isOn: Binding(get: { warp.smooth }, set: {
                           warp.smooth = $0
                           model.updateWarpPreview()
                       }))
                Text("Drag a handle; right-click resets it")
                    .font(DS.Font.caption).foregroundStyle(DSColor.textSecondary)
                Spacer(minLength: DS.Space.sm)
                Button("Reset") {
                    warp.reset()
                    model.updateWarpPreview()
                    model.objectWillChange.send()
                }.buttonStyle(DSBarButtonStyle())
                Button("Cancel") { model.cancelWarp() }.buttonStyle(DSBarButtonStyle()).help("Discard the warp (Esc)")
                    .keyboardShortcut(.cancelAction)
                Button("Apply") { model.applyWarp() }.buttonStyle(DSBarButtonStyle(prominent: true))
                    .help("Apply the warp (Return)").keyboardShortcut(.defaultAction)
            }
        }
    }
}
