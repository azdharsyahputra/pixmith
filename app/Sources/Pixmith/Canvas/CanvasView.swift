import AppKit
import SwiftUI

struct CanvasView: NSViewRepresentable {
    @ObservedObject var model: SpriteModel

    func makeNSView(context: Context) -> CanvasNSView {
        let view = CanvasNSView()
        view.model = model
        return view
    }

    func updateNSView(_ view: CanvasNSView, context: Context) {
        view.needsDisplay = true
    }
}

final class CanvasNSView: NSView {
    weak var model: SpriteModel?
    private var last: (Int, Int)?
    private var erasing = false
    private let brushPreview = BrushPreview()

    override var isFlipped: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        needsDisplay = true
    }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .crosshair)
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let m = model, let ctx = NSGraphicsContext.current?.cgContext else { return }
        ctx.interpolationQuality = .none
        CanvasOverlays.drawBackdrop(ctx, model: m, dirty: dirtyRect)
        CanvasOverlays.drawSprite(ctx, model: m, dirty: dirtyRect)
        CanvasOverlays.drawFront(ctx, model: m, dirty: dirtyRect, backingScale: window?.backingScaleFactor ?? 1)
        let rect = CGRect(x: 0, y: 0, width: CGFloat(m.width) * m.zoom, height: CGFloat(m.height) * m.zoom)
        m.drawTileGrid(ctx, size: rect.size, scale: window?.backingScaleFactor ?? 1, dirty: dirtyRect)
        m.drawMissingCells(ctx)
        m.drawTileCopies(ctx)
        m.drawTileGhost(ctx)
        m.drawIsoOverlay(ctx, dirty: dirtyRect)
        drawSelectionOutline(ctx, m)
        drawSlices(ctx, m)
        drawTransformOverlay(ctx, m)
        drawWarpOverlay(ctx, m)
        m.drawTweenOverlay(ctx)
        brushPreview.draw(ctx, model: m, dirty: dirtyRect, scale: window?.backingScaleFactor ?? 1)
        m.drawMirrorGuides(ctx, size: rect.size)
    }

    private func pixel(_ e: NSEvent) -> (Int, Int) {
        let p = convert(e.locationInWindow, from: nil)
        let z = model?.zoom ?? 1
        let pos = (Int((p.x / z).rounded(.down)), Int((p.y / z).rounded(.down)))
        if StatusInfo.shared.cursor?.x != pos.0 || StatusInfo.shared.cursor?.y != pos.1 { StatusInfo.shared.cursor = pos }
        if let m = model { brushPreview.pointerMoved(pos, in: self, model: m) }
        return pos
    }

    override func mouseDown(with e: NSEvent) {
        if beginPan(e) { return }
        guard let m = model else { return }
        m.applyMirrorAxes()
        TimelineSelection.shared.focused = false
        let p = pixel(e)
        IsoGrid.lastClick = p
        m.isoPointerMoved(p, in: self)
        if m.tweenMouseDown(e, in: self) { return }
        if m.warpMouseDown(e) { return }
        if m.transformMouseDown(e) { return }
        if m.consumeSelectByColorClick(p) { return }
        if m.handleTerrainMouse(e, at: p, began: true) { return }
        if m.handleTileMouse(e, at: p, began: true) { return }
        let tool = e.modifierFlags.contains(.option) && !m.tool.isSelection ? .picker : m.tool
        if m.drawingDown(p, tool: tool, event: e) { return }
        switch tool {
        case .picker:
            m.pick(at: p)
        case .fill:
            m.fill(at: p)
        case .marquee, .move, .wand, .lasso, .polygon:
            m.selectionMouseDown(p, tool: tool, event: e)
        case .slice:
            m.sliceMouseDown(p, event: e)
        case .pencil, .eraser:
            erasing = tool == .eraser
            m.beginStroke()
            m.line(from: p, to: p, erase: erasing)
            last = p
        default:
            break
        }
    }

    override func mouseDragged(with e: NSEvent) {
        if continuePan(e) { return }
        guard let m = model else { return }
        let p = pixel(e)
        m.isoPointerMoved(p, in: self)
        if m.tweenDragging { m.tweenMouseDragged(e, in: self); return }
        if m.warpMouseDragged(e) { return }
        if m.transformActive { _ = m.transformMouseDragged(e); return }
        if m.selectionDragging { return m.selectionMouseDragged(p) }
        if m.sliceDragging { return m.sliceMouseDragged(p) }
        if m.handleTerrainMouse(e, at: p, began: false) { return }
        if m.handleTileMouse(e, at: p, began: false) { return }
        if m.drawingDrag(p, event: e) { return }
        if let a = last {
            if p != a {
                m.line(from: a, to: p, erase: erasing)
                last = p
            }
        } else if m.tool == .picker || e.modifierFlags.contains(.option) {
            m.pick(at: p)
        }
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
                                       owner: self))
    }

    override func mouseMoved(with e: NSEvent) {
        let p = pixel(e)
        model?.shapeMoved(p)
        model?.isoPointerMoved(p, in: self)
        model?.tileCopiesPointerMoved(p, at: convert(e.locationInWindow, from: nil), in: self)
        model?.tileGhostPointerMoved(p, in: self)
    }

    override func mouseExited(with e: NSEvent) {
        StatusInfo.shared.cursor = nil
        if let m = model { brushPreview.pointerMoved(nil, in: self, model: m) }
        model?.isoPointerMoved(nil, in: self)
        model?.tileCopiesPointerMoved(nil, in: self)
        model?.tileGhostPointerMoved(nil, in: self)
    }

    override func otherMouseDown(with e: NSEvent) { _ = beginPan(e) }
    override func otherMouseDragged(with e: NSEvent) { _ = continuePan(e) }
    override func otherMouseUp(with e: NSEvent) { _ = endPan() }

    override func mouseUp(with e: NSEvent) {
        if endPan() { return }
        if let m = model, m.tweenDragging { m.tweenMouseUp(); return }
        if let m = model, m.warpMouseUp() { return }
        if let m = model, m.transformActive { _ = m.transformMouseUp(); return }
        if let m = model, m.selectionDragging { m.selectionMouseUp() }
        if let m = model, m.sliceDragging { m.sliceMouseUp() }
        if model?.handleTerrainMouseUp() == true { return }
        if model?.drawingUp() == true { return }
        last = nil
    }

    override func rightMouseDown(with e: NSEvent) { mouseDown(with: e) }
    override func rightMouseDragged(with e: NSEvent) { mouseDragged(with: e) }
    override func rightMouseUp(with e: NSEvent) { mouseUp(with: e) }

    override func magnify(with e: NSEvent) {
        guard let m = model else { return }
        ViewSettings.shared.zoomBy(m, factor: 1 + e.magnification)
    }
}
