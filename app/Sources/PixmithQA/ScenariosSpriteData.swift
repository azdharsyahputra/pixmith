import AppKit
import SwiftUI
@testable import PixmithKit
import PixmithCore

// Color profile, cel z-index and reference layers through the model the way the dialogs and menus use them.

/// `core/tests/fixtures/aseprite`, found from this file's location.
private let fixtureDir = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    .appendingPathComponent("core/tests/fixtures/aseprite")

/// Writes a solid-color PNG with CoreGraphics.
private func writePNG(_ url: URL, width: Int, height: Int, rgba: UInt32) {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height, bitsPerSample: 8,
                               samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
                               bytesPerRow: width * 4, bitsPerPixel: 32)!
    let r = UInt8(rgba >> 24), g = UInt8(rgba >> 16 & 0xFF), b = UInt8(rgba >> 8 & 0xFF), a = UInt8(rgba & 0xFF)
    for y in 0..<height {
        for x in 0..<width { rep.setColor(NSColor(deviceRed: CGFloat(r) / 255, green: CGFloat(g) / 255, blue: CGFloat(b) / 255, alpha: CGFloat(a) / 255), atX: x, y: y) }
    }
    try? rep.representation(using: .png, properties: [:])?.write(to: url)
}

/// Renders a SwiftUI view in an offscreen window with the given appearance and saves it as `name.png`.
@discardableResult
private func snapshotView<V: View>(_ view: V, name: String, dark: Bool, size: NSSize) -> NSBitmapImageRep {
    let host = NSHostingView(rootView: view)
    let window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: [.titled], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
    let backdrop = Backdrop()
    backdrop.frame = NSRect(origin: .zero, size: size)
    host.frame = backdrop.bounds
    host.autoresizingMask = [.width, .height]
    backdrop.addSubview(host)
    window.contentView = backdrop
    pumpRunLoop(0.2)
    window.layoutIfNeeded()
    backdrop.displayIfNeeded()
    let rep = backdrop.bitmapImageRepForCachingDisplay(in: backdrop.bounds)!
    backdrop.cacheDisplay(in: backdrop.bounds, to: rep)
    try? FileManager.default.createDirectory(atPath: QA.outDir, withIntermediateDirectories: true)
    try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: QA.outDir).appendingPathComponent("\(name).png"))
    return rep
}

func registerSpriteData() {
    scenario("spritedata/color-profile") {
        let m = fresh()
        expectEq(m.colorProfileKind, 0, "a new sprite has no profile")
        expectEq(m.colorProfileLabel, "None")
        // A real file with an embedded ICC profile.
        m.open(fixtureDir.appendingPathComponent("ase-go/prop.ase"))
        expectEq(m.colorProfileKind, 2)
        expectEq(m.colorProfileLabel, "Embedded ICC (Display P3)")
        let before = m.pixels()
        // Saving keeps the profile in both formats, and pixels are never converted.
        for ext in ["aseprite", "pxmt"] {
            let url = tmp("profile.\(ext)")
            m.fileURL = url
            m.save()
            m.open(url)
            expectEq(m.colorProfileLabel, "Embedded ICC (Display P3)", ".\(ext) keeps the ICC profile")
            expectEq(m.pixels(), before, ".\(ext) pixels")
        }
        // Assign sRGB, remove, undo each; the profile never touches the pixels.
        m.setColorProfile(1)
        expectEq(m.colorProfileLabel, "sRGB")
        expectEq(m.pixels(), before, "assigning sRGB does not convert pixels")
        m.setColorProfile(0)
        expectEq(m.colorProfileLabel, "None")
        m.undo()
        expectEq(m.colorProfileLabel, "sRGB")
        m.undo()
        expectEq(m.colorProfileLabel, "Embedded ICC (Display P3)")
        m.redo()
        m.redo()
        expectEq(m.colorProfileKind, 0)
        for (kind, label) in [(1, "sRGB"), (0, "None")] {
            m.setColorProfile(kind)
            let url = tmp("profile2.aseprite")
            m.fileURL = url
            m.save()
            m.open(url)
            expectEq(m.colorProfileLabel, label, "saved and reopened")
        }
        // The dialog shows the label.
        m.setColorProfile(1)
        _ = snapshotView(SpritePropertiesSheet(model: m), name: "spritedata-sprite-properties-light", dark: false, size: NSSize(width: 460, height: 340))
        _ = snapshotView(SpritePropertiesSheet(model: m), name: "spritedata-sprite-properties-dark", dark: true, size: NSSize(width: 460, height: 340))
    }

    scenario("spritedata/cel-z-index") {
        let m = fresh(4, 4), d = CanvasDriver()
        m.color = RED
        put(d, 0, 0, RED)
        m.addLayer()
        put(d, 0, 0, GREEN)
        expectEq(m.pixel(0, 0), GREEN, "the upper layer draws on top")
        m.layer = 0
        expectEq(m.celZIndex, 0)
        m.setCelZIndex(1)
        expectEq(m.pixel(0, 0), GREEN, "an equal order keeps the lower layer first")
        m.setCelZIndex(2)
        expectEq(m.celZIndex, 2)
        expectEq(m.pixel(0, 0), RED, "the red cel now draws above the green one")
        m.undo()
        expectEq(m.celZIndex, 1)
        m.redo()
        expectEq(m.pixel(0, 0), RED)
        m.setCelZIndex(-5)
        expectEq(m.pixel(0, 0), GREEN)
        m.setCelZIndex(2)
        // Round trips through both file formats and the cel dialog.
        for ext in ["aseprite", "pxmt"] {
            let url = tmp("zindex.\(ext)")
            m.fileURL = url
            m.save()
            m.open(url)
            m.layer = 0
            expectEq(m.celZIndex, 2, ".\(ext) keeps the z-index")
            expectEq(m.pixel(0, 0), RED, ".\(ext) renders in the same order")
        }
        // An empty cel has no z-index.
        m.addLayer()
        expectEq(m.celZIndex, nil)
        m.setCelZIndex(3)
        expectEq(m.celZIndex, nil, "an empty cel refuses the value")
        m.layer = 0
        _ = snapshotView(CelSheet(model: m), name: "spritedata-cel-properties-light", dark: false, size: NSSize(width: 420, height: 440))
        _ = snapshotView(CelSheet(model: m), name: "spritedata-cel-properties-dark", dark: true, size: NSSize(width: 420, height: 440))
    }

    scenario("spritedata/reference-layers") {
        let m = fresh(8, 8), d = CanvasDriver()
        put(d, 0, 0, RED)
        let png = tmp("ref-wide.png")
        writePNG(png, width: 16, height: 4, rgba: BLUE)
        m.addReferenceLayer(from: png)
        expectEq(m.layerCount, 2)
        expect(m.isReference(1), "the new layer is a reference layer")
        expect(m.isLocked(1), "and locked")
        expectEq(m.layerName(1), "ref-wide")
        expectEq(m.layer, 1)
        // Scaled to fit: 8 wide, 2 high, centered (rows 3 and 4).
        expectEq(m.pixel(0, 3), BLUE)
        expectEq(m.pixel(7, 4), BLUE)
        expectEq(m.pixel(0, 2), CLEAR)
        expectEq(m.pixel(0, 5), CLEAR)
        expectEq(m.pixel(0, 0), RED, "the normal layer below is still visible")
        // The editor can hide it or draw it semi-transparent; this is not an edit.
        let undo = m.historyCounts.undo
        m.referenceDisplay = 50
        let half = m.pixel(0, 3)
        expect(half & 0xFF == 0x7F || half & 0xFF == 0x80, "half opacity: \(hex(half))")
        m.referenceDisplay = 0
        expectEq(m.pixel(0, 3), CLEAR, "hidden")
        m.referenceDisplay = 100
        expectEq(m.historyCounts.undo, undo, "the display setting is not an undo step")
        // Exports leave it out.
        let base = tmp("ref-export/frame")
        try? FileManager.default.createDirectory(at: base.deletingLastPathComponent(), withIntermediateDirectories: true)
        expect(base.path.withCString { pixmith_export_png_sequence(m.doc, 1, $0) }, "export")
        let files = (try? FileManager.default.contentsOfDirectory(atPath: base.deletingLastPathComponent().path)) ?? []
        expect(!files.isEmpty, "the sequence was written")
        for f in files {
            let rep = NSBitmapImageRep(data: (try? Data(contentsOf: base.deletingLastPathComponent().appendingPathComponent(f))) ?? Data())
            expect(rep != nil, f)
            var blue = 0
            for y in 0..<(rep?.pixelsHigh ?? 0) {
                for x in 0..<(rep?.pixelsWide ?? 0) {
                    if let c = rep?.colorAt(x: x, y: y), c.alphaComponent > 0, c.blueComponent > 0.9, c.redComponent < 0.1 { blue += 1 }
                }
            }
            expectEq(blue, 0, "\(f) has no reference pixels")
        }
        // Locked: drawing is refused; unlocked: it works.
        m.color = GREEN
        m.layer = 1
        d.click(3, 6)
        expectEq(m.pixel(3, 6), CLEAR, "the locked layer refuses drawing")
        m.setLocked(1, false)
        d.click(3, 6)
        expectEq(m.pixel(3, 6), GREEN)
        m.undo()
        m.setLocked(1, true)
        // Both file formats keep the flag.
        for ext in ["aseprite", "pxmt"] {
            let url = tmp("reference.\(ext)")
            m.fileURL = url
            m.save()
            m.open(url)
            expect(m.isReference(1) && m.isLocked(1) && !m.isReference(0), ".\(ext) keeps the reference layer")
            expectEq(m.pixel(0, 3), BLUE, ".\(ext) shows it in the editor")
        }
        // Toggle off and on, undoable.
        m.setReference(1, false)
        expect(!m.isReference(1))
        m.undo()
        expect(m.isReference(1))
        // New Reference Layer from a non-PNG image goes through the system decoder.
        let jpg = tmp("ref-photo.jpg")
        if let tiff = NSImage(contentsOf: png)?.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff),
           let data = rep.representation(using: .jpeg, properties: [:]) {
            try? data.write(to: jpg)
            let n = m.layerCount
            m.addReferenceLayer(from: jpg)
            expectEq(m.layerCount, n + 1, "a JPEG is accepted")
            expect(m.isReference(m.layer))
        }
        // The timeline row and the layer dialog show it.
        m.layer = 1
        let app = AppDriver(dark: false)
        app.snapshot("spritedata-timeline-light")
        let appDark = AppDriver(dark: true)
        appDark.snapshot("spritedata-timeline-dark")
        _ = snapshotView(LayerPropertiesSheet(model: m, layer: 1), name: "spritedata-layer-properties-light", dark: false, size: NSSize(width: 400, height: 420))
        _ = snapshotView(LayerPropertiesSheet(model: m, layer: 1), name: "spritedata-layer-properties-dark", dark: true, size: NSSize(width: 400, height: 420))
    }
}
