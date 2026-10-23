// Renders the README banner (assets/banner-dark.png and assets/banner-light.png): the app icon, the
// wordmark and a tagline, with a few pixels in the icon's color ramp scattered on the right.
// Usage: swift scripts/make-banner.swift <assets directory>
import AppKit
import CoreGraphics
import CoreText
import Foundation
import ImageIO
import UniformTypeIdentifiers

let space = CGColorSpace(name: CGColorSpace.sRGB)!

func color(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(srgbRed: CGFloat(hex >> 16 & 255) / 255, green: CGFloat(hex >> 8 & 255) / 255,
            blue: CGFloat(hex & 255) / 255, alpha: alpha)
}

func mix(_ a: UInt32, _ b: UInt32, _ t: CGFloat) -> UInt32 {
    [16, 8, 0].reduce(UInt32(0)) { out, shift in
        let x = CGFloat(a >> shift & 255), y = CGFloat(b >> shift & 255)
        return out | UInt32((x + (y - x) * t).rounded()) << shift
    }
}

let ramp: [UInt32] = [0xFF9A3C, 0xFF4F7B, 0x9B5CFF, 0x3FB6FF]
func rampColor(_ t: CGFloat) -> UInt32 {
    let f = max(0, min(1, t)) * CGFloat(ramp.count - 1)
    let i = min(Int(f), ramp.count - 2)
    return mix(ramp[i], ramp[i + 1], f - CGFloat(i))
}

struct Theme {
    var name: String
    var background: [UInt32]
    var grid: CGColor
    var title: UInt32
    var tagline: UInt32
}

let width = 2400, height = 720
/// Where the pixel field starts, as a fraction of the width.
let field: CGFloat = 0.72

func text(_ ctx: CGContext, _ string: String, size: CGFloat, weight: CGFloat, fill: UInt32, at p: CGPoint, kern: CGFloat = 0) {
    let font = NSFont.systemFont(ofSize: size, weight: NSFont.Weight(rawValue: weight)) as CTFont
    let attrs = [kCTFontAttributeName: font, kCTForegroundColorAttributeName: color(fill), kCTKernAttributeName: kern] as CFDictionary
    let line = CTLineCreateWithAttributedString(CFAttributedStringCreate(nil, string as CFString, attrs))
    ctx.textPosition = p
    CTLineDraw(line, ctx)
}

func render(_ theme: Theme, icon: CGImage) -> CGImage {
    let w = CGFloat(width), h = CGFloat(height)
    let ctx = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: space,
                        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    let bg = CGGradient(colorsSpace: space, colors: theme.background.map { color($0) } as CFArray, locations: nil)!
    ctx.drawLinearGradient(bg, start: CGPoint(x: 0, y: h), end: CGPoint(x: w, y: 0), options: [])

    // A faint pixel grid, fading out towards the text.
    let cell: CGFloat = 40
    ctx.setStrokeColor(theme.grid)
    ctx.setLineWidth(1)
    var x: CGFloat = w * field
    while x < w {
        ctx.move(to: CGPoint(x: x, y: 0)); ctx.addLine(to: CGPoint(x: x, y: h)); x += cell
    }
    var y: CGFloat = 0
    while y < h {
        ctx.move(to: CGPoint(x: w * field, y: y)); ctx.addLine(to: CGPoint(x: w, y: y)); y += cell
    }
    ctx.strokePath()

    // Scattered pixels: denser and brighter towards the right edge.
    var seed: UInt32 = 0x9E3779B9
    func next() -> CGFloat {
        seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5
        return CGFloat(seed % 10_000) / 10_000
    }
    let cols = Int((w * (1 - field)) / cell), rows = Int(h / cell)
    for c in 0..<cols {
        for r in 0..<rows {
            let t = CGFloat(c) / CGFloat(cols)
            guard next() < 0.08 + t * t * 0.5 else { continue }
            let px = w * field + CGFloat(c) * cell, py = CGFloat(r) * cell
            let rect = CGRect(x: px + 3, y: py + 3, width: cell - 6, height: cell - 6)
            ctx.addPath(CGPath(roundedRect: rect, cornerWidth: 7, cornerHeight: 7, transform: nil))
            ctx.setFillColor(color(rampColor(CGFloat(r) / CGFloat(rows) * 0.6 + t * 0.4), 0.35 + 0.6 * t))
            ctx.fillPath()
        }
    }

    // Icon, wordmark, tagline.
    let iconSize: CGFloat = 400
    ctx.draw(icon, in: CGRect(x: 150, y: (h - iconSize) / 2, width: iconSize, height: iconSize))
    text(ctx, "Pixmith", size: 176, weight: NSFont.Weight.bold.rawValue, fill: theme.title, at: CGPoint(x: 590, y: h / 2 - 10), kern: -4)
    text(ctx, "Pixel art editor  ·  .aseprite compatible  ·  Godot-ready", size: 44, weight: NSFont.Weight.regular.rawValue, fill: theme.tagline,
         at: CGPoint(x: 598, y: h / 2 - 110))
    return ctx.makeImage()!
}

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write(Data("usage: make-banner.swift <assets directory>\n".utf8))
    exit(2)
}
let dir = URL(fileURLWithPath: CommandLine.arguments[1])
guard let src = CGImageSourceCreateWithURL(dir.appendingPathComponent("logo.png") as CFURL, nil),
      let icon = CGImageSourceCreateImageAtIndex(src, 0, nil) else { fatalError("run make-icon.sh first (needs logo.png)") }
let themes = [
    Theme(name: "dark", background: [0x16161B, 0x0B0B0E], grid: color(0xFFFFFF, 0.05), title: 0xFFFFFF, tagline: 0x9A9AA6),
    Theme(name: "light", background: [0xFFFFFF, 0xF1F1F5], grid: color(0x000000, 0.05), title: 0x111116, tagline: 0x6B6B76),
]
for theme in themes {
    let url = dir.appendingPathComponent("banner-\(theme.name).png")
    let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(dest, render(theme, icon: icon), nil)
    guard CGImageDestinationFinalize(dest) else { fatalError("could not write \(url.path)") }
}
