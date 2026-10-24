import AppKit
import ImageIO
@testable import PixmithKit
import PixmithCore
import SwiftUI

// WebP, JPEG, BMP, TGA, ICO, PCX and FLI/FLC: export, open as an import, the icon size question, damaged files and the
// Export sheet. macOS's own image decoders (ImageIO) check the files independently of the Rust core.

/// 8x6, three frames (60/90/120 ms), transparent background, red / blue / green / yellow pixels.
private func animation() -> SpriteModel {
    let m = fresh(8, 6), d = CanvasDriver()
    put(d, 1, 1, RED)
    put(d, 6, 4, BLUE)
    m.addFrame(duplicate: true)
    put(d, 3, 3, GREEN)
    m.addFrame(duplicate: true)
    put(d, 4, 2, 0xFFFF00FF)
    for (f, ms) in [60, 90, 120].enumerated() {
        m.frame = f
        m.setDuration(ms)
    }
    m.frame = 0
    return m
}

/// The pixels with transparent ones replaced by `bg` (0xRRGGBB), like the formats without alpha do.
private func flattened(_ p: [UInt32], _ bg: UInt32) -> [UInt32] { p.map { $0 & 0xFF == 0 ? (bg << 8 | 0xFF) : $0 } }

/// RGBA pixels of an image file as decoded by ImageIO (straight alpha, 0xRRGGBBAA).
private func imageIOPixels(_ url: URL, index: Int = 0) -> (w: Int, h: Int, px: [UInt32])? {
    guard let src = CGImageSourceCreateWithURL(url as CFURL, nil), let img = CGImageSourceCreateImageAtIndex(src, index, nil) else { return nil }
    let w = img.width, h = img.height
    var buf = [UInt8](repeating: 0, count: w * h * 4)
    let cs = CGColorSpaceCreateDeviceRGB()
    guard let ctx = CGContext(data: &buf, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4, space: cs,
                              bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
    ctx.draw(img, in: CGRect(x: 0, y: 0, width: w, height: h))
    var out = [UInt32](repeating: 0, count: w * h)
    for i in 0..<w * h {
        let a = UInt32(buf[i * 4 + 3])
        func un(_ v: UInt8) -> UInt32 { a == 0 ? 0 : min(255, (UInt32(v) * 255 + a / 2) / a) }
        out[i] = un(buf[i * 4]) << 24 | un(buf[i * 4 + 1]) << 16 | un(buf[i * 4 + 2]) << 8 | a
    }
    return (w, h, out)
}

private func channelDiff(_ a: UInt32, _ b: UInt32) -> Int {
    (0..<3).map { abs(Int((a >> (24 - 8 * UInt32($0))) & 255) - Int((b >> (24 - 8 * UInt32($0))) & 255)) }.max() ?? 0
}

func registerImageFormats() {
    scenario("formats/export-then-open-every-format") {
        let m = animation()
        let frame0 = m.pixels(frame: 0)
        // Lossless and indexed formats come back as they went out.
        for ext in ["bmp", "tga", "webp", "ico"] {
            let url = tmp("lossless.\(ext)")
            expect(m.exportImage(to: url), "export \(ext): \(coreError)")
            guard let back = m.readImage(url) else { QA.fail("open \(ext): \(coreError)", #fileID, #line); continue }
            expectEq(Int(pixmith_width(back)), 8, "\(ext) width")
            expectEq(Int(pixmith_height(back)), 6, "\(ext) height")
            let got = renderPixels(back, frame: 0)
            expectEq(got.map { $0 & 0xFF == 0 ? 0 : $0 }, frame0.map { $0 & 0xFF == 0 ? 0 : $0 }, "\(ext) pixels")
            expectEq(got.map { $0 & 0xFF }, frame0.map { $0 & 0xFF }, "\(ext) alpha")
            pixmith_free(back)
        }
        // Formats without transparency flatten onto the chosen background.
        var o = ImageExportOptions()
        o.background = "#336699"
        for ext in ["pcx", "flc"] {
            let url = tmp("flat.\(ext)")
            expect(m.exportImage(to: url, options: o), "export \(ext): \(coreError)")
            guard let back = m.readImage(url) else { QA.fail("open \(ext): \(coreError)", #fileID, #line); continue }
            expectEq(renderPixels(back, frame: 0), flattened(frame0, 0x336699), "\(ext) pixels on the background")
            pixmith_free(back)
        }
        // JPEG: close, never exact, flattened.
        o.quality = 100
        let jpg = tmp("flat.jpg")
        expect(m.exportImage(to: jpg, options: o), "export jpg: \(coreError)")
        if let back = m.readImage(jpg) {
            let want = flattened(frame0, 0x336699), got = renderPixels(back, frame: 0)
            let worst = zip(got, want).map { channelDiff($0, $1) }.max() ?? 0
            expect(worst <= 40, "jpeg stays close to the original (worst channel error \(worst))")
            expect(got.allSatisfy { $0 & 0xFF == 0xFF }, "jpeg is opaque")
            pixmith_free(back)
        } else { QA.fail("open jpg: \(coreError)", #fileID, #line) }
    }

    scenario("formats/animations-keep-frames-and-durations") {
        let m = animation()
        for ext in ["webp", "flc"] {
            let url = tmp("anim.\(ext)")
            expect(m.exportImage(to: url, allFrames: true), "export \(ext): \(coreError)")
            guard let back = m.readImage(url) else { QA.fail("open \(ext): \(coreError)", #fileID, #line); continue }
            expectEq(Int(pixmith_frame_count(back)), 3, "\(ext) frame count")
            expectEq((0..<3).map { Int(pixmith_frame_duration(back, UInt32($0))) }, [60, 90, 120], "\(ext) durations")
            for f in 0..<3 {
                let want = ext == "flc" ? flattened(m.pixels(frame: f), 0xFFFFFF) : m.pixels(frame: f)
                expectEq(renderPixels(back, frame: f).map { $0 & 0xFF == 0 ? 0 : $0 }, want.map { $0 & 0xFF == 0 ? 0 : $0 }, "\(ext) frame \(f + 1)")
            }
            pixmith_free(back)
        }
        // One frame only when the animation option is off.
        var o = ImageExportOptions()
        o.animated = false
        let still = tmp("still.webp")
        m.frame = 2
        expect(m.exportImage(to: still, options: o), "export still webp: \(coreError)")
        if let back = m.readImage(still) {
            expectEq(Int(pixmith_frame_count(back)), 1, "single frame")
            expectEq(renderPixels(back, frame: 0).map { $0 & 0xFF == 0 ? 0 : $0 }, m.pixels(frame: 2).map { $0 & 0xFF == 0 ? 0 : $0 }, "the current frame")
            pixmith_free(back)
        } else { QA.fail("open still webp: \(coreError)", #fileID, #line) }
    }

    scenario("formats/macos-image-io-reads-our-files") {
        let m = animation()
        let want = m.pixels(frame: 0)
        for ext in ["bmp", "webp", "ico", "tga"] {
            let url = tmp("io.\(ext)")
            expect(m.exportImage(to: url), "export \(ext): \(coreError)")
            if ext == "ico" { continue }
            guard let r = imageIOPixels(url) else { QA.fail("ImageIO cannot read the \(ext) file", #fileID, #line); continue }
            expectEq(r.w, 8, "\(ext) width as ImageIO sees it")
            expectEq(r.h, 6, "\(ext) height as ImageIO sees it")
            expectEq(r.px.map { $0 & 0xFF == 0 ? 0 : $0 }, want.map { $0 & 0xFF == 0 ? 0 : $0 }, "\(ext) pixels as ImageIO sees them")
            expectEq(r.px.map { $0 & 0xFF }, want.map { $0 & 0xFF }, "\(ext) alpha as ImageIO sees it")
        }
        // ImageIO reads icons from 16 pixels up: the 16 pixel image is the sprite at 2x, centered in the square.
        var ico = ImageExportOptions()
        ico.icoSizes = [16, 32]
        let icoURL = tmp("io.ico")
        expect(m.exportImage(to: icoURL, options: ico), "export ico: \(coreError)")
        if let r = imageIOPixels(icoURL) {
            expect(r.w == 16 || r.w == 32, "ImageIO opens an image of the icon (\(r.w) pixels)")
            let k = r.w / 8, oy = (r.w - 6 * k) / 2
            expectEq(r.px[(oy + 1 * k) * r.w + 1 * k], want[1 * 8 + 1], "the sprite's red pixel inside the icon")
            expectEq(r.px[0] & 0xFF, 0, "transparent margin")
        } else { QA.fail("ImageIO cannot read the ico file", #fileID, #line) }
        var o = ImageExportOptions()
        o.quality = 95
        let jpg = tmp("io.jpg")
        expect(m.exportImage(to: jpg, options: o), "export jpg: \(coreError)")
        if let r = imageIOPixels(jpg) {
            expectEq(r.w * r.h, 48)
            let worst = zip(r.px, flattened(want, 0xFFFFFF)).map { channelDiff($0, $1) }.max() ?? 0
            expect(worst <= 60, "ImageIO's JPEG decode is close (worst \(worst))")
        } else { QA.fail("ImageIO cannot read the jpg file", #fileID, #line) }
        // An animated WebP is one image with three frames for ImageIO.
        let anim = tmp("io-anim.webp")
        expect(m.exportImage(to: anim, allFrames: true), "export animated webp: \(coreError)")
        if let src = CGImageSourceCreateWithURL(anim as CFURL, nil) {
            expectEq(CGImageSourceGetCount(src), 3, "animated WebP frames as ImageIO counts them")
        } else { QA.fail("ImageIO cannot read the animated webp", #fileID, #line) }
    }

    scenario("formats/open-routes-every-extension-to-an-import") {
        let m = animation()
        var made: [(String, URL)] = []
        for ext in ["webp", "jpg", "bmp", "tga", "ico", "pcx", "flc"] {
            let url = tmp("open.\(ext)")
            expect(m.exportImage(to: url, allFrames: true), "export \(ext): \(coreError)")
            made.append((ext, url))
        }
        // Rename copies to the other spellings: detection is by content, the extension only routes.
        try? FileManager.default.copyItem(at: tmp("open.jpg"), to: tmp("open.jpeg"))
        try? FileManager.default.copyItem(at: tmp("open.flc"), to: tmp("open.fli"))
        made.append(("jpeg", tmp("open.jpeg")))
        made.append(("fli", tmp("open.fli")))
        for (ext, url) in made {
            let tabs = m.slots.count
            m.errorMessage = nil
            m.open(url)
            expect(m.errorMessage == nil, "open .\(ext): \(m.errorMessage ?? "")")
            expect(m.slots.count == tabs + 1 || m.slots.count == tabs, "open .\(ext) keeps the tab list sane")
            expectEq(m.width, ext == "ico" ? 8 : 8, ".\(ext) width")
            expectEq(m.height, 6, ".\(ext) height")
            expect(m.fileURL == nil, ".\(ext) opens as an unsaved sprite, not as a file of that format")
            expect(m.isDirty, ".\(ext) counts as unsaved")
            expectEq(m.frameCount, ext == "webp" || ext == "flc" || ext == "fli" ? 3 : 1, ".\(ext) frames")
        }
        expect(ImageFormats.isImage(URL(fileURLWithPath: "/x/pic.WEBP")), "extension check ignores case")
        expect(!ImageFormats.isImage(URL(fileURLWithPath: "/x/pic.png")), "PNG keeps its own path")
        expect(!ImageFormats.openTypes.isEmpty, "the Open panel offers the image types")
    }

    scenario("formats/icon-with-several-sizes-asks-which-to-open") {
        let m = animation()
        var o = ImageExportOptions()
        o.icoSizes = [16, 32, 48]
        let url = tmp("sizes.ico")
        expect(m.exportImage(to: url, options: o), "export ico: \(coreError)")
        expectEq(SpriteModel.iconSizes(url) ?? [], [16, 32, 48], "the sizes the file reports")
        let original = SpriteModel.alertRunner
        defer { SpriteModel.alertRunner = original }
        var offered: [Int] = []
        var calls = 0
        SpriteModel.alertRunner = { alert in
            calls += 1
            guard let popup = alert.accessoryView as? NSPopUpButton else { return .alertSecondButtonReturn }
            offered = popup.itemArray.map(\.tag)
            popup.selectItem(withTag: 32)
            return .alertFirstButtonReturn
        }
        m.open(url)
        expectEq(calls, 1, "one question")
        expectEq(offered, [48, 32, 16], "largest first")
        expectEq(m.width, 32, "the chosen size opens")
        expectEq(m.height, 32)
        // Cancel opens nothing.
        let before = m.slots.count, width = m.width
        SpriteModel.alertRunner = { _ in .alertSecondButtonReturn }
        m.open(url)
        expectEq(m.slots.count, before, "cancel opens nothing")
        expectEq(m.width, width)
        // A single-size icon opens without a question.
        let m2 = animation()
        let single = tmp("single.ico")
        expect(m2.exportImage(to: single), "export single-size ico: \(coreError)")
        calls = 0
        SpriteModel.alertRunner = { _ in calls += 1; return .alertSecondButtonReturn }
        m2.open(single)
        expectEq(calls, 0, "no question for one size")
        expectEq(m2.width, 8)
    }

    scenario("formats/damaged-files-report-an-error") {
        let m = animation()
        var good: [String: Data] = [:]
        for ext in ["webp", "jpg", "bmp", "tga", "ico", "pcx", "flc"] {
            let url = tmp("good.\(ext)")
            expect(m.exportImage(to: url, allFrames: true), "export \(ext): \(coreError)")
            good[ext] = try? Data(contentsOf: url)
        }
        let tabs = m.slots.count
        var failures = 0
        for (ext, data) in good {
            // Truncated to half, and garbage after the first 20 bytes.
            for (kind, bytes) in [("half", data.prefix(data.count / 2)), ("garbage", data.prefix(20) + Data((0..<300).map { UInt8(($0 * 37 + 11) & 255) }))] {
                let url = tmp("bad-\(kind).\(ext)")
                try? Data(bytes).write(to: url)
                m.errorMessage = nil
                m.open(url)
                if m.errorMessage != nil { failures += 1 }
                m.errorMessage = nil
            }
        }
        expect(failures >= 10, "damaged files are reported as errors (\(failures) of 14)")
        expect(m.slots.count <= tabs + 4, "a damaged file never opens more than a partial image")
        // Plain text with an image extension.
        try? Data("not an image".utf8).write(to: tmp("text.bmp"))
        m.errorMessage = nil
        let count = m.slots.count
        m.open(tmp("text.bmp"))
        expect(m.errorMessage != nil, "text named .bmp is an error: \(m.errorMessage ?? "-")")
        expectEq(m.slots.count, count, "and opens nothing")
        m.errorMessage = nil
    }

    scenario("formats/unified-export-with-tags-layers-and-options") {
        let m = animation()
        var t = m.newTagDraft()
        t.name = "walk"; t.from = 0; t.to = 1; t.direction = 0
        m.addTag(t)
        // A tag of two frames as an animated WebP at 2x, through the entry point the Export sheet uses.
        var e = PixmithExport()
        e.format = 5; e.scale = 2; e.tag = 0; e.apply_direction = true; e.layers = 0
        var path = tmp("tag.webp").path
        var n = path.withCString { p in ImageExportOptions().json.withCString { pixmith_export_run_ex(m.doc, &e, $0, p) } }
        expectEq(Int(n), 1, "one file: \(coreError)")
        if let back = m.readImage(URL(fileURLWithPath: path)) {
            expectEq(Int(pixmith_frame_count(back)), 2, "the tag's frames")
            expectEq(Int(pixmith_width(back)), 16, "scale 2")
            pixmith_free(back)
        } else { QA.fail("open tag webp: \(coreError)", #fileID, #line) }
        // A still format writes the frame of the export that the options name.
        e.format = 7; e.frame = 1; e.scale = 1
        path = tmp("tag.bmp").path
        n = path.withCString { p in ImageExportOptions().json.withCString { pixmith_export_run_ex(m.doc, &e, $0, p) } }
        expectEq(Int(n), 1, "bmp written: \(coreError)")
        if let back = m.readImage(URL(fileURLWithPath: path)) {
            expectEq(renderPixels(back, frame: 0).filter { $0 & 0xFF != 0 }.count, m.pixels(frame: 1).filter { $0 & 0xFF != 0 }.count, "frame 2 of the tag")
            pixmith_free(back)
        }
        // Options that make no sense come back as an error and write nothing.
        var bad = ImageExportOptions()
        bad.bits = 5
        path = tmp("never.bmp").path
        n = path.withCString { p in bad.json.withCString { pixmith_export_run_ex(m.doc, &e, $0, p) } }
        expectEq(Int(n), -1, "bits 5 is refused")
        expect(!FileManager.default.fileExists(atPath: path), "nothing written")
        // The settings survive a save and a load.
        var s = ImageExportOptions()
        s.quality = 33; s.lossless = false; s.icoSizes = [64, 16]; s.background = "#102030"; s.bits = 24; s.rle = false
        let key = URL(fileURLWithPath: "/qa/formats-settings.pxmt")
        s.save(for: key)
        expectEq(ImageExportOptions.load(for: key), s, "settings round trip")
        expect(s.json.contains("\"ico_sizes\":[16,64]"), "sizes sorted in the JSON: \(s.json)")
        UserDefaults.standard.removeObject(forKey: ImageExportOptions.key(for: key))
        expectEq(Color(hexString: "#336699").hexString, "#336699", "color hex round trip")
    }

    scenario("formats/export-sheet-renders") {
        let m = animation()
        for dark in [true, false] {
            let suffix = dark ? "dark" : "light"
            for (format, name) in [(5, "webp"), (6, "jpeg"), (8, "tga"), (9, "ico"), (11, "flc")] {
                let sheet = ExportOptionsSheet(model: m, dismiss: {}, initialFormat: format)
                    .environment(\.colorScheme, dark ? .dark : .light)
                let host = NSHostingView(rootView: sheet)
                let size = host.fittingSize
                let win = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: [.titled], backing: .buffered, defer: false)
                win.isReleasedWhenClosed = false
                win.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
                win.contentView = host
                pumpRunLoop(0.15)
                host.layoutSubtreeIfNeeded()
                host.displayIfNeeded()
                expect(size.width >= 400 && size.height >= 300, "\(name) sheet has a size (\(size))")
                if let rep = host.bitmapImageRepForCachingDisplay(in: host.bounds) {
                    host.cacheDisplay(in: host.bounds, to: rep)
                    try? FileManager.default.createDirectory(atPath: QA.outDir, withIntermediateDirectories: true)
                    try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: QA.outDir).appendingPathComponent("export-\(name)-\(suffix).png"))
                    var colors = Set<UInt32>()
                    for y in stride(from: 0, to: rep.pixelsHigh, by: 7) {
                        for x in stride(from: 0, to: rep.pixelsWide, by: 7) {
                            if let c = rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) {
                                colors.insert(UInt32(c.redComponent * 255) << 16 | UInt32(c.greenComponent * 255) << 8 | UInt32(c.blueComponent * 255))
                            }
                        }
                    }
                    expect(colors.count > 4, "\(name) sheet \(suffix) is drawn (\(colors.count) colors)")
                }
            }
        }
    }
}
