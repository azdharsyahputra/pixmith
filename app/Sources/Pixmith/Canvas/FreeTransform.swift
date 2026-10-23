import AppKit
import PixmithCore
import SwiftUI

/// Options of the free transform that outlive a single session.
final class TransformOptions: ObservableObject {
    static let shared = TransformOptions()
    @Published var rotsprite: Bool = UserDefaults.standard.bool(forKey: "transformRotSprite") {
        didSet { UserDefaults.standard.set(rotsprite, forKey: "transformRotSprite") }
    }
    /// Which box point typed width and height keep fixed: 0...8, row by row (4 is the center).
    @Published var anchor: Int = UserDefaults.standard.object(forKey: "transformAnchor") as? Int ?? 4 {
        didSet { UserDefaults.standard.set(anchor, forKey: "transformAnchor") }
    }
    var anchorDirection: (Int32, Int32) { (Int32(anchor % 3 - 1), Int32(anchor / 3 - 1)) }
}

/// Pointer-drag state of the transform handles.
private final class TransformDrag {
    static let shared = TransformDrag()
    var active = false
    var kind: UInt32 = 0
    var handle = (0, 0)
    var start = CGPoint.zero
}

private let handleOrder = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)]

extension SpriteModel {
    var transformActive: Bool { pixmith_xf_active(doc) }

    /// Handle positions (in `handleOrder`) and the pivot, in canvas coordinates.
    func transformGeometry() -> (handles: [CGPoint], pivot: CGPoint)? {
        var g = [Double](repeating: 0, count: 18)
        guard pixmith_xf_geometry(doc, &g) else { return nil }
        return ((0..<8).map { CGPoint(x: g[$0 * 2], y: g[$0 * 2 + 1]) }, CGPoint(x: g[16], y: g[17]))
    }

    /// X, Y, width, height, angle, skew x, skew y.
    func transformFields() -> [Double] {
        var f = [Double](repeating: 0, count: 7)
        _ = pixmith_xf_fields(doc, &f)
        return f
    }

    func setTransformField(_ index: Int, _ value: Double) {
        pixmith_xf_set_field(doc, UInt32(index), value)
        changed()
    }

    func startTransform() {
        stop()
        guard !transformActive else { return }
        if pixmith_xf_begin(doc, UInt32(layer), UInt32(frame), TransformOptions.shared.rotsprite) {
            labelLastStep("Free Transform")
            let a = TransformOptions.shared.anchorDirection
            pixmith_xf_set_anchor(doc, a.0, a.1)
            changed()
        }
    }

    func commitTransform() {
        guard transformActive else { return }
        pixmith_xf_commit(doc)
        changed()
    }

    func cancelTransform() {
        if pixmith_xf_cancel(doc) { changed() }
    }

    func nudgeTransform(dx: Double, dy: Double) {
        pixmith_xf_drag_begin(doc)
        pixmith_xf_drag(doc, 0, 0, 0, false, 0, 0, dx, dy)
        changed()
    }

    // MARK: Mouse

    /// Handle positions of the selection's bounding box, shown while the move tool is active.
    func selectionHandles() -> [CGPoint]? {
        guard tool == .move, !transformActive, hasSelection else { return nil }
        var b = [Int32](repeating: 0, count: 4)
        guard pixmith_sel_bounds(doc, &b) else { return nil }
        let (x, y, w, h) = (CGFloat(b[0]), CGFloat(b[1]), CGFloat(b[2]), CGFloat(b[3]))
        return handleOrder.map { CGPoint(x: x + CGFloat($0.0 + 1) / 2 * w, y: y + CGFloat($0.1 + 1) / 2 * h) }
    }

    private func canvasPoint(_ e: NSEvent) -> CGPoint? {
        guard let canvas = SelectionOutline.shared.canvas else { return nil }
        let p = canvas.convert(e.locationInWindow, from: nil)
        return CGPoint(x: p.x / zoom, y: p.y / zoom)
    }

    /// Handles a press while a transform is active. Returns false when none is.
    func transformMouseDown(_ e: NSEvent) -> Bool {
        if !transformActive {
            // With the move tool, grabbing a handle of the selection box starts a transform. Small boxes get small
            // handles, and a click inside such a box always moves the selection.
            guard let p = canvasPoint(e), let box = selectionHandles() else { return false }
            let r = Self.hitRadius(box, zoom: zoom)
            let inside = p.x > box[0].x && p.x < box[7].x && p.y > box[0].y && p.y < box[7].y
            guard box.contains(where: { hypot($0.x - p.x, $0.y - p.y) * zoom <= r }), !(inside && r < Self.maxHitRadius)
            else { return false }
            startTransform()
            guard transformActive else { return false }
        }
        guard let p = canvasPoint(e), let g = transformGeometry() else { return true }
        let d = TransformDrag.shared
        let r = Self.hitRadius(g.handles, zoom: zoom)
        let near = { (q: CGPoint) in hypot(q.x - p.x, q.y - p.y) * self.zoom <= r }
        d.start = p
        d.handle = (0, 0)
        if near(g.pivot) {
            d.kind = 4
        } else if let i = g.handles.firstIndex(where: near) {
            let h = handleOrder[i]
            d.handle = h
            let edge = (h.0 == 0) != (h.1 == 0)
            d.kind = edge && e.modifierFlags.contains(.command) ? 3 : 1
        } else if Self.contains(g.handles, p) {
            d.kind = 0
        } else {
            d.kind = 2
        }
        pixmith_xf_drag_begin(doc)
        d.active = true
        return true
    }

    func transformMouseDragged(_ e: NSEvent) -> Bool {
        let d = TransformDrag.shared
        guard d.active, var p = canvasPoint(e) else { return transformActive }
        let start = d.start
        if d.kind == 0 {
            // Whole-pixel moves keep the nearest-neighbor result crisp (scaling rounds in the core).
            p = CGPoint(x: start.x + (p.x - start.x).rounded(), y: start.y + (p.y - start.y).rounded())
        }
        pixmith_xf_drag(doc, d.kind, Int32(d.handle.0), Int32(d.handle.1), e.modifierFlags.contains(.shift),
                     start.x, start.y, p.x, p.y)
        changed()
        return true
    }

    func transformMouseUp() -> Bool {
        let d = TransformDrag.shared
        guard d.active else { return transformActive }
        d.active = false
        return true
    }

    /// Screen distance within which a click grabs a handle: 8 points, but never more than a third of the box's shorter side.
    static let maxHitRadius: CGFloat = 8

    static func hitRadius(_ handles: [CGPoint], zoom: CGFloat) -> CGFloat {
        let side = min(hypot(handles[2].x - handles[0].x, handles[2].y - handles[0].y),
                       hypot(handles[5].x - handles[0].x, handles[5].y - handles[0].y)) * zoom
        return max(1, min(maxHitRadius, side / 3))
    }

    private static func contains(_ h: [CGPoint], _ p: CGPoint) -> Bool {
        let poly = [h[0], h[2], h[7], h[5]]
        var inside = false
        var j = poly.count - 1
        for i in 0..<poly.count {
            let (a, b) = (poly[i], poly[j])
            if (a.y > p.y) != (b.y > p.y), p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x { inside.toggle() }
            j = i
        }
        return inside
    }
}

extension CanvasNSView {
    /// Box outline, scale handles and pivot of the active transform.
    func drawTransformOverlay(_ ctx: CGContext, _ m: SpriteModel) {
        let z = m.zoom
        var pivot: CGPoint?
        let handles: [CGPoint]
        if m.transformActive, let g = m.transformGeometry() {
            handles = g.handles
            pivot = g.pivot
        } else if let h = m.selectionHandles() {
            handles = h
        } else {
            return
        }
        let pts = handles.map { CGPoint(x: $0.x * z, y: $0.y * z) }
        let path = CGMutablePath()
        path.addLines(between: [pts[0], pts[2], pts[7], pts[5]])
        path.closeSubpath()
        ctx.saveGState()
        ctx.setLineWidth(1)
        ctx.setStrokeColor(NSColor.white.cgColor)
        ctx.addPath(path)
        ctx.strokePath()
        ctx.setStrokeColor(NSColor.black.cgColor)
        ctx.setLineDash(phase: 0, lengths: [4, 4])
        ctx.addPath(path)
        ctx.strokePath()
        ctx.setLineDash(phase: 0, lengths: [])
        for p in pts {
            let r = CGRect(x: p.x - 4, y: p.y - 4, width: 8, height: 8)
            ctx.setFillColor(NSColor.white.cgColor)
            ctx.fill(r)
            ctx.setStrokeColor(NSColor.black.cgColor)
            ctx.stroke(r)
        }
        if let pivot {
        let c = CGPoint(x: pivot.x * z, y: pivot.y * z)
        ctx.setFillColor(NSColor.systemOrange.cgColor)
        ctx.fillEllipse(in: CGRect(x: c.x - 4, y: c.y - 4, width: 8, height: 8))
        ctx.setStrokeColor(NSColor.black.cgColor)
        ctx.strokeEllipse(in: CGRect(x: c.x - 4, y: c.y - 4, width: 8, height: 8))
        }
        ctx.restoreGState()
    }
}

/// Numeric fields and options of the active transform: a second strip under the context bar.
struct TransformBar: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var opts = TransformOptions.shared

    private static let anchorNames = ["Top left", "Top", "Top right", "Left", "Center", "Right", "Bottom left", "Bottom", "Bottom right"]

    private func field(_ title: String, _ index: Int, help: String, step: Double = 0.25) -> some View {
        DSNumberField(title, value: Binding(
            get: { model.transformFields()[index] },
            set: { model.setTransformField(index, $0) }),
            range: -9999...9999, step: step, digits: 2, width: 78, help: help)
    }

    private func setAnchor(_ i: Int) {
        opts.anchor = i
        let a = opts.anchorDirection
        pixmith_xf_set_anchor(model.doc, a.0, a.1)
    }

    private var title: some View {
        HStack(spacing: DS.Space.xs + 2) {
            Image(systemName: "arrow.up.left.and.arrow.down.right").font(.system(size: 12)).foregroundStyle(DSColor.accent)
            Text("Transform").font(DS.Font.bodyLarge.weight(.semibold))
        }
    }

    @ViewBuilder private var boxFields: some View {
        field("X", 0, help: "Left edge of the box")
        field("Y", 1, help: "Top edge of the box")
        field("W", 2, help: "Width. Drag the label to scrub.")
        field("H", 3, help: "Height. Drag the label to scrub.")
        anchorPicker
    }

    @ViewBuilder private var angleFields: some View {
        field("∠", 4, help: "Rotation angle in degrees", step: 0.5)
        field("Sx", 5, help: "Horizontal skew", step: 0.5)
        field("Sy", 6, help: "Vertical skew", step: 0.5)
        DSChip("RotSprite", help: "Pixel-art rotation: 8x edge-aware upscale, rotate, then downsample",
               isOn: Binding(get: { opts.rotsprite }, set: {
                   opts.rotsprite = $0
                   pixmith_xf_set_rotsprite(model.doc, $0)
                   model.changed()
               }))
    }

    @ViewBuilder private var buttons: some View {
        Button("Cancel") { model.cancelTransform() }.buttonStyle(DSBarButtonStyle())
            .help("Discard the transform (Esc)")
        Button("Apply") { model.commitTransform() }.buttonStyle(DSBarButtonStyle(prominent: true))
            .help("Commit the transform (Return)")
    }

    /// One strip when it fits, otherwise two so nothing is clipped at the minimum window width.
    var body: some View {
        if model.transformActive {
            ViewThatFits(in: .horizontal) {
                DSToolbar {
                    title
                    DSGroupDivider()
                    boxFields
                    DSGroupDivider()
                    angleFields
                    Spacer(minLength: DS.Space.sm)
                    buttons
                }
                VStack(spacing: 0) {
                    DSToolbar {
                        title
                        DSGroupDivider()
                        boxFields
                        Spacer(minLength: DS.Space.sm)
                        buttons
                    }
                    DSToolbar { angleFields; Spacer(minLength: 0) }
                }
            }
        }
    }

    /// 3x3 grid of the box point that typed W and H keep in place.
    private var anchorPicker: some View {
        VStack(spacing: 2) {
            ForEach(0..<3, id: \.self) { r in
                HStack(spacing: 2) {
                    ForEach(0..<3, id: \.self) { c in
                        let i = r * 3 + c
                        Button { setAnchor(i) } label: {
                            Circle().fill(opts.anchor == i ? DSColor.accent : DSColor.textTertiary)
                                .frame(width: opts.anchor == i ? 5 : 3, height: opts.anchor == i ? 5 : 3)
                                .frame(width: 7, height: 7)
                        }
                        .buttonStyle(.plain)
                        .help("Anchor: \(Self.anchorNames[i]). The point of the box that typed W and H keep in place.")
                    }
                }
            }
        }
        .padding(4)
        .background(RoundedRectangle(cornerRadius: DS.Radius.chip).fill(DSColor.field))
    }
}

extension SpriteModel {
    /// Tab switch or document replacement: forget the handle drag of the document being left.
    func resetTransformTransients() { TransformDrag.shared.active = false }
}
