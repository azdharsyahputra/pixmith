import AppKit
@testable import PixmithKit
import PixmithCore

/// Blur and Jumble: shortcuts, one undo step per stroke, selection clipping, mirroring.
func registerRetouch() {
    /// Red left half, blue right half.
    func halves(_ m: SpriteModel, _ d: CanvasDriver) {
        m.tool = .pencil
        m.brushSize = 1
        m.color = RED
        for x in 0..<8 { d.stroke([(x, 0), (x, 15)]) }
        m.color = BLUE
        for x in 8..<16 { d.stroke([(x, 0), (x, 15)]) }
    }

    scenario("tools/blur-keys-and-undo") {
        let m = fresh(), d = CanvasDriver()
        halves(m, d)
        d.key("r")
        expect(m.tool == .blur, "R selects Blur")
        d.key("r", flags: .shift)
        expect(m.tool == .jumble, "Shift+R selects Jumble")
        m.tool = .blur
        m.brushSize = 3
        m.opts.retouchStrength = 100
        let before = Mark(m)
        d.stroke([(4, 8), (12, 8)])
        expectEq(m.historyCounts.undo, before.undo + 1, "one undo step")
        expect(m.pixel(7, 8) != RED && m.pixel(8, 8) != BLUE, "the edge softened")
        expectPixel(1, 1, RED, "far from the brush nothing changed")
        expectPixel(14, 14, BLUE)
        _ = d.snapshot("tool-blur")
        m.undo()
        expectEq(m.pixels(), before.pixels, "undo restores the sprite")
    }

    scenario("tools/jumble-mixes-and-mirrors") {
        let m = fresh(), d = CanvasDriver()
        halves(m, d)
        m.tool = .jumble
        m.brushSize = 4
        let before = Mark(m)
        d.stroke([(5, 4), (11, 4)])
        expectEq(m.historyCounts.undo, before.undo + 1, "one undo step")
        expectEq(m.count(of: RED) + m.count(of: BLUE), 256, "jumble only moves pixels around")
        expect(m.pixels() != before.pixels, "something moved")
        expectPixel(1, 14, RED)
        _ = d.snapshot("tool-jumble")
        m.undo()
        expectEq(m.pixels(), before.pixels)

        // Blur with the vertical mirror reaches the bottom too, and the result is symmetric.
        m.tool = .blur
        m.brushSize = 3
        m.opts.retouchStrength = 100
        m.opts.mirrorY = true
        d.stroke([(4, 3), (12, 3)])
        expect(isSymmetric(m, x: false, y: true), "mirrored blur is symmetric")
        expect(m.pixel(7, 12) != RED, "the mirror image blurred at the bottom")
        m.opts.mirrorY = false
    }

    scenario("tools/blur-respects-selection") {
        let m = fresh(), d = CanvasDriver()
        halves(m, d)
        m.tool = .marquee
        d.stroke([(0, 0), (7, 15)])
        m.tool = .blur
        m.brushSize = 5
        m.opts.retouchStrength = 100
        d.stroke([(2, 8), (14, 8)])
        for y in 0..<16 { for x in 8..<16 { expectPixel(x, y, BLUE, "outside the selection") } }
        expect(m.pixel(7, 8) != RED, "inside the selection the edge blurred")
    }

    scenario("tools/blur-jumble-window") {
        // The rail and options bar of both tools, in both appearances.
        for dark in [true, false] {
            let m = fresh(32, 32), d = CanvasDriver()
            m.zoom = 12
            halves(m, d)
            m.brushSize = 6
            m.tool = .blur
            d.stroke([(4, 10), (28, 14)])
            let app = AppDriver(m, dark: dark)
            _ = app.snapshot("window-blur-\(dark ? "dark" : "light")")
            m.tool = .jumble
            d.stroke([(4, 22), (28, 20)])
            _ = app.snapshot("window-jumble-\(dark ? "dark" : "light")")
        }
    }
}
