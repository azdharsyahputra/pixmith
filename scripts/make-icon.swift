// Renders the Pixmith app icon: a pixel "P" in a warm-to-cool ramp on a dark rounded square, with one
// white pixel lifting off its corner.
// Usage: swift scripts/make-icon.swift <output.iconset directory>
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

let space = CGColorSpace(name: CGColorSpace.sRGB)!

func color(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(srgbRed: CGFloat(hex >> 16 & 255) / 255, green: CGFloat(hex >> 8 & 255) / 255,
            blue: CGFloat(hex & 255) / 255, alpha: alpha)
}

/// Mixes two 0xRRGGBB colors, `t` in 0...1.
func mix(_ a: UInt32, _ b: UInt32, _ t: CGFloat) -> UInt32 {
    [16, 8, 0].reduce(UInt32(0)) { out, shift in
        let x = CGFloat(a >> shift & 255), y = CGFloat(b >> shift & 255)
        return out | UInt32((x + (y - x) * t).rounded()) << shift
    }
}

/// The letter on a 5 x 7 grid, top row first.
let glyph = [
    "XXXX.",
    "X...X",
    "X...X",
    "XXXX.",
    "X....",
    "X....",
    "X....",
]
/// Pixel colors run diagonally from the bottom-left to the top-right through these stops.
let ramp: [UInt32] = [0xFF9A3C, 0xFF4F7B, 0x9B5CFF, 0x3FB6FF]

func render(_ size: Int) -> CGImage {
    let s = CGFloat(size)
    let ctx = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0, space: space,
                        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!

    // Body: Apple's icon grid leaves 100/1024 of margin on each side for the shadow.
    let body = CGRect(x: 0, y: 0, width: s, height: s).insetBy(dx: s * 100 / 1024, dy: s * 100 / 1024)
    let shape = CGPath(roundedRect: body, cornerWidth: body.width * 0.2237, cornerHeight: body.width * 0.2237, transform: nil)
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -s * 0.012), blur: s * 0.03, color: color(0x000000, 0.35))
    ctx.addPath(shape)
    ctx.setFillColor(color(0x18181D))
    ctx.fillPath()
    ctx.restoreGState()

    ctx.saveGState()
    ctx.addPath(shape)
    ctx.clip()
    let background = CGGradient(colorsSpace: space, colors: [color(0x24242B), color(0x0E0E12)] as CFArray, locations: nil)!
    ctx.drawLinearGradient(background, start: CGPoint(x: body.minX, y: body.maxY), end: CGPoint(x: body.maxX, y: body.minY), options: [])
    let highlight = CGGradient(colorsSpace: space, colors: [color(0xFFFFFF, 0.08), color(0xFFFFFF, 0)] as CFArray, locations: nil)!
    ctx.drawLinearGradient(highlight, start: CGPoint(x: body.midX, y: body.maxY), end: CGPoint(x: body.midX, y: body.midY), options: [])
    ctx.restoreGState()

    let cell = body.height * 0.112
    let origin = CGPoint(x: body.midX - cell * 2.6, y: body.midY - cell * 3.7)
    func pixel(_ col: CGFloat, _ row: CGFloat, _ fill: UInt32) {
        let gap = cell * 0.06
        let rect = CGRect(x: origin.x + col * cell + gap / 2, y: origin.y + (6 - row) * cell + gap / 2,
                          width: cell - gap, height: cell - gap)
        ctx.addPath(CGPath(roundedRect: rect, cornerWidth: cell * 0.18, cornerHeight: cell * 0.18, transform: nil))
        ctx.setFillColor(color(fill))
        ctx.fillPath()
    }
    for (row, line) in glyph.enumerated() {
        for (col, ch) in line.enumerated() where ch == "X" {
            let f = CGFloat(col + 6 - row) / 10 * CGFloat(ramp.count - 1)
            let i = min(Int(f), ramp.count - 2)
            pixel(CGFloat(col), CGFloat(row), mix(ramp[i], ramp[i + 1], f - CGFloat(i)))
        }
    }
    pixel(4.35, -0.35, 0xFFFFFF)
    return ctx.makeImage()!
}

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write(Data("usage: make-icon.swift <output.iconset>\n".utf8))
    exit(2)
}
let dir = URL(fileURLWithPath: CommandLine.arguments[1])
try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
for base in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let name = "icon_\(base)x\(base)\(scale == 2 ? "@2x" : "").png"
        let dest = CGImageDestinationCreateWithURL(dir.appendingPathComponent(name) as CFURL, UTType.png.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(dest, render(base * scale), nil)
        guard CGImageDestinationFinalize(dest) else { fatalError("could not write \(name)") }
    }
}
