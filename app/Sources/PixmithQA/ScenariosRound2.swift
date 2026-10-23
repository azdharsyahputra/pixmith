import AppKit
@testable import PixmithKit
import PixmithCore

/// A PNG with many different colors, as another app might put on the pasteboard.
func rainbowPNG(_ w: Int, _ h: Int) -> Data {
    let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                        space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    for y in 0..<h {
        for x in 0..<w {
            ctx.setFillColor(CGColor(srgbRed: CGFloat(x) / CGFloat(w), green: CGFloat(y) / CGFloat(h), blue: 0.37, alpha: 1))
            ctx.fill(CGRect(x: x, y: y, width: 1, height: 1))
        }
    }
    return NSBitmapImageRep(cgImage: ctx.makeImage()!).representation(using: .png, properties: [:])!
}

func registerRound2() {
    scenario("select/tiny-selection-moves-instead-of-transforming") {
        let m = fresh(16, 16), d = CanvasDriver()
        m.zoom = 8
        for (px, py) in [(4, 4), (5, 4), (4, 5), (5, 5)] {
            m.deselect()
            paintRect(d, 4, 4, 5, 5, RED)
            m.tool = .marquee
            d.stroke([(4, 4), (5, 5)])
            d.bitmap()
            m.tool = .move
            // Every pixel of the 2x2 box lies close to a handle; a click on any of them still moves the selection.
            d.down(px, py)
            expect(!m.transformActive, "clicking pixel (\(px),\(py)) of a 2x2 selection does not start a transform")
            d.drag(px + 3, py + 1)
            d.up(px + 3, py + 1)
            expect(!m.transformActive)
            expectPixel(7, 5, RED, "the selection moved by (3,1) after grabbing (\(px),\(py))")
            expectPixel(4, 4, CLEAR)
            m.deselect()
            while m.historyCounts.undo > 0 { m.undo() }
        }
        // A handle radius never exceeds a third of the box, and caps at 8 points.
        let small = [CGPoint(x: 4, y: 4), .zero, CGPoint(x: 6, y: 4), .zero, .zero, CGPoint(x: 4, y: 6), .zero, .zero]
        expectEq(SpriteModel.hitRadius(small, zoom: 8), 16.0 / 3)
        let big = [CGPoint(x: 0, y: 0), .zero, CGPoint(x: 100, y: 0), .zero, .zero, CGPoint(x: 0, y: 100), .zero, .zero]
        expectEq(SpriteModel.hitRadius(big, zoom: 8), 8)
        // Big boxes still have usable handles at their corners.
        paintRect(d, 2, 2, 11, 11, BLUE)
        m.tool = .marquee
        d.stroke([(2, 2), (11, 11)])
        d.bitmap()
        m.tool = .move
        d.down(12, 12)
        expect(m.transformActive, "the corner handle of a 10x10 box still grabs")
        d.up(12, 12)
        m.cancelTransform()
    }

    scenario("color/indexed-commits-conform-to-the-palette") {
        let m = fresh(16, 16), d = CanvasDriver()
        m.color = RED
        put(d, 1, 1, RED)
        m.convertColorMode(to: .indexed)
        let palette = Set(m.palette)
        func clean(_ what: String, file: String = #fileID, line: UInt = #line) {
            let bad = m.pixels().filter { $0 & 0xFF != 0 && !palette.contains($0) }
            expect(bad.isEmpty, "\(what) left colors outside the palette: \(bad.prefix(3).map(hex))", file: file, line: line)
            expect(m.visibleCount() > 1, "\(what) added pixels", file: file, line: line)
        }
        // Paste from another app: before it is committed and after.
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.setData(rainbowPNG(8, 8), forType: .png)
        m.pasteFromPasteboard()
        clean("a floating paste (preview)")
        d.enter()
        clean("the committed paste")
        m.undo()
        // A free transform of imported colors.
        m.pasteFromPasteboard()
        d.enter()
        m.tool = .move
        m.startTransform()
        m.setTransformField(4, 33)
        m.commitTransform()
        clean("a rotated free transform")
        m.undo(); m.undo()
        // Import as a layer.
        let png = tmp("rainbow.png")
        try? rainbowPNG(8, 8).write(to: png)
        let idx = png.path.withCString { pixmith_import_png_layer(m.doc, 0, 0, $0) }
        expect(idx >= 0, "import as layer: \(coreError)")
        m.changed()
        clean("an imported layer")
        // The same in grayscale.
        m.undo()
        m.convertColorMode(to: .grayscale)
        pb.clearContents()
        pb.setData(rainbowPNG(8, 8), forType: .png)
        m.pasteFromPasteboard()
        d.enter()
        for (x, y) in m.opaque() {
            let (r, g, b) = rgb(m.pixel(x, y))
            expect(r == g && g == b, "grayscale paste at (\(x),\(y)) is gray: \(r),\(g),\(b)")
        }
        d.snapshot("color-indexed-paste")
    }

    scenario("tilemap/long-stroke-allocates-one-tile-per-cell") {
        let m = fresh(64, 32), d = CanvasDriver()
        m.zoom = 8
        m.newTilemapLayer(tileWidth: 8, tileHeight: 8)
        let before = m.tileCount(of: 0)
        m.color = RED
        // A stroke of 60 segments through several cells.
        let path = (2..<62).map { ($0, 3 + ($0 / 20)) }
        d.stroke(path)
        let after = m.tileCount(of: 0)
        let cells = Set(path.map { "\($0.0 / 8),\($0.1 / 8)" }).count
        expectEq(after - before, cells, "one new tile for each of the \(cells) touched cells")
        // A second pass over the same cells reuses their tiles.
        m.color = BLUE
        d.stroke((2..<62).map { ($0, 3 + ($0 / 20)) })
        expectEq(m.tileCount(of: 0), after, "painting over the same cells allocates nothing")
        m.undo(); m.undo()
        expectEq(m.tileCount(of: 0), before, "undo takes the tiles with it")
        // A rectangle preview dragged around leaves only the final shape's tiles.
        m.tool = .rectangle
        m.opts.filled = true
        d.down(2, 2)
        for p in [(20, 12), (40, 20), (60, 28), (30, 10), (12, 12)] { d.drag(p.0, p.1) }
        d.up(12, 12)
        expectEq(m.tileCount(of: 0) - before, 4, "only the cells of the final rectangle have tiles")
        d.snapshot("tilemap-long-stroke")
    }

    scenario("filters/apply-to-tilemap-layers") {
        let m = fresh(32, 16), d = CanvasDriver()
        put(d, 1, 1, GREEN)   // a normal layer below
        m.newTilemapLayer(tileWidth: 8, tileHeight: 8)
        let tm = m.layer
        m.color = RED
        paintRect(d, 2, 2, 5, 5, RED)
        paintRect(d, 18, 2, 21, 5, RED)
        expect(m.applyFilter(.invert), "a filter applies to a tilemap layer")
        expectPixel(3, 3, 0x00FFFFFF, "the tile pixels changed")
        expectPixel(19, 3, 0x00FFFFFF)
        expectPixel(1, 1, GREEN, "the normal layer below is untouched by a cel filter")
        let look = m.pixels()
        let rebuilt = tilemapPixels(m, layer: tm)
        for (i, c) in rebuilt.enumerated() where c != 0 { expectEq(look[i], c, "tile image vs canvas at \(i % 32),\(i / 32)") }
        let url = tmp("tm-filter.aseprite")
        m.fileURL = url
        m.save()
        m.open(url)
        expectEq(m.pixels(), look, "the filtered tilemap reopens the same")
        m.undo()
        expectPixel(3, 3, RED, "undo restores the tiles")
        // A selection limits the filter on tilemaps too; all-layers also filters the normal layer.
        m.tool = .marquee
        d.stroke([(0, 0), (9, 7)])
        expect(m.applyFilter(.invert, target: 2))
        expectPixel(3, 3, 0x00FFFFFF)
        expectPixel(19, 3, RED, "outside the selection")
        expectPixel(1, 1, 0xFF00FFFF, "the normal layer was filtered as well")
    }
}
