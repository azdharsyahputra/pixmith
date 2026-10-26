import AppKit
import PixmithCore

/// The frame-difference view of the current frame: which pixels differ from the previous frame (`pixmith_frame_diff`),
/// as a tint image for the canvas and counts for the status chip. The result is kept until the document, the frame or
/// the view settings change, so repainting the canvas costs nothing.
final class FrameDiffCache {
    static let shared = FrameDiffCache()

    struct Entry {
        let changed: Int, appeared: Int, disappeared: Int
        /// The frame compared against, nil when the frame has no previous one.
        let reference: Int?
        /// Magenta (changed), green (appeared) and red (disappeared) pixels, opaque, on a transparent image; nil when
        /// nothing differs.
        let image: CGImage?

        var total: Int { changed + appeared + disappeared }

        /// Text of the canvas status chip.
        var chip: String {
            if reference == nil { return "Diff: no previous frame" }
            if total == 0 { return "Diff: no change" }
            return "Diff \(total) px: \(changed) changed, \(appeared) new, \(disappeared) gone"
        }
    }

    private struct Key: Equatable {
        let doc: OpaquePointer, width: Int, height: Int, frame: Int, layer: Int?, wrap: Bool, generation: Int
    }

    private var key: Key?
    private var cached: Entry?

    /// The difference of the model's current frame from the previous one, nil if it cannot be computed (for example the
    /// active layer is a group).
    func entry(_ m: SpriteModel) -> Entry? {
        let s = ViewSettings.shared
        let render = RenderCache.shared
        render.refresh(m)
        let layer = s.diffActiveLayer ? m.layer : nil
        let k = Key(doc: m.doc, width: m.width, height: m.height, frame: m.frame, layer: layer, wrap: s.diffWrap,
                    generation: render.generation)
        if k == key { return cached }
        key = k
        cached = Self.compute(m, layer: layer, wrap: s.diffWrap)
        return cached
    }

    private static let colors: [(UInt8, UInt8, UInt8)] = [(255, 0, 255), (0, 255, 0), (255, 0, 0)]

    static func compute(_ m: SpriteModel, layer: Int?, wrap: Bool) -> Entry? {
        let (w, h) = (m.width, m.height)
        guard w > 0, h > 0 else { return nil }
        var mask = [UInt8](repeating: 0, count: w * h)
        var out = PixmithFrameDiff()
        guard pixmith_frame_diff(m.doc, UInt32(m.frame), Int32(layer ?? -1), wrap, &mask, mask.count, &out) else { return nil }
        var image: CGImage?
        if out.changed + out.appeared + out.disappeared > 0 {
            var rgba = [UInt8](repeating: 0, count: w * h * 4)
            for i in 0..<mask.count where mask[i] != 0 {
                let c = colors[Int(min(mask[i], 3)) - 1]
                rgba[i * 4] = c.0
                rgba[i * 4 + 1] = c.1
                rgba[i * 4 + 2] = c.2
                rgba[i * 4 + 3] = 255
            }
            if let provider = CGDataProvider(data: Data(rgba) as CFData) {
                image = CGImage(
                    width: w, height: h, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: w * 4,
                    space: CGColorSpace(name: CGColorSpace.sRGB)!,
                    bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
                    provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
            }
        }
        return Entry(changed: Int(out.changed), appeared: Int(out.appeared), disappeared: Int(out.disappeared),
                     reference: out.reference_frame >= 0 ? Int(out.reference_frame) : nil, image: image)
    }
}
