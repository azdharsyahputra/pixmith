import AppKit
@testable import PixmithKit
import PixmithCore

func registerMode() {
    scenario("color/indexed-tools-stay-on-palette") {
        let m = fresh(16, 16), d = CanvasDriver()
        m.convertColorMode(to: .indexed)
        expectEq(m.colorMode, .indexed)
        let palette = Set(m.palette)
        func onPalette(_ what: String, file: String = #fileID, line: UInt = #line) {
            let bad = m.pixels().filter { $0 & 0xFF != 0 && !palette.contains($0) }
            expect(bad.isEmpty, "\(what) left pixels outside the palette: \(bad.prefix(3).map(hex))", file: file, line: line)
            expectEq(m.visibleCount() > 0, true, "\(what) painted something", file: file, line: line)
            m.undo()
        }
        m.color = 0x123456FF            // snaps to a palette color
        expect(palette.contains(m.color))
        m.opts.secondary = 0xFEDCBAFF   // not snapped by itself
        m.opts.filled = true
        m.tool = .pencil; d.stroke([(0, 0), (10, 4)]); onPalette("pencil")
        m.tool = .line; d.stroke([(0, 0), (10, 4)]); onPalette("line")
        m.tool = .rectangle; d.stroke([(1, 1), (8, 8)]); onPalette("rectangle")
        m.tool = .ellipse; d.stroke([(1, 1), (14, 14)]); onPalette("ellipse")
        m.tool = .fill; d.click(5, 5); onPalette("fill")
        m.tool = .spray; d.down(8, 8); pumpRunLoop(0.1); d.up(8, 8); onPalette("spray")
        m.tool = .gradient; d.stroke([(0, 0), (15, 15)]); onPalette("gradient")
        m.opts.fillPattern = 3
        m.tool = .fill; d.click(5, 5); onPalette("pattern fill")
        m.opts.fillPattern = 0
        m.tool = .text; m.opts.textScale = 1; d.click(1, 1); m.opts.textString = "Hi"; m.renderText(); m.placeText(); onPalette("text")
        // Alpha ink with a translucent primary color cannot create new colors in indexed mode.
        m.opts.ink = .alpha
        m.tool = .pencil
        m.color = 0xFF000080
        d.click(2, 2)
        d.click(2, 2)
        onPalette("alpha ink")
        m.opts.ink = .simple
        // Filters in indexed mode.
        m.color = RED
        m.opts.secondary = BLUE
        m.tool = .pencil
        d.stroke([(2, 2), (9, 2)])
        for (kind, params) in [(FilterKind.blur, [Int32]()), (.sharpen, []), (.hsl, [40, 0, 0]), (.brightnessContrast, [30, 10]), (.dropShadow, [1, 1, Int32(bitPattern: 0x00000080)])] {
            if m.applyFilter(kind, params: params) {
                let bad = m.pixels().filter { $0 & 0xFF != 0 && !palette.contains($0) }
                expect(bad.isEmpty, "\(kind.title) in indexed mode left colors outside the palette: \(bad.prefix(3).map(hex))")
                m.undo()
            }
        }
        // Saving and reopening keeps what was drawn.
        m.tool = .gradient
        d.stroke([(0, 0), (15, 0)])
        let before = DocSnap(m)
        let url = tmp("indexed-tools.aseprite")
        m.fileURL = url
        m.save()
        fresh()
        m.open(url)
        expectSame(before, DocSnap(m), "indexed after tools")
        d.snapshot("color-indexed-tools")
    }

    scenario("color/grayscale-tools-stay-gray") {
        let m = fresh(16, 16), d = CanvasDriver()
        m.convertColorMode(to: .grayscale)
        m.color = 0x336699FF
        let (r, g, b) = rgb(m.color)
        expect(r == g && g == b, "drawing color is gray: \(r),\(g),\(b)")
        m.opts.secondary = 0xFFFFFFFF
        m.opts.filled = true
        for (tool, run) in [(Tool.pencil, { d.stroke([(0, 0), (10, 4)]) }), (.rectangle, { d.stroke([(1, 1), (8, 8)]) }),
                            (.gradient, { d.stroke([(0, 0), (15, 15)]) }), (.fill, { d.click(5, 5) })] as [(Tool, () -> Void)] {
            m.tool = tool
            run()
            for (x, y) in m.opaque() {
                let (r, g, b) = rgb(m.pixel(x, y))
                expect(r == g && g == b, "\(tool) pixel (\(x),\(y)) is gray: \(r),\(g),\(b)")
            }
            m.undo()
        }
        // Back to color: the pixels stay as they were.
        m.tool = .pencil
        d.stroke([(0, 0), (10, 4)])
        let before = m.pixels()
        m.convertColorMode(to: .rgb)
        expectEq(m.pixels(), before)
    }

    scenario("history/limit-and-trim") {
        let m = fresh(), d = CanvasDriver()
        m.color = RED
        let url = tmp("trim.aseprite")
        m.fileURL = url
        m.save()
        expect(!m.isDirty)
        for i in 0..<130 { d.click(i % 16, (i / 16) % 16) }
        let n = m.historyCounts.undo
        expect(n <= 130 && n >= 90, "history is capped but long: \(n)")
        expect(m.isDirty, "edits after the save are dirty")
        // Undoing everything that is left cannot reach the saved state any more.
        m.goToHistory(0)
        expect(m.isDirty, "once the saved state fell off the history it cannot be matched")
        m.goToHistory(n)
        m.goToHistory(n - 1)
        m.redo()
        expectEq(m.historyCounts.undo, n)
    }
}
