import Accelerate
import AppKit
import PixmithCore

/// One rendered frame: straight-alpha pixels (for the eyedropper) and premultiplied pixels (for drawing),
/// updated a rectangle at a time.
final class FrameBuffer {
    let width: Int, height: Int
    let straight: UnsafeMutablePointer<UInt8>
    let premul: UnsafeMutablePointer<UInt8>
    private var scratch: UnsafeMutablePointer<UInt8>?
    private var scratchSize = 0
    private var cached: CGImage?

    init?(width: Int, height: Int) {
        let n = width * height * 4
        guard width > 0, height > 0, let s = calloc(n, 1), let p = calloc(n, 1) else { return nil }
        self.width = width
        self.height = height
        straight = s.assumingMemoryBound(to: UInt8.self)
        premul = p.assumingMemoryBound(to: UInt8.self)
    }

    deinit {
        free(straight)
        free(premul)
        scratch.map { free($0) }
    }

    /// Renders the whole frame.
    func renderAll(_ doc: OpaquePointer, frame: Int) -> Bool {
        guard pixmith_render(doc, UInt32(frame), straight, width * height * 4) else { return false }
        premultiply(0, 0, width, height)
        return true
    }

    /// Re-renders only a rectangle, which must lie inside the canvas.
    func render(_ doc: OpaquePointer, frame: Int, x: Int, y: Int, w: Int, h: Int) -> Bool {
        if (x, y, w, h) == (0, 0, width, height) { return renderAll(doc, frame: frame) }
        let need = w * h * 4
        if scratchSize < need {
            scratch.map { free($0) }
            scratch = malloc(need)?.assumingMemoryBound(to: UInt8.self)
            scratchSize = scratch == nil ? 0 : need
        }
        guard let scratch, pixmith_render_rect(doc, UInt32(frame), UInt32(x), UInt32(y), UInt32(w), UInt32(h), scratch, need)
        else { return false }
        for row in 0..<h {
            memcpy(straight + ((y + row) * width + x) * 4, scratch + row * w * 4, w * 4)
        }
        premultiply(x, y, w, h)
        return true
    }

    private func premultiply(_ x: Int, _ y: Int, _ w: Int, _ h: Int) {
        let offset = (y * width + x) * 4
        var src = vImage_Buffer(data: straight + offset, height: vImagePixelCount(h), width: vImagePixelCount(w), rowBytes: width * 4)
        var dst = vImage_Buffer(data: premul + offset, height: vImagePixelCount(h), width: vImagePixelCount(w), rowBytes: width * 4)
        vImagePremultiplyData_RGBA8888(&src, &dst, vImage_Flags(kvImageNoFlags))
        cached = nil
    }

    /// An image over the premultiplied pixels (no copy). A new object after every change, so nothing downstream
    /// keeps showing stale pixels.
    var image: CGImage? {
        if let cached { return cached }
        let info = Unmanaged.passRetained(self).toOpaque()
        guard let provider = CGDataProvider(dataInfo: info, data: premul, size: width * height * 4,
                                            releaseData: { info, _, _ in
                                                if let info { Unmanaged<FrameBuffer>.fromOpaque(info).release() }
                                            })
        else {
            Unmanaged<FrameBuffer>.fromOpaque(info).release()
            return nil
        }
        cached = CGImage(
            width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
            provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
        return cached
    }

    func pixel(x: Int, y: Int) -> UInt32 {
        let p = straight + (y * width + x) * 4
        return UInt32(p[0]) << 24 | UInt32(p[1]) << 16 | UInt32(p[2]) << 8 | UInt32(p[3])
    }
}

/// What changed on screen since the canvas last looked, in sprite pixels.
enum ViewChange: Equatable {
    case none
    case rect(frame: Int, x: Int, y: Int, w: Int, h: Int)
    case all

    func merged(with o: ViewChange) -> ViewChange {
        switch (self, o) {
        case (.none, _): return o
        case (_, .none): return self
        case let (.rect(f, x, y, w, h), .rect(g, x2, y2, w2, h2)) where f == g:
            let (x0, y0) = (min(x, x2), min(y, y2))
            return .rect(frame: f, x: x0, y: y0, w: max(x + w, x2 + w2) - x0, h: max(y + h, y2 + h2) - y0)
        default: return .all
        }
    }
}

/// Rendered frames, shared by the canvas, onion skin and preview. Each frame is rendered once and afterwards only the
/// rectangle an edit reports is re-rendered (`pixmith_take_dirty_rect`), so a brush stroke on a large sprite costs a few
/// pixels per segment, not a full composite, copy and premultiply.
final class RenderCache {
    static let shared = RenderCache()

    struct Key: Hashable {
        let frame: Int
        let tint: OnionTint
    }

    private(set) var buffers: [Int: FrameBuffer] = [:]
    /// Onion-skin tinted copies of frames.
    private var tinted: [Key: CGImage] = [:]
    /// Renders performed (full or partial), for diagnostics and tests.
    private(set) var renderCount = 0
    private var doc: OpaquePointer?
    private var size = (w: 0, h: 0)
    private var pending = ViewChange.none
    /// The filter preview and its premultiplied form. Kept apart from the frame buffers so that moving a filter
    /// slider does not invalidate them.
    private var previewSource: CGImage?
    private var previewImage: CGImage?

    /// Straight-alpha previews are re-premultiplied by CoreGraphics on every draw; do that once per preview.
    func premultipliedPreview(_ p: CGImage) -> CGImage {
        if previewSource !== p {
            previewSource = p
            previewImage = Self.premultiplied(p) ?? p
        }
        return previewImage ?? p
    }

    private static func premultiplied(_ base: CGImage) -> CGImage? {
        guard let ctx = CGContext(data: nil, width: base.width, height: base.height, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return nil }
        ctx.setBlendMode(.copy)
        ctx.draw(base, in: CGRect(x: 0, y: 0, width: base.width, height: base.height))
        return ctx.makeImage()
    }

    /// Applies what the document reports as changed. Cheap when nothing changed.
    func refresh(_ m: SpriteModel) {
        let (w, h) = (m.width, m.height)
        var frame: UInt32 = 0
        var r = [Int32](repeating: 0, count: 4)
        let kind = pixmith_take_dirty_rect(m.doc, &frame, &r)
        if m.doc != doc || size != (w, h) || kind == 2 {
            buffers.removeAll()
            tinted.removeAll()
            doc = m.doc
            size = (w, h)
            pending = .all
            return
        }
        guard kind == 1 else { return }
        let f = Int(frame)
        tinted = tinted.filter { $0.key.frame != f }
        let change = ViewChange.rect(frame: f, x: Int(r[0]), y: Int(r[1]), w: Int(r[2]), h: Int(r[3]))
        pending = pending.merged(with: change)
        if let b = buffers[f], !b.render(m.doc, frame: f, x: Int(r[0]), y: Int(r[1]), w: Int(r[2]), h: Int(r[3])) {
            buffers[f] = nil
        } else if buffers[f] != nil {
            renderCount += 1
        }
    }

    /// The changes since the previous call, for invalidating the canvas view.
    func consumeChange() -> ViewChange {
        defer { pending = .none }
        return pending
    }

    func buffer(_ m: SpriteModel, frame f: Int) -> FrameBuffer? {
        if let b = buffers[f] { return b }
        guard f >= 0, f < m.frameCount, let b = FrameBuffer(width: m.width, height: m.height), b.renderAll(m.doc, frame: f)
        else { return nil }
        renderCount += 1
        buffers[f] = b
        // Two buffers per frame: keep only the current frame when the cache would grow past the budget.
        if buffers.count * m.width * m.height * 8 > 256 << 20 {
            buffers = buffers.filter { $0.key == m.frame || $0.key == f }
            tinted = tinted.filter { $0.key.frame == m.frame || $0.key.frame == f }
        }
        return b
    }

    /// The pixel (0xRRGGBBAA, straight alpha) of the current frame, for the eyedropper.
    func pixel(_ m: SpriteModel, x: Int, y: Int) -> UInt32? {
        refresh(m)
        return buffer(m, frame: m.frame)?.pixel(x: x, y: y)
    }

    func tintedImage(_ m: SpriteModel, frame f: Int, tint: OnionTint) -> CGImage? {
        let key = Key(frame: f, tint: tint)
        if let img = tinted[key] { return img }
        guard let base = buffer(m, frame: f)?.image, let img = Self.tinted(base, tint) else { return nil }
        tinted[key] = img
        return img
    }

    private static func tinted(_ base: CGImage, _ tint: OnionTint) -> CGImage? {
        let rect = CGRect(x: 0, y: 0, width: base.width, height: base.height)
        guard let ctx = CGContext(data: nil, width: base.width, height: base.height, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return nil }
        ctx.draw(base, in: rect)
        ctx.setBlendMode(.sourceAtop)
        ctx.setFillColor(tint == .past ? CGColor(srgbRed: 1, green: 0.15, blue: 0.15, alpha: 0.55)
                                       : CGColor(srgbRed: 0.15, green: 0.35, blue: 1, alpha: 0.55))
        ctx.fill(rect)
        return ctx.makeImage()
    }
}

extension SpriteModel {
    /// The rendered frame as an image (premultiplied, cached and updated incrementally); optionally tinted for
    /// onion-skin frames.
    func cachedImage(frame f: Int, tint: OnionTint = .none) -> CGImage? {
        let cache = RenderCache.shared
        if tint == .none, f == frame, let preview = FilterState.shared.preview { return cache.premultipliedPreview(preview) }
        cache.refresh(self)
        if tint != .none { return cache.tintedImage(self, frame: f, tint: tint) }
        return cache.buffer(self, frame: f)?.image
    }
}
