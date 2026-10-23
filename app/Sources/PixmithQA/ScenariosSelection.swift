import AppKit
@testable import PixmithKit
import PixmithCore

/// Paints a filled rectangle with the rectangle tool, then goes back to the previous tool.
func paintRect(_ d: CanvasDriver, _ x0: Int, _ y0: Int, _ x1: Int, _ y1: Int, _ color: UInt32) {
    let m = d.model
    let (tool, filled, c) = (m.tool, m.opts.filled, m.color)
    m.tool = .rectangle
    m.opts.filled = true
    m.color = color
    d.stroke([(x0, y0), (x1, y1)])
    m.tool = tool
    m.opts.filled = filled
    m.color = c
}

func put(_ d: CanvasDriver, _ x: Int, _ y: Int, _ color: UInt32) {
    let (tool, c) = (d.model.tool, d.model.color)
    d.model.tool = .pencil
    d.model.color = color
    d.click(x, y)
    d.model.tool = tool
    d.model.color = c
}

extension SpriteModel {
    func selected(_ x: Int, _ y: Int) -> Bool { pixmith_sel_contains(doc, Int32(x), Int32(y)) }

    var selectionBounds: [Int32]? {
        var b = [Int32](repeating: 0, count: 4)
        return pixmith_sel_bounds(doc, &b) ? b : nil
    }

    var selectedCount: Int {
        var n = 0
        for y in 0..<height { for x in 0..<width where selected(x, y) { n += 1 } }
        return n
    }
}

/// A solid-color sRGB PNG, as another app would put on the pasteboard.
func pngData(_ w: Int, _ h: Int, fill c: UInt32) -> Data {
    let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                        space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.setFillColor(c.cgColor)
    ctx.fill(CGRect(x: 0, y: 0, width: w, height: h))
    return NSBitmapImageRep(cgImage: ctx.makeImage()!).representation(using: .png, properties: [:])!
}

func registerSelection() {
    scenario("select/marquee-add-subtract") {
        let m = fresh(), d = CanvasDriver()
        m.tool = .marquee
        d.stroke([(2, 2), (9, 9)])
        expect(m.hasSelection)
        expectEq(m.selectionBounds ?? [], [2, 2, 8, 8])
        expect(m.selected(2, 2) && m.selected(9, 9) && !m.selected(10, 10) && !m.selected(1, 5))
        expectEq(m.selectedCount, 64)
        // Shift adds, option subtracts.
        d.stroke([(12, 12), (14, 14)], flags: .shift)
        expectEq(m.selectedCount, 64 + 9)
        d.stroke([(4, 4), (5, 5)], flags: .option)
        expectEq(m.selectedCount, 64 + 9 - 4)
        expect(!m.selected(4, 4) && m.selected(6, 6))
        // A plain click without dragging clears; Select All and Invert work.
        d.click(0, 0)
        expect(!m.hasSelection, "click deselects")
        m.selectAll()
        expectEq(m.selectedCount, 256)
        m.invertSelection()
        expect(!m.hasSelection || m.selectedCount == 0, "inverting select-all selects nothing")
        m.selectAll()
        m.deselect()
        expect(!m.hasSelection)
        // Each selection change is undoable.
        let n = m.historyCounts.undo
        d.stroke([(1, 1), (3, 3)])
        expectEq(m.historyCounts.undo, n + 1)
        m.undo()
        expect(!m.hasSelection)
        m.redo()
        expectEq(m.selectedCount, 9)
        d.snapshot("select-marquee")
    }

    scenario("select/wand") {
        let m = fresh(), d = CanvasDriver()
        paintRect(d, 4, 4, 7, 7, RED)
        paintRect(d, 10, 10, 12, 12, RED)
        put(d, 1, 1, 0xF40000FF) // nearly red
        m.tool = .wand
        let o = SelectionOptions.shared
        o.tolerance = 0
        o.contiguous = true
        d.click(5, 5)
        expectEq(m.selectedCount, 16, "contiguous wand picks one block")
        expectEq(m.selectionBounds ?? [], [4, 4, 4, 4])
        o.contiguous = false
        d.click(5, 5)
        expectEq(m.selectedCount, 16 + 9, "global wand picks every matching pixel")
        o.tolerance = 20
        d.click(5, 5)
        expectEq(m.selectedCount, 16 + 9 + 1, "tolerance includes the nearly-red pixel")
        // Transparent area, contiguous.
        o.tolerance = 0
        o.contiguous = true
        d.click(0, 0)
        expect(m.selected(0, 0) && !m.selected(5, 5) && !m.selected(1, 1))
        expectEq(m.selectedCount, 256 - 16 - 9 - 1)
        // Shift adds the red block to it.
        d.click(5, 5, flags: .shift)
        expect(m.selected(5, 5))
        d.click(5, 5, flags: .option)
        expect(!m.selected(5, 5))
        d.snapshot("select-wand")
    }

    scenario("select/lasso-polygon") {
        let m = fresh(), d = CanvasDriver()
        m.tool = .lasso
        d.stroke([(2, 2), (12, 2), (12, 12), (2, 2)])
        expect(m.selected(10, 4), "inside the lasso triangle")
        expect(!m.selected(3, 10), "outside the lasso triangle")
        let lasso = m.selectedCount
        expect(lasso > 30 && lasso < 80, "lasso area \(lasso)")
        m.deselect()
        m.tool = .polygon
        let nPoly = m.historyCounts.undo
        d.click(2, 2)
        d.click(12, 2)
        d.click(12, 12)
        d.click(2, 12)
        d.click(2, 12, clicks: 2)
        expect(m.selected(10, 4) && m.selected(4, 10) && m.selected(6, 6), "polygon selection covers the quad")
        expect(!m.selected(14, 14))
        expect(m.selectedCount > 80)
        expectEq(m.historyCounts.undo, nPoly + 1, "a polygon selection is one undo step")
        m.undo()
        expect(!m.hasSelection)
        d.snapshot("select-lasso-polygon")
    }

    scenario("select/modify-dialog") {
        let m = fresh(), d = CanvasDriver()
        m.tool = .marquee
        d.stroke([(5, 5), (8, 8)])
        expectEq(m.selectedCount, 16)
        answerModal("OK", fill: { $0.textFields.first?.stringValue = "2" }) { m.modifySelection(0) }
        expectEq(m.selectionBounds ?? [], [3, 3, 8, 8], "expand by 2")
        answerModal("OK", fill: { $0.textFields.first?.stringValue = "1" }) { m.modifySelection(1) }
        expectEq(m.selectionBounds ?? [], [4, 4, 6, 6], "contract by 1")
        answerModal("Cancel") { m.modifySelection(0) }
        expectEq(m.selectionBounds ?? [], [4, 4, 6, 6], "cancel changes nothing")
        answerModal("OK", fill: { $0.textFields.first?.stringValue = "1" }) { m.modifySelection(2) }
        expect(!m.selected(6, 6), "border selection is a ring")
        expect(m.selected(3, 3) && !m.selected(4, 4), "border of width 1 lies just outside the old edge")
    }

    scenario("select/move-and-nudge") {
        let m = fresh(), d = CanvasDriver()
        paintRect(d, 4, 4, 7, 7, RED)
        m.tool = .marquee
        d.stroke([(4, 4), (7, 7)])
        m.tool = .move
        let n = m.historyCounts.undo
        d.stroke([(5, 5), (9, 7)])
        expectEq(m.historyCounts.undo, n + 1, "a move is one undo step")
        expectEq(m.visibleCount(), 16)
        expectPixel(8, 6, RED)
        expectPixel(11, 9, RED)
        expectPixel(4, 4, CLEAR, "the old position is empty")
        expectEq(m.selectionBounds ?? [], [8, 6, 4, 4], "the selection follows")
        m.undo()
        expectPixel(4, 4, RED)
        expectPixel(8, 6, CLEAR)
        m.redo()
        // Arrow keys nudge (shift moves 10).
        d.key("\u{F703}", code: 124)
        expectPixel(9, 6, RED)
        expectPixel(8, 6, CLEAR)
        // Moving the whole selection out of the canvas clips it.
        m.deselect()
        d.snapshot("select-move")
    }

    scenario("select/copy-cut-paste") {
        let m = fresh(), d = CanvasDriver()
        paintRect(d, 4, 4, 9, 9, RED)
        put(d, 9, 9, BLUE)
        m.tool = .marquee
        d.stroke([(4, 4), (9, 9)])
        m.copySelection(cut: false)
        expect(m.canPaste)
        expectEq(m.visibleCount(), 36, "copy keeps the pixels")
        // Paste lands where it was copied from, as a floating selection on the move tool.
        m.deselect()
        m.pasteFromPasteboard()
        expect(m.tool == .move, "paste switches to the move tool")
        expect(pixmith_float_active(m.doc), "paste floats")
        expectPixel(4, 4, RED)
        expectPixel(9, 9, BLUE)
        // Drag the floating pixels, then commit with Enter.
        d.stroke([(6, 6), (12, 6)])
        expectPixel(10, 4, RED)
        expectPixel(15, 9, BLUE)
        expect(pixmith_float_active(m.doc), "still floating after the drag")
        d.enter()
        expect(!pixmith_float_active(m.doc), "Enter commits the floating paste")
        expectPixel(10, 4, RED)
        expectPixel(15, 9, BLUE)
        // Escape cancels a floating paste.
        m.deselect()
        m.pasteFromPasteboard()
        expect(pixmith_float_active(m.doc))
        d.escape()
        expect(!pixmith_float_active(m.doc), "Escape cancels")
        // Cut removes and copies.
        m.tool = .marquee
        d.stroke([(4, 4), (9, 9)])
        m.copySelection(cut: true)
        expectPixel(4, 4, CLEAR)
        expectPixel(9, 9, CLEAR)
        m.deselect()
        m.pasteFromPasteboard()
        d.enter()
        expectPixel(4, 4, RED)
        expectPixel(9, 9, BLUE)
        // An image from another app is pasted centered.
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.setData(pngData(4, 2, fill: GREEN), forType: .png)
        let m2 = fresh()
        m2.pasteFromPasteboard()
        d.enter()
        expectEq(m2.count(of: GREEN), 8)
        expectPixel(6, 7, GREEN)
        expectPixel(9, 8, GREEN)
        d.snapshot("select-paste")
    }

    scenario("select/flip-rotate") {
        let m = fresh(), d = CanvasDriver()
        let R = RED, G = GREEN, B = BLUE, W = WHITE
        for (x, y, c) in [(4, 4, R), (5, 4, G), (4, 5, B), (5, 5, W)] { put(d, x, y, c) }
        m.tool = .marquee
        d.stroke([(4, 4), (5, 5)])
        func grid() -> [UInt32] { [m.pixel(4, 4), m.pixel(5, 4), m.pixel(4, 5), m.pixel(5, 5)] }
        m.transform(0)
        expectEq(grid(), [G, R, W, B], "flip horizontal")
        m.transform(0)
        expectEq(grid(), [R, G, B, W])
        m.transform(1)
        expectEq(grid(), [B, W, R, G], "flip vertical")
        m.transform(1)
        m.transform(2)
        expectEq(grid(), [B, R, W, G], "rotate clockwise")
        m.undo()
        m.transform(3)
        expectEq(grid(), [G, W, R, B], "rotate counterclockwise")
        // Non-square selections swap their dimensions.
        m.undo()
        m.deselect()
        paintRect(d, 8, 8, 10, 8, RED)
        m.tool = .marquee
        d.stroke([(8, 8), (10, 8)])
        m.transform(2)
        expectEq(m.selectionBounds?.suffix(2).map { $0 } ?? [], [1, 3], "3x1 becomes 1x3")
        expectEq(m.visibleCount(), 7, "pixel count is preserved")
    }
}
