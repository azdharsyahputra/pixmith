import AppKit
@testable import PixmithKit
import PixmithCore

func registerScenarios() {
    registerMore()
    registerApp()
    registerUI()
    registerMode()
    registerRound2()
    registerDraw()
    registerSelection()
    registerLayers()
    registerPalette()
    registerHistory()
    registerFiles()
    registerTabs()
    registerProjects()
    registerExtensions()
    registerLuaDialog()
    registerLive()
    registerColor()
    registerRetouch()
    registerSpriteData()
    registerImageFormats()
    registerIso()
}

/// Snapshot of the whole composite plus undo counts; used to check that an operation is one undo step.
struct Mark {
    let pixels: [UInt32]
    let undo: Int
    init(_ m: SpriteModel) {
        pixels = m.pixels()
        undo = m.historyCounts.undo
    }
}

func isSymmetric(_ m: SpriteModel, x: Bool, y: Bool) -> Bool {
    let p = m.pixels(), w = m.width, h = m.height
    for yy in 0..<h {
        for xx in 0..<w {
            if x, p[yy * w + xx] != p[yy * w + (w - 1 - xx)] { return false }
            if y, p[yy * w + xx] != p[(h - 1 - yy) * w + xx] { return false }
        }
    }
    return true
}

func registerDraw() {
    scenario("draw/pencil-eraser-undo") {
        let m = fresh(), d = CanvasDriver()
        m.color = RED
        d.click(3, 3)
        expectPixel(3, 3, RED)
        expectEq(m.visibleCount(), 1)
        // A multi-point stroke is one undo step.
        let before = m.historyCounts.undo
        d.stroke([(0, 8), (5, 8), (5, 12)])
        expectEq(m.historyCounts.undo, before + 1, "stroke is one undo step")
        for x in 0...5 { expectPixel(x, 8, RED) }
        for y in 8...12 { expectPixel(5, y, RED) }
        m.undo()
        expectEq(m.visibleCount(), 1, "undo removes the whole stroke")
        m.redo()
        expectPixel(5, 12, RED)
        // Brush size 3 stamps a 3x3 block.
        m.brushSize = 3
        d.click(10, 3)
        for y in 2...4 { for x in 9...11 { expectPixel(x, y, RED, "3px brush") } }
        expectPixel(8, 3, CLEAR)
        expectPixel(12, 3, CLEAR)
        // Eraser removes, also with a larger brush.
        m.tool = .eraser
        d.click(10, 3)
        expectEq(m.count(of: RED, frame: 0) , m.visibleCount())
        for y in 2...4 { for x in 9...11 { expectPixel(x, y, CLEAR, "erased") } }
        m.brushSize = 1
        d.click(3, 3)
        expectPixel(3, 3, CLEAR)
        d.snapshot("draw-pencil")
    }

    scenario("draw/secondary-color-right-button") {
        let m = fresh(), d = CanvasDriver()
        m.color = RED
        m.opts.secondary = GREEN
        d.click(2, 2, right: true)
        expectPixel(2, 2, GREEN)
        d.click(4, 2)
        expectPixel(4, 2, RED)
        m.swapColors()
        expectEq(m.color, GREEN)
        expectEq(m.opts.secondary, RED)
        d.click(6, 2)
        expectPixel(6, 2, GREEN)
    }

    scenario("draw/fill-contiguous-global") {
        let m = fresh(), d = CanvasDriver()
        m.color = RED
        d.stroke((0..<16).map { (8, $0) }) // vertical wall
        m.tool = .fill
        m.color = BLUE
        d.click(2, 2)
        expectPixel(2, 2, BLUE)
        expectPixel(7, 15, BLUE)
        expectPixel(12, 2, CLEAR, "contiguous fill stays on its side of the wall")
        expectPixel(8, 5, RED)
        expectEq(m.count(of: BLUE), 8 * 16)
        m.undo()
        // Global: every matching (transparent) pixel in the cel.
        m.opts.contiguous = false
        d.click(2, 2)
        expectPixel(12, 2, BLUE, "global fill reaches the other side")
        expectEq(m.count(of: BLUE), 15 * 16)
        expectPixel(8, 5, RED)
        // Filling a colored area replaces just that color.
        m.opts.contiguous = true
        m.color = GREEN
        d.click(8, 5)
        expectEq(m.count(of: GREEN), 16)
        d.snapshot("draw-fill")
    }

    scenario("draw/fill-patterns") {
        let m = fresh(), d = CanvasDriver()
        m.tool = .fill
        m.color = RED
        m.opts.secondary = BLUE
        m.opts.fillPattern = 1 // checker
        d.click(0, 0)
        let p = m.pixels()
        expectEq(p[0], RED)
        expectEq(p[1], RED, "the checker has 2x2 cells")
        expectEq(p[2], BLUE)
        expectEq(p[32], BLUE)
        expectEq(p[34], RED)
        expectEq(m.count(of: RED), 128)
        expectEq(m.count(of: BLUE), 128)
        for (pattern, lo, hi) in [(2, 0.2, 0.3), (3, 0.45, 0.55), (4, 0.7, 0.8)] {
            m.undo()
            m.opts.fillPattern = pattern
            d.click(0, 0)
            let share = Double(m.count(of: RED)) / 256
            expect(share >= lo && share <= hi, "pattern \(pattern) covers \(share) with the primary color")
            expectEq(m.count(of: RED) + m.count(of: BLUE), 256)
        }
        m.undo()
        m.opts.fillPattern = 0
        d.click(0, 0)
        expectEq(m.count(of: RED), 256)
        // Right-click fills with the secondary color as the main one.
        m.undo()
        d.click(0, 0, right: true)
        expectEq(m.count(of: BLUE), 256)
    }

    scenario("draw/line-rect-ellipse") {
        let m = fresh(), d = CanvasDriver()
        m.color = RED
        m.tool = .line
        let n0 = m.historyCounts.undo
        d.stroke([(1, 1), (10, 1)])
        for x in 1...10 { expectPixel(x, 1, RED) }
        expectEq(m.visibleCount(), 10)
        expectEq(m.historyCounts.undo, n0 + 1)
        // Dragging previews through the same undo step: only the final line remains.
        d.down(1, 4)
        d.drag(10, 10)
        d.drag(10, 4)
        d.up(10, 4)
        for x in 1...10 { expectPixel(x, 4, RED) }
        expectEq(m.visibleCount(), 20, "intermediate previews are rewound")
        // Diagonal.
        m.tool = .line
        d.stroke([(0, 8), (7, 15)])
        for i in 0...7 { expectPixel(i, 8 + i, RED) }
        _ = fresh()
        let m2 = SpriteModel.shared
        m2.color = RED
        m2.tool = .rectangle
        d.stroke([(2, 2), (9, 7)])
        expectEq(m2.visibleCount(), 2 * 8 + 2 * 6 - 4, "outline rectangle 8x6")
        expectPixel(2, 2, RED); expectPixel(9, 7, RED); expectPixel(5, 4, CLEAR)
        m2.undo()
        m2.opts.filled = true
        d.stroke([(2, 2), (9, 7)])
        expectEq(m2.visibleCount(), 48, "filled rectangle 8x6")
        expectPixel(5, 4, RED)
        // Shift constrains to a square.
        m2.undo()
        d.stroke([(2, 2), (9, 4)], flags: .shift)
        let pts = m2.opaque()
        let xs = pts.map(\.0), ys = pts.map(\.1)
        expectEq(xs.max()! - xs.min()!, ys.max()! - ys.min()!, "shift gives a square")
        // Ellipse: outline is hollow and symmetric; filled has a center.
        m2.undo()
        m2.opts.filled = false
        m2.tool = .ellipse
        d.stroke([(2, 2), (13, 13)])
        expectPixel(7, 7, CLEAR, "hollow ellipse")
        expect(m2.visibleCount() > 20, "ellipse outline has pixels")
        expect(isSymmetric(m2, x: true, y: true), "ellipse is symmetric")
        expectPixel(2, 7, RED); expectPixel(13, 7, RED); expectPixel(7, 2, RED); expectPixel(7, 13, RED)
        m2.undo()
        m2.opts.filled = true
        d.stroke([(2, 2), (13, 13)])
        expectPixel(7, 7, RED)
        expectPixel(2, 2, CLEAR, "ellipse corners stay empty")
        d.snapshot("draw-shapes")
    }

    scenario("draw/spray") {
        let m = fresh(32, 32), d = CanvasDriver()
        m.color = RED
        m.tool = .spray
        m.brushSize = 3 // radius 9
        let n0 = m.historyCounts.undo
        d.down(16, 16)
        pumpRunLoop(0.25)
        d.up(16, 16)
        let pts = m.opaque()
        expect(pts.count > 10, "spray paints pixels (\(pts.count))")
        expect(pts.allSatisfy { abs($0.0 - 16) <= 10 && abs($0.1 - 16) <= 10 }, "spray stays within its radius")
        expect(m.pixels().allSatisfy { $0 == CLEAR || $0 == RED })
        expectEq(m.historyCounts.undo, n0 + 1)
        m.undo()
        expectEq(m.visibleCount(), 0)
        d.snapshot("draw-spray")
    }

    scenario("draw/gradient") {
        let m = fresh(), d = CanvasDriver()
        m.tool = .gradient
        m.color = RED
        m.opts.secondary = BLUE
        // A bare click leaves no undo step.
        let n0 = m.historyCounts.undo
        d.click(4, 4)
        expectEq(m.historyCounts.undo, n0)
        d.stroke([(0, 8), (8, 8), (15, 8)])
        expectEq(m.historyCounts.undo, n0 + 1)
        let left = m.pixel(0, 3), right = m.pixel(15, 3), mid = m.pixel(8, 3)
        expectEq(left, RED, "gradient starts with the primary color")
        expectEq(right, BLUE, "gradient ends with the secondary color")
        expect(mid != RED && mid != BLUE, "gradient blends in the middle \(hex(mid))")
        var last: UInt32 = 256
        for x in 0..<16 {
            let r = m.pixel(x, 8) >> 24
            expect(r <= last, "red falls along the gradient at x=\(x)")
            last = r
        }
        expectEq(m.pixel(5, 0), m.pixel(5, 15), "linear gradient is constant across the drag direction")
        // Radial, and dithered: only the two end colors appear.
        m.undo()
        m.opts.gradientRadial = true
        d.stroke([(8, 8), (15, 8)])
        expectEq(m.pixel(8, 8), RED)
        expectEq(m.pixel(8, 15), BLUE)
        m.undo()
        m.opts.gradientRadial = false
        m.opts.gradientDither = 4
        d.stroke([(0, 8), (15, 8)])
        expect(m.pixels().allSatisfy { $0 == RED || $0 == BLUE }, "dithered gradient uses only the two colors")
        expect(m.count(of: RED) > 40 && m.count(of: BLUE) > 40)
        // Inside a selection only the selection is filled.
        m.undo()
        m.opts.gradientDither = 0
        m.tool = .marquee
        d.stroke([(4, 4), (9, 9)])
        m.tool = .gradient
        d.stroke([(4, 6), (9, 6)])
        expectPixel(2, 6, CLEAR, "outside the selection stays empty")
        expect(m.pixel(5, 6) != CLEAR)
        d.snapshot("draw-gradient")
    }

    scenario("draw/curve") {
        let m = fresh(), d = CanvasDriver()
        m.tool = .curve
        m.color = RED
        let n0 = m.historyCounts.undo
        d.down(1, 8)
        d.drag(14, 8)
        d.up(14, 8)
        expect(m.drawingSessionOpen, "curve waits for control points")
        d.down(8, 2)
        d.up(8, 2)
        d.down(8, 2)
        d.up(8, 2)
        expect(!m.drawingSessionOpen, "curve finishes after the second control point")
        expectPixel(1, 8, RED)
        expectPixel(14, 8, RED)
        expect(m.opaque().contains { $0.1 <= 5 }, "curve bends toward the control points")
        expectEq(m.historyCounts.undo, n0 + 1, "curve is one undo step")
        m.undo()
        expectEq(m.visibleCount(), 0)
        // A plain click with no drag draws nothing lasting.
        d.click(5, 5)
        expect(!m.drawingSessionOpen)
        d.snapshot("draw-curve")
    }

    scenario("draw/polygon") {
        let m = fresh(), d = CanvasDriver()
        m.tool = .shapePolygon
        m.color = RED
        let n0 = m.historyCounts.undo
        d.click(2, 2)
        d.click(12, 2)
        d.click(12, 12)
        expect(m.drawingSessionOpen)
        d.click(2, 12, clicks: 2)
        expect(!m.drawingSessionOpen, "double click finishes the polygon")
        for p in [(2, 2), (12, 2), (12, 12)] { expectPixel(p.0, p.1, RED) }
        expectPixel(10, 5, CLEAR, "outline polygon is hollow")
        expectEq(m.historyCounts.undo, n0 + 1)
        m.undo()
        m.opts.filled = true
        d.click(2, 2); d.click(12, 2); d.click(12, 12)
        d.enter()
        expect(!m.drawingSessionOpen, "Enter finishes the polygon")
        expectPixel(10, 5, RED, "filled polygon covers its interior")
        expectPixel(2, 12, CLEAR)
        m.undo()
        // Escape cancels and leaves nothing.
        d.click(2, 2); d.click(12, 2)
        d.escape()
        expect(!m.drawingSessionOpen)
        expectEq(m.visibleCount(), 0, "cancelled polygon leaves nothing")
        d.snapshot("draw-polygon")
    }

    scenario("draw/contour") {
        let m = fresh(), d = CanvasDriver()
        m.tool = .contour
        m.color = RED
        let n0 = m.historyCounts.undo
        d.stroke([(3, 3), (10, 3), (10, 10), (3, 10), (3, 6)])
        expect(!m.drawingSessionOpen)
        expectPixel(6, 6, CLEAR, "an unfilled contour is just the closed outline")
        expectPixel(3, 4, RED, "the contour closes back to its start")
        expectPixel(7, 3, RED)
        expectPixel(12, 12, CLEAR)
        expectEq(m.historyCounts.undo, n0 + 1)
        m.undo()
        expectEq(m.visibleCount(), 0)
        m.opts.filled = true
        d.stroke([(3, 3), (10, 3), (10, 10), (3, 10), (3, 6)])
        expectPixel(6, 6, RED, "a filled contour fills the closed outline")
        expectPixel(12, 12, CLEAR)
        d.snapshot("draw-contour")
    }

    scenario("draw/text") {
        let m = fresh(32, 16), d = CanvasDriver()
        m.tool = .text
        m.color = RED
        m.opts.textScale = 1
        d.click(2, 2)
        expect(m.opts.textRequest != nil, "text tool asks for text")
        m.opts.textString = "AB"
        m.renderText()
        let n = m.visibleCount()
        expect(n > 8, "text pixels drawn (\(n))")
        m.opts.textString = "ABC"
        m.renderText()
        expect(m.visibleCount() > n, "re-rendering replaces the preview with the longer text")
        m.placeText()
        expect(m.opts.textRequest == nil)
        let placed = m.pixels()
        m.undo()
        expectEq(m.visibleCount(), 0, "text is one undo step")
        m.redo()
        expectEq(m.pixels(), placed)
        // Cancelling removes the preview.
        d.click(2, 2)
        m.opts.textString = "Z"
        m.renderText()
        m.cancelText()
        expectEq(m.pixels(), placed)
        // The system-font path rasterizes through CoreText.
        m.undo()
        let raster = SpriteModel.rasterize("Hi", fontName: nil, size: 10)
        expect(raster != nil, "CoreText rasterizes text")
        expect(raster?.mask.contains(255) == true, "rasterized mask has ink (max \(raster?.mask.max() ?? 0), \(raster?.w ?? 0)x\(raster?.h ?? 0))")
        m.opts.textSystemFont = true
        m.opts.textFontSize = 10
        d.click(2, 2)
        m.opts.textString = "Hi"
        m.renderText()
        m.placeText()
        expect(m.visibleCount() > 4, "system font text")
        m.opts.textSystemFont = false
        d.snapshot("draw-text")
    }

    scenario("draw/ink-modes") {
        let m = fresh(), d = CanvasDriver()
        m.color = BLUE
        d.click(4, 4)
        m.color = 0xFF000080
        // Simple replaces.
        m.opts.ink = .simple
        d.click(4, 4)
        expectPixel(4, 4, 0xFF000080, "simple ink replaces")
        m.undo(); m.undo()
        m.color = BLUE
        d.click(4, 4)
        m.color = 0xFF000080
        // Alpha compositing blends over what is there.
        m.opts.ink = .alpha
        d.click(4, 4)
        let blended = m.pixel(4, 4)
        expect(blended >> 24 > 100 && blended >> 24 < 160 && blended & 0xFF == 0xFF && (blended >> 16) & 0xFF < 5,
               "alpha ink blends red over blue: \(hex(blended))")
        m.undo()
        // Lock alpha recolors without changing alpha, and does nothing on empty pixels.
        m.opts.ink = .lockAlpha
        m.color = RED
        d.click(4, 4)
        expectPixel(4, 4, RED, "lock alpha keeps the opaque alpha")
        d.click(8, 8)
        expectPixel(8, 8, CLEAR, "lock alpha does not paint on transparent pixels")
        // Behind only paints where there is nothing yet.
        m.undo(); m.undo()
        expectPixel(4, 4, BLUE)
        m.opts.ink = .behind
        m.color = GREEN
        d.click(4, 4)
        expectPixel(4, 4, BLUE, "behind leaves existing pixels alone")
        d.click(8, 8)
        expectPixel(8, 8, GREEN, "behind paints empty pixels")
        // Shading steps existing ramp colors toward the secondary color (right-click steps back) and
        // does not paint on empty pixels.
        let pal = m.palette
        m.opts.ink = .simple
        m.color = pal[0]
        d.click(12, 12)
        expectPixel(12, 12, pal[0])
        m.opts.ink = .shading
        m.opts.secondary = pal[3]
        d.click(14, 14)
        expectPixel(14, 14, CLEAR, "shading does not paint empty pixels")
        d.click(12, 12)
        expectPixel(12, 12, pal[1], "shading steps toward the secondary color")
        d.click(12, 12)
        expectPixel(12, 12, pal[2])
        d.click(12, 12)
        expectPixel(12, 12, pal[3])
        d.click(12, 12)
        expectPixel(12, 12, pal[3], "shading stops at the end of the ramp")
        d.click(12, 12, right: true)
        expectPixel(12, 12, pal[2], "right-click steps back")
        d.snapshot("draw-ink")
    }

    scenario("draw/symmetry") {
        let m = fresh(), d = CanvasDriver()
        m.color = RED
        m.opts.mirrorX = true
        d.click(3, 5)
        expectPixel(3, 5, RED)
        expectPixel(12, 5, RED, "mirrored horizontally")
        expectEq(m.visibleCount(), 2)
        m.undo()
        m.opts.mirrorY = true
        d.click(3, 5)
        expectEq(m.visibleCount(), 4)
        for p in [(3, 5), (12, 5), (3, 10), (12, 10)] { expectPixel(p.0, p.1, RED) }
        m.undo()
        // Shapes, strokes, fills and sprays follow the guides too.
        d.stroke([(1, 1), (6, 2), (3, 7)])
        expect(isSymmetric(m, x: true, y: true), "stroke symmetric")
        m.undo()
        m.tool = .rectangle
        m.opts.filled = true
        d.stroke([(2, 3), (5, 6)])
        expect(isSymmetric(m, x: true, y: true), "rectangle symmetric")
        m.undo()
        m.tool = .line
        d.stroke([(1, 2), (6, 5)])
        expect(isSymmetric(m, x: true, y: true), "line symmetric")
        m.undo()
        m.tool = .spray
        d.down(4, 4); pumpRunLoop(0.1); d.up(4, 4)
        expect(isSymmetric(m, x: true, y: true), "spray symmetric")
        m.undo()
        m.tool = .fill
        d.click(0, 0)
        expectEq(m.visibleCount(), 256)
        d.snapshot("draw-symmetry")
    }

    scenario("draw/pixel-perfect-brush-picker") {
        let m = fresh(), d = CanvasDriver()
        m.color = RED
        d.stroke([(2, 2), (3, 2), (3, 3)])
        expectPixel(3, 2, RED, "without pixel-perfect the corner stays")
        m.undo()
        m.opts.pixelPerfect = true
        d.stroke([(2, 2), (3, 2), (3, 3)])
        expectPixel(3, 2, CLEAR, "pixel-perfect removes the L corner")
        expectPixel(2, 2, RED); expectPixel(3, 3, RED)
        m.undo()
        m.opts.pixelPerfect = false
        // Round brush.
        m.brushSize = 5
        m.opts.circleBrush = true
        d.click(8, 8)
        expectPixel(8, 8, RED)
        expectPixel(6, 6, CLEAR, "round brush leaves the corners")
        expectPixel(8, 6, RED)
        expectEq(m.visibleCount() < 25, true)
        m.undo()
        m.opts.circleBrush = false
        m.brushSize = 1
        // Custom brush from a selection.
        m.color = BLUE
        d.click(1, 1); d.click(2, 2)
        m.tool = .marquee
        d.stroke([(1, 1), (2, 2)])
        m.brushFromSelection()
        expectEq(m.opts.customBrush?.w, 2)
        expectEq(m.opts.customBrush?.h, 2)
        expect(m.tool == .pencil)
        m.deselect()
        d.click(8, 8)
        let stamped = m.opaque().filter { $0.0 > 5 }
        expectEq(stamped.count, 2, "the custom brush stamps its two pixels")
        if stamped.count == 2 {
            expect(abs(stamped[0].0 - stamped[1].0) == 1 && abs(stamped[0].1 - stamped[1].1) == 1, "diagonal pair \(stamped)")
        }
        m.resetBrush()
        expect(m.opts.customBrush == nil)
        // The eyedropper picks the composite color; option-click picks while drawing.
        m.color = BLACK
        m.tool = .picker
        d.click(8, 8)
        expectEq(m.color, BLUE)
        m.tool = .pencil
        m.color = GREEN
        d.click(1, 1, flags: .option)
        expectEq(m.color, BLUE, "option-click picks")
    }

    scenario("draw/tool-keys") {
        let m = fresh(), d = CanvasDriver()
        for (key, tool) in [("e", Tool.eraser), ("g", .fill), ("i", .picker), ("m", .marquee), ("b", .pencil), ("l", .line)] {
            d.key(key)
            expect(m.tool == tool, "key \(key) selects \(tool)")
        }
        m.brushSize = 4
        d.key("[")
        expectEq(m.brushSize, 3)
        d.key("]")
        d.key("]")
        expectEq(m.brushSize, 5)
        m.color = RED
        m.opts.secondary = BLUE
        d.key("x")
        expectEq(m.color, BLUE)
    }
}
