import AppKit
import SwiftUI

/// Scroll view hosting the canvas: centers a sprite smaller than the viewport, keeps the point under the cursor
/// fixed while zooming (pinch, Cmd-scroll, menu), and reports the viewport size for zoom-to-fit.
struct CanvasScroller: NSViewRepresentable {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var settings = ViewSettings.shared

    init(model: SpriteModel) { self.model = model }

    func makeNSView(context: Context) -> CanvasScrollView {
        let canvas = CanvasNSView()
        canvas.model = model
        return CanvasScrollView(canvas: canvas, model: model)
    }

    func updateNSView(_ view: CanvasScrollView, context: Context) {
        view.sync()
    }
}

/// Clip view that centers its document when the document is smaller than the viewport.
final class CenteringClipView: NSClipView {
    override func constrainBoundsRect(_ proposedBounds: NSRect) -> NSRect {
        var rect = super.constrainBoundsRect(proposedBounds)
        if let doc = documentView {
            if rect.width > doc.frame.width { rect.origin.x = (doc.frame.width - rect.width) / 2 }
            if rect.height > doc.frame.height { rect.origin.y = (doc.frame.height - rect.height) / 2 }
        }
        return rect
    }
}

final class CanvasScrollView: NSScrollView {
    private let canvas: CanvasNSView
    private weak var model: SpriteModel?
    private var applied: (zoom: CGFloat, width: Int, height: Int, cx: Int, cy: Int)?
    private var pendingCenter = true
    private var lastRevision = -1
    private var appliedSlot: UUID?
    /// The scroll position just restored for a tab; re-applied while the viewport settles (the tab bar appearing or
    /// disappearing resizes it right after a switch).
    private var lastRestore: (origin: CGPoint, at: Date, size: CGSize)?
    private var defaultsObserver: NSObjectProtocol?

    init(canvas: CanvasNSView, model: SpriteModel) {
        self.canvas = canvas
        self.model = model
        super.init(frame: .zero)
        let clip = CenteringClipView()
        clip.drawsBackground = true
        clip.backgroundColor = Prefs.canvasBackground
        contentView = clip
        documentView = canvas
        hasHorizontalScroller = true
        hasVerticalScroller = true
        autohidesScrollers = true
        drawsBackground = true
        backgroundColor = Prefs.canvasBackground
        defaultsObserver = NotificationCenter.default.addObserver(forName: UserDefaults.didChangeNotification, object: nil, queue: .main) { [weak self] _ in
            self?.backgroundColor = Prefs.canvasBackground
            self?.contentView.backgroundColor = Prefs.canvasBackground
        }
        sync()
    }

    deinit { defaultsObserver.map(NotificationCenter.default.removeObserver) }

    required init?(coder: NSCoder) { fatalError("not used") }

    override func layout() {
        super.layout()
        ViewSettings.shared.viewportSize = contentSize
        if ViewSettings.shared.tiled != .none { sync() }
        if pendingCenter, contentSize.width > 0 {
            pendingCenter = false
            scrollToCenter()
        }
        if let m = model { restorePendingScroll(m) }
        if let r = lastRestore {
            if Date().timeIntervalSince(r.at) > 0.6 {
                lastRestore = nil
            } else if r.size != contentSize {
                lastRestore?.size = contentSize
                contentView.scroll(to: r.origin)
                reflectScrolledClipView(contentView)
            }
        }
    }

    /// Cmd-scroll zooms (anchored at the cursor); plain scrolling is untouched.
    override func scrollWheel(with event: NSEvent) {
        guard event.modifierFlags.contains(.command), let m = model else { return super.scrollWheel(with: event) }
        let delta = event.scrollingDeltaY * (event.hasPreciseScrollingDeltas ? 0.01 : 0.1)
        ViewSettings.shared.zoomBy(m, factor: exp(delta))
    }

    private func scrollToCenter() {
        let clip = contentView
        let doc = canvas.frame
        scroll(to: NSPoint(x: (doc.width - clip.bounds.width) / 2, y: (doc.height - clip.bounds.height) / 2))
    }

    /// Scrolls the clip view to `origin`, kept inside the document (and centered when it is smaller).
    private func scroll(to origin: NSPoint) {
        let clip = contentView
        clip.scroll(to: clip.constrainBoundsRect(NSRect(origin: origin, size: clip.bounds.size)).origin)
        reflectScrolledClipView(clip)
    }

    /// Applies the current zoom / tiling to the document view, keeping the sprite point under the cursor (or the
    /// viewport center) where it was.
    func sync() {
        guard let m = model else { return }
        // A different document: lay out from scratch (centered) instead of anchoring to the previous one's view.
        if appliedSlot != m.activeSlotID {
            appliedSlot = m.activeSlotID
            applied = nil
        }
        let tiled = ViewSettings.shared.tiled
        let copies = CanvasLayout.copies(zoom: m.zoom, width: m.width, height: m.height, tiled: tiled,
                                         viewport: ViewSettings.shared.viewportSize)
        ViewSettings.shared.tileCopies = copies
        let new = (zoom: m.zoom, width: m.width, height: m.height, cx: copies.x, cy: copies.y)
        // An edit that reported a small changed rectangle repaints just that part of the view.
        RenderCache.shared.refresh(m)
        let change = RenderCache.shared.consumeChange()
        let revisionChanged = m.revision != lastRevision
        lastRevision = m.revision
        var partial: CGRect?
        if revisionChanged, tiled == .none, case let .rect(f, x, y, w, h) = change, f == m.frame,
           let old = applied, old == new {
            let z = m.zoom
            partial = CGRect(x: CGFloat(x) * z - 2, y: CGFloat(y) * z - 2, width: CGFloat(w) * z + 4, height: CGFloat(h) * z + 4)
        }
        defer {
            if let partial { canvas.setNeedsDisplay(partial) } else { canvas.needsDisplay = true }
        }
        if let old = applied, old == new { return }

        let clip = contentView
        var anchor: (sprite: CGPoint, view: CGPoint)?
        if let old = applied, old.width == new.width, old.height == new.height {
            let margin = CanvasLayout.margins(zoom: old.zoom, width: old.width, height: old.height, copies: (old.cx, old.cy))
            let view = anchorPoint()
            let doc = CGPoint(x: clip.bounds.minX + view.x, y: clip.bounds.minY + view.y)
            anchor = (CGPoint(x: (doc.x - margin.width) / old.zoom, y: (doc.y - margin.height) / old.zoom), view)
        }
        let sizeChanged = applied.map { $0.width != new.width || $0.height != new.height } ?? true
        defer { restorePendingScroll(m) }
        applied = new

        let size = CanvasLayout.docSize(zoom: new.zoom, width: new.width, height: new.height, copies: copies)
        let margin = CanvasLayout.margins(zoom: new.zoom, width: new.width, height: new.height, copies: copies)
        canvas.setFrameSize(size)
        canvas.bounds = NSRect(x: -margin.width, y: -margin.height, width: size.width, height: size.height)
        canvas.window?.invalidateCursorRects(for: canvas)

        if let a = anchor {
            let doc = CGPoint(x: margin.width + a.sprite.x * new.zoom, y: margin.height + a.sprite.y * new.zoom)
            scroll(to: NSPoint(x: doc.x - a.view.x, y: doc.y - a.view.y))
        } else if sizeChanged {
            if contentSize.width > 0 { scrollToCenter() } else { pendingCenter = true }
        }
    }

    /// Puts back the scroll position a document had when its tab was left.
    private func restorePendingScroll(_ m: SpriteModel) {
        guard let origin = m.pendingScroll, contentSize.width > 0, !pendingCenter else { return }
        m.pendingScroll = nil
        contentView.scroll(to: origin)
        reflectScrolledClipView(contentView)
        lastRestore = (origin, Date(), contentSize)
    }

    /// Cursor position relative to the viewport's top-left, or the viewport center when the cursor is elsewhere.
    private func anchorPoint() -> CGPoint {
        let clip = contentView
        let center = CGPoint(x: clip.bounds.width / 2, y: clip.bounds.height / 2)
        guard let w = window else { return center }
        let p = clip.convert(w.mouseLocationOutsideOfEventStream, from: nil)
        guard clip.bounds.contains(p) else { return center }
        return CGPoint(x: p.x - clip.bounds.minX, y: p.y - clip.bounds.minY)
    }
}
