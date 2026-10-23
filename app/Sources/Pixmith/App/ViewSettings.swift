import AppKit
import PixmithCore
import SwiftUI

// MARK: - Settings types

enum TiledMode: String, CaseIterable, Identifiable {
    case none, x, y, both

    var id: String { rawValue }
    var title: String {
        switch self {
        case .none: "None"
        case .x: "Tiled in X Axis"
        case .y: "Tiled in Y Axis"
        case .both: "Tiled in Both Axes"
        }
    }
    var tilesX: Bool { self == .x || self == .both }
    var tilesY: Bool { self == .y || self == .both }
}

enum OnionTint { case none, past, future }

enum ReferencePlacement: String, CaseIterable, Identifiable {
    case behind, above

    var id: String { rawValue }
    var title: String { self == .behind ? "Behind sprite" : "Above sprite" }
}

/// View-only state (never saved in the .aseprite file). Everything here only affects how the canvas is drawn.
final class ViewSettings: ObservableObject {
    static let shared = ViewSettings()

    /// Optional hook for the layers/tags track: return the frame range of the active tag, or nil for all frames.
    static var previewRangeProvider: (SpriteModel) -> ClosedRange<Int>? = { $0.activeTagRange }

    // Onion skin (the on/off switch stays `SpriteModel.onionSkin`).
    @Published var onionPast = 1 { didSet { persist() } }
    @Published var onionFuture = 0 { didSet { persist() } }
    @Published var onionOpacity = 0.3 { didSet { persist() } }
    @Published var onionFalloff = 0.6 { didSet { persist() } }
    @Published var onionTint = false { didSet { persist() } }

    @Published var tiled: TiledMode = .none { didSet { persist() } }

    // Pixel grid (the on/off switch stays `SpriteModel.showGrid`).
    @Published var gridColor = Color(cgColor: Prefs.gridColor) { didSet { persist() } }
    @Published var gridOpacity = 0.12 { didSet { persist() } }
    @Published var gridMinZoom = 6 { didSet { persist() } }

    @Published var customGrid = false { didSet { persist() } }
    @Published var customGridW = 16 { didSet { persist() } }
    @Published var customGridH = 16 { didSet { persist() } }
    @Published var customGridX = 0 { didSet { persist() } }
    @Published var customGridY = 0 { didSet { persist() } }
    @Published var customGridColor = Color(red: 0.1, green: 0.5, blue: 1) { didSet { persist() } }
    @Published var customGridOpacity = 0.6 { didSet { persist() } }

    // Reference image.
    @Published var referenceImage: CGImage?
    @Published var referenceName = ""
    @Published var showReference = true
    @Published var referenceOpacity = 0.5
    @Published var referencePlacement: ReferencePlacement = .behind
    @Published var referenceX = 0
    @Published var referenceY = 0
    @Published var referenceScale = 100.0

    // MARK: Persistence

    private static let defaultsKey = "viewSettings"
    private var loading = false

    private struct Snapshot: Codable {
        var onionPast, onionFuture: Int?
        var onionOpacity, onionFalloff: Double?
        var onionTint: Bool?
        var tiled: String?
        var gridColor: UInt32?
        var gridOpacity: Double?
        var gridMinZoom: Int?
        var customGrid: Bool?
        var customGridW, customGridH, customGridX, customGridY: Int?
        var customGridColor: UInt32?
        var customGridOpacity: Double?
    }

    private static func rgba(_ c: Color) -> UInt32 { UInt32(NSColor(c).usingColorSpace(.sRGB)?.cgColor ?? .black) }

    private func persist() {
        guard !loading else { return }
        var snap = Snapshot(
            onionPast: onionPast, onionFuture: onionFuture, onionOpacity: onionOpacity, onionFalloff: onionFalloff,
            onionTint: onionTint, tiled: tiled.rawValue, gridOpacity: gridOpacity, gridMinZoom: gridMinZoom,
            customGrid: customGrid, customGridW: customGridW, customGridH: customGridH, customGridX: customGridX,
            customGridY: customGridY, customGridColor: Self.rgba(customGridColor), customGridOpacity: customGridOpacity)
        // The grid color follows the Preferences default until the user picks another one.
        let color = Self.rgba(gridColor)
        if color != Self.rgba(Color(cgColor: Prefs.gridColor)) { snap.gridColor = color }
        if let data = try? JSONEncoder().encode(snap) { UserDefaults.standard.set(data, forKey: Self.defaultsKey) }
    }

    private init() {
        guard let data = UserDefaults.standard.data(forKey: Self.defaultsKey),
              let s = try? JSONDecoder().decode(Snapshot.self, from: data) else { return }
        loading = true
        defer { loading = false }
        onionPast = (s.onionPast ?? onionPast).clamped(to: 0...3)
        onionFuture = (s.onionFuture ?? onionFuture).clamped(to: 0...3)
        onionOpacity = s.onionOpacity ?? onionOpacity
        onionFalloff = s.onionFalloff ?? onionFalloff
        onionTint = s.onionTint ?? onionTint
        tiled = s.tiled.flatMap(TiledMode.init(rawValue:)) ?? tiled
        if let c = s.gridColor { gridColor = Color(cgColor: c.cgColor) }
        gridOpacity = s.gridOpacity ?? gridOpacity
        gridMinZoom = s.gridMinZoom ?? gridMinZoom
        customGrid = s.customGrid ?? customGrid
        customGridW = s.customGridW ?? customGridW
        customGridH = s.customGridH ?? customGridH
        customGridX = s.customGridX ?? customGridX
        customGridY = s.customGridY ?? customGridY
        if let c = s.customGridColor { customGridColor = Color(cgColor: c.cgColor) }
        customGridOpacity = s.customGridOpacity ?? customGridOpacity
    }

    /// Size of the canvas viewport, kept up to date by the scroll view (used by zoom-to-fit).
    var viewportSize = CGSize.zero
    /// Sprite copies drawn on each side of the center sprite, set by the scroll view together with the document size.
    var tileCopies = (x: 0, y: 0)

    /// The tag that contains the current frame, if tag looping is on.
    static func activeTag(_ m: SpriteModel) -> TagInfo? {
        guard m.loopTags else { return nil }
        return (0..<m.tagCount).lazy.compactMap { m.tag($0) }.first { $0.from <= m.frame && m.frame <= $0.to }
    }

    func previewRange(for m: SpriteModel) -> ClosedRange<Int> {
        let all = 0...max(0, m.frameCount - 1)
        guard let r = Self.previewRangeProvider(m) else { return all }
        let lo = r.lowerBound.clamped(to: all), hi = r.upperBound.clamped(to: all)
        return lo <= hi ? lo...hi : all
    }

    // MARK: Zoom

    static let minZoom: CGFloat = 0.1
    static let maxZoom: CGFloat = 64

    func zoomToFit(_ m: SpriteModel) {
        let vp = viewportSize
        let avail = CGSize(width: vp.width - 2 * CanvasLayout.pad, height: vp.height - 2 * CanvasLayout.pad)
        guard avail.width > 0, avail.height > 0, m.width > 0, m.height > 0 else { return }
        var z = min(avail.width / CGFloat(m.width), avail.height / CGFloat(m.height))
        if z >= 1 { z = z.rounded(.down) }
        m.zoom = z.clamped(Self.minZoom, Self.maxZoom)
    }

    /// Multiplies the zoom (pinch, Cmd-scroll); the scroll view keeps the point under the cursor fixed.
    func zoomBy(_ m: SpriteModel, factor: CGFloat) {
        m.zoom = (m.zoom * factor).clamped(Self.minZoom, Self.maxZoom)
    }

    func actualSize(_ m: SpriteModel) { m.zoom = 1 }

    // MARK: Reference image

    func chooseReference() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.image]
        panel.message = "Choose a reference image (it is not saved in the sprite)"
        if panel.runModal() == .OK, let url = panel.url { loadReference(url) }
    }

    func loadReference(_ url: URL) {
        guard let src = CGImageSourceCreateWithURL(url as CFURL, nil),
              let img = CGImageSourceCreateImageAtIndex(src, 0, nil)
        else {
            SpriteModel.shared.errorMessage = "Could not read \(url.lastPathComponent) as an image."
            return
        }
        referenceImage = img
        referenceName = url.lastPathComponent
        showReference = true
    }

    func clearReference() {
        referenceImage = nil
        referenceName = ""
    }
}

extension Comparable {
    func clamped(to r: ClosedRange<Self>) -> Self { Swift.min(Swift.max(self, r.lowerBound), r.upperBound) }
}

// MARK: - Menu commands

struct ViewCommands: Commands {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var settings = ViewSettings.shared

    var body: some Commands {
        CommandGroup(after: .toolbar) {
            Divider()
            Button("Zoom to Fit") { settings.zoomToFit(model) }.keyboardShortcut("0")
            Button("Actual Size") { settings.actualSize(model) }.keyboardShortcut("1")
            Divider()
            Button("Preview") { PreviewController.shared.toggle(model: model) }
                .keyboardShortcut("p", modifiers: [.command, .option])
            Picker("Tiled Mode", selection: $settings.tiled) {
                ForEach(TiledMode.allCases) { Text($0.title).tag($0) }
            }
            Toggle("Show Reference Image", isOn: $settings.showReference).disabled(settings.referenceImage == nil)
            Button("Load Reference Image…") { settings.chooseReference() }
            Button("History") { HistoryController.shared.toggle(model: model) }
                .keyboardShortcut("y", modifiers: [.command, .option])
            Button("View Options…") { ViewOptionsController.shared.show() }
                .keyboardShortcut(",", modifiers: [.command, .option])
        }
    }
}
