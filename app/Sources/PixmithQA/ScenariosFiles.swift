import AppKit
import ImageIO
@testable import PixmithKit
import PixmithCore

// MARK: - Whole-document snapshots

/// Everything a save/open round trip has to preserve, read through the model the way the UI reads it.
struct DocSnap {
    var width = 0, height = 0, mode: UInt32 = 0
    var layers: [String] = []
    var durations: [Int] = []
    var frames: [[UInt8]] = []
    var tags: [String] = []
    var palette: [UInt32] = []
    var slices: [String] = []
    var userData: [String] = []
    var linked: [String] = []
    var tilesets: [String] = []

    init(_ m: SpriteModel) {
        width = m.width; height = m.height; mode = pixmith_cm_get_mode(m.doc)
        for i in 0..<m.layerCount {
            layers.append("\(m.layerName(i))|v\(m.isVisible(i))|g\(m.isGroup(i))|l\(m.level(i))|o\(m.rawOpacity(i))|b\(m.blendMode(i))|t\(pixmith_layer_is_tilemap(m.doc, UInt32(i)))")
        }
        durations = (0..<m.frameCount).map(m.duration)
        frames = (0..<m.frameCount).map { m.render(frame: $0) }
        tags = (0..<m.tagCount).map { i in m.tag(i).map { "\($0.name)|\($0.from)-\($0.to)|d\($0.direction)|r\($0.repeatCount)|c\(String($0.color, radix: 16))" } ?? "?" }
        palette = m.palette
        for i in 0..<m.sliceCount {
            let s = m.sliceInfo(i)
            slices.append("\(m.sliceName(i))|\(s?.x ?? -1),\(s?.y ?? -1),\(s?.w ?? -1),\(s?.h ?? -1)|c\(String(describing: s?.center))|p\(String(describing: s?.pivot))")
        }
        userData.append("sprite:\(m.userDataText(0, 0))|\(String(describing: m.userDataColor(0, 0)))")
        for l in 0..<m.layerCount { userData.append("layer\(l):\(m.userDataText(1, l))") }
        for t in 0..<Int(pixmith_ud_tag_count(m.doc)) { userData.append("tag\(t):\(m.userDataText(3, t))|\(String(describing: m.userDataColor(3, t)))") }
        for l in 0..<m.layerCount where !m.isGroup(l) {
            for f in 0..<m.frameCount where pixmith_cel_is_linked(m.doc, UInt32(l), UInt32(f)) { linked.append("\(l).\(f)") }
        }
        for ts in 0..<m.tilesetCount { tilesets.append("\(m.tilesetName(ts))|\(m.tileSize(of: ts))|\(m.tileCount(of: ts))") }
    }
}

func expectSame(_ a: DocSnap, _ b: DocSnap, _ what: String, file: String = #fileID, line: UInt = #line) {
    expectEq(a.width, b.width, "\(what): width", file: file, line: line)
    expectEq(a.height, b.height, "\(what): height", file: file, line: line)
    expectEq(a.mode, b.mode, "\(what): color mode", file: file, line: line)
    expectEq(a.layers, b.layers, "\(what): layers", file: file, line: line)
    expectEq(a.durations, b.durations, "\(what): durations", file: file, line: line)
    expectEq(a.frames.count, b.frames.count, "\(what): frame count", file: file, line: line)
    for (i, (x, y)) in zip(a.frames, b.frames).enumerated() where x != y {
        let diff = zip(x, y).enumerated().filter { $0.element.0 != $0.element.1 }.count
        QA.fail("\(what): frame \(i + 1) differs in \(diff) bytes", file, line)
    }
    expectEq(a.tags, b.tags, "\(what): tags", file: file, line: line)
    expectEq(a.palette, b.palette, "\(what): palette", file: file, line: line)
    expectEq(a.slices, b.slices, "\(what): slices", file: file, line: line)
    expectEq(a.userData, b.userData, "\(what): user data", file: file, line: line)
    expectEq(a.linked, b.linked, "\(what): linked cels", file: file, line: line)
    expectEq(a.tilesets, b.tilesets, "\(what): tilesets", file: file, line: line)
}

/// A document touching most of the file format.
func buildRichDocument(_ d: CanvasDriver) {
    let m = fresh(16, 12)
    m.color = RED
    d.stroke([(1, 1), (6, 1), (6, 5)])
    paintRect(d, 8, 2, 12, 5, GREEN)
    m.addLayer()
    m.color = BLUE
    d.stroke([(2, 8), (10, 9)])
    put(d, 3, 3, 0x80808080)
    m.setLayerProps(1, name: "Details", opacity: 200, blend: 1, visible: true)
    m.addLayer()
    put(d, 14, 10, 0xFFFF00FF)
    m.wrapInGroup(2)
    m.setLayerProps(2, name: "Group", opacity: 180, blend: 0, visible: true)
    m.addFrame(duplicate: true)
    m.layer = 0
    put(d, 0, 11, WHITE)
    m.addFrame(duplicate: false)
    put(d, 5, 5, GREEN)
    m.layer = 0
    m.linkCels(from: 0, to: 2)
    m.frame = 0
    m.setDuration(80)
    m.frame = 1
    m.setDuration(120)
    var t = m.newTagDraft()
    t.name = "loop"; t.from = 0; t.to = 1; t.direction = 2; t.repeatCount = 3; t.color = 0xAA5500
    m.addTag(t)
    t.name = "tail"; t.from = 2; t.to = 2; t.direction = 1; t.repeatCount = 0; t.color = 0x0055AA
    m.addTag(t)
    m.tool = .slice
    d.stroke([(2, 2), (9, 6)])
    "hero".withCString { _ = pixmith_slice_set_props(m.doc, 0, 0, $0, [1, 1, 3, 2] as [Int32], [2, 2] as [Int32]) }
    "sprite note".withCString { _ = pixmith_userdata_set(m.doc, 0, 0, 0, $0, true, 0xCC00CCFF) }
    "layer note".withCString { _ = pixmith_userdata_set(m.doc, 1, 1, 0, $0, false, 0) }
    "tag note".withCString { _ = pixmith_userdata_set(m.doc, 3, 0, 0, $0, true, 0x112233FF) }
    m.tool = .pencil
    m.frame = 0
}

func registerHistory() {
    scenario("history/undo-redo-labels-goto") {
        let m = fresh(), d = CanvasDriver()
        expectEq(m.historyCounts.undo, 0)
        m.color = RED
        d.click(1, 1)                        // Draw
        m.tool = .eraser
        d.click(1, 1)                        // Erase
        m.tool = .fill
        d.click(5, 5)                        // Fill
        m.tool = .rectangle
        d.stroke([(2, 2), (4, 4)])           // Rectangle
        m.addLayer()                         // Add Layer
        m.addFrame(duplicate: true)          // Duplicate Frame
        let n = m.historyCounts.undo
        expectEq(n, 6)
        let labels = (0..<n).map(m.historyLabel)
        expectEq(labels, ["Draw", "Erase", "Fill", "Rectangle", "Add Layer", "Duplicate Frame"])
        // Walk back through every state, comparing with the recorded composites.
        var states: [[UInt32]] = []
        for _ in 0..<n { states.append(m.pixels(frame: 0)); m.undo() }
        expectEq(m.historyCounts.undo, 0)
        expectEq(m.historyCounts.redo, n)
        expectEq(m.visibleCount(), 0, "original document")
        m.goToHistory(3)
        expectEq(m.historyCounts.undo, 3)
        expectEq(m.pixels(frame: 0), states[n - 3], "state after three steps")
        m.goToHistory(n)
        expectEq(m.layerCount, 2)
        expectEq(m.frameCount, 2)
        m.goToHistory(1)
        expectEq(m.layerCount, 1)
        expectEq(m.frameCount, 1)
        expectEq(m.visibleCount(), 1)
        // A new edit after undoing drops the redo steps.
        m.tool = .pencil
        d.click(9, 9)
        expectEq(m.historyCounts.redo, 0)
        expectEq(m.historyLabel(1), "Draw")
    }

    scenario("history/dirty-tracking") {
        let m = fresh(), d = CanvasDriver()
        expect(!m.isDirty, "a new sprite is clean")
        m.color = RED
        d.click(1, 1)
        expect(m.isDirty)
        expect(m.windowTitle.contains("Edited"), "title shows the edited state: \(m.windowTitle)")
        m.undo()
        expect(!m.isDirty, "undoing back to the saved state is clean again")
        m.redo()
        expect(m.isDirty)
        // Saving makes it clean; further edits and undo to the saved state are tracked.
        let url = tmp("dirty.aseprite")
        m.fileURL = url
        m.save()
        expect(!m.isDirty, "saved")
        expect(FileManager.default.fileExists(atPath: url.path))
        expect(!m.windowTitle.contains("Edited"))
        d.click(2, 2)
        expect(m.isDirty)
        m.undo()
        expect(!m.isDirty, "undo to the saved state")
        m.undo()
        expect(m.isDirty, "undoing past the saved state is dirty")
        m.redo()
        expect(!m.isDirty)
        // Edits outside the undo stack still count.
        m.setVisible(0, false)
        expect(m.isDirty, "toggling visibility dirties the document")
        m.setVisible(0, true)
        // Discard confirmation: Cancel keeps the document, Don't Save lets it go, Save writes it.
        d.click(3, 3)
        answerModal("Cancel") { expect(!m.confirmDiscard(), "cancel keeps the document") }
        answerModal("Don’t Save") { expect(m.confirmDiscard(), "don't save discards") }
        answerModal("Save") { expect(m.confirmDiscard(), "save then discard") }
        expect(!m.isDirty, "Save wrote the file")
        let reopened = SpriteModel.shared
        reopened.open(url)
        expectPixel(3, 3, RED, "the saved file has the last edit")
        expect(!reopened.isDirty)
        expectEq(reopened.fileURL, url)
        // Recent files remember it.
        expect(RecentFiles.shared.urls.contains(url))
    }

    scenario("history/transactions") {
        let m = fresh(), d = CanvasDriver()
        let n = m.historyCounts.undo
        m.inTransaction {
            put(d, 1, 1, RED)
            put(d, 2, 2, GREEN)
            put(d, 3, 3, BLUE)
        }
        expectEq(m.historyCounts.undo, n + 1, "a transaction is one undo step")
        expectEq(m.visibleCount(), 3)
        m.undo()
        expectEq(m.visibleCount(), 0)
    }
}

func registerFiles() {
    for (name, prepare) in [
        ("rgba", { (_: SpriteModel) in }),
        ("indexed", { (m: SpriteModel) in m.convertColorMode(to: .indexed) }),
        ("grayscale", { (m: SpriteModel) in m.convertColorMode(to: .grayscale) }),
    ] as [(String, (SpriteModel) -> Void)] {
        scenario("files/roundtrip-\(name)") {
            let d = CanvasDriver()
            buildRichDocument(d)
            let m = SpriteModel.shared
            if name != "rgba" {
                // Colors that survive a palette mapping: convert, then draw only palette colors.
                prepare(m)
                expectEq(UInt32(m.colorMode.rawValue), name == "indexed" ? 2 : 1)
            }
            let before = DocSnap(m)
            let url = tmp("rich-\(name).aseprite")
            m.fileURL = url
            m.save()
            expect(FileManager.default.fileExists(atPath: url.path), "file written: \(coreError)")
            fresh()
            m.open(url)
            expectSame(before, DocSnap(m), name)
            // Saving the reopened document again gives the same content.
            let url2 = tmp("rich-\(name)-2.aseprite")
            m.fileURL = url2
            m.save()
            fresh()
            m.open(url2)
            expectSame(before, DocSnap(m), "\(name) after a second round trip")
            d.snapshot("files-roundtrip-\(name)")
        }
    }

    scenario("files/roundtrip-tilemap") {
        let m = fresh(32, 32), d = CanvasDriver()
        put(d, 1, 1, RED)
        m.newTilemapLayer(tileWidth: 8, tileHeight: 8)
        m.color = BLUE
        d.click(3, 3)
        d.click(11, 19)
        let tm = TilemapState.shared
        tm.stamp = true
        tm.selected = Int(pixmith_tile_at(m.doc, UInt32(m.layer), 0, 3, 3) & 0x1FFFFFFF)
        d.click(24, 24)
        tm.flipX = true
        d.click(25, 4)
        tm.flipX = false
        tm.stamp = false
        m.renameTileset(0, to: "Ground")
        m.addFrame(duplicate: true)
        m.frame = 0
        let before = DocSnap(m)
        expectEq(before.tilesets.count, 1)
        let tileValues = (0..<4).flatMap { y in (0..<4).map { x in pixmith_tile_at(m.doc, UInt32(m.layer), 0, Int32(x * 8), Int32(y * 8)) } }
        let url = tmp("tiles.aseprite")
        m.fileURL = url
        m.save()
        fresh()
        m.open(url)
        m.layer = 1
        expect(m.isTilemapLayer, "the layer is still a tilemap layer")
        expectSame(before, DocSnap(m), "tilemap")
        let again = (0..<4).flatMap { y in (0..<4).map { x in pixmith_tile_at(m.doc, UInt32(m.layer), 0, Int32(x * 8), Int32(y * 8)) } }
        expectEq(again, tileValues, "tile grid including flip flags")
        // A tilemap sprite saved to the RGBA fallback still opens.
        d.snapshot("files-roundtrip-tilemap")
    }

    scenario("files/export-png-sheet-gif-apng-sequence") {
        let m = fresh(8, 6), d = CanvasDriver()
        put(d, 1, 1, RED)
        put(d, 6, 4, BLUE)
        m.addFrame(duplicate: true)
        put(d, 3, 3, GREEN)
        m.addFrame(duplicate: true)
        put(d, 4, 2, 0xFFFF00FF)
        let frames = (0..<3).map { m.pixels(frame: $0) }
        func importPNG(_ url: URL) -> OpaquePointer? { url.path.withCString { pixmith_import_png($0) } }
        // Single frame, 1x and scaled.
        for (scale, frame) in [(1, 0), (4, 2)] {
            let url = tmp("frame\(scale).png")
            expect(url.path.withCString { pixmith_export_png(m.doc, UInt32(frame), UInt32(scale), $0) }, "export png: \(coreError)")
            guard let back = importPNG(url) else { QA.fail("import png: \(coreError)", #fileID, #line); continue }
            expectEq(Int(pixmith_width(back)), 8 * scale)
            expectEq(Int(pixmith_height(back)), 6 * scale)
            let p = renderPixels(back, frame: 0)
            var same = true
            for y in 0..<6 * scale { for x in 0..<8 * scale where p[y * 8 * scale + x] != frames[frame][(y / scale) * 8 + x / scale] { same = false } }
            expect(same, "scaled PNG \(scale)x matches frame \(frame + 1)")
            pixmith_free(back)
        }
        // The rest needs the original sprite again.
        func rebuild() {
            let m = fresh(8, 6)
            put(d, 1, 1, RED); put(d, 6, 4, BLUE)
            m.addFrame(duplicate: true); put(d, 3, 3, GREEN)
            m.addFrame(duplicate: true); put(d, 4, 2, 0xFFFF00FF)
        }
        rebuild()
        // Sprite sheet: horizontal, vertical, grid, padded, with JSON.
        for (layout, columns, padding) in [(0, 0, 0), (1, 0, 0), (2, 2, 0), (0, 0, 2), (2, 2, 3)] {
            let png = tmp("sheet\(layout)\(padding).png")
            expect(png.path.withCString { pixmith_export_sheet_ex(m.doc, UInt32(layout), UInt32(columns), UInt32(padding), 1, true, $0) }, "export sheet: \(coreError)")
            let json = png.deletingPathExtension().appendingPathExtension("json")
            expect(FileManager.default.fileExists(atPath: json.path), "sheet JSON written next to the PNG")
            let doc = json.path.withCString { j in png.path.withCString { pixmith_import_sheet_json(j, $0) } }
            expect(doc != nil, "import sheet JSON (layout \(layout), padding \(padding)): \(coreError)")
            if let doc {
                expectEq(Int(pixmith_frame_count(doc)), 3, "sheet frames")
                for f in 0..<3 {
                    expectEq(renderPixels(doc, frame: f), frames[f], "sheet layout \(layout) padding \(padding) frame \(f + 1)")
                }
                pixmith_free(doc)
            }
        }
        // Sheet PNG imported as frames by cell size (horizontal strip, no padding).
        let strip = tmp("sheet00.png")
        let asFrames = strip.path.withCString { pixmith_import_sheet($0, 8, 6) }
        expect(asFrames != nil, "import sheet as frames: \(coreError)")
        if let asFrames {
            expectEq(Int(pixmith_frame_count(asFrames)), 3)
            pixmith_free(asFrames)
        }
        // GIF: the animation round-trips (colors are exact for a small palette).
        let gif = tmp("anim.gif")
        expect(gif.path.withCString { pixmith_export_gif(m.doc, 1, $0) }, "export gif: \(coreError)")
        let g = gif.path.withCString { pixmith_import_gif($0) }
        expect(g != nil, "import gif: \(coreError)")
        if let g {
            expectEq(Int(pixmith_frame_count(g)), 3, "gif frames")
            for f in 0..<3 { expectEq(renderPixels(g, frame: f), frames[f], "gif frame \(f + 1)") }
            pixmith_free(g)
        }
        // APNG: readable by ImageIO with the right frame count and first frame.
        let apng = tmp("anim.png")
        expect(apng.path.withCString { pixmith_export_apng(m.doc, 2, $0) }, "export apng: \(coreError)")
        if let src = CGImageSourceCreateWithURL(apng as CFURL, nil) {
            expectEq(CGImageSourceGetCount(src), 3, "apng frames")
            if let img = CGImageSourceCreateImageAtIndex(src, 0, nil) {
                expectEq(img.width, 16)
                expectEq(img.height, 12)
            } else { QA.fail("apng first frame unreadable", #fileID, #line) }
        } else { QA.fail("apng unreadable by ImageIO", #fileID, #line) }
        let sig: [UInt8] = Array((try? Data(contentsOf: apng))?.prefix(8) ?? Data())
        expectEq(sig, [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        // PNG sequence: name-001.png ... each frame equals the render.
        let base = tmp("seq")
        expect(base.path.withCString { pixmith_export_png_sequence(m.doc, 1, $0) }, "export sequence: \(coreError)")
        for f in 0..<3 {
            let url = tmp(String(format: "seq-%03d.png", f + 1))
            expect(FileManager.default.fileExists(atPath: url.path), "sequence file \(f + 1)")
            if let back = importPNG(url) {
                expectEq(renderPixels(back, frame: 0), frames[f], "sequence frame \(f + 1)")
                pixmith_free(back)
            } else { QA.fail("import sequence frame \(f + 1): \(coreError)", #fileID, #line) }
        }
        // The File > Import path replaces the document and counts as unsaved.
        m.importPNG(tmp("frame1.png"))
        expectEq(m.width, 8)
        expectEq(m.pixels(), frames[0])
        expect(m.isDirty, "an imported sprite is unsaved")
        expect(m.fileURL == nil)
        // Import a PNG as a new layer on top of a document.
        rebuild()
        let layerPNG = tmp("frame1.png")
        let idx = layerPNG.path.withCString { pixmith_import_png_layer(m.doc, 0, 0, $0) }
        expect(idx >= 0, "import PNG as layer: \(coreError)")
        expectEq(m.layerCount, 2)
        m.changed()
        m.undo()
        expectEq(m.layerCount, 1, "importing a layer is undoable")
        // Sheet import with a bad file reports an error instead of crashing.
        let bad = tmp("bad.png")
        try? Data("not a png".utf8).write(to: bad)
        expect(bad.path.withCString { pixmith_import_png($0) } == nil, "bad PNG is rejected")
        expect(!coreError.isEmpty, "an error message is available")
    }

    scenario("files/open-errors-and-recovery") {
        let m = fresh()
        // Opening garbage fails with a message and keeps the current document.
        let bad = tmp("garbage.aseprite")
        try? Data(repeating: 7, count: 300).write(to: bad)
        m.color = RED
        let d = CanvasDriver()
        d.click(1, 1)
        m.errorMessage = nil
        m.open(bad)
        expect(m.errorMessage != nil, "an error is reported")
        expectPixel(1, 1, RED, "the document is untouched")
        m.errorMessage = nil
        m.open(tmp("missing.aseprite"))
        expect(m.errorMessage != nil, "missing file reported")
        // Autosave writes a recovery copy while dirty and removes it once clean (the write is asynchronous).
        expect(m.isDirty)
        let dir = AppSupport.directory.appendingPathComponent("Pixmith/Recovery")
        expect(dir.path.hasPrefix(qaHome.path), "the QA run keeps recovery files out of the real Application Support: \(dir.path)")
        func recoveryFiles() -> [String] { (try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? [] }
        func waitFor(_ cond: () -> Bool) { for _ in 0..<100 where !cond() { pumpRunLoop(0.03) } }
        Autosaver.shared.start(m)
        Autosaver.shared.tick()
        waitFor { recoveryFiles().contains { $0.hasSuffix(".pxmt") } && recoveryFiles().contains { $0.hasSuffix(".json") } }
        expect(recoveryFiles().contains { $0.hasSuffix(".pxmt") }, "recovery copy written under \(dir.path): \(recoveryFiles())")
        expect(recoveryFiles().contains { $0.hasSuffix(".json") })
        m.undo()
        Autosaver.shared.tick()
        waitFor { !recoveryFiles().contains { $0.hasSuffix(".pxmt") } }
        expect(!recoveryFiles().contains { $0.hasSuffix(".pxmt") }, "recovery copy removed when the document is clean")
    }
}
