import AppKit
import SwiftUI
@testable import PixmithKit
import PixmithCore

/// The whole SwiftUI window (ContentView) in an offscreen NSWindow.
/// Paints the window background, which an NSHostingView does not draw itself.
final class Backdrop: NSView {
    override func draw(_ dirtyRect: NSRect) {
        NSColor.windowBackgroundColor.setFill()
        dirtyRect.fill()
    }
}

final class AppDriver {
    let window: NSWindow
    let host: NSHostingView<ContentView>
    let container = Backdrop()

    init(_ model: SpriteModel = .shared, size: NSSize = NSSize(width: 1180, height: 760)) {
        host = NSHostingView(rootView: ContentView(model: model))
        window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        container.frame = NSRect(origin: .zero, size: size)
        host.frame = container.bounds
        host.autoresizingMask = [.width, .height]
        container.addSubview(host)
        window.contentView = container
        settle()
    }

    /// Lets SwiftUI process pending state changes and lays the window out.
    func settle(_ seconds: Double = 0.15) {
        pumpRunLoop(seconds)
        window.layoutIfNeeded()
        host.layoutSubtreeIfNeeded()
        pumpRunLoop(0.05)
    }

    @discardableResult
    func snapshot(_ name: String) -> NSBitmapImageRep {
        settle()
        container.displayIfNeeded()
        let rep = container.bitmapImageRepForCachingDisplay(in: container.bounds)!
        container.cacheDisplay(in: container.bounds, to: rep)
        try? FileManager.default.createDirectory(atPath: QA.outDir, withIntermediateDirectories: true)
        if let png = rep.representation(using: .png, properties: [:]) {
            try? png.write(to: URL(fileURLWithPath: QA.outDir).appendingPathComponent("\(name).png"))
        }
        return rep
    }

    /// Every view of the given type in the hierarchy.
    func views<V: NSView>(_ type: V.Type) -> [V] { allViews(host).compactMap { $0 as? V } }
}

func registerApp() {
    scenario("app/window-renders") {
        let m = fresh(24, 16)
        let c = CanvasDriver()
        m.zoom = 12
        m.color = RED
        c.stroke([(2, 2), (12, 2), (12, 10)])
        m.addLayer()
        put(c, 5, 5, BLUE)
        m.wrapInGroup(1)
        m.addFrame(duplicate: true)
        put(c, 8, 8, GREEN)
        m.addFrame(duplicate: true)
        var t = m.newTagDraft()
        t.name = "walk"; t.from = 0; t.to = 1
        m.addTag(t)
        t.name = "idle"; t.from = 1; t.to = 2; t.direction = 2; t.color = 0xAA3366
        m.addTag(t)
        m.onionSkin = true
        m.linkCels(from: 1, to: 2)
        let app = AppDriver()
        let rep = app.snapshot("app-window")
        expect(rep.pixelsWide > 1000, "window rendered")
        // The canvas inside the window is the real canvas view; the status bar, tools and panels exist.
        expect(!app.views(CanvasNSView.self).isEmpty, "the canvas view is part of the window")
        expect(!app.views(CanvasScrollView.self).isEmpty)
        // Interact with the canvas view inside the full window.
        if let canvas = app.views(CanvasNSView.self).first {
            expect(canvas.model === m)
        }
        // Changing the model updates the window without crashing, in every kind of document state.
        m.tool = .marquee
        m.selectAll()
        app.snapshot("app-window-selected")
        m.deselect()
        m.newTilemapLayer(tileWidth: 8, tileHeight: 8)
        app.snapshot("app-window-tilemap")
        m.convertToNormal()
        m.convertColorMode(to: .indexed)
        app.snapshot("app-window-indexed")
        m.convertColorMode(to: .grayscale)
        app.snapshot("app-window-grayscale")
        m.tool = .text
        m.frame = 1
        app.snapshot("app-window-frame2")
    }
}
