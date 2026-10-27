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

    // Frame difference (the on/off switch stays `SpriteModel.frameDiff`).
    @Published var diffOpacity = 0.65 { didSet { persist() } }
    /// Compare only the active layer instead of the composite of the visible layers.
    @Published var diffActiveLayer = false { didSet { persist() } }
    /// The first frame is compared with the last one (a looping animation); otherwise it has nothing to compare with.
    @Published var diffWrap = true { didSet { persist() } }

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
    /// Rectangular cells (`customGridW` x `customGridH`), or isometric diamonds (`isoGridW` x `isoGridH`, even, at least 4).
    /// Both types share the offset's meaning, the color and the opacity.
    @Published var customGridType: GridType = .rectangular { didSet { persist() } }
    @Published var isoGridW = 32 { didSet { if isoGridW != IsoGrid.size(isoGridW) { isoGridW = IsoGrid.size(isoGridW) } else { persist() } } }
    @Published var isoGridH = 16 { didSet { if isoGridH != IsoGrid.size(isoGridH) { isoGridH = IsoGrid.size(isoGridH) } else { persist() } } }
    @Published var isoGridX = 0 { didSet { persist() } }
    @Published var isoGridY = 0 { didSet { persist() } }
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
        var diffOpacity: Double?
        var diffActiveLayer, diffWrap: Bool?
        var tiled: String?
        var gridColor: UInt32?
        var gridOpacity: Double?
        var gridMinZoom: Int?
        var customGrid: Bool?
        var customGridW, customGridH, customGridX, customGridY: Int?
        var customGridType: String?
        var isoGridW, isoGridH, isoGridX, isoGridY: Int?
        var customGridColor: UInt32?
        var customGridOpacity: Double?
    }

    private static func rgba(_ c: Color) -> UInt32 { UInt32(NSColor(c).usingColorSpace(.sRGB)?.cgColor ?? .black) }

    private func persist() {
        guard !loading else { return }
        var snap = Snapshot(
            onionPast: onionPast, onionFuture: onionFuture, onionOpacity: onionOpacity, onionFalloff: onionFalloff,
            onionTint: onionTint, diffOpacity: diffOpacity, diffActiveLayer: diffActiveLayer, diffWrap: diffWrap,
            tiled: tiled.rawValue, gridOpacity: gridOpacity, gridMinZoom: gridMinZoom,
            customGrid: customGrid, customGridW: customGridW, customGridH: customGridH, customGridX: customGridX,
            customGridY: customGridY, customGridType: customGridType.rawValue, isoGridW: isoGridW, isoGridH: isoGridH,
            isoGridX: isoGridX, isoGridY: isoGridY, customGridColor: Self.rgba(customGridColor),
            customGridOpacity: customGridOpacity)
        // The grid color follows the Preferences default until the user picks another one.
        let color = Self.rgba(gridColor)
        if color != Self.rgba(Color(cgColor: Prefs.gridColor)) { snap.gridColor = color }
        if let data = try? JSONEncoder().encode(snap) { UserDefaults.standard.set(data, forKey: Self.defaultsKey) }
    }

    private init() { reloadFromDefaults() }

    /// Loads the saved settings (at launch; the QA runner uses it to check what was saved).
    func reloadFromDefaults() {
        guard let data = UserDefaults.standard.data(forKey: Self.defaultsKey),
              let s = try? JSONDecoder().decode(Snapshot.self, from: data) else { return }
        loading = true
        defer { loading = false }
        onionPast = (s.onionPast ?? onionPast).clamped(to: 0...3)
        onionFuture = (s.onionFuture ?? onionFuture).clamped(to: 0...3)
        onionOpacity = s.onionOpacity ?? onionOpacity
        onionFalloff = s.onionFalloff ?? onionFalloff
        onionTint = s.onionTint ?? onionTint
        diffOpacity = (s.diffOpacity ?? diffOpacity).clamped(to: 0.1...1)
        diffActiveLayer = s.diffActiveLayer ?? diffActiveLayer
        diffWrap = s.diffWrap ?? diffWrap
        tiled = s.tiled.flatMap(TiledMode.init(rawValue:)) ?? tiled
        if let c = s.gridColor { gridColor = Color(cgColor: c.cgColor) }
        gridOpacity = s.gridOpacity ?? gridOpacity
        gridMinZoom = s.gridMinZoom ?? gridMinZoom
        customGrid = s.customGrid ?? customGrid
        customGridW = s.customGridW ?? customGridW
        customGridH = s.customGridH ?? customGridH
        customGridX = s.customGridX ?? customGridX
        customGridY = s.customGridY ?? customGridY
        customGridType = s.customGridType.flatMap(GridType.init(rawValue:)) ?? customGridType
        isoGridW = IsoGrid.size(s.isoGridW ?? isoGridW)
        isoGridH = IsoGrid.size(s.isoGridH ?? isoGridH)
        isoGridX = s.isoGridX ?? isoGridX
        isoGridY = s.isoGridY ?? isoGridY
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

    // MARK: Isometric grid

    /// View > Isometric Grid: the custom grid shown as isometric diamonds. Turning it off hides the custom grid.
    var isometricGrid: Bool {
        get { customGrid && customGridType == .isometric }
        set {
            if newValue {
                customGridType = .isometric
                customGrid = true
            } else if customGridType == .isometric {
                customGrid = false
            }
        }
    }

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
    @ObservedObject private var preview = PreviewController.shared.state

    var body: some Commands {
        CommandGroup(after: .toolbar) {
            Divider()
            Button("Zoom to Fit") { settings.zoomToFit(model) }.keyboardShortcut("0")
            Button("Actual Size") { settings.actualSize(model) }.keyboardShortcut("1")
            Divider()
            Button("Preview") { PreviewController.shared.toggle(model: model) }
                .keyboardShortcut("p", modifiers: [.command, .option])
            Toggle("Dock Preview", isOn: Binding(get: { preview.docked },
                                                 set: { PreviewController.shared.setDocked($0, model: model) }))
            Picker("Tiled Mode", selection: $settings.tiled) {
                ForEach(TiledMode.allCases) { Text($0.title).tag($0) }
            }
            Toggle("Show Reference Image", isOn: $settings.showReference).disabled(settings.referenceImage == nil)
            Toggle("Isometric Grid", isOn: Binding(get: { settings.isometricGrid }, set: { settings.isometricGrid = $0 }))
            Button("Load Reference Image…") { settings.chooseReference() }
            ReferenceLayerViewMenu(model: model)
            Button("History") { HistoryController.shared.toggle(model: model) }
                .keyboardShortcut("y", modifiers: [.command, .option])
            Button("View Options…") { ViewOptionsController.shared.show() }
                .keyboardShortcut(",", modifiers: [.command, .option])
        }
    }
}
