import AppKit
import PixmithCore
import SwiftUI

/// The filters offered in the FX menu. Raw values match the kind ids in `pixmith_filters.h`;
/// `scale` is not a pixel filter and uses its own sheet.
enum FilterKind: UInt32, CaseIterable, Identifiable {
    case replaceColor = 0, invert, desaturate, hsl, brightnessContrast, colorBalance
    case outline, dropShadow, despeckle, blur, sharpen, curves, cleanEdges
    case scale = 1000
    case lintSprite

    var id: UInt32 { rawValue }

    var title: String {
        switch self {
        case .replaceColor: "Replace Color"
        case .invert: "Invert"
        case .desaturate: "Desaturate"
        case .hsl: "Hue/Saturation/Lightness"
        case .brightnessContrast: "Brightness/Contrast"
        case .colorBalance: "Color Balance"
        case .curves: "Color Curves"
        case .outline: "Outline"
        case .dropShadow: "Drop Shadow"
        case .despeckle: "Remove Isolated Pixels"
        case .blur: "Blur"
        case .sharpen: "Sharpen"
        case .cleanEdges: "Clean Edges"
        case .lintSprite: "Lint Sprite"
        case .scale: "Scale Sprite"
        }
    }

    /// The Rust kind id (color balance maps to the channel-offset filter).
    var coreKind: UInt32 { self == .colorBalance ? 5 : rawValue }
}

/// What the FX menu and filter sheets share. Kept apart from `SpriteModel` so the feature stays
/// self-contained.
final class FilterState: ObservableObject {
    static let shared = FilterState()

    /// The filter whose sheet is open.
    @Published var active: FilterKind?
    /// The frame image shown on the canvas while a sheet is open (nil = show the real document).
    private(set) var preview: CGImage?
    /// Optional rectangle limiting filters; width 0 means the whole canvas. Wire a selection here.
    var region = (x: 0, y: 0, w: 0, h: 0)

    /// Changes with every new preview; the canvas redraws without touching the document's render cache.
    func setPreview(_ image: CGImage?) {
        preview = image
        SpriteModel.shared.objectWillChange.send()
    }

    private var pendingPreview: DispatchWorkItem?

    /// Runs `block` once the slider has been still for a moment (the previous pending call is dropped).
    func schedulePreview(_ block: @escaping () -> Void) {
        pendingPreview?.cancel()
        let item = DispatchWorkItem(block: block)
        pendingPreview = item
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.06, execute: item)
    }

    func cancelPendingPreview() {
        pendingPreview?.cancel()
        pendingPreview = nil
    }
}

struct FXCommands: Commands {
    @ObservedObject private var sheet = SheetState.shared

    var body: some Commands {
        CommandMenu("FX") {
            item(.replaceColor)
            item(.invert)
            item(.desaturate)
            Divider()
            item(.hsl)
            item(.brightnessContrast)
            item(.colorBalance)
            item(.curves)
            Divider()
            item(.outline)
            item(.dropShadow)
            item(.despeckle)
            item(.cleanEdges)
            Divider()
            item(.lintSprite)
            Divider()
            item(.blur)
            item(.sharpen)
            Divider()
            item(.scale)
        }
    }

    private func item(_ kind: FilterKind) -> some View {
        Button(kind.title + "…") {
            var b = [Int32](repeating: 0, count: 4)
            let hasSel = pixmith_sel_bounds(SpriteModel.shared.doc, &b)
            FilterState.shared.region = hasSel ? (Int(b[0]), Int(b[1]), Int(b[2]), Int(b[3])) : (0, 0, 0, 0)
            FilterState.shared.active = kind
        }
        .disabled(sheet.isOpen)
    }
}

extension View {
    /// Presents the filter sheets; attach once to the main content view.
    func filterSheets(model: SpriteModel) -> some View {
        modifier(FilterSheetsModifier(model: model))
    }
}

private struct FilterSheetsModifier: ViewModifier {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var state = FilterState.shared

    func body(content: Content) -> some View {
        content.sheet(item: $state.active) { kind in
            if kind == .scale {
                ScaleSheet(model: model)
            } else if kind == .lintSprite {
                LintSheet(model: model)
            } else {
                FilterSheet(model: model, kind: kind)
            }
        }
    }
}

// MARK: - Parameters

enum FilterTarget: UInt32, CaseIterable, Identifiable {
    case cel = 0, allFrames, allLayers
    var id: UInt32 { rawValue }
    var title: String {
        switch self {
        case .cel: "Current cel"
        case .allFrames: "All frames of this layer"
        case .allLayers: "All layers of this frame"
        }
    }
}

/// Everything the sheets can edit. Only the values relevant to the open filter are sent.
private struct FilterParams: Equatable {
    var target: FilterTarget = .cel
    var fromColor: UInt32 = 0x000000FF
    var toColor: UInt32 = 0xFF0000FF
    var tolerance = 0
    var hue = 0, saturation = 0, lightness = 0
    var brightness = 0, contrast = 0
    var red = 0, green = 0, blue = 0
    var outlineColor: UInt32 = 0x000000FF
    var inside = false
    var eight = false
    var shadowX = 1, shadowY = 1
    var shadowColor: UInt32 = 0x00000099
    var curves = CurveSet()
    var clean = CleanParams()

    func values(_ kind: FilterKind) -> [Int32] {
        func c(_ v: UInt32) -> Int32 { Int32(bitPattern: v) }
        switch kind {
        case .replaceColor: return [c(fromColor), c(toColor), Int32(tolerance)]
        case .hsl: return [Int32(hue), Int32(saturation), Int32(lightness)]
        case .brightnessContrast: return [Int32(brightness), Int32(contrast)]
        case .colorBalance: return [Int32(red), Int32(green), Int32(blue)]
        case .curves: return curves.encoded()
        case .outline: return [c(outlineColor), inside ? 1 : 0, eight ? 1 : 0]
        case .dropShadow: return [Int32(shadowX), Int32(shadowY), c(shadowColor)]
        case .despeckle: return [eight ? 1 : 0]
        case .cleanEdges: return clean.values
        case .invert, .desaturate, .blur, .sharpen, .scale, .lintSprite: return []
        }
    }
}

/// The Clean Edges sheet: which passes run and their thresholds (`PIXMITH_FILTER_CLEAN_EDGES` in `pixmith_filters.h`).
private struct CleanParams: Equatable {
    var halo = true, rings = 2, gain = 40
    var specks = true, minSize = 4
    var holes = false, holeSize = 2
    var light = false
    var thin = true, thickness = 3, luma = 70
    var fixedColor = false
    var color: UInt32 = 0x000000FF

    /// The parameter block of the core call.
    var values: [Int32] {
        let flags = (halo ? 1 : 0) | (specks ? 2 : 0) | (thin ? 4 : 0) | (light ? 8 : 0)
        return [Int32(flags), Int32(rings), Int32(gain), 30, Int32(minSize), holes ? Int32(holeSize) : 0,
                Int32(thickness), Int32(luma), fixedColor ? 1 : 0, Int32(bitPattern: color), 0]
    }
}

// MARK: - Filter sheet

struct FilterSheet: View {
    @ObservedObject var model: SpriteModel
    let kind: FilterKind
    @ObservedObject private var state = FilterState.shared
    @State private var p = FilterParams()

    var body: some View {
        DSDialog(title: kind.title, primary: "Apply", width: 360, onPrimary: { apply() }, onCancel: { close() }) {
            controls
            DSDivider()
            DSFormRow(label: "Apply to") {
                DSPopup(selection: $p.target, options: FilterTarget.allCases.map { ($0, $0.title) }, fill: true)
            }
        }
        .onAppear {
            model.stop()
            p.fromColor = model.color
            updatePreview()
        }
        .onChange(of: p) { _ in state.schedulePreview { updatePreview() } }
        .onDisappear { state.setPreview(nil) }
    }

    @ViewBuilder private var controls: some View {
        switch kind {
        case .replaceColor:
            ColorRow(label: "From", value: $p.fromColor)
            ColorRow(label: "To", value: $p.toColor)
            ValueSlider(label: "Tolerance", value: $p.tolerance, range: 0...255)
        case .hsl:
            ValueSlider(label: "Hue", value: $p.hue, range: -180...180)
            ValueSlider(label: "Saturation", value: $p.saturation, range: -100...100)
            ValueSlider(label: "Lightness", value: $p.lightness, range: -100...100)
        case .brightnessContrast:
            ValueSlider(label: "Brightness", value: $p.brightness, range: -100...100)
            ValueSlider(label: "Contrast", value: $p.contrast, range: -100...100)
        case .colorBalance:
            ValueSlider(label: "Red", value: $p.red, range: -255...255)
            ValueSlider(label: "Green", value: $p.green, range: -255...255)
            ValueSlider(label: "Blue", value: $p.blue, range: -255...255)
        case .curves:
            CurvesEditor(curves: $p.curves)
        case .outline:
            ColorRow(label: "Color", value: $p.outlineColor)
            DSFormRow(label: "Position") {
                DSSegmented(selection: $p.inside, options: [.init(false, title: "Outside"), .init(true, title: "Inside")])
            }
            DSFormRow(label: "Connectivity") {
                DSSegmented(selection: $p.eight, options: [.init(false, title: "4-connected"), .init(true, title: "8-connected")])
            }
        case .dropShadow:
            ColorRow(label: "Color", value: $p.shadowColor)
            ValueSlider(label: "Offset X", value: $p.shadowX, range: -16...16)
            ValueSlider(label: "Offset Y", value: $p.shadowY, range: -16...16)
        case .despeckle:
            DSFormRow(label: "Connectivity") {
                DSSegmented(selection: $p.eight, options: [.init(false, title: "4-connected"), .init(true, title: "8-connected")])
            }
            DSNote("Clears pixels that have no opaque neighbor.")
        case .invert:
            DSNote("Inverts the RGB channels, keeping alpha.")
        case .desaturate:
            DSNote("Converts colors to grayscale.")
        case .blur:
            DSNote("3x3 blur that ignores transparent pixels' color.")
        case .sharpen:
            DSNote("3x3 sharpen that ignores transparent pixels' color.")
        case .cleanEdges:
            Toggle("Remove paper halo (light, grayish outer pixels)", isOn: $p.clean.halo).toggleStyle(.checkbox)
            if p.clean.halo {
                ValueSlider(label: "Rings", value: $p.clean.rings, range: 1...4)
                ValueSlider(label: "Lighter than inside by", value: $p.clean.gain, range: 10...120)
            }
            Toggle("Remove detached specks", isOn: $p.clean.specks).toggleStyle(.checkbox)
            if p.clean.specks {
                ValueSlider(label: "Smaller than (px)", value: $p.clean.minSize, range: 2...32)
            }
            Toggle("Fill tiny enclosed holes", isOn: $p.clean.holes).toggleStyle(.checkbox)
            if p.clean.holes {
                ValueSlider(label: "Smaller than (px)", value: $p.clean.holeSize, range: 2...16)
            }
            Toggle("Replace lone light pixels inside dark areas", isOn: $p.clean.light).toggleStyle(.checkbox)
            Toggle("Thin the outer outline to 1 px", isOn: $p.clean.thin).toggleStyle(.checkbox)
            if p.clean.thin {
                ValueSlider(label: "Thinner than (px)", value: $p.clean.thickness, range: 2...8)
                Toggle("Use a fixed outline color", isOn: $p.clean.fixedColor).toggleStyle(.checkbox)
                if p.clean.fixedColor {
                    ColorRow(label: "Outline color", value: $p.clean.color)
                } else {
                    ValueSlider(label: "Outline darkness", value: $p.clean.luma, range: 0...255)
                }
            }
            DSNote("Small interior details (eyes, highlights, thin fingers) are kept. Holes and lone light pixels are off by default because they can be details.")
        case .scale, .lintSprite:
            EmptyView()
        }
    }

    /// Arguments shared by the preview and apply calls.
    private func call<T>(_ body: (UnsafePointer<Int32>?, Int) -> T) -> T {
        let values = p.values(kind)
        return values.withUnsafeBufferPointer { body($0.baseAddress, $0.count) }
    }

    private func updatePreview() {
        let (w, h) = (model.width, model.height)
        var buf = [UInt8](repeating: 0, count: w * h * 4)
        let r = state.region
        let ok = call { ptr, n in
            pixmith_filter_preview(model.doc, kind.coreKind, p.target.rawValue, UInt32(model.layer),
                                UInt32(model.frame), Int32(r.x), Int32(r.y), UInt32(r.w), UInt32(r.h),
                                ptr, n, &buf, buf.count)
        }
        state.setPreview(ok ? Self.image(buf, w, h) : nil)
    }

    private func apply() {
        let r = state.region
        let applied = call { ptr, n in
            pixmith_filter_apply(model.doc, kind.coreKind, p.target.rawValue, UInt32(model.layer),
                              UInt32(model.frame), Int32(r.x), Int32(r.y), UInt32(r.w), UInt32(r.h), ptr, n)
        }
        if applied {
            model.labelLastStep("Filter: \(kind.title)")
            model.revision &+= 1
        }
        close()
    }

    private func close() {
        state.cancelPendingPreview()
        state.setPreview(nil)
        state.active = nil
    }

    static func image(_ buf: [UInt8], _ w: Int, _ h: Int) -> CGImage? {
        guard let provider = CGDataProvider(data: Data(buf) as CFData) else { return nil }
        return CGImage(
            width: w, height: h, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: w * 4,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue),
            provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
    }
}

// MARK: - Scale sheet

struct ScaleSheet: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var state = FilterState.shared
    @State private var byPercent = true
    @State private var percent = 200
    @State private var width = 0
    @State private var height = 0
    @State private var keepAspect = true

    private var newSize: (w: Int, h: Int) {
        if byPercent {
            return (max(1, Int((Double(model.width) * Double(percent) / 100).rounded())),
                    max(1, Int((Double(model.height) * Double(percent) / 100).rounded())))
        }
        return (width, height)
    }

    private var valid: Bool {
        let s = newSize
        return (1...8192).contains(s.w) && (1...8192).contains(s.h) && (s.w != model.width || s.h != model.height)
    }

    var body: some View {
        DSDialog(title: "Scale Sprite", subtitle: "Nearest-neighbor, all frames and layers.", primary: "Apply",
                 primaryEnabled: valid, width: 340, onPrimary: {
                     let s = newSize
                     model.stop()
                     if pixmith_scale_sprite(model.doc, UInt32(s.w), UInt32(s.h)) {
                         model.labelLastStep("Scale Sprite")
                         model.zoom = max(1, min(32, (512 / CGFloat(max(s.w, s.h))).rounded(.down)))
                         model.revision &+= 1
                     }
                     state.active = nil
                 }, onCancel: { state.active = nil }) {
            DSSegmented(selection: $byPercent, options: [.init(true, title: "Percent"), .init(false, title: "Size")])
            if byPercent {
                DSFormRow(label: "Scale") { DSNumberField("%", value: $percent, range: 1...10000, unit: "%", width: 120) }
            } else {
                DSFormRow(label: "Width") {
                    DSNumberField("W", value: Binding(get: { width }, set: { setWidth($0) }), range: 1...8192, unit: "px", width: 120)
                }
                DSFormRow(label: "Height") {
                    DSNumberField("H", value: Binding(get: { height }, set: { setHeight($0) }), range: 1...8192, unit: "px", width: 120)
                }
                Toggle("Keep aspect ratio", isOn: $keepAspect).toggleStyle(.checkbox)
            }
            HStack(spacing: DS.Space.sm) {
                Text("\(model.width) × \(model.height)").foregroundStyle(DSColor.textSecondary)
                Image(systemName: "arrow.right").font(.system(size: 10)).foregroundStyle(DSColor.textTertiary)
                Text("\(newSize.w) × \(newSize.h)").foregroundStyle(valid ? DSColor.text : DSColor.textSecondary)
            }
            .font(DS.Font.numberLarge)
        }
        .onAppear {
            model.stop()
            width = model.width
            height = model.height
        }
    }

    private func setWidth(_ v: Int) {
        let old = max(width, 1)
        width = v
        if keepAspect { height = max(1, Int((Double(height) * Double(v) / Double(old)).rounded())) }
    }

    private func setHeight(_ v: Int) {
        let old = max(height, 1)
        height = v
        if keepAspect { width = max(1, Int((Double(width) * Double(v) / Double(old)).rounded())) }
    }
}

// MARK: - Controls

struct ValueSlider: View {
    let label: String
    @Binding var value: Int
    let range: ClosedRange<Int>

    var body: some View {
        DSSlider(label, value: Binding(get: { Double(value) }, set: { value = Int($0.rounded()) }),
                 range: Double(range.lowerBound)...Double(range.upperBound),
                 format: { ($0 > 0 && range.lowerBound < 0 ? "+" : "") + String(Int($0.rounded())) })
    }
}

struct ColorRow: View {
    let label: String
    @Binding var value: UInt32

    var body: some View {
        DSFormRow(label: label) {
            ColorPicker(label, selection: Binding(get: { Color(cgColor: value.cgColor) },
                                                  set: { value = UInt32($0.cgColor ?? .black) }))
                .labelsHidden()
            Spacer(minLength: 0)
        }
    }
}
