// Renders the Pixmith app icon: a pixel-art paintbrush on a transparency checkerboard, 32x32 cells.
// Usage: swift scripts/make-icon.swift <output.iconset directory>
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

let n = 32
var grid = [UInt32](repeating: 0, count: n * n)  // 0xRRGGBBAA, fully opaque or fully transparent

func set(_ x: Int, _ y: Int, _ c: UInt32) {
    if x >= 0, y >= 0, x < n, y < n { grid[y * n + x] = c }
}

let navy: UInt32 = 0x1D2B53FF
let paperLight: UInt32 = 0xFFF1E8FF, paperDark: UInt32 = 0xC2C3C7FF
let brown: [UInt32] = [0xD2814FFF, 0xAB5236FF, 0x7E3A25FF]
let bristle: [UInt32] = [0xFFE0B8FF, 0xFFCCAAFF, 0xD99F7AFF]
let steel: [UInt32] = [0xFFF1E8FF, 0xC2C3C7FF, 0x83769CFF]
let swatches: [(light: UInt32, dark: UInt32)] = [
    (0xFF004DFF, 0xB30038FF), (0xFFA300FF, 0xB87200FF), (0xFFEC27FF, 0xB8A91CFF),
    (0x00E436FF, 0x00A026FF), (0x29ADFFFF, 0x1B7FC4FF), (0xFF77A8FF, 0xB8567BFF),
]

// Rounded square with stepped corners.
for y in 1...30 {
    for x in 1...30 { set(x, y, navy) }
}
for (x, y) in [(1, 1), (2, 1), (1, 2), (30, 1), (29, 1), (30, 2), (1, 30), (2, 30), (1, 29), (30, 30), (29, 30), (30, 29)] {
    set(x, y, 0)
}

// Canvas with a checkerboard.
for y in 4...23 {
    for x in 4...27 { set(x, y, (x / 2 + y / 2) % 2 == 0 ? paperLight : paperDark) }
}

// A painted squiggle behind the brush.
let squiggle = [(6, 9), (7, 9), (8, 9), (8, 10), (9, 10), (10, 10), (10, 11), (11, 11), (12, 11), (12, 12), (13, 12),
                (13, 13), (14, 13), (14, 14), (15, 14), (16, 14), (16, 15)]
for (x, y) in squiggle {
    set(x, y, 0x29ADFFFF)
    set(x, y + 1, 0x1B7FC4FF)
}

// Outline so the brush stands out from the canvas.
for y in 0..<n {
    for x in 0..<n {
        let s = x + y, u = x - y
        if ((s == 30 || s == 34) && (-14...12).contains(u)) || ((31...33).contains(s) && (u == -15 || u == 13)) {
            set(x, y, navy)
        }
    }
}

// Brush: a diagonal band. s = x + y is the width coordinate, u = x - y runs along the brush.
for y in 0..<n {
    for x in 0..<n {
        let s = x + y, u = x - y
        guard (31...33).contains(s) else { continue }
        let shade = s - 31  // 0 light, 1 mid, 2 dark
        switch u {
        case -14 ... -6: set(x, y, brown[shade])
        case -5 ... -3: set(x, y, steel[shade])
        case -2 ... 5: set(x, y, bristle[shade])
        case 6 ... 9: set(x, y, [0xFF77A8FF, 0xFF004DFF, 0xB30038FF][shade])
        case 10 ... 11: if shade != 0 { set(x, y, shade == 1 ? 0xFF004DFF : 0xB30038FF) }
        case 12: if shade == 1 { set(x, y, 0xFF004DFF) }
        default: break
        }
    }
}

// Palette swatches along the bottom edge.
for (i, c) in swatches.enumerated() {
    for y in 25...27 {
        for x in (5 + i * 4)...(7 + i * 4) { set(x, y, y == 27 ? c.dark : c.light) }
    }
}

let space = CGColorSpace(name: CGColorSpace.sRGB)!

func render(_ size: Int) -> CGImage {
    var bytes = [UInt8](repeating: 0, count: n * n * 4)
    for (i, c) in grid.enumerated() {
        bytes[i * 4] = UInt8(c >> 24)
        bytes[i * 4 + 1] = UInt8(c >> 16 & 255)
        bytes[i * 4 + 2] = UInt8(c >> 8 & 255)
        bytes[i * 4 + 3] = UInt8(c & 255)
    }
    let small = bytes.withUnsafeMutableBytes { buf in
        CGContext(data: buf.baseAddress, width: n, height: n, bitsPerComponent: 8, bytesPerRow: n * 4, space: space,
                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!.makeImage()!
    }
    let ctx = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0, space: space,
                        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.interpolationQuality = .none
    ctx.draw(small, in: CGRect(x: 0, y: 0, width: size, height: size))
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
