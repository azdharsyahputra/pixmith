import AppKit

/// The brush cursor preview: the pixels a click would paint, drawn under the pointer (`brush_preview.rs` in the
/// desktop app follows the same rules).
///
/// The footprint is the core's brush rule (`Painter::dot`, `retouch::brush_cells`): a square or round brush of the
/// current size centered on the pixel under the pointer, plus the mirror images the symmetry toggles add, clipped to
/// the sprite. It is snapped to whole sprite pixels; the outline is one device pixel wide at any zoom. A custom brush
/// shows its bounding box, the spray the disc it scatters dots in.
final class BrushPreview {
    enum Look {
        /// The current color at about 60% opacity, with an outline.
        case filled
        /// The outline only.
        case outline
        case spray
    }

    struct Cell: Hashable {
        var x: Int
        var y: Int
    }

    /// A stretch of the footprint's boundary on one grid line.
    private struct Edge: Comparable {
        var horizontal: Bool
        /// Which way the outside lies across the line, -1 or 1.
        var out: Int
        /// The grid line, in pixels along the axis the edge does not run on.
        var line: Int
        var from: Int
        var to: Int

        static func < (a: Edge, b: Edge) -> Bool {
            (a.horizontal ? 1 : 0, a.out, a.line, a.from) < (b.horizontal ? 1 : 0, b.out, b.line, b.from)
        }
    }

    /// Opacity of the color fill.
    private static let fillAlpha: CGFloat = 0.6
    /// Below this zoom the inner (light) half of the outline would cover the whole pixel, so only the dark half shows.
    private static let innerMinZoom: CGFloat = 3

    /// The sprite pixel under the pointer; `nil` while it is off the canvas.
    private(set) var pixel: (Int, Int)?
    /// Where the preview was drawn last (view coordinates, outline included), to repaint when it moves.
    private var shown: [CGRect] = []
    private weak var view: NSView?
    private weak var model: SpriteModel?
    private var monitor: Any?

    deinit {
        if let monitor { NSEvent.removeMonitor(monitor) }
    }

    /// The pointer moved to sprite pixel `p`, or left the canvas (`nil`). Repaints only the old and new footprints.
    func pointerMoved(_ p: (Int, Int)?, in view: NSView, model m: SpriteModel) {
        self.view = view
        model = m
        if monitor == nil { installMonitor() }
        guard p?.0 != pixel?.0 || p?.1 != pixel?.1 else { return }
        pixel = p
        invalidate()
    }

    // MARK: State that changes without the pointer moving

    /// Space, Option, the middle button and scrolling hide, show or move the preview under a resting pointer.
    private func installMonitor() {
        let kinds: NSEvent.EventTypeMask = [.keyDown, .keyUp, .flagsChanged, .otherMouseDown, .otherMouseUp, .scrollWheel]
        monitor = NSEvent.addLocalMonitorForEvents(matching: kinds) { [weak self] e in
            DispatchQueue.main.async { self?.refresh() }
            return e
        }
    }

    private func refresh() {
        guard let view, let m = model, pixel != nil else { return }
        let live = livePixel(in: view, model: m)
        let visible = lookFor(m) != nil
        guard live?.0 != pixel?.0 || live?.1 != pixel?.1 || visible == shown.isEmpty else { return }
        pixel = live
        invalidate()
    }

    /// The sprite pixel under the mouse right now, if it is over the visible canvas.
    private func livePixel(in view: NSView, model m: SpriteModel) -> (Int, Int)? {
        guard let w = view.window else { return nil }
        let p = view.convert(w.mouseLocationOutsideOfEventStream, from: nil)
        guard view.visibleRect.contains(p) else { return nil }
        return (Int((p.x / m.zoom).rounded(.down)), Int((p.y / m.zoom).rounded(.down)))
    }

    private func invalidate() {
        guard let view, let m = model else { return }
        var rects = shown
        if let p = pixel, let look = lookFor(m) { rects += boxes(m, look, at: p) }
        for r in rects { view.setNeedsDisplay(r) }
    }

    /// How the preview looks for the current tool, or `nil` while it is hidden: another tool, or something else
    /// owns the pointer (panning, a selection, transform, warp, tween, slice, tile stamp or terrain brush, a shape
    /// being dragged or half drawn).
    private func lookFor(_ m: SpriteModel) -> Look? {
        let look: Look
        switch m.tool {
        case .pencil, .line, .rectangle, .ellipse, .curve, .shapePolygon, .contour: look = .filled
        case .eraser, .blur, .jumble: look = .outline
        case .spray: look = .spray
        default: return nil
        }
        // Space and the middle button pan; Option picks a color.
        if CanvasPan.space != nil || CanvasPan.last != nil || NSEvent.modifierFlags.contains(.option) { return nil }
        if m.transformActive || m.warpActive || m.tweenDragging || TweenUI.shared.editOnCanvas { return nil }
        if m.selectionDragging || m.sliceDragging || m.drawingSessionOpen { return nil }
        if let d = m.opts.drag, d.tool == .line || d.tool == .rectangle || d.tool == .ellipse { return nil }
        if m.isTilemapLayer {
            if AutotileState.shared.brush && m.terrainMode >= 0 { return nil }
            if TilemapState.shared.stamp && (m.tool == .pencil || m.tool == .eraser) { return nil }
        }
        return look
    }

    // MARK: Footprint

    /// The symmetry of the drawing calls: which toggles are on and where the axes sit (twice their position in
    /// pixels, as the mirror guides read them).
    private struct Symmetry {
        var x: Bool
        var y: Bool
        var ax: Int
        var ay: Int

        init(_ m: SpriteModel) {
            x = m.opts.mirrorX
            y = m.opts.mirrorY
            ax = m.mirrorX2
            ay = m.mirrorY2
        }

        /// `c` and its mirror images.
        func images(of c: Cell) -> [Cell] {
            var out = [c]
            if x { out.append(Cell(x: ax - 1 - c.x, y: c.y)) }
            if y { out.append(Cell(x: c.x, y: ay - 1 - c.y)) }
            if x && y { out.append(Cell(x: ax - 1 - c.x, y: ay - 1 - c.y)) }
            return out
        }
    }

    /// Pixels the brush covers when centered on `p`, unclipped and unmirrored.
    static func brushCells(size: Int, round: Bool, custom: (w: Int, h: Int)?, at p: (Int, Int)) -> [Cell] {
        if let c = custom {
            let (ox, oy) = (p.0 - c.w / 2, p.1 - c.h / 2)
            return (0..<max(c.h, 0)).flatMap { y in (0..<max(c.w, 0)).map { Cell(x: ox + $0, y: oy + y) } }
        }
        let size = min(max(size, 1), 64)
        let half = (size - 1) / 2
        var out: [Cell] = []
        for row in 0..<size {
            // A round brush trims `l` pixels from both ends of each row.
            var l = 0
            if round && size > 2 {
                let r = Double(size) / 2
                let dy = Double(row) + 0.5 - r
                l = min(max(Int((r - max(r * r - dy * dy, 0).squareRoot() - 0.5 - 1e-9).rounded(.up)), 0), size - 1)
            }
            for col in l..<(size - l) { out.append(Cell(x: p.0 - half + col, y: p.1 - half + row)) }
        }
        return out
    }

    /// The pixels of the sprite a click at `p` paints.
    private static func painted(_ m: SpriteModel, at p: (Int, Int)) -> Set<Cell> {
        let (w, h) = (m.width, m.height)
        let sym = Symmetry(m)
        var out = Set<Cell>()
        for c in brushCells(size: m.brushSize, round: m.opts.circleBrush, custom: m.opts.customBrush, at: p) {
            for q in sym.images(of: c) where q.x >= 0 && q.y >= 0 && q.x < w && q.y < h { out.insert(q) }
        }
        return out
    }

    /// The boundary of `cells`, with neighboring unit edges joined.
    private static func edges(_ cells: Set<Cell>) -> [Edge] {
        var all: [Edge] = []
        for c in cells {
            if !cells.contains(Cell(x: c.x, y: c.y - 1)) { all.append(Edge(horizontal: true, out: -1, line: c.y, from: c.x, to: c.x + 1)) }
            if !cells.contains(Cell(x: c.x, y: c.y + 1)) { all.append(Edge(horizontal: true, out: 1, line: c.y + 1, from: c.x, to: c.x + 1)) }
            if !cells.contains(Cell(x: c.x - 1, y: c.y)) { all.append(Edge(horizontal: false, out: -1, line: c.x, from: c.y, to: c.y + 1)) }
            if !cells.contains(Cell(x: c.x + 1, y: c.y)) { all.append(Edge(horizontal: false, out: 1, line: c.x + 1, from: c.y, to: c.y + 1)) }
        }
        all.sort()
        var out: [Edge] = []
        for e in all {
            if let l = out.last, l.horizontal == e.horizontal, l.out == e.out, l.line == e.line, l.to == e.from {
                out[out.count - 1].to = e.to
            } else {
                out.append(e)
            }
        }
        return out
    }

    /// View-space boxes that cover the preview at `p` (the brush at each mirror image, outline included).
    private func boxes(_ m: SpriteModel, _ look: Look, at p: (Int, Int)) -> [CGRect] {
        let size = min(max(m.brushSize, 1), 64)
        let e: (x: Int, y: Int, w: Int, h: Int)
        if look == .spray {
            let r = Self.sprayRadius(size)
            e = (-r - 1, -r - 1, 2 * r + 3, 2 * r + 3)
        } else if let c = m.opts.customBrush {
            e = (-c.w / 2, -c.h / 2, c.w, c.h)
        } else {
            let half = (size - 1) / 2
            e = (-half, -half, size, size)
        }
        let sym = Symmetry(m)
        let (bx, by) = (p.0 + e.x, p.1 + e.y)
        var origins = [(bx, by)]
        if sym.x { origins.append((sym.ax - bx - e.w, by)) }
        if sym.y { origins.append((bx, sym.ay - by - e.h)) }
        if sym.x && sym.y { origins.append((sym.ax - bx - e.w, sym.ay - by - e.h)) }
        let z = m.zoom
        return origins.map {
            CGRect(x: CGFloat($0.0) * z - 2, y: CGFloat($0.1) * z - 2, width: CGFloat(e.w) * z + 4, height: CGFloat(e.h) * z + 4)
        }
    }

    /// Radius of the disc the spray scatters its dots in (`spray` in DrawingTools.swift).
    private static func sprayRadius(_ size: Int) -> Int { min(max(size * 3, 3), 256) }

    // MARK: Drawing

    func draw(_ ctx: CGContext, model m: SpriteModel, dirty: CGRect, scale: CGFloat) {
        guard let p = pixel, var look = lookFor(m) else {
            shown = []
            return
        }
        shown = boxes(m, look, at: p)
        guard shown.contains(where: { $0.intersects(dirty) }) else { return }
        let z = m.zoom, s = max(scale, 1), t = 1 / max(scale, 1)
        ctx.saveGState()
        defer { ctx.restoreGState() }
        if look == .spray { return drawSpray(ctx, m, at: p, zoom: z, line: t) }
        // A custom brush shows its bounding box without a fill.
        if m.opts.customBrush != nil { look = .outline }
        let cells = Self.painted(m, at: p)
        guard !cells.isEmpty else { return }

        // Everything sits on whole device pixels, so adjacent rectangles share their edges exactly.
        ctx.setShouldAntialias(false)
        func snap(_ v: CGFloat) -> CGFloat { (v * s).rounded() / s }
        func xs(_ x: Int) -> CGFloat { snap(CGFloat(x) * z) }
        func ys(_ y: Int) -> CGFloat { snap(CGFloat(y) * z) }

        if look == .filled {
            var fills: [CGRect] = []
            for (y, row) in Dictionary(grouping: cells, by: \.y) {
                let run = row.map(\.x).sorted()
                var i = 0
                while i < run.count {
                    var j = i
                    while j + 1 < run.count, run[j + 1] == run[j] + 1 { j += 1 }
                    fills.append(CGRect(x: xs(run[i]), y: ys(y), width: xs(run[j] + 1) - xs(run[i]), height: ys(y + 1) - ys(y)))
                    i = j + 1
                }
            }
            let c = m.color.cgColor
            ctx.setFillColor(c.copy(alpha: c.alpha * Self.fillAlpha) ?? c)
            ctx.fill(fills)
        }

        // Dark just outside the footprint, light just inside it: readable on any color.
        let inner = z >= Self.innerMinZoom
        var dark: [CGRect] = [], light: [CGRect] = []
        for e in Self.edges(cells) {
            let out = e.out < 0
            if e.horizontal {
                let (y, x0, x1) = (ys(e.line), xs(e.from), xs(e.to))
                dark.append(CGRect(x: x0 - t, y: out ? y - t : y, width: x1 - x0 + 2 * t, height: t))
                if inner { light.append(CGRect(x: x0, y: out ? y : y - t, width: x1 - x0, height: t)) }
            } else {
                let (x, y0, y1) = (xs(e.line), ys(e.from), ys(e.to))
                dark.append(CGRect(x: out ? x - t : x, y: y0 - t, width: t, height: y1 - y0 + 2 * t))
                if inner { light.append(CGRect(x: out ? x : x - t, y: y0, width: t, height: y1 - y0)) }
            }
        }
        ctx.setFillColor(CGColor(gray: 0, alpha: 1))
        ctx.fill(dark)
        ctx.setFillColor(CGColor(gray: 1, alpha: 1))
        ctx.fill(light)
    }

    /// The spray's reach as an outline around each mirror image of the pointer pixel.
    private func drawSpray(_ ctx: CGContext, _ m: SpriteModel, at p: (Int, Int), zoom z: CGFloat, line t: CGFloat) {
        let r = (CGFloat(Self.sprayRadius(min(max(m.brushSize, 1), 64))) + 0.5) * z
        ctx.setLineWidth(t)
        for c in Set(Symmetry(m).images(of: Cell(x: p.0, y: p.1))) {
            let center = CGPoint(x: (CGFloat(c.x) + 0.5) * z, y: (CGFloat(c.y) + 0.5) * z)
            for (radius, gray) in [(r + t / 2, CGFloat(0)), (r - t / 2, CGFloat(1))] {
                ctx.setStrokeColor(CGColor(gray: gray, alpha: 1))
                ctx.strokeEllipse(in: CGRect(x: center.x - radius, y: center.y - radius, width: 2 * radius, height: 2 * radius))
            }
        }
    }
}
