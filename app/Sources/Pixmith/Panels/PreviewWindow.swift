import AppKit
import SwiftUI

enum PreviewZoom: String, CaseIterable, Identifiable {
    case x1, x2, x4, x8, fit

    var id: String { rawValue }
    var title: String { self == .fit ? "Fit" : String(rawValue.dropFirst()) + "x" }
    var scale: Int? {
        switch self {
        case .x1: 1
        case .x2: 2
        case .x4: 4
        case .x8: 8
        case .fit: nil
        }
    }
}

/// State of the preview, floating or docked. While paused it follows the editor's current frame (and so shows every
/// stroke as it is drawn); only Play runs its own playback, independent of the main editor's play/pause.
final class PreviewState: ObservableObject {
    /// The frame shown while playing.
    @Published var frame = 0
    @Published var isPlaying = false
    @Published var zoom: PreviewZoom = PreviewZoom(rawValue: UserDefaults.standard.string(forKey: "previewZoom") ?? "") ?? .x2 {
        didSet { UserDefaults.standard.set(zoom.rawValue, forKey: "previewZoom") }
    }
    /// Shown as a section of the side panel instead of a floating window.
    @Published var docked = UserDefaults.standard.bool(forKey: "previewDocked") {
        didSet { UserDefaults.standard.set(docked, forKey: "previewDocked") }
    }
    private var reversed = false
    private(set) var active = false
    private var cursor = PlaybackCursor()
    private var task: Task<Void, Never>?

    /// The frame to draw: the playing frame, or the editor's current one while paused.
    func shownFrame(_ model: SpriteModel) -> Int {
        (isPlaying ? frame : model.frame).clamped(to: 0...max(0, model.frameCount - 1))
    }

    /// Called when the preview is shown / hidden; the loop only runs while it is visible.
    func setActive(_ on: Bool, model: SpriteModel) {
        active = on
        task?.cancel()
        task = nil
        guard on else { return }
        frame = ViewSettings.shared.previewRange(for: model).clamp(isPlaying ? frame : model.frame)
        if isPlaying { startLoop(model) }
    }

    /// Play starts at the editor's frame; pausing goes back to following it.
    func togglePlay(model: SpriteModel) {
        isPlaying.toggle()
        task?.cancel()
        task = nil
        if isPlaying {
            frame = model.frame
            if active { startLoop(model) }
        }
    }

    /// Moves the editor's frame, which the paused preview follows; stepping while playing pauses first.
    func step(_ delta: Int, model: SpriteModel) {
        let current = shownFrame(model)
        isPlaying = false
        task?.cancel()
        task = nil
        let n = max(model.frameCount, 1)
        model.stop()
        model.frame = ((current + delta) % n + n) % n
    }

    /// Plays inside the active tag (the one containing the editor's current frame) honoring its direction,
    /// or through all frames when there is none.
    private func startLoop(_ model: SpriteModel) {
        cursor = PlaybackCursor()
        task = Task { @MainActor [weak self, weak model] in
            while !Task.isCancelled {
                guard let self, let model else { return }
                let tag = ViewSettings.activeTag(model)
                let r = ViewSettings.shared.previewRange(for: model)
                if !r.contains(frame) {
                    // Entered a different tag (or left one): restart at the tag's first frame for its direction.
                    let backwards = tag.map { $0.direction == 1 || $0.direction == 3 } ?? false
                    frame = backwards ? r.upperBound : r.lowerBound
                    reversed = backwards
                }
                let ms = max(model.duration(frame), 10)
                try? await Task.sleep(nanoseconds: UInt64(ms) * 1_000_000)
                guard !Task.isCancelled else { return }
                let r2 = ViewSettings.shared.previewRange(for: model)
                let next = model.nextPlayFrame(after: frame, cursor: &cursor)
                frame = r2.contains(next) ? next : r2.lowerBound
            }
        }
    }
}

private extension ClosedRange where Bound == Int {
    func clamp(_ v: Int) -> Int { v.clamped(to: self) }
}

/// Floating preview panel (View > Preview).
final class PreviewController: NSObject, NSWindowDelegate {
    static let shared = PreviewController()
    let state = PreviewState()
    private var panel: NSPanel?
    private weak var model: SpriteModel?

    /// Wide enough for the whole control bar.
    private static let minWidth: CGFloat = 290

    var isVisible: Bool { panel?.isVisible ?? false }

    func toggle(model: SpriteModel) {
        if state.docked {
            // Docked: View > Preview expands or collapses the side panel section.
            let key = "panel.preview.collapsed"
            UserDefaults.standard.set(!UserDefaults.standard.bool(forKey: key), forKey: key)
        } else {
            isVisible ? panel?.close() : show(model: model)
        }
    }

    /// Moves the preview between the floating window and the side panel section (View > Dock Preview).
    func setDocked(_ on: Bool, model: SpriteModel) {
        guard state.docked != on else { return }
        state.docked = on
        if on {
            UserDefaults.standard.set(false, forKey: "panel.preview.collapsed")
            panel?.close()
        } else {
            show(model: model)
        }
    }

    func show(model: SpriteModel) {
        self.model = model
        if panel == nil {
            let p = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 300, height: 340),
                            styleMask: [.titled, .closable, .resizable, .utilityWindow],
                            backing: .buffered, defer: false)
            p.title = "Preview"
            p.isFloatingPanel = true
            p.hidesOnDeactivate = false
            p.becomesKeyOnlyIfNeeded = true
            p.isReleasedWhenClosed = false
            p.delegate = self
            p.contentMinSize = NSSize(width: Self.minWidth, height: 160)
            p.setFrameAutosaveName("PixmithPreview")
            if !p.setFrameUsingName("PixmithPreview") { p.center() }
            panel = p
        }
        // Built on every show and dropped on close, so a hidden preview does no work per edit.
        panel?.contentView = NSHostingView(rootView: PreviewView(model: model, state: state))
        panel?.orderFront(nil)
        state.setActive(true, model: model)
        fitWindow(to: state.zoom, model: model)
    }

    func windowWillClose(_ notification: Notification) {
        if let model { state.setActive(false, model: model) }
        panel?.contentView = NSView()
    }

    /// Resizes the panel so a fixed-zoom preview shows the whole sprite.
    func fitWindow(to zoom: PreviewZoom, model: SpriteModel) {
        guard let panel, let n = zoom.scale else { return }
        let controls: CGFloat = 40
        var size = CGSize(width: CGFloat(model.width * n), height: CGFloat(model.height * n) + controls)
        if let screen = panel.screen ?? NSScreen.main {
            size.width = min(size.width, screen.visibleFrame.width * 0.9)
            size.height = min(size.height, screen.visibleFrame.height * 0.9)
        }
        size.width = max(size.width, Self.minWidth)
        size.height = max(size.height, 120 + controls)
        panel.setContentSize(size)
    }
}

/// The preview's content: the floating window (canvas above a control bar) or, with `docked`, the body of the side
/// panel section.
struct PreviewView: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject var state: PreviewState
    var docked = false

    private var shown: Int { state.shownFrame(model) }

    var body: some View {
        if docked { dockedBody } else { floatingBody }
    }

    private var floatingBody: some View {
        VStack(spacing: 0) {
            PreviewCanvas(model: model, frame: shown, zoom: state.zoom)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            DSDivider()
            HStack(spacing: DS.Space.xxs) {
                transport
                Text("\(shown + 1) / \(model.frameCount)")
                    .font(DS.Font.numberLarge).foregroundStyle(DSColor.textSecondary)
                    .padding(.leading, DS.Space.xs)
                Spacer()
                zoomPopup
                IconButton("sidebar.right", help: "Dock the preview in the side panel") {
                    PreviewController.shared.setDocked(true, model: model)
                }
            }
            .padding(.horizontal, DS.Space.sm)
            .frame(height: 40)
            .background(DSColor.panel)
        }
        .background(DSColor.workspace)
    }

    private var dockedBody: some View {
        VStack(spacing: DS.Space.sm - 2) {
            PreviewCanvas(model: model, frame: shown, zoom: state.zoom)
                .frame(maxWidth: .infinity)
                .frame(height: dockedHeight)
                .background(DSColor.workspace)
                .clipShape(RoundedRectangle(cornerRadius: DS.Radius.field))
            HStack(spacing: DS.Space.xxs) {
                transport
                Spacer()
                zoomPopup
            }
            .padding(.horizontal, -DS.Space.xs)
        }
        .onAppear { state.setActive(true, model: model) }
        // Moving to the floating window activates it instead.
        .onDisappear { if state.docked { state.setActive(false, model: model) } }
    }

    /// Fixed zooms show the sprite at its size, within limits; Fit uses a fixed height.
    private var dockedHeight: CGFloat {
        guard let n = state.zoom.scale else { return 176 }
        return CGFloat(model.height * n).clamped(to: 64...240)
    }

    private var transport: some View {
        HStack(spacing: DS.Space.xxs) {
            IconButton(state.isPlaying ? "pause.fill" : "play.fill", help: "Play / pause the preview. Paused, it shows the editor's frame.",
                       selected: state.isPlaying) { state.togglePlay(model: model) }
            IconButton("backward.frame.fill", help: "Previous frame") { state.step(-1, model: model) }
            IconButton("forward.frame.fill", help: "Next frame") { state.step(1, model: model) }
        }
    }

    private var zoomPopup: some View {
        DSPopup(selection: $state.zoom, options: PreviewZoom.allCases.map { ($0, $0.title) }, width: 76, help: "Preview zoom")
            .onChange(of: state.zoom) { z in if !docked { PreviewController.shared.fitWindow(to: z, model: model) } }
    }
}

/// The preview as a collapsible section near the top of the side panel (View > Dock Preview).
struct DockedPreviewSection: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject var state: PreviewState

    var body: some View {
        if state.docked {
            InspectorSection("Preview", id: "preview", subtitle: "frame \(state.shownFrame(model) + 1) / \(model.frameCount)") {
                IconButton("macwindow", help: "Float the preview in its own window", compact: true) {
                    PreviewController.shared.setDocked(false, model: model)
                }
            } content: {
                PreviewView(model: model, state: state, docked: true)
            }
        }
    }
}

struct PreviewCanvas: NSViewRepresentable {
    @ObservedObject var model: SpriteModel
    let frame: Int
    let zoom: PreviewZoom

    func makeNSView(context: Context) -> PreviewCanvasNSView { PreviewCanvasNSView() }

    func updateNSView(_ view: PreviewCanvasNSView, context: Context) {
        view.model = model
        view.frameIndex = frame
        view.zoom = zoom
        view.needsDisplay = true
    }
}

final class PreviewCanvasNSView: NSView {
    weak var model: SpriteModel?
    var frameIndex = 0
    var zoom: PreviewZoom = .x2

    override var isFlipped: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        guard let m = model, let ctx = NSGraphicsContext.current?.cgContext, m.width > 0, m.height > 0 else { return }
        let w = CGFloat(m.width), h = CGFloat(m.height)
        var scale = zoom.scale.map(CGFloat.init) ?? min(bounds.width / w, bounds.height / h)
        if zoom == .fit, scale >= 1 { scale = scale.rounded(.down) }
        let size = CGSize(width: w * scale, height: h * scale)
        let rect = CGRect(x: ((bounds.width - size.width) / 2).rounded(), y: ((bounds.height - size.height) / 2).rounded(),
                          width: size.width, height: size.height)
        ctx.interpolationQuality = .none
        ctx.saveGState()
        ctx.clip(to: rect)
        ctx.draw(Prefs.checkerImage, in: CGRect(x: rect.minX, y: rect.minY, width: 16, height: 16), byTiling: true)
        ctx.restoreGState()
        let f = frameIndex.clamped(to: 0...max(0, m.frameCount - 1))
        if let img = m.cachedImage(frame: f) { CanvasOverlays.drawImage(ctx, img, in: rect) }
    }
}
