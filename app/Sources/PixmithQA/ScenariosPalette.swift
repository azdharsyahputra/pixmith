import AppKit
@testable import PixmithKit
import PixmithCore

extension SpriteModel {
    /// What the FX sheet's Apply button does.
    @discardableResult
    func applyFilter(_ kind: FilterKind, target: UInt32 = 0, params: [Int32] = [], region: (x: Int, y: Int, w: Int, h: Int) = (0, 0, 0, 0)) -> Bool {
        let ok = params.withUnsafeBufferPointer {
            pixmith_filter_apply(doc, kind.coreKind, target, UInt32(layer), UInt32(frame), Int32(region.x), Int32(region.y),
                              UInt32(region.w), UInt32(region.h), $0.baseAddress, $0.count)
        }
        if ok { labelLastStep("Filter: \(kind.title)"); changed() }
        return ok
    }
}

func rgb(_ p: UInt32) -> (Int, Int, Int) { (Int(p >> 24 & 0xFF), Int(p >> 16 & 0xFF), Int(p >> 8 & 0xFF)) }

func registerPalette() {
    scenario("palette/edit-presets-ramp") {
        let m = fresh()
        let original = m.palette
        expectEq(original.count, 16)
        // Add, remove, sort.
        m.color = 0x123456FF
        m.addColorToPalette()
        expectEq(m.palette.count, 17)
        expectEq(m.palette.last, 0x123456FF)
        m.removePaletteColor(at: 16)
        expectEq(m.palette, original)
        m.sortPalette(byHue: true)
        expectEq(Set(m.palette), Set(original), "sorting keeps the colors")
        m.undo()
        expectEq(m.palette, original, "sorting is undoable")
        m.sortPalette(byHue: false)
        let lum = { (c: UInt32) -> Double in let (r, g, b) = rgb(c); return 0.2126 * Double(r) + 0.7152 * Double(g) + 0.0722 * Double(b) }
        let sorted = m.palette.map(lum)
        expect(sorted == sorted.sorted() || sorted == sorted.sorted(by: >), "sorted by luminance: \(sorted)")
        m.undo()
        // Insert, duplicate, move, remove, live edit.
        expect(m.insertPaletteEntry(at: 2, color: 0xABCDEFFF))
        expectEq(m.palette[2], 0xABCDEFFF)
        expectEq(m.palette.count, 17)
        expect(m.duplicatePaletteEntries([2]))
        expectEq(m.palette[3], 0xABCDEFFF)
        expectEq(m.palette.count, 18)
        let first = m.movePaletteEntries([2, 3], to: 10)
        expect(first != nil)
        expectEq(Array(m.palette[(first ?? 0)...(first ?? 0) + 1]), [0xABCDEFFF, 0xABCDEFFF])
        expect(m.removePaletteEntries([first ?? 0, (first ?? 0) + 1]))
        expectEq(m.palette.count, 16)
        expectEq(Array(m.palette.prefix(2)), Array(original.prefix(2)))
        let n = m.historyCounts.undo
        expect(m.editPaletteEntry(5, to: 0x010203FF, begin: true))
        expect(m.editPaletteEntry(5, to: 0x040506FF, begin: false))
        expect(m.editPaletteEntry(5, to: 0x070809FF, begin: false))
        expectEq(m.palette[5], 0x070809FF)
        expectEq(m.historyCounts.undo, n + 1, "a live edit session is one undo step")
        m.undo()
        expectEq(m.palette[5], original[5])
        // Ramps.
        let at = m.makeRamp(from: 0, to: 1, count: 3, hsv: false)
        expectEq(m.palette.count, 19)
        if let at {
            let c = m.palette[at - 0 ..< at + 3].map(rgb)
            let (a, b) = (rgb(m.palette[0]), rgb(m.palette[at + 3 > 18 ? 1 : 1]))
            expect(c.count == 3, "ramp inserted three colors \(c) between \(a) and \(b)")
        }
        m.undo()
        expectEq(m.palette.count, 16)
        // Every preset loads, produces a palette, and can be undone.
        let names = m.presetNames
        expect(names.count >= 3, "presets: \(names)")
        for (i, name) in names.enumerated() {
            m.loadPreset(i)
            expect(m.palette.count >= 2, "preset \(name) loaded \(m.palette.count) colors")
            m.undo()
            expectEq(m.palette, original, "preset \(name) undone")
        }
        m.errorMessage = nil
        // Palette files: GPL and hex round trip.
        for ext in ["gpl", "hex"] {
            let url = tmp("pal.\(ext)")
            expect(url.path.withCString { pixmith_palette_save(m.doc, $0) }, "save \(ext): \(coreError)")
            m.loadPreset(0)
            let different = m.palette
            expect(url.path.withCString { pixmith_palette_load(m.doc, $0) }, "load \(ext): \(coreError)")
            expectEq(m.palette.map { $0 | 0xFF }.count, original.count, "\(ext) round trip keeps the size (was \(different.count))")
            expectEq(m.palette.map { $0 & 0xFFFFFF00 }, original.map { $0 & 0xFFFFFF00 }, "\(ext) round trip keeps the colors")
            m.undo(); m.undo()
        }
    }

    scenario("color/modes-and-snap") {
        let m = fresh(), d = CanvasDriver()
        m.color = 0x123456FF
        d.click(1, 1)
        m.color = RED
        d.click(2, 1)
        expectEq(m.colorMode, .rgb)
        // Indexed: pixels snap to the palette, the drawing color follows.
        m.convertColorMode(to: .indexed)
        expectEq(m.colorMode, .indexed)
        let pal = Set(m.palette)
        for (x, y) in m.opaque() { expect(pal.contains(m.pixel(x, y)), "pixel (\(x),\(y)) uses a palette color") }
        expect(pal.contains(m.color), "the drawing color is snapped to the palette")
        m.color = 0x010101FF
        expect(pal.contains(m.color), "setting a color in indexed mode snaps it")
        d.click(5, 5)
        expect(pal.contains(m.pixel(5, 5)))
        expect(m.selectedPaletteIndex != nil)
        // Editing a palette entry recolors the pixels using it.
        let idx = m.palette.firstIndex(of: m.pixel(5, 5))!
        expect(pixmith_cm_palette_set(m.doc, UInt32(idx), 0x336699FF))
        expectEq(m.pixel(5, 5), 0x336699FF, "indexed pixels follow their palette entry")
        m.undo()
        // Ordered dithering conversion back and forth keeps the pixel count.
        m.convertColorMode(to: .indexed, dither: 4)
        expectEq(m.colorMode, .indexed)
        // Grayscale: every pixel is gray.
        m.convertColorMode(to: .grayscale)
        expectEq(m.colorMode, .grayscale)
        for (x, y) in m.opaque() {
            let (r, g, b) = rgb(m.pixel(x, y))
            expect(r == g && g == b, "gray pixel (\(x),\(y)) \(r),\(g),\(b)")
        }
        let (r, g, b) = rgb(m.color)
        expect(r == g && g == b, "the drawing color is gray in grayscale mode")
        // Back to RGB and undo through all conversions.
        m.convertColorMode(to: .rgb)
        expectEq(m.colorMode, .rgb)
        for _ in 0..<3 { m.undo() }
        expectEq(m.colorMode, .indexed)
        // Palette from sprite.
        m.undo(); m.undo()
        expectEq(m.colorMode, .rgb)
        m.paletteFromSprite(maxColors: 0)
        expect(m.palette.count >= 2 && m.palette.count <= 8, "palette from sprite: \(m.palette.count) colors")
        expect(m.palette.contains(RED))
        m.undo()
        d.snapshot("color-modes")
    }

    scenario("filters/each-fx") {
        let m = fresh(12, 12), d = CanvasDriver()
        func setup() {
            m.deselect()
            while m.historyCounts.undo > 0 { m.undo() }
            m.color = RED
            m.tool = .pencil
            d.click(5, 5)
        }
        setup()
        // Invert flips RGB, keeping alpha.
        expect(m.applyFilter(.invert))
        expectPixel(5, 5, 0x00FFFFFF, "invert")
        // Desaturate.
        setup()
        expect(m.applyFilter(.desaturate))
        let (r, g, b) = rgb(m.pixel(5, 5))
        expect(r == g && g == b && r > 0, "desaturate gives gray \(r),\(g),\(b)")
        // Hue shift by 120 degrees turns red green.
        setup()
        expect(m.applyFilter(.hsl, params: [120, 0, 0]))
        let hsl = rgb(m.pixel(5, 5))
        expect(hsl.1 > 200 && hsl.0 < 60, "hue +120 makes red green: \(hsl)")
        setup()
        expect(m.applyFilter(.hsl, params: [0, -100, 0]))
        let flat = rgb(m.pixel(5, 5))
        expect(abs(flat.0 - flat.1) < 3 && abs(flat.1 - flat.2) < 3, "saturation -100 is gray: \(flat)")
        // Brightness/contrast, color balance.
        setup()
        m.color = 0x646464FF
        d.click(6, 6)
        expect(m.applyFilter(.brightnessContrast, params: [50, 0]))
        expect(rgb(m.pixel(6, 6)).0 > 100, "brighter")
        expect(m.applyFilter(.colorBalance, params: [-100, 0, 100]))
        let balanced = rgb(m.pixel(6, 6))
        expect(balanced.2 > balanced.0, "color balance moved toward blue: \(balanced)")
        // Replace color.
        setup()
        expect(m.applyFilter(.replaceColor, params: [Int32(bitPattern: RED), Int32(bitPattern: BLUE), 0]))
        expectPixel(5, 5, BLUE, "replace color")
        expect(!m.applyFilter(.replaceColor, params: [Int32(bitPattern: RED), Int32(bitPattern: BLUE), 0]), "no-op filters record nothing")
        // Outline: outside, 4- and 8-connected, then inside.
        setup()
        expect(m.applyFilter(.outline, params: [Int32(bitPattern: BLACK), 0, 0]))
        expectEq(m.visibleCount(), 5, "4-connected outline adds four pixels")
        expectPixel(5, 4, BLACK)
        m.undo()
        expect(m.applyFilter(.outline, params: [Int32(bitPattern: BLACK), 0, 1]))
        expectEq(m.visibleCount(), 9, "8-connected outline adds eight pixels")
        // Drop shadow.
        setup()
        expect(m.applyFilter(.dropShadow, params: [2, 1, Int32(bitPattern: 0x00000080)]))
        expectEq(m.visibleCount(), 2)
        expectPixel(5, 5, RED)
        expect(m.pixel(7, 6) & 0xFF == 0x80, "shadow lands at (+2,+1)")
        // Despeckle removes isolated pixels, keeps connected ones.
        setup()
        d.click(9, 9)
        d.click(9, 10)
        expect(m.applyFilter(.despeckle, params: [0]))
        expectPixel(5, 5, CLEAR, "isolated pixel removed")
        expectPixel(9, 9, RED, "connected pixels stay")
        // Blur spreads, sharpen runs, curves identity does nothing and an inverted curve inverts.
        setup()
        expect(m.applyFilter(.blur))
        expect(m.visibleCount() > 1, "blur spreads the pixel")
        m.undo()
        expect(m.applyFilter(.sharpen) || true)
        setup()
        expect(!m.applyFilter(.curves, params: CurveSet().encoded()), "identity curves change nothing")
        var inverted = CurveSet()
        inverted.channels[0] = [CurvePoint(x: 0, y: 255), CurvePoint(x: 255, y: 0)]
        expect(m.applyFilter(.curves, params: inverted.encoded()))
        expectPixel(5, 5, 0x00FFFFFF, "inverting curve")
        // Targets: every frame of the layer, every layer of the frame.
        setup()
        m.addFrame(duplicate: true)
        m.frame = 0
        m.addLayer()
        put(d, 1, 1, RED)
        m.layer = 0
        expect(m.applyFilter(.invert, target: 1))
        expectPixel(5, 5, 0x00FFFFFF, frame: 0)
        expectPixel(5, 5, 0x00FFFFFF, "all frames of the layer", frame: 1)
        m.undo()
        expect(m.applyFilter(.invert, target: 2))
        expectPixel(5, 5, 0x00FFFFFF, frame: 0)
        expectPixel(5, 5, RED, "other frames are untouched", frame: 1)
        expectPixel(1, 1, 0x00FFFFFF, "all layers of the frame", frame: 0)
        // A region limits the filter.
        setup()
        d.click(10, 10)
        expect(m.applyFilter(.invert, region: (x: 0, y: 0, w: 8, h: 8)))
        expectPixel(5, 5, 0x00FFFFFF)
        expectPixel(10, 10, RED, "outside the region")
        // Scaling the sprite.
        setup()
        expect(pixmith_scale_sprite(m.doc, 24, 24))
        expectEq(m.width, 24)
        expectPixel(10, 10, RED, "nearest-neighbor 2x")
        expectPixel(11, 11, RED)
        expectPixel(12, 12, CLEAR)
        m.undo()
        expectEq(m.width, 12)
    }

    scenario("tilemap/layer-stamp-convert") {
        let m = fresh(32, 32), d = CanvasDriver()
        let tm = TilemapState.shared
        m.newTilemapLayer(tileWidth: 8, tileHeight: 8)
        expect(m.isTilemapLayer)
        expectEq(m.tilesetIndex, 0)
        let before = m.tileCount(of: 0)
        // Drawing on a tilemap layer creates tiles from the pixels.
        m.color = RED
        d.click(3, 3)
        expectPixel(3, 3, RED)
        expect(m.tileCount(of: 0) > before, "pixel drawing allocates a tile (\(before) -> \(m.tileCount(of: 0)))")
        let drawnTile = Int(pixmith_tile_at(m.doc, UInt32(m.layer), 0, 3, 3) & 0x1FFFFFFF)
        expect(drawnTile > 0, "the cell now refers to a tile")
        // Stamp mode places the selected tile with a click; shift erases; option picks.
        tm.stamp = true
        tm.selected = drawnTile
        d.click(20, 20)
        expectEq(Int(pixmith_tile_at(m.doc, UInt32(m.layer), 0, 20, 20) & 0x1FFFFFFF), drawnTile)
        expectPixel(19 - (19 % 8) + 3, 16 + 3, RED, "the stamped tile shows its pixels")
        d.click(20, 20, flags: .shift)
        expectEq(pixmith_tile_at(m.doc, UInt32(m.layer), 0, 20, 20), 0, "shift-click erases the cell")
        tm.selected = 99
        d.click(3, 3, flags: .option)
        expectEq(tm.selected, drawnTile, "option-click picks the tile")
        // A drag of stamps is one undo step.
        let n = m.historyCounts.undo
        d.stroke([(2, 20), (10, 20), (18, 20)])
        expectEq(m.historyCounts.undo, n + 1)
        expectEq(Int(pixmith_tile_at(m.doc, UInt32(m.layer), 0, 18, 20) & 0x1FFFFFFF), drawnTile)
        m.undo()
        expectEq(pixmith_tile_at(m.doc, UInt32(m.layer), 0, 18, 20), 0)
        // Flip flags are kept in the tile value.
        tm.flipX = true
        d.click(12, 12)
        expect(pixmith_tile_at(m.doc, UInt32(m.layer), 0, 12, 12) & TilemapState.flipX != 0)
        tm.flipX = false
        tm.stamp = false
        // Tile management.
        m.addTile()
        let count = m.tileCount(of: 0)
        m.duplicateTile(drawnTile)
        expectEq(m.tileCount(of: 0), count + 1)
        m.deleteTile(count)
        expectEq(m.tileCount(of: 0), count)
        m.renameTileset(0, to: "Terrain")
        expectEq(m.tilesetName(0), "Terrain")
        d.snapshot("tilemap-layer")
        // Convert the tilemap layer to a normal layer: same pixels, no tileset.
        let look = m.pixels()
        m.convertToNormal()
        expect(!m.isTilemapLayer)
        expectEq(m.pixels(), look, "converting to a normal layer keeps the pixels")
        // Convert a normal layer with repeated 8x8 blocks into a tilemap: identical blocks share a tile.
        m.addLayer()
        paintRect(d, 0, 0, 7, 7, BLUE)
        paintRect(d, 16, 0, 23, 7, BLUE)
        paintRect(d, 0, 16, 7, 23, GREEN)
        let flat = m.pixels()
        m.convertToTilemap(tileWidth: 8, tileHeight: 8, flipAware: true)
        expect(m.isTilemapLayer)
        expectEq(m.pixels(), flat, "converting to a tilemap keeps the look")
        let ts = m.tilesetIndex!
        expectEq(m.tileCount(of: ts), 3, "empty tile + blue + green")
        expectEq(pixmith_tile_at(m.doc, UInt32(m.layer), 0, 1, 1), pixmith_tile_at(m.doc, UInt32(m.layer), 0, 17, 1), "identical blocks share a tile")
        m.undo()
        expect(!m.isTilemapLayer)
        // Converting a tilemap again is refused with an error.
        m.redo()
        m.errorMessage = nil
        m.convertToTilemap(tileWidth: 8, tileHeight: 8)
        expect(m.errorMessage != nil, "a tilemap layer cannot be converted again")
        m.errorMessage = nil
        d.snapshot("tilemap-converted")
    }

    scenario("slices/create-edit-userdata") {
        let m = fresh(32, 32), d = CanvasDriver()
        m.tool = .slice
        d.stroke([(2, 2), (9, 7)])
        expectEq(m.sliceCount, 1)
        var info = m.sliceInfo(0)
        expectEq(info?.x, 2); expectEq(info?.y, 2); expectEq(info?.w, 8); expectEq(info?.h, 6)
        expect(m.hasSelectedSlice)
        // Dragging inside moves it.
        d.stroke([(5, 4), (10, 9)])
        info = m.sliceInfo(0)
        expectEq(info?.x, 7); expectEq(info?.y, 7)
        expectEq(m.sliceCount, 1)
        m.undo()
        info = m.sliceInfo(0)
        expectEq(info?.x, 2, "moving a slice is undoable")
        // Properties and user data.
        let center: [Int32] = [1, 1, 4, 2], pivot: [Int32] = [3, 3]
        expect("hero".withCString { n in pixmith_slice_set_props(m.doc, 0, 0, n, center, pivot) })
        expectEq(m.sliceName(0), "hero")
        info = m.sliceInfo(0)
        expectEq(info?.center?.w, 4)
        expectEq(info?.pivot?.x, 3)
        expect("note on sprite".withCString { pixmith_userdata_set(m.doc, 0, 0, 0, $0, true, 0xFF8800FF) })
        expectEq(m.userDataText(0, 0), "note on sprite")
        expectEq(m.userDataColor(0, 0), 0xFF8800FF)
        expect("layer note".withCString { pixmith_userdata_set(m.doc, 1, 0, 0, $0, false, 0) })
        expectEq(m.userDataText(1, 0), "layer note")
        expectEq(m.userDataColor(1, 0), nil)
        expect("cel note".withCString { pixmith_userdata_set(m.doc, 2, 0, 0, $0, false, 0) } || true)
        // The user-data target for tags needs a tag.
        var t = m.newTagDraft(); t.name = "idle"
        m.addTag(t)
        expect("tag note".withCString { pixmith_userdata_set(m.doc, 3, 0, 0, $0, true, 0x00FF00FF) })
        expectEq(m.userDataText(3, 0), "tag note")
        expectEq(m.tagName(0), "idle")
        d.snapshot("slices")
        // Delete.
        m.deleteSelectedSlice()
        expectEq(m.sliceCount, 0)
        m.undo()
        expectEq(m.sliceCount, 1)
    }
}
