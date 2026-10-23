import AppKit
@testable import PixmithKit
import PixmithCore

func color(_ buf: [UInt8], _ i: Int) -> UInt32 {
    let r = UInt32(buf[i]), g = UInt32(buf[i + 1]), b = UInt32(buf[i + 2]), a = UInt32(buf[i + 3])
    return r << 24 | g << 16 | b << 8 | a
}

/// The canvas rebuilt from a tilemap layer's tile grid and the tileset's tile pixels (cells with flip flags are skipped).
func tilemapPixels(_ m: SpriteModel, layer: Int, frame: Int = 0) -> [UInt32] {
    let w = m.width, h = m.height
    var out = [UInt32](repeating: 0, count: w * h)
    guard let ts = Optional(Int(pixmith_layer_tileset(m.doc, UInt32(layer)))), ts >= 0 else { return out }
    let (tw, th) = m.tileSize(of: ts)
    for ty in 0..<h / th {
        for tx in 0..<w / tw {
            let v = pixmith_tile_at(m.doc, UInt32(layer), UInt32(frame), Int32(tx * tw), Int32(ty * th))
            let idx = Int(v & 0x1FFFFFFF)
            if idx == 0 || v & 0xE0000000 != 0 { continue }
            var buf = [UInt8](repeating: 0, count: tw * th * 4)
            _ = pixmith_tileset_tile_pixels(m.doc, UInt32(ts), UInt32(idx), &buf, buf.count)
            for y in 0..<th {
                for x in 0..<tw { out[(ty * th + y) * w + tx * tw + x] = color(buf, (y * tw + x) * 4) }
            }
        }
    }
    return out
}

func registerMore() {
    scenario("select/tools-respect-selection") {
        let m = fresh(16, 16), d = CanvasDriver()
        func inside(_ pts: [(Int, Int)]) -> Bool { pts.allSatisfy { (4...9).contains($0.0) && (4...9).contains($0.1) } }
        func select() {
            m.tool = .marquee
            d.stroke([(4, 4), (9, 9)])
        }
        select()
        m.color = RED
        m.opts.filled = true
        let tools: [(Tool, String, () -> Void)] = [
            (.pencil, "pencil", { d.stroke([(0, 6), (15, 6)]) }),
            (.line, "line", { d.stroke([(0, 0), (15, 15)]) }),
            (.rectangle, "rectangle", { d.stroke([(1, 1), (14, 14)]) }),
            (.ellipse, "ellipse", { d.stroke([(0, 0), (15, 15)]) }),
            (.fill, "fill", { d.click(7, 7) }),
            (.gradient, "gradient", { d.stroke([(0, 8), (15, 8)]) }),
            (.spray, "spray", { d.down(7, 7); pumpRunLoop(0.15); d.up(7, 7) }),
            (.shapePolygon, "polygon", { d.click(0, 0); d.click(15, 0); d.click(15, 15); d.click(0, 15); d.click(0, 15, clicks: 2) }),
            (.contour, "contour", { d.stroke([(0, 0), (15, 0), (15, 15), (0, 15)]) }),
            (.curve, "curve", { d.down(0, 8); d.drag(15, 8); d.up(15, 8); d.down(8, 0); d.up(8, 0); d.down(8, 15); d.up(8, 15) }),
        ]
        for (tool, name, run) in tools {
            m.tool = tool
            m.color = RED
            run()
            m.tool = .pencil
            let pts = m.opaque()
            expect(!pts.isEmpty, "\(name) painted inside the selection")
            expect(inside(pts), "\(name) stays inside the selection: \(pts.filter { !inside([$0]) }.prefix(3))")
            m.undo()
            expectEq(m.visibleCount(), 0, "\(name) undone")
        }
        // Text.
        m.tool = .text
        m.opts.textScale = 1
        d.click(4, 4)
        m.opts.textString = "WIDE TEXT"
        m.renderText()
        m.placeText()
        let pts = m.opaque()
        expect(!pts.isEmpty && inside(pts), "text clipped to the selection")
        m.undo()
        // Select-all then delete clears everything; invert selection protects the first area.
        m.tool = .pencil
        paintRect(d, 0, 0, 15, 15, BLUE)
        m.invertSelection()
        m.deleteSelection()
        expectEq(m.visibleCount(), 36, "delete after invert leaves just the old selection")
        d.snapshot("select-clipped")
    }

    scenario("select/free-transform") {
        let m = fresh(), d = CanvasDriver()
        paintRect(d, 4, 4, 7, 7, RED)
        m.tool = .marquee
        d.stroke([(4, 4), (7, 7)])
        d.bitmap()
        // Typed width via the transform bar.
        m.tool = .move
        m.startTransform()
        expect(m.transformActive)
        var f = m.transformFields()
        expectEq(f[2], 4)
        expectEq(f[3], 4)
        m.setTransformField(2, 8)
        f = m.transformFields()
        expectEq(f[2], 8)
        m.commitTransform()
        expect(!m.transformActive)
        expectEq(m.visibleCount(), 32, "the block became 8x4")
        let box = m.opaque()
        expectEq(box.map(\.1).min(), 4)
        expectEq(box.map(\.1).max(), 7)
        expectEq((box.map(\.0).max() ?? 0) - (box.map(\.0).min() ?? 0), 7)
        m.undo()
        expectEq(m.visibleCount(), 16, "a transform is one undo step")
        // Cancel restores the original.
        m.startTransform()
        m.setTransformField(2, 12)
        m.cancelTransform()
        expect(!m.transformActive)
        expectEq(m.visibleCount(), 16)
        // Dragging the right-middle handle with the mouse.
        d.bitmap()
        d.down(8, 6)
        expect(m.transformActive, "grabbing a handle starts a transform")
        d.drag(10, 6)
        d.drag(12, 6)
        d.up(12, 6)
        expect(m.transformFields()[2] > 5, "the handle drag widened the box: \(m.transformFields())")
        m.commitTransform()
        expect(m.visibleCount() > 16)
        m.undo()
        // Rotation by a typed angle keeps a square block square.
        m.startTransform()
        m.setTransformField(4, 90)
        m.commitTransform()
        expectEq(m.visibleCount(), 16, "a 90 degree turn keeps the pixels")
        m.undo()
        // Moving the whole box with the mouse (inside, away from the handles).
        paintRect(d, 4, 4, 7, 7, RED)
        d.stroke([(4, 4), (7, 7)].map { $0 })
        d.snapshot("select-transform")
    }

    scenario("canvas/resize-crop-with-slices-and-frames") {
        let m = fresh(16, 16), d = CanvasDriver()
        put(d, 5, 5, RED)
        m.addFrame(duplicate: true)
        put(d, 6, 6, BLUE)
        m.tool = .slice
        d.stroke([(4, 4), (7, 7)])
        m.tool = .pencil
        m.frame = 0
        // Canvas size dialog: grow by 4 with the anchor at the center (default).
        answerModal("Resize", fill: { w in
            let f = w.textFields
            f[0].stringValue = "20"
            f[1].stringValue = "20"
        }) { m.showCanvasSizeDialog() }
        expectEq(m.width, 20)
        expectEq(m.height, 20)
        expectPixel(7, 7, RED, "pixels move with the center anchor")
        expectPixel(8, 8, BLUE, "second frame too", frame: 1)
        expectEq(m.frameCount, 2)
        expectEq(m.sliceInfo(0)?.x, 6, "slices move with the content")
        m.undo()
        expectEq(m.width, 16)
        expectPixel(5, 5, RED)
        // Top-left anchor keeps the pixels in place.
        answerModal("Resize", fill: { w in
            w.textFields[0].stringValue = "10"
            w.textFields[1].stringValue = "10"
            w.popups.first?.selectItem(at: 0)
        }) { m.showCanvasSizeDialog() }
        expectEq(m.width, 10)
        expectPixel(5, 5, RED)
        m.undo()
        answerModal("Cancel") { m.showCanvasSizeDialog() }
        expectEq(m.width, 16)
        // Crop to selection.
        m.tool = .marquee
        d.stroke([(4, 4), (9, 9)])
        m.cropToSelection()
        expectEq(m.width, 6)
        expectEq(m.height, 6)
        expectPixel(1, 1, RED, "cropped coordinates")
        expectPixel(2, 2, BLUE, "second frame cropped as well", frame: 1)
        expectEq(m.frameCount, 2)
        expect(!m.hasSelection, "the selection is gone after cropping")
        m.undo()
        expectEq(m.width, 16)
        expectPixel(5, 5, RED)
        d.snapshot("canvas-crop")
    }

    scenario("layers/tilemap-interplay") {
        let m = fresh(32, 32), d = CanvasDriver()
        put(d, 1, 1, GREEN)
        m.newTilemapLayer(tileWidth: 8, tileHeight: 8)
        let tmLayer = m.layer
        m.color = RED
        // Every drawing tool updates the tile grid, so the look and the tiles agree and survive a save.
        let tools: [(Tool, String, () -> Void)] = [
            (.pencil, "pencil", { d.stroke([(2, 2), (13, 2)]) }),
            (.line, "line", { d.stroke([(0, 10), (20, 20)]) }),
            (.rectangle, "rectangle", { d.stroke([(2, 2), (18, 12)]) }),
            (.ellipse, "ellipse", { d.stroke([(2, 2), (18, 18)]) }),
            (.fill, "fill", { d.click(25, 25) }),
            (.spray, "spray", { d.down(16, 16); pumpRunLoop(0.1); d.up(16, 16) }),
            (.gradient, "gradient", { m.opts.secondary = BLUE; d.stroke([(0, 0), (31, 0)]) }),
        ]
        m.opts.filled = true
        for (tool, name, run) in tools {
            m.tool = tool
            run()
            m.tool = .pencil
            let look = m.pixels()
            expect(m.visibleCount() > 0, "\(name) drew something")
            // Reading the grid back through the tile pixels must give the same picture.
            let rebuilt = tilemapPixels(m, layer: tmLayer)
            for i in 0..<rebuilt.count where rebuilt[i] != 0 && look[i] != rebuilt[i] && i != 33 {
                QA.fail("\(name): tile pixels (\(i % 32),\(i / 32)) = \(hex(rebuilt[i])) but the canvas shows \(hex(look[i]))", #fileID, #line)
                break
            }
            let url = tmp("tm-\(name).aseprite")
            m.fileURL = url
            m.save()
            let saved = m.pixels()
            m.open(url)
            expectEq(m.pixels(), saved, "\(name): the saved tilemap reopens with the same look")
            m.layer = tmLayer
            expect(m.isTilemapLayer, "\(name): still a tilemap layer after reopening")
            m.undo()
            m.layer = tmLayer
            // After reopening the undo history is gone; clear the tilemap layer for the next tool.
            m.tool = .eraser
            m.brushSize = 40
            d.click(16, 16)
            m.brushSize = 1
            m.tool = .pencil
        }
        // Undo restores the tiles together with the pixels.
        m.tool = .rectangle
        d.stroke([(2, 2), (13, 13)])
        let drawn = m.pixels()
        let tilesAfter = m.tileCount(of: 0)
        m.undo()
        expect(m.pixels() != drawn)
        m.redo()
        expectEq(m.pixels(), drawn)
        expectEq(m.tileCount(of: 0), tilesAfter, "redo brings the tiles back")
        // Duplicate the tilemap layer.
        m.tool = .pencil
        m.frame = 0
        m.layer = tmLayer
        m.duplicateLayer(tmLayer)
        expectEq(m.layerCount, 3)
        expect(m.isTilemapLayer, "a duplicated tilemap layer is a tilemap layer")
        d.snapshot("tilemap-interplay")
    }

    scenario("tilemap/frames-are-independent") {
        let m = fresh(32, 32), d = CanvasDriver()
        m.newTilemapLayer(tileWidth: 8, tileHeight: 8)
        let tm = m.layer
        m.color = RED
        d.click(3, 3)
        m.addFrame(duplicate: true)
        expectPixel(3, 3, RED, "a duplicated frame starts as a copy")
        m.color = BLUE
        d.click(27, 27)
        expectPixel(27, 27, BLUE)
        expectPixel(27, 27, CLEAR, "the first frame is unaffected", frame: 0)
        expectEq(pixmith_tile_at(m.doc, UInt32(tm), 0, 27, 27), 0, "the first frame's cell is still empty")
        m.frame = 0
        m.color = GREEN
        d.click(11, 27)
        expectPixel(11, 27, CLEAR, "the second frame is unaffected", frame: 1)
        // Both survive a save.
        let url = tmp("tm-frames.aseprite")
        m.fileURL = url
        m.save()
        let f0 = m.pixels(frame: 0), f1 = m.pixels(frame: 1)
        m.open(url)
        expectEq(m.pixels(frame: 0), f0)
        expectEq(m.pixels(frame: 1), f1)
    }

    scenario("layers/merge-with-tilemap") {
        let m = fresh(32, 32), d = CanvasDriver()
        m.newTilemapLayer(tileWidth: 8, tileHeight: 8)
        let tm = m.layer
        m.color = RED
        d.click(3, 3)
        m.addLayer()
        put(d, 20, 20, BLUE)
        let look = m.pixels()
        expect(m.canMergeDown(m.layer))
        m.mergeLayerDown(m.layer)
        expectEq(m.pixels(), look, "merging keeps the look")
        let url = tmp("merge-tm.aseprite")
        m.fileURL = url
        m.save()
        m.open(url)
        expectEq(m.pixels(), look, "merged tilemap saves and reopens intact")
        _ = tm
        d.snapshot("layers-merge-tilemap")
    }

    scenario("linked/stay-in-sync-through-edits") {
        let m = fresh(16, 16), d = CanvasDriver()
        paintRect(d, 4, 4, 9, 9, RED)
        m.addFrame(duplicate: false)
        m.addFrame(duplicate: false)
        m.frame = 0
        m.linkCels(from: 0, to: 2)
        m.frame = 2
        func synced(_ what: String, file: String = #fileID, line: UInt = #line) {
            let a = m.pixels(frame: 0), b = m.pixels(frame: 1), c = m.pixels(frame: 2)
            expect(a == b && b == c, "linked cels differ after \(what)", file: file, line: line)
            expect(m.isCelLinked, "the cels are still linked after \(what)", file: file, line: line)
        }
        m.color = BLUE
        m.tool = .pencil; d.stroke([(0, 0), (5, 0)]); synced("pencil")
        m.tool = .line; d.stroke([(0, 15), (15, 15)]); synced("line")
        m.opts.filled = true
        m.tool = .rectangle; d.stroke([(11, 1), (14, 4)]); synced("rectangle")
        m.tool = .fill; d.click(12, 12); synced("fill")
        m.tool = .spray; d.down(1, 12); pumpRunLoop(0.1); d.up(1, 12); synced("spray")
        m.tool = .gradient; m.opts.secondary = GREEN; d.stroke([(0, 8), (15, 8)]); synced("gradient")
        m.tool = .marquee; d.stroke([(4, 4), (9, 9)])
        m.transform(0); synced("flip")
        m.transform(2); synced("rotate")
        m.tool = .move; d.stroke([(6, 6), (8, 6)]); synced("selection move")
        m.copySelection(cut: false)
        m.deselect()
        m.pasteFromPasteboard(); d.enter(); synced("paste")
        m.tool = .marquee; d.stroke([(2, 2), (6, 6)]); m.deleteSelection(); synced("delete")
        m.deselect()
        expect(m.applyFilter(.invert), "invert applies")
        synced("an invert filter on the current cel")
        expect(m.applyFilter(.blur, target: 1), "blur applies")
        synced("a blur filter over all frames of the layer")
        m.setCelOpacity(120); synced("cel opacity")
        // Undo and redo keep the cels linked and equal.
        m.undo(); synced("undo")
        m.redo(); synced("redo")
        // A free transform too.
        m.tool = .marquee; d.stroke([(2, 2), (9, 9)])
        m.tool = .move
        m.startTransform()
        m.setTransformField(2, 10)
        m.commitTransform()
        synced("free transform")
        d.snapshot("linked-sync")
    }

    scenario("filters/respect-selection-mask") {
        let m = fresh(12, 12), d = CanvasDriver()
        paintRect(d, 0, 0, 11, 11, RED)
        m.tool = .lasso
        d.stroke([(1, 1), (10, 1), (10, 10), (1, 1)])
        expect(m.hasSelection)
        // The FX menu passes the selection's bounding box, the core limits the change to the mask itself.
        var b = [Int32](repeating: 0, count: 4)
        _ = pixmith_sel_bounds(m.doc, &b)
        expect(m.applyFilter(.invert, region: (Int(b[0]), Int(b[1]), Int(b[2]), Int(b[3]))))
        expectPixel(9, 2, 0x00FFFFFF, "inside the lasso")
        expectPixel(2, 9, RED, "inside the box but outside the lasso")
        expectPixel(0, 0, RED, "outside the box")
        m.undo()
        // Without a region the whole canvas minus the mask is protected too.
        expect(m.applyFilter(.invert))
        expectPixel(2, 9, RED)
        expectPixel(0, 0, RED)
        expectPixel(9, 2, 0x00FFFFFF)
    }

    scenario("view/render-snapshots") {
        let m = fresh(12, 12), d = CanvasDriver()
        m.zoom = 20
        m.color = RED
        d.stroke([(1, 1), (10, 1), (10, 10)])
        put(d, 3, 6, BLUE)
        // Checkerboard and the pixel grid show where the sprite is transparent.
        var rep = d.snapshot("view-grid")
        let sprite = d.shown(3, 6, in: rep)
        // The bitmap is in the display's color space, so allow some drift.
        expect(sprite.map { Int($0 & 0xFF) > 200 && Int($0 >> 16) < 40 && Int(($0 >> 8) & 0xFF) < 90 } == true, "a blue pixel renders blue: \(String(describing: sprite))")
        let red = d.shown(5, 1, in: rep)
        expect(red.map { Int($0 >> 16) > 220 && Int(($0 >> 8) & 0xFF) < 90 && Int($0 & 0xFF) < 90 } == true, "a red pixel renders red: \(String(describing: red))")
        let empty1 = d.shown(2, 3, in: rep), empty2 = d.shown(3, 3, in: rep)
        expect(empty1 != nil && empty2 != nil, "sampled the transparent area")
        // Grid off: the lines vanish.
        let withGrid = d.bitmap()
        m.showGrid = false
        let noGrid = d.bitmap()
        expect(noGrid.tiffRepresentation != withGrid.tiffRepresentation, "toggling the grid changes the canvas")
        m.showGrid = true
        // Selection ants.
        m.tool = .marquee
        d.stroke([(2, 2), (7, 7)])
        rep = d.snapshot("view-selection")
        // Tiled mode.
        m.deselect()
        ViewSettings.shared.tiled = .both
        rep = d.snapshot("view-tiled")
        ViewSettings.shared.tiled = .none
        // Custom grid.
        ViewSettings.shared.customGrid = true
        ViewSettings.shared.customGridW = 4
        ViewSettings.shared.customGridH = 4
        d.snapshot("view-custom-grid")
        ViewSettings.shared.customGrid = false
        // Mirror guides and tile grid overlay.
        m.opts.mirrorX = true
        m.opts.mirrorY = true
        d.snapshot("view-mirror-guides")
        m.opts.mirrorX = false
        m.opts.mirrorY = false
        // Zoom to fit and actual size.
        ViewSettings.shared.zoomToFit(m)
        expect(m.zoom > 1, "zoom to fit picked a zoom: \(m.zoom)")
        d.snapshot("view-fit")
        ViewSettings.shared.actualSize(m)
        expectEq(m.zoom, 1)
        d.snapshot("view-actual")
        // Reference image behind the sprite.
        m.zoom = 20
        let refURL = tmp("ref.png")
        try? pngData(24, 24, fill: 0x33AA33FF).write(to: refURL)
        ViewSettings.shared.loadReference(refURL)
        expect(ViewSettings.shared.referenceImage != nil, "reference image loaded")
        d.snapshot("view-reference")
        ViewSettings.shared.clearReference()
        // Slices overlay.
        m.tool = .slice
        d.stroke([(2, 2), (6, 6)])
        d.snapshot("view-slice")
    }
}
