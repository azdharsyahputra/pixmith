import AppKit

/// The hand: dragging the canvas with Space held or with the middle button scrolls it. Space tapped on its own still
/// runs its shortcut (Play / Pause), on release, unless the canvas was dragged while it was held.
enum CanvasPan {
    /// The Space key-down being held, kept to run its shortcut on release.
    static var space: NSEvent?
    /// The canvas was dragged while Space was held.
    static var panned = false
    /// Window location of the last drag event while panning; `nil` when not panning.
    static var last: NSPoint?

    static func spaceDown(_ e: NSEvent) {
        if !e.isARepeat {
            space = e
            panned = false
        }
    }

    /// Ends the Space hold; returns its key-down when it should run its shortcut.
    static func spaceUp() -> NSEvent? {
        defer { space = nil }
        return panned ? nil : space
    }
}

extension CanvasNSView {
    /// Starts panning on a middle-button press, or a primary press while Space is held.
    func beginPan(_ e: NSEvent) -> Bool {
        let middle = e.type == .otherMouseDown && e.buttonNumber == 2
        guard middle || (e.type == .leftMouseDown && CanvasPan.space != nil) else { return false }
        if !middle { CanvasPan.panned = true }
        CanvasPan.last = e.locationInWindow
        NSCursor.closedHand.push()
        return true
    }

    /// Scrolls the canvas with the pointer while panning.
    func continuePan(_ e: NSEvent) -> Bool {
        guard let last = CanvasPan.last else { return false }
        let now = e.locationInWindow
        CanvasPan.last = now
        guard let scroll = enclosingScrollView else { return true }
        let clip = scroll.contentView
        var origin = clip.bounds.origin
        origin.x -= now.x - last.x
        origin.y += now.y - last.y
        clip.scroll(to: clip.constrainBoundsRect(NSRect(origin: origin, size: clip.bounds.size)).origin)
        scroll.reflectScrolledClipView(clip)
        return true
    }

    func endPan() -> Bool {
        guard CanvasPan.last != nil else { return false }
        CanvasPan.last = nil
        NSCursor.pop()
        return true
    }
}
