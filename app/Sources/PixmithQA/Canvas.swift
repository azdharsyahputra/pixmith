import AppKit
@testable import PixmithKit
import PixmithCore

/// The real canvas stack (CanvasScrollView + CanvasNSView) in an offscreen window. Mouse and key events are
/// real NSEvents handed to the view's own handlers, so tools go through exactly the code the app runs.
final class CanvasDriver {
    let model: SpriteModel
    let window: NSWindow
    let scroll: CanvasScrollView
    let canvas: CanvasNSView
    private var eventNumber = 0

    init(_ model: SpriteModel = .shared, size: NSSize = NSSize(width: 720, height: 560)) {
        self.model = model
        canvas = CanvasNSView()
        canvas.model = model
        scroll = CanvasScrollView(canvas: canvas, model: model)
        window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: [.titled, .resizable],
                          backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = scroll
        sync()
    }

    /// Applies zoom and tiling to the document view, like SwiftUI's updateNSView does.
    func sync() {
        scroll.sync()
        window.layoutIfNeeded()
        scroll.layoutSubtreeIfNeeded()
    }

    // MARK: Events

    private func point(_ x: Int, _ y: Int) -> NSPoint {
        let z = model.zoom
        return canvas.convert(NSPoint(x: (CGFloat(x) + 0.5) * z, y: (CGFloat(y) + 0.5) * z), to: nil)
    }

    private func mouse(_ type: NSEvent.EventType, _ x: Int, _ y: Int, flags: NSEvent.ModifierFlags, clicks: Int) -> NSEvent {
        eventNumber += 1
        return NSEvent.mouseEvent(with: type, location: point(x, y), modifierFlags: flags,
                                  timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
                                  context: nil, eventNumber: eventNumber, clickCount: clicks, pressure: 1)!
    }

    private var drawnRevision = -1

    /// The app redraws between user actions, which also refreshes the eyedropper's pixel buffer; do the same.
    func redrawIfNeeded() {
        guard model.revision != drawnRevision else { return }
        drawnRevision = model.revision
        _ = bitmap()
    }

    func down(_ x: Int, _ y: Int, flags: NSEvent.ModifierFlags = [], right: Bool = false, clicks: Int = 1) {
        redrawIfNeeded()
        let e = mouse(right ? .rightMouseDown : .leftMouseDown, x, y, flags: flags, clicks: clicks)
        right ? canvas.rightMouseDown(with: e) : canvas.mouseDown(with: e)
    }

    func drag(_ x: Int, _ y: Int, flags: NSEvent.ModifierFlags = [], right: Bool = false) {
        let e = mouse(right ? .rightMouseDragged : .leftMouseDragged, x, y, flags: flags, clicks: 1)
        right ? canvas.rightMouseDragged(with: e) : canvas.mouseDragged(with: e)
    }

    func up(_ x: Int, _ y: Int, flags: NSEvent.ModifierFlags = [], right: Bool = false) {
        let e = mouse(right ? .rightMouseUp : .leftMouseUp, x, y, flags: flags, clicks: 1)
        right ? canvas.rightMouseUp(with: e) : canvas.mouseUp(with: e)
    }

    func move(_ x: Int, _ y: Int) {
        canvas.mouseMoved(with: mouse(.mouseMoved, x, y, flags: [], clicks: 0))
    }

    /// The pointer leaves the canvas (hides the brush preview).
    func leave() {
        eventNumber += 1
        let e = NSEvent.enterExitEvent(with: .mouseExited, location: point(-1, -1), modifierFlags: [],
                                       timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
                                       context: nil, eventNumber: eventNumber, trackingNumber: 0, userData: nil)!
        canvas.mouseExited(with: e)
    }

    func click(_ x: Int, _ y: Int, flags: NSEvent.ModifierFlags = [], right: Bool = false, clicks: Int = 1) {
        down(x, y, flags: flags, right: right, clicks: clicks)
        up(x, y, flags: flags, right: right)
    }

    /// Press at the first point, drag through the rest, release at the last.
    func stroke(_ pts: [(Int, Int)], flags: NSEvent.ModifierFlags = [], right: Bool = false) {
        guard let first = pts.first else { return }
        down(first.0, first.1, flags: flags, right: right)
        for p in pts.dropFirst() { drag(p.0, p.1, flags: flags, right: right) }
        let last = pts.last!
        up(last.0, last.1, flags: flags, right: right)
    }

    /// Key presses go through NSApp.sendEvent so the app's local key monitors see them.
    func key(_ chars: String, code: UInt16 = 0, flags: NSEvent.ModifierFlags = []) {
        for type in [NSEvent.EventType.keyDown, .keyUp] {
            guard let e = NSEvent.keyEvent(with: type, location: .zero, modifierFlags: flags,
                                           timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
                                           context: nil, characters: chars, charactersIgnoringModifiers: chars,
                                           isARepeat: false, keyCode: code) else { continue }
            NSApp.sendEvent(e)
        }
    }

    func enter() { key("\r", code: 36) }
    func escape() { key("\u{1B}", code: 53) }
    func delete() { key("\u{7F}", code: 51) }

    // MARK: Snapshots

    /// Renders the whole scroll view to a bitmap.
    @discardableResult
    func bitmap() -> NSBitmapImageRep {
        sync()
        scroll.display()
        let rep = scroll.bitmapImageRepForCachingDisplay(in: scroll.bounds)!
        scroll.cacheDisplay(in: scroll.bounds, to: rep)
        return rep
    }

    @discardableResult
    func snapshot(_ name: String) -> NSBitmapImageRep {
        let rep = bitmap()
        try? FileManager.default.createDirectory(atPath: QA.outDir, withIntermediateDirectories: true)
        if let png = rep.representation(using: .png, properties: [:]) {
            try? png.write(to: URL(fileURLWithPath: QA.outDir).appendingPathComponent("\(name).png"))
        }
        return rep
    }

    /// Color the canvas shows at the center of sprite pixel (x, y), as 0xRRGGBB.
    func shown(_ x: Int, _ y: Int, in rep: NSBitmapImageRep) -> UInt32? {
        let inWindow = point(x, y)
        let p = scroll.convert(inWindow, from: nil)
        let scale = CGFloat(rep.pixelsWide) / scroll.bounds.width
        let px = Int(p.x * scale), py = Int((scroll.isFlipped ? p.y : scroll.bounds.height - p.y) * scale)
        guard px >= 0, py >= 0, px < rep.pixelsWide, py < rep.pixelsHigh,
              let c = rep.colorAt(x: px, y: py)?.usingColorSpace(.sRGB) else { return nil }
        let q = { (v: CGFloat) in UInt32((v * 255).rounded()) }
        return q(c.redComponent) << 16 | q(c.greenComponent) << 8 | q(c.blueComponent)
    }
}
