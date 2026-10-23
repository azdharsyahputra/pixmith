import AppKit

/// Geometry of the canvas document view. The view's frame contains the sprite plus margins (enough sprite copies on
/// each side along tiled axes to fill the viewport) and its bounds origin is shifted so the center sprite sits at
/// (0, 0). Mouse math in `CanvasNSView` therefore keeps working in sprite space.
enum CanvasLayout {
    /// Breathing room around the sprite (when not tiled).
    static let pad: CGFloat = 64
    static let maxCopies = 64

    /// Sprite copies needed on each side of the center sprite so that a viewport is covered at any scroll position.
    static func copies(zoom z: CGFloat, width w: Int, height h: Int, tiled: TiledMode,
                       viewport: CGSize) -> (x: Int, y: Int) {
        func n(_ on: Bool, _ view: CGFloat, _ size: Int) -> Int {
            on ? min(maxCopies, max(1, Int((view / max(CGFloat(size) * z, 1)).rounded(.up)))) : 0
        }
        return (n(tiled.tilesX, viewport.width, w), n(tiled.tilesY, viewport.height, h))
    }

    static func margins(zoom z: CGFloat, width w: Int, height h: Int, copies: (x: Int, y: Int)) -> CGSize {
        // Tiled axes need no padding: the copies already reach past the viewport.
        CGSize(width: copies.x > 0 ? CGFloat(copies.x * w) * z : pad, height: copies.y > 0 ? CGFloat(copies.y * h) * z : pad)
    }

    static func docSize(zoom z: CGFloat, width w: Int, height h: Int, copies: (x: Int, y: Int)) -> CGSize {
        let m = margins(zoom: z, width: w, height: h, copies: copies)
        return CGSize(width: CGFloat(w) * z + 2 * m.width, height: CGFloat(h) * z + 2 * m.height)
    }
}

/// Drawing helpers for `CanvasNSView.draw`, called in order: backdrop, sprite, front.
enum CanvasOverlays {
    static func spriteRect(_ m: SpriteModel) -> CGRect {
        CGRect(x: 0, y: 0, width: CGFloat(m.width) * m.zoom, height: CGFloat(m.height) * m.zoom)
    }

    /// Everything the checkerboard covers: the sprite, plus neighbouring copies in tiled mode.
    static func region(_ m: SpriteModel, _ s: ViewSettings) -> CGRect {
        let r = spriteRect(m)
        let c = s.tileCopies
        return CGRect(x: -CGFloat(c.x) * r.width, y: -CGFloat(c.y) * r.height,
                      width: r.width * CGFloat(2 * c.x + 1), height: r.height * CGFloat(2 * c.y + 1))
    }

    /// Calls `body` with the rect of every sprite copy (the center one included) that touches `dirty`.
    static func forEachTile(_ m: SpriteModel, dirty: CGRect, _ body: (CGRect) -> Void) {
        let r = spriteRect(m)
        let c = ViewSettings.shared.tileCopies
        guard r.width > 0, r.height > 0 else { return }
        func range(_ lo: CGFloat, _ hi: CGFloat, _ size: CGFloat, _ n: Int) -> ClosedRange<Int>? {
            let a = max(-n, Int((lo / size).rounded(.down))), b = min(n, Int((hi / size).rounded(.down)))
            return a <= b ? a...b : nil
        }
        guard let xs = range(dirty.minX, dirty.maxX, r.width, c.x), let ys = range(dirty.minY, dirty.maxY, r.height, c.y)
        else { return }
        for iy in ys {
            for ix in xs { body(r.offsetBy(dx: CGFloat(ix) * r.width, dy: CGFloat(iy) * r.height)) }
        }
    }

    /// Draws `image` into `rect` of a flipped view (CGImages are bottom-up).
    ///
    /// With `visible`, only the part of the image that shows inside that rectangle is drawn. CoreGraphics converts an
    /// image's pixels to its own format the first time it is drawn, so drawing a 4096 x 4096 image just to repaint a
    /// brush stroke's few pixels would convert all 64 MB; a cropped sub-image converts only the pixels needed.
    static func drawImage(_ ctx: CGContext, _ image: CGImage, in rect: CGRect, alpha: CGFloat = 1, visible: CGRect? = nil) {
        if let visible, rect.width > 0, rect.height > 0, image.width > 0, image.height > 0 {
            let vis = rect.intersection(visible)
            guard !vis.isNull, !vis.isEmpty else { return }
            let (sx, sy) = (rect.width / CGFloat(image.width), rect.height / CGFloat(image.height))
            let x0 = max(0, Int(((vis.minX - rect.minX) / sx).rounded(.down)))
            let y0 = max(0, Int(((vis.minY - rect.minY) / sy).rounded(.down)))
            let x1 = min(image.width, Int(((vis.maxX - rect.minX) / sx).rounded(.up)))
            let y1 = min(image.height, Int(((vis.maxY - rect.minY) / sy).rounded(.up)))
            guard x1 > x0, y1 > y0 else { return }
            if (x0, y0, x1, y1) != (0, 0, image.width, image.height),
               let part = image.cropping(to: CGRect(x: x0, y: y0, width: x1 - x0, height: y1 - y0)) {
                let dest = CGRect(x: rect.minX + CGFloat(x0) * sx, y: rect.minY + CGFloat(y0) * sy,
                                  width: CGFloat(x1 - x0) * sx, height: CGFloat(y1 - y0) * sy)
                return drawImage(ctx, part, in: dest, alpha: alpha)
            }
        }
        ctx.saveGState()
        ctx.setAlpha(alpha)
        ctx.translateBy(x: 0, y: rect.minY + rect.maxY)
        ctx.scaleBy(x: 1, y: -1)
        ctx.draw(image, in: rect)
        ctx.restoreGState()
    }

    // MARK: Backdrop: checkerboard, reference (behind), onion skin

    private static var isDark: Bool {
        NSApplication.shared.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
    }

    /// Soft drop shadow under the sprite (untiled only), so the canvas reads as an object on the workspace.
    private static func drawShadow(_ ctx: CGContext, model m: SpriteModel, dirty: CGRect) {
        let r = spriteRect(m)
        guard dirty.intersects(r.insetBy(dx: -48, dy: -48)) else { return }
        ctx.saveGState()
        ctx.clip(to: dirty)
        ctx.setShadow(offset: CGSize(width: 0, height: 8), blur: 28,
                      color: NSColor.black.withAlphaComponent(isDark ? 0.5 : 0.22).cgColor)
        ctx.setFillColor(NSColor.black.cgColor)
        ctx.fill(r)
        ctx.restoreGState()
    }

    /// One-pixel outline just outside the sprite.
    private static func drawOutline(_ ctx: CGContext, model m: SpriteModel, backingScale: CGFloat) {
        ctx.saveGState()
        ctx.setStrokeColor(NSColor.black.withAlphaComponent(isDark ? 0.6 : 0.18).cgColor)
        ctx.setLineWidth(1 / max(backingScale, 1))
        ctx.stroke(spriteRect(m).insetBy(dx: -0.5 / max(backingScale, 1), dy: -0.5 / max(backingScale, 1)))
        ctx.restoreGState()
    }

    static func drawBackdrop(_ ctx: CGContext, model m: SpriteModel, dirty: CGRect) {
        let s = ViewSettings.shared
        if s.tiled == .none { drawShadow(ctx, model: m, dirty: dirty) }
        let region = region(m, s)
        let clip = region.intersection(dirty)
        guard !clip.isNull, !clip.isEmpty else { return }
        ctx.saveGState()
        ctx.clip(to: clip)
        ctx.draw(Prefs.checkerImage, in: CGRect(x: 0, y: 0, width: 16, height: 16), byTiling: true)
        ctx.restoreGState()

        if s.referencePlacement == .behind { drawReference(ctx, model: m, dirty: dirty) }

        guard m.onionSkin, m.frameCount > 1 else { return }
        let n = m.frameCount
        var layers: [(frame: Int, step: Int, tint: OnionTint)] = []
        var seen: Set<Int> = [m.frame]
        for step in 1...3 {
            if step <= s.onionPast {
                let f = ((m.frame - step) % n + n) % n
                if seen.insert(f).inserted { layers.append((f, step, s.onionTint ? .past : .none)) }
            }
            if step <= s.onionFuture {
                let f = (m.frame + step) % n
                if seen.insert(f).inserted { layers.append((f, step, s.onionTint ? .future : .none)) }
            }
        }
        // Farthest first so nearer frames sit on top.
        for l in layers.sorted(by: { $0.step > $1.step }) {
            guard let img = m.cachedImage(frame: l.frame, tint: l.tint) else { continue }
            let alpha = s.onionOpacity * pow(s.onionFalloff, Double(l.step - 1))
            forEachTile(m, dirty: dirty) { drawImage(ctx, img, in: $0, alpha: CGFloat(alpha), visible: dirty) }
        }
    }

    // MARK: Sprite (+ tiled copies)

    static func drawSprite(_ ctx: CGContext, model m: SpriteModel, dirty: CGRect) {
        guard let img = m.cachedImage(frame: m.frame) else { return }
        let s = ViewSettings.shared
        let r = spriteRect(m)
        forEachTile(m, dirty: dirty) { drawImage(ctx, img, in: $0, visible: dirty) }
        if s.tiled != .none {
            ctx.setStrokeColor(NSColor.systemOrange.withAlphaComponent(0.8).cgColor)
            ctx.setLineWidth(1)
            ctx.stroke(r.insetBy(dx: -0.5, dy: -0.5))
        }
    }

    // MARK: Front: reference (above), grids

    static func drawFront(_ ctx: CGContext, model m: SpriteModel, dirty: CGRect, backingScale: CGFloat) {
        let s = ViewSettings.shared
        if s.referencePlacement == .above { drawReference(ctx, model: m, dirty: dirty) }
        let r = spriteRect(m)
        let hairline = 1 / max(backingScale, 1)
        if s.tiled == .none { drawOutline(ctx, model: m, backingScale: backingScale) }
        if m.showGrid, m.zoom >= CGFloat(s.gridMinZoom) {
            strokeGrid(ctx, in: r, dirty: dirty, stepX: m.zoom, stepY: m.zoom, offsetX: 0, offsetY: 0,
                       color: NSColor(s.gridColor).withAlphaComponent(s.gridOpacity).cgColor, width: hairline)
        }
        if s.customGrid, s.customGridW > 0, s.customGridH > 0 {
            strokeGrid(ctx, in: r, dirty: dirty, stepX: CGFloat(s.customGridW) * m.zoom,
                       stepY: CGFloat(s.customGridH) * m.zoom,
                       offsetX: CGFloat(s.customGridX) * m.zoom, offsetY: CGFloat(s.customGridY) * m.zoom,
                       color: NSColor(s.customGridColor).withAlphaComponent(s.customGridOpacity).cgColor,
                       width: max(hairline, 1))
        }
    }

    /// Grid lines inside `area`, only those that touch `dirty`.
    private static func strokeGrid(_ ctx: CGContext, in area: CGRect, dirty: CGRect, stepX: CGFloat, stepY: CGFloat,
                                   offsetX: CGFloat, offsetY: CGFloat, color: CGColor, width: CGFloat) {
        let vis = area.intersection(dirty.insetBy(dx: -1, dy: -1))
        guard !vis.isNull, !vis.isEmpty, stepX > 0.5, stepY > 0.5 else { return }
        func firstLine(_ lo: CGFloat, _ origin: CGFloat, _ step: CGFloat) -> CGFloat {
            origin + ((lo - origin) / step).rounded(.up) * step
        }
        ctx.saveGState()
        ctx.clip(to: area)
        ctx.setStrokeColor(color)
        ctx.setLineWidth(width)
        var x = firstLine(vis.minX, offsetX, stepX)
        while x <= vis.maxX {
            ctx.move(to: CGPoint(x: x, y: vis.minY))
            ctx.addLine(to: CGPoint(x: x, y: vis.maxY))
            x += stepX
        }
        var y = firstLine(vis.minY, offsetY, stepY)
        while y <= vis.maxY {
            ctx.move(to: CGPoint(x: vis.minX, y: y))
            ctx.addLine(to: CGPoint(x: vis.maxX, y: y))
            y += stepY
        }
        ctx.strokePath()
        ctx.restoreGState()
    }

    private static func drawReference(_ ctx: CGContext, model m: SpriteModel, dirty: CGRect) {
        let s = ViewSettings.shared
        guard s.showReference, let img = s.referenceImage else { return }
        let k = CGFloat(s.referenceScale / 100) * m.zoom
        let rect = CGRect(x: CGFloat(s.referenceX) * m.zoom, y: CGFloat(s.referenceY) * m.zoom,
                          width: CGFloat(img.width) * k, height: CGFloat(img.height) * k)
        guard rect.intersects(dirty) else { return }
        ctx.saveGState()
        ctx.interpolationQuality = k >= 2 ? .none : .medium
        drawImage(ctx, img, in: rect, alpha: CGFloat(s.referenceOpacity))
        ctx.restoreGState()
    }
}
