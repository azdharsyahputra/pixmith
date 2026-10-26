import AppKit
@testable import PixmithKit
import PixmithCore

// pixmith-qa scenarios for the isometric grid (View > Isometric Grid, View Options > Custom Grid, Select > Isometric Tile).

/// Alpha of every pixel of `CanvasOverlays.drawFront` for the model, rendered at zoom 4 (row-major, one value per sprite pixel).
private func frontAlpha(_ m: SpriteModel) -> [UInt8] {
    let z = 4
    let (w, h) = (m.width * z, m.height * z)
    let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                        space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    // The canvas view is flipped: y grows downward.
    ctx.translateBy(x: 0, y: CGFloat(h))
    ctx.scaleBy(x: 1, y: -1)
    m.zoom = CGFloat(z)
    CanvasOverlays.drawFront(ctx, model: m, dirty: CGRect(x: 0, y: 0, width: w, height: h), backingScale: 1)
    let data = ctx.data!.assumingMemoryBound(to: UInt8.self)
    var out = [UInt8](repeating: 0, count: m.width * m.height)
    for y in 0..<m.height {
        for x in 0..<m.width {
            // Memory row 0 is the top row of the image, which is user-space y = 0 after the flip.
            out[y * m.width + x] = data[(y * z + z / 2) * ctx.bytesPerRow + (x * z + z / 2) * 4 + 3]
        }
    }
    return out
}

func registerIso() {
    scenario("isogrid/settings-round-trip") {
        _ = fresh(64, 32)
        let s = ViewSettings.shared
        let rectangular = [s.customGridW, s.customGridH, s.customGridX, s.customGridY]
        // Defaults.
        expectEq(s.customGridType, .rectangular)
        expectEq([s.isoGridW, s.isoGridH, s.isoGridX, s.isoGridY], [32, 16, 0, 0])
        expect(!s.isometricGrid)
        // The View menu toggle turns the custom grid on as isometric, and off again.
        s.isometricGrid = true
        expect(s.customGrid && s.customGridType == .isometric && s.isometricGrid, "toggle on")
        s.isometricGrid = false
        expect(!s.customGrid && !s.isometricGrid, "toggle off")
        // Turning it off while the grid is rectangular leaves that grid alone.
        s.customGridType = .rectangular
        s.customGrid = true
        s.isometricGrid = false
        expect(s.customGrid, "a rectangular grid is not an isometric one")
        // Sizes: even and at least 4.
        s.isoGridW = 3
        expectEq(s.isoGridW, 4)
        s.isoGridH = 13
        expectEq(s.isoGridH, 12)
        s.isoGridW = 104
        s.isoGridH = 56
        s.isoGridX = -5
        s.isoGridY = 7
        s.isometricGrid = true
        // Saved: read the stored JSON, then put it back over changed values and reload.
        let saved = UserDefaults.standard.data(forKey: "viewSettings")
        let json = saved.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] } ?? [:]
        expectEq(json["customGridType"] as? String, "isometric")
        expectEq(json["isoGridW"] as? Int, 104)
        expectEq(json["isoGridH"] as? Int, 56)
        expectEq(json["isoGridX"] as? Int, -5)
        expectEq(json["isoGridY"] as? Int, 7)
        s.customGridType = .rectangular
        s.isoGridW = 8
        s.isoGridH = 8
        s.isoGridX = 0
        s.isoGridY = 0
        s.customGrid = false
        UserDefaults.standard.set(saved, forKey: "viewSettings")
        s.reloadFromDefaults()
        expectEq(s.customGridType, .isometric)
        expectEq([s.isoGridW, s.isoGridH, s.isoGridX, s.isoGridY], [104, 56, -5, 7])
        expect(s.customGrid && s.isometricGrid, "the grid is on after reload")
        expectEq(IsoGrid.current, IsoGrid(w: 104, h: 56, x: -5, y: 7))
        // The rectangular settings are untouched.
        expectEq([s.customGridW, s.customGridH, s.customGridX, s.customGridY], rectangular)
    }

    scenario("isogrid/draws-diamond-lines") {
        let m = fresh(64, 32)
        let s = ViewSettings.shared
        m.showGrid = false
        s.customGrid = false
        expect(frontAlpha(m).allSatisfy { $0 == 0 }, "nothing is drawn without a grid")
        s.isometricGrid = true
        let alpha = frontAlpha(m)
        // Lines step 2 across per 1 down: row y holds 32n + 14 - 2y and 15 - 2y (down-left), 32n + 16 + 2y and 17 + 2y.
        for y in 0..<m.height {
            var want = Set<Int>()
            for n in -4...4 {
                for x in [32 * n + 14 - 2 * y, 32 * n + 15 - 2 * y, 32 * n + 16 + 2 * y, 32 * n + 17 + 2 * y] where x >= 0 && x < m.width {
                    want.insert(x)
                }
            }
            let got = Set((0..<m.width).filter { alpha[y * m.width + $0] > 0 })
            expectEq(got, want, "row \(y)")
        }
        // The custom grid's opacity (0.6).
        expect(alpha[15] >= 150 && alpha[15] <= 156, "opacity \(alpha[15])")
        // The outline of a cell's mask lies on the lines: its first and last pixel of every row.
        let grid = IsoGrid.current
        for r in grid.rows(grid.cell(atX: 16, y: 8)) {
            expect(alpha[r.y * 64 + r.l] > 0 && alpha[r.y * 64 + r.r] > 0, "mask edge of row \(r.y) is on a grid line")
        }
        // The offset moves the lines.
        s.isoGridX = 3
        let moved = frontAlpha(m)
        expect(moved[18] > 0 && moved[19] > 0 && moved[15] == 0, "the apex follows the offset")
        // A rectangular grid draws no diamond: its 1-pixel lines run between pixels, so no pixel center is covered.
        s.customGridType = .rectangular
        s.customGridW = 16
        s.customGridH = 16
        expect(frontAlpha(m).allSatisfy { $0 == 0 }, "no diamond pixels in a rectangular grid")
    }

    scenario("isogrid/select-tile-mask") {
        let m = fresh(64, 32), d = CanvasDriver()
        let s = ViewSettings.shared
        s.isoGridX = 16
        s.isoGridY = 8
        // The last click (the cursor is not over the canvas): the tile whose box is (16, 8) to (48, 24).
        m.tool = .marquee
        d.click(32, 16)
        StatusInfo.shared.cursor = nil
        expect(m.selectIsometricTile(), "selected")
        expectEq(m.selectedCount, 256)
        expectEq(m.selectionBounds ?? [], [17, 8, 30, 16])
        for y in 0..<32 {
            for x in 0..<64 {
                let dx: Double = abs(Double(x) + 0.5 - 32.0) / 16.0
                let dy: Double = abs(Double(y) + 0.5 - 16.0) / 8.0
                let inside = dx + dy < 1.0
                if m.selected(x, y) != inside { expect(false, "pixel (\(x),\(y)) selected \(m.selected(x, y)), the diamond says \(inside)") }
            }
        }
        // Corners and edges.
        let on: [(Int, Int)] = [(31, 8), (32, 8), (31, 23), (32, 23), (17, 15), (17, 16), (46, 15), (46, 16), (29, 9), (34, 9)]
        let off: [(Int, Int)] = [(30, 8), (33, 8), (30, 23), (16, 15), (17, 14), (47, 16), (28, 9), (35, 9)]
        for p in on { expect(m.selected(p.0, p.1), "corner/edge pixel \(p) is in the tile") }
        for p in off { expect(!m.selected(p.0, p.1), "pixel \(p) just outside the tile") }
        // One undo step.
        expectEq(m.historyCounts.undo, 1)
        m.undo()
        expect(!m.hasSelection, "undone")
        // The cursor wins over the last click.
        d.move(48, 24)
        expect(m.selectIsometricTile(), "selected under the cursor")
        expectEq(m.selectedCount, 256)
        expect(m.selected(48, 24) && !m.selected(32, 16), "the tile around (48, 24)")
        // Selecting again replaces the selection.
        StatusInfo.shared.cursor = nil
        expect(m.selectIsometricTile())
        expect(m.selected(32, 16) && !m.selected(48, 24), "replaced")
        // A tile cut by the canvas edge: pixel (0, 0) belongs to the tile centered there, of which a quarter shows.
        expect(m.selectIsometricTile(at: (0, 0)))
        expectEq(m.selectedCount, 64)
        let quarter: [Bool] = [m.selected(0, 0), m.selected(14, 0), !m.selected(15, 0), m.selected(0, 7), !m.selected(0, 8)]
        expect(!quarter.contains(false), "quarter tile \(quarter)")
        // No pixel, no selection change.
        expect(!m.selectIsometricTile(at: (64, 0)))
        expectEq(m.selectedCount, 64)
        // Painting inside the tile stays inside it.
        m.deselect()
        expect(m.selectIsometricTile(at: (32, 16)))
        m.color = RED
        m.fill(at: (32, 16))
        expectPixel(32, 16, RED)
        expectPixel(17, 15, RED)
        expectPixel(16, 15, CLEAR)
        expectPixel(32, 7, CLEAR)
        expectPixel(30, 8, CLEAR)
        // A larger tile (104x56): the mask holds w * h / 2 pixels.
        let big = fresh(208, 112)
        s.isoGridW = 104
        s.isoGridH = 56
        s.isoGridX = 0
        s.isoGridY = 0
        expect(big.selectIsometricTile(at: (104, 56)))
        expectEq(big.selectedCount, 104 * 56 / 2)
        expectEq(big.selectionBounds ?? [], [53, 28, 102, 56])
        // Tiles around it share no pixel.
        var union = Set<Int>()
        var total = 0
        let centers: [(Int, Int)] = [(104, 56), (52, 28), (156, 28), (52, 84), (156, 84), (104, 0), (0, 56)]
        for c in centers {
            big.deselect()
            if big.selectIsometricTile(at: c) {
                for y in 0..<112 { for x in 0..<208 where big.selected(x, y) { union.insert(y * 208 + x); total += 1 } }
            }
        }
        expectEq(union.count, total, "tiles do not overlap")
        expect(total > 5 * 2912, "and the tiles were selected: \(total)")
    }

    scenario("isotilemap/layer-stamp-offset-pivot") {
        let m = fresh(128, 64), d = CanvasDriver()
        let tm = TilemapState.shared
        let ts0 = m.tilesetCount
        // New isometric layer: 32x16 diamonds, 32x32 tile pictures, grid at (64, 16).
        m.newIsoTilemapLayer(tileWidth: 32, tileHeight: 32, cellWidth: 32, cellHeight: 16, offsetX: 64, offsetY: 16)
        expect(m.isTilemapLayer && m.isIsoLayer, "an isometric tilemap layer")
        expectEq(m.tilesetCount, ts0 + 1)
        let ts = m.tilesetIndex ?? -1
        expectEq(m.isoCellSize(of: ts)?.w, 32)
        expectEq(m.isoCellSize(of: ts)?.h, 16)
        expectEq(m.isoLayerGrid, IsoGrid(w: 32, h: 16, x: 64, y: 16))
        // The layer's grid replaces the View grid for Select > Isometric Tile and the overlay.
        expectEq(IsoGrid.active(for: m), IsoGrid(w: 32, h: 16, x: 64, y: 16))
        expect(m.selectIsometricTile(at: (80, 24)), "iso tile selected")
        expectEq(m.selectedCount, 32 * 16 / 2)
        expect(m.selected(80, 24) && !m.selected(64, 16), "the diamond of cell (0, 0)")
        m.deselect()
        // Stamp mode: click puts the tile in the diamond cell under the pointer.
        m.addTile()
        let tile = tm.selected
        expect(tile > 0, "a tile to stamp")
        tm.stamp = true
        m.tool = .pencil
        tm.stamp = true
        let layer = UInt32(m.layer)
        d.move(80, 24)
        expect(IsoHover.cell?.i == 0 && IsoHover.cell?.j == 0, "hover cell (0, 0)")
        d.move(96, 32)
        expect(IsoHover.cell?.i == 1 && IsoHover.cell?.j == 0, "hover cell (1, 0)")
        d.click(80, 24)
        expectEq(Int(pixmith_tile_in_cell(m.doc, layer, 0, 0, 0) & 0x1FFF_FFFF), tile)
        let undo = m.historyCounts.undo
        d.stroke([(96, 32), (112, 40), (80, 40)])
        expectEq(m.historyCounts.undo, undo + 1, "a stamp drag is one undo step")
        expectEq(Int(pixmith_tile_in_cell(m.doc, layer, 0, 1, 0) & 0x1FFF_FFFF), tile)
        expectEq(Int(pixmith_tile_in_cell(m.doc, layer, 0, 1, 1) & 0x1FFF_FFFF), tile)
        // Shift erases, Option picks.
        d.click(96, 32, flags: .shift)
        expectEq(pixmith_tile_in_cell(m.doc, layer, 0, 1, 0), 0, "shift-click erases the cell")
        tm.selected = 77
        d.click(80, 24, flags: .option)
        expectEq(tm.selected, tile, "option-click picks the tile")
        tm.stamp = false
        // Layer offset: one undo step, moves the grid.
        let n = m.historyCounts.undo
        m.setIsoOffset(x: 64, y: 40)
        expectEq(m.isoOffset(of: m.layer)?.y, 40)
        expectEq(m.historyCounts.undo, n + 1)
        expectEq(m.isoLayerGrid, IsoGrid(w: 32, h: 16, x: 64, y: 40))
        m.undo()
        expectEq(m.isoOffset(of: m.layer)?.y, 16)
        // Tile pivot: default, set, clear.
        let def = m.tilePivot(ts, tile)
        expect(def != nil && !m.isPivotSet(ts, tile), "default pivot")
        m.setPivot(ts, tile, x: 10, y: 20)
        expectEq(m.tilePivot(ts, tile)?.x, 10)
        expectEq(m.tilePivot(ts, tile)?.y, 20)
        expect(m.isPivotSet(ts, tile))
        expectEq(m.tileTextureOrigin(ts, tile)?.x, 10 - 16)
        expectEq(m.tileTextureOrigin(ts, tile)?.y, 20 - 16)
        m.clearPivot(ts, tile)
        expect(!m.isPivotSet(ts, tile))
        expectEq(m.tilePivot(ts, tile)?.x, def?.x)
        // Terrain: the dual-grid Corners mode is refused on isometric layers, Blob works.
        m.createTerrain(name: "Grass", mode: 2)
        expect(m.terrainMode < 0, "no corners terrain on an isometric layer")
        m.createTerrain(name: "Grass", mode: 1)
        expectEq(m.terrainMode, 1)
        AutotileState.shared.brush = true
        AutotileState.shared.terrain = m.terrainIds.first ?? 1
        d.click(80, 24)
        expectEq(pixmith_autotile_terrain_at(m.doc, layer, 0, 80, 24), AutotileState.shared.terrain, "terrain painted cell (0, 0)")
        expectEq(pixmith_autotile_terrain_at(m.doc, layer, 0, 96, 32), 0, "and not its neighbour")
        AutotileState.shared.brush = false
        // Not an isometric layer on a square tilemap layer.
        m.newTilemapLayer(tileWidth: 8, tileHeight: 8)
        expect(m.isTilemapLayer && !m.isIsoLayer && m.isoLayerGrid == nil, "square layers have no iso grid")
        expectEq(IsoGrid.active(for: m), IsoGrid.current)
        d.snapshot("iso-tilemap")
    }
}
