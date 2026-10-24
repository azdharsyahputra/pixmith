import AppKit
import SwiftUI
@testable import PixmithKit
import PixmithCore

private func near(_ a: Double, _ b: Double, _ tol: Double) -> Bool { abs(a - b) <= tol }

private func bytes(_ c: UInt32) -> (r: Int, g: Int, b: Int, a: Int) {
    (Int(c >> 24 & 0xFF), Int(c >> 16 & 0xFF), Int(c >> 8 & 0xFF), Int(c & 0xFF))
}

/// The inspector column of a window snapshot, saved as a PNG.
@discardableResult
private func saveInspector(_ rep: NSBitmapImageRep, name: String, height: CGFloat = 420) -> CGImage? {
    guard let cg = rep.cgImage else { return nil }
    let scale = CGFloat(cg.width) / CGFloat(rep.size.width)
    // The inspector is the 272 pt column on the right, below the 40 pt title row.
    let rect = CGRect(x: CGFloat(cg.width) - 272 * scale, y: 40 * scale, width: 272 * scale, height: height * scale)
    guard let crop = cg.cropping(to: rect) else { return nil }
    try? FileManager.default.createDirectory(atPath: QA.outDir, withIntermediateDirectories: true)
    let out = NSBitmapImageRep(cgImage: crop)
    if let png = out.representation(using: .png, properties: [:]) {
        try? png.write(to: URL(fileURLWithPath: QA.outDir).appendingPathComponent("\(name).png"))
    }
    return crop
}

private func pixelHash(_ image: CGImage) -> Int {
    // Redraw the crop: a cropped CGImage shares the whole snapshot's data.
    let rep = NSBitmapImageRep(cgImage: image)
    guard let png = rep.representation(using: .png, properties: [:]) else { return 0 }
    return png.hashValue
}

func registerColor() {
    scenario("color/math-roundtrips") {
        // Every sampled byte color survives RGB -> HSV -> RGB and RGB -> HSL -> RGB through the core.
        var checked = 0
        for r in stride(from: 0, through: 255, by: 15) {
            for g in stride(from: 0, through: 255, by: 15) {
                for b in stride(from: 0, through: 255, by: 15) {
                    let c = UInt32(r) << 24 | UInt32(g) << 16 | UInt32(b) << 8 | 0xFF
                    let v = ColorMath.hsv(c)
                    let l = ColorMath.hsl(c)
                    if ColorMath.rgba(h: v.a, s: v.b, v: v.c) != c { expectEq(ColorMath.rgba(h: v.a, s: v.b, v: v.c), c, "hsv round trip") }
                    if ColorMath.rgba(hslH: l.a, s: l.b, l: l.c) != c { expectEq(ColorMath.rgba(hslH: l.a, s: l.b, l: l.c), c, "hsl round trip") }
                    checked += 1
                }
            }
        }
        expect(checked > 4000)
        let orange = ColorMath.hsv(0xF2A65AFF)
        expect(near(orange.a, 30, 0.6) && near(orange.b, 0.63, 0.01) && near(orange.c, 0.95, 0.01), "orange is 30/63/95: \(orange)")
        expectEq(ColorMath.rgba(h: 120, s: 1, v: 1), 0x00FF00FF)
        expectEq(ColorMath.rgba(hslH: 240, s: 1, l: 0.5, a: 0.5), 0x0000FF80)
        expectEq(ColorMath.rgba(h: 360, s: 1, v: 1), RED, "hue 360 wraps to red")
        // Gray has no hue, black no saturation: the hint fills them in.
        let gray = ColorMath.hsv(0x808080FF, hint: (200, 0.7))
        expect(gray.a == 200 && gray.b == 0, "gray keeps the hue hint: \(gray)")
        let black = ColorMath.hsv(BLACK, hint: (200, 0.7))
        expect(black.a == 200 && black.b == 0.7, "black keeps hue and saturation: \(black)")
        expectEq(HSVA(rgba: 0x808080FF, hue: 90).h, 90)
        // Wheel geometry: red at the east, hue grows counter-clockwise (up on the screen), the rim clamps.
        for hue in stride(from: 0.0, to: 360, by: 37) {
            for sat in [0.2, 0.7, 1.0] {
                let p = ColorMath.wheelPoint(hue: hue, sat: sat)
                let hit = ColorMath.wheelHit(x: p.x, y: p.y, hintHue: 0)
                expect(near(hit.hue, hue, 1e-6) && near(hit.sat, sat, 1e-9), "wheel \(hue) \(sat) -> \(hit)")
            }
        }
        expect(near(ColorMath.wheelHit(x: 0, y: -0.5, hintHue: 0).hue, 90, 1e-6), "up is hue 90")
        expectEq(ColorMath.wheelHit(x: 3, y: 0, hintHue: 0).sat, 1.0, "the rim clamps")
        expectEq(ColorMath.wheelHit(x: 0, y: 0, hintHue: 77).hue, 77, "the center keeps the hint")
        expectEq(ColorMath.harmonyHues(10, .none), [])
        expectEq(ColorMath.harmonyHues(10, .complementary), [190])
        expectEq(ColorMath.harmonyHues(10, .triadic), [130, 250])
        expectEq(ColorMath.harmonyHues(10, .analogous), [340, 40])
        expectEq(ColorMath.harmonyHues(0, .square), [90, 180, 270])
        // The rendered wheel: transparent outside, red at the east, white in the middle at full value.
        let n = 64
        let px = ColorMath.wheelPixels(size: n, model: .hsv, value: 1)
        func at(_ x: Int, _ y: Int) -> (Int, Int, Int, Int) {
            let i = (y * n + x) * 4
            return (Int(px[i]), Int(px[i + 1]), Int(px[i + 2]), Int(px[i + 3]))
        }
        expectEq(at(0, 0).3, 0, "outside the disc is transparent")
        let east = at(n - 2, n / 2)
        expect(east.0 > 240 && east.1 < 40 && east.3 == 255, "red at the east: \(east)")
        let mid = at(n / 2, n / 2)
        expect(mid.0 > 245 && mid.1 > 245 && mid.2 > 245, "white at the center: \(mid)")
        // Normals: the flat normal is #8080FF, X right, Y up, the rim is the horizon.
        expectEq(ColorMath.normalEncode([0, 0, 1], alpha: 1), 0x8080FFFF)
        let up = ColorMath.normalFromDisc(x: 0, y: -0.6)
        expect(near(up[1], 0.6, 1e-9) && near(up[2], 0.8, 1e-9), "up on the screen is +Y: \(up)")
        let rim = ColorMath.normalFromDisc(x: 4, y: 0)
        expect(near(rim[0], 1, 1e-9) && near(rim[2], 0, 1e-9), "past the rim is the horizon: \(rim)")
        let back = ColorMath.normalDecode(ColorMath.normalEncode(up, alpha: 1))
        for i in 0..<3 { expect(near(back[i], up[i], 0.012), "normal byte round trip \(i): \(back) vs \(up)") }
        let tilted = ColorMath.normalSet([0, 0.9, 0.436], axis: 0, to: 0.8)
        expect(near(tilted[0], 0.8, 1e-6) && near(tilted[1], 0.6, 1e-6), "setting X shrinks Y to stay on the sphere: \(tilted)")
        let z = ColorMath.normalSet([0.6, 0.0, 0.8], axis: 2, to: 0)
        expect(near(z[0], 1, 1e-6) && near(z[2], 0, 1e-6), "Z to zero tilts to the horizon: \(z)")
        let nr = ColorMath.normalPixels(size: n)
        expectEq(Int(nr[3]), 0, "the normal disc is round")
        let c = (Int(nr[(n / 2 * n + n / 2) * 4]), Int(nr[(n / 2 * n + n / 2) * 4 + 2]))
        expect(abs(c.0 - 128) < 6 && c.1 > 250, "flat blue at the center of the normal disc: \(c)")
    }

    scenario("color/selector-edits") {
        // Each selector's edit, on the same ColorSpaces the views bind to.
        var sp = ColorSpaces(rgba: 0x336699FF)
        // RGB sliders: one channel at a time, alpha kept.
        sp.setChannel(0, 255)
        expectEq(sp.rgba, 0xFF6699FF)
        sp.setChannel(1, 0)
        sp.setChannel(2, 300)
        expectEq(sp.rgba, 0xFF00FFFF, "values clamp to a byte")
        sp.setAlpha(0.5)
        sp.setChannel(1, 128)
        expectEq(sp.rgba, 0xFF80FF80, "editing a channel keeps alpha")
        // HSV sliders: the hue survives a trip through gray.
        sp = ColorSpaces(rgba: RED)
        sp.setHSV(120, 1, 1)
        expectEq(sp.rgba, GREEN)
        sp.setHSV(120, 0, 1)
        expectEq(sp.rgba, WHITE)
        sp.setRGBA(sp.rgba)
        expectEq(sp.hsva.h, 120, "the hue is kept at zero saturation")
        sp.setHSV(120, 1, 0)
        expectEq(sp.hsva.s, 1, "the saturation is kept at zero value")
        sp.setHSV(sp.hsva.h, sp.hsva.s, 0.5)
        expectEq(sp.rgba, 0x008000FF)
        // HSL sliders hold their own values even when the RGB bytes round.
        sp = ColorSpaces(rgba: RED)
        sp.setHSL(210, 0.55, 0.37)
        expectEq(sp.hsl, [210, 0.55, 0.37])
        sp.setHSL(210, 0, 0.37)
        expect(bytes(sp.rgba).r == bytes(sp.rgba).g && bytes(sp.rgba).g == bytes(sp.rgba).b, "no saturation is gray")
        sp.setHSL(210, 0.9, 1)
        expectEq(sp.rgba, WHITE, "lightness 1 is white")
        expectEq(sp.hsl[1], 0.9, "the saturation survives white")
        // Gray.
        sp = ColorSpaces(rgba: 0x336699C0)
        sp.setGray(0)
        expectEq(sp.rgba, 0x000000C0)
        sp.setGray(255)
        expectEq(sp.rgba, 0xFFFFFFC0)
        sp.setGray(128)
        expect(bytes(sp.rgba).r == 128 && bytes(sp.rgba).a == 0xC0)
        expectEq(sp.gray, 128)
        // Wheel, HSV model: hue and saturation from the pointer, the value kept.
        sp = ColorSpaces(rgba: 0x808080FF)
        sp.pickOnWheel(.hsv, x: 0.98, y: 0)
        let hsv = ColorMath.hsv(sp.rgba)
        expect((hsv.a < 3 || hsv.a > 357) && hsv.b > 0.95 && near(hsv.c, 0.5, 0.02), "red at the east, V kept: \(hsv)")
        sp.pickOnWheel(.hsv, x: 0, y: -0.5)
        let up = ColorMath.hsv(sp.rgba)
        expect(near(up.a, 90, 2) && near(up.b, 0.5, 0.02), "halfway up is hue 90, half saturation: \(up)")
        sp.pickOnWheel(.hsv, x: 0, y: 4)
        let rim = ColorMath.hsv(sp.rgba)
        expect(near(rim.a, 270, 2) && rim.b > 0.99, "past the rim clamps to it: \(rim)")
        sp.setWheel(.hsv, h: sp.hsva.h, s: sp.hsva.s, v: 0)
        expectEq(sp.rgba, BLACK)
        sp.pickOnWheel(.hsv, x: 0.5, y: 0)
        expectEq(sp.rgba, BLACK, "the wheel at value 0 stays black")
        expect(near(sp.hsva.s, 0.5, 1e-9), "but remembers the pick")
        sp.setWheel(.hsv, h: sp.hsva.h, s: sp.hsva.s, v: 1)
        expect(sp.rgba != BLACK && near(ColorMath.hsv(sp.rgba).b, 0.5, 0.01), "the saturation came back with the value")
        // Wheel, HSL model.
        sp = ColorSpaces(rgba: RED)
        sp.pickOnWheel(.hsl, x: 0.98, y: 0)
        expect(near(sp.hsl[2], 0.5, 0.01) && sp.hsl[1] > 0.95, "the rim at L 0.5 is a pure color: \(sp.hsl)")
        sp.setWheel(.hsl, h: sp.hsl[0], s: sp.hsl[1], v: 1)
        expectEq(sp.rgba, WHITE)
        sp.setWheel(.hsl, h: sp.hsl[0], s: sp.hsl[1], v: 0.25)
        expect(near(sp.wheelTriple(.hsl).v, 0.25, 1e-9))
        // Normal map.
        sp = ColorSpaces(rgba: 0x336699A0)
        sp.pickNormal(x: 0, y: 0)
        let flat = bytes(sp.rgba)
        expect(abs(flat.r - 128) <= 2 && abs(flat.g - 128) <= 2 && flat.b >= 253 && flat.a == 0xA0, "flat normal keeps alpha: \(hex(sp.rgba))")
        sp.pickNormal(x: 0.9, y: 0)
        expect(near(sp.normal[0], 0.9, 0.03) && abs(sp.normal[1]) < 0.03, "right is +X: \(sp.normal)")
        sp.pickNormal(x: 0, y: -0.6)
        expect(near(sp.normal[1], 0.6, 0.03) && near(sp.normal[2], 0.8, 0.03), "up is +Y: \(sp.normal)")
        sp.pickNormal(x: -1.5, y: 0)
        expect(sp.normal[0] < -0.98 && sp.normal[2] < 0.1, "past the rim is the horizon: \(sp.normal)")
        sp.setNormal([0, 0, 1])
        expect(sp.normal[2] > 0.99, "the Flat button")
        sp.setNormal(ColorMath.normalSet(sp.normal, axis: 2, to: 0.5))
        expect(near(sp.normal[2], 0.5, 0.012), "Z field: \(sp.normal)")
        // A color that is not a normal still shows some normal.
        sp.setRGBA(0xFF8000FF)
        expect(sp.normal[2] == 0 && near(sp.normal.map { $0 * $0 }.reduce(0, +), 1, 1e-9), "lifted to the horizon: \(sp.normal)")
    }

    scenario("color/selector-writes-the-model") {
        // The selectors write through SpriteModel.color / opts.secondary, which snap to the color mode.
        let m = fresh()
        var sp = ColorSpaces(rgba: m.color)
        sp.setChannel(0, 200)
        m.color = sp.rgba
        expectEq(m.color, 0xC80000FF)
        m.opts.secondary = ColorSpaces(rgba: m.opts.secondary).rgba
        // Indexed: the primary snaps to a palette color, the spectrum follows it.
        m.convertColorMode(to: .indexed)
        let palette = m.palette
        expect(!palette.isEmpty)
        sp.setChannel(1, 120)
        m.color = sp.rgba
        expect(palette.contains(m.color), "\(hex(m.color)) is a palette color in indexed mode")
        sp.setRGBA(m.color)
        expectEq(sp.rgba, m.color, "the selectors follow the snapped color")
        m.convertColorMode(to: .rgb)
    }

    scenario("color/selector-prefs") {
        let suite = UserDefaults(suiteName: "pixmith-qa-colorpicker-\(ProcessInfo.processInfo.processIdentifier)")!
        defer { suite.removePersistentDomain(forName: "pixmith-qa-colorpicker-\(ProcessInfo.processInfo.processIdentifier)") }
        var p = ColorPickerPrefs(defaults: suite)
        expect(p.selector == .spectrum && p.wheelModel == .hsv && p.harmony == .none, "defaults")
        p.selector = .normal
        p.wheelModel = .hsl
        p.harmony = .splitComplementary
        p = ColorPickerPrefs(defaults: suite)
        expect(p.selector == .normal && p.wheelModel == .hsl && p.harmony == .splitComplementary, "restored")
        suite.set("bogus", forKey: "colorPicker.selector")
        suite.set(99, forKey: "colorPicker.harmony")
        p = ColorPickerPrefs(defaults: suite)
        expect(p.selector == .spectrum && p.harmony == .none, "unknown values fall back")
        expectEq(ColorSelector.allCases.count, 7)
    }

    scenario("color/selector-screens") {
        // Every selector in the real window, both appearances; the inspector column is saved for comparison with the egui
        // app. Each selector must look different from the others, and the color must not change by showing one.
        let m = fresh(16, 16)
        m.color = 0xF2A65AC0
        m.opts.secondary = 0x2B3566FF
        let prefs = ColorPickerPrefs.shared
        let saved = (prefs.selector, prefs.wheelModel, prefs.harmony)
        defer { (prefs.selector, prefs.wheelModel, prefs.harmony) = saved }
        for dark in [true, false] {
            let app = AppDriver(m, size: NSSize(width: 1180, height: 760), dark: dark)
            var hashes = Set<Int>()
            for selector in ColorSelector.allCases {
                prefs.selector = selector
                if selector == .wheel { prefs.harmony = .triadic }
                if selector == .normal { m.color = 0xA05CF0C0 }
                let rep = app.snapshot("window-color-\(selector.rawValue)")
                expectEq(m.color, selector == .normal ? 0xA05CF0C0 : 0xF2A65AC0, "showing \(selector) leaves the color alone")
                let name = "color-\(selector.rawValue)-\(dark ? "dark" : "light")"
                if let crop = saveInspector(rep, name: name) { hashes.insert(pixelHash(crop)) } else { expect(false, "no inspector crop") }
                prefs.harmony = .none
                m.color = 0xF2A65AC0
            }
            expectEq(hashes.count, ColorSelector.allCases.count, "every selector renders differently")
            prefs.selector = .wheel
            prefs.wheelModel = .hsl
            prefs.harmony = .analogous
            m.color = 0x4F8FE0FF
            saveInspector(app.snapshot("window-color-wheel-hsl"), name: "color-wheel-hsl-\(dark ? "dark" : "light")")
            prefs.wheelModel = .hsv
            prefs.harmony = .none
            m.color = 0xF2A65AC0
        }
    }
}
