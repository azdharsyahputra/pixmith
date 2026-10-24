import AppKit
import PixmithCore
import SwiftUI

// The Color section's selectors besides the Spectrum: color wheel, RGB / HSV / HSL sliders, Gray and the normal-map
// disc (`selectors.rs` in the egui app). All of them edit a `ColorSpaces`, the color in every space at once. The math
// is the core's `colorpick` module (`pixmith_colorpick.h`), shared with the egui app.

// MARK: - Core math

/// Swift wrappers over `pixmith_cp_*`. Colors are `0xRRGGBBAA`, hue is in degrees, everything else in 0...1.
enum ColorMath {
    typealias Triple = (a: Double, b: Double, c: Double)

    static func hsv(_ rgba: UInt32, hint: (hue: Double, sat: Double)? = nil) -> Triple {
        var out = [Double](repeating: 0, count: 3)
        if let hint {
            let h = [hint.hue, hint.sat]
            pixmith_cp_rgba_to_hsv(rgba, h, &out)
        } else {
            pixmith_cp_rgba_to_hsv(rgba, nil, &out)
        }
        return (out[0], out[1], out[2])
    }

    static func hsl(_ rgba: UInt32, hint: (hue: Double, sat: Double)? = nil) -> Triple {
        var out = [Double](repeating: 0, count: 3)
        if let hint {
            let h = [hint.hue, hint.sat]
            pixmith_cp_rgba_to_hsl(rgba, h, &out)
        } else {
            pixmith_cp_rgba_to_hsl(rgba, nil, &out)
        }
        return (out[0], out[1], out[2])
    }

    static func rgba(h: Double, s: Double, v: Double, a: Double = 1) -> UInt32 { pixmith_cp_hsv_to_rgba(h, s, v, a) }
    static func rgba(hslH h: Double, s: Double, l: Double, a: Double = 1) -> UInt32 { pixmith_cp_hsl_to_rgba(h, s, l, a) }
    static func luma(_ rgba: UInt32) -> Double { pixmith_cp_luma(rgba) }

    /// Hue and saturation of a wheel-disc point (x right, y down, -1...1).
    static func wheelHit(x: Double, y: Double, hintHue: Double) -> (hue: Double, sat: Double) {
        var out = [0.0, 0.0]
        pixmith_cp_wheel_hit(x, y, hintHue, &out)
        return (out[0], out[1])
    }

    static func wheelPoint(hue: Double, sat: Double) -> (x: Double, y: Double) {
        var out = [0.0, 0.0]
        pixmith_cp_wheel_point(hue, sat, &out)
        return (out[0], out[1])
    }

    static func harmonyHues(_ hue: Double, _ harmony: Harmony) -> [Double] {
        var out = [Double](repeating: 0, count: 4)
        let n = Int(pixmith_cp_harmony(hue, UInt32(harmony.rawValue), &out))
        return Array(out.prefix(n))
    }

    /// Straight-alpha RGBA pixels of the wheel (`size` x `size`).
    static func wheelPixels(size: Int, model: WheelModel, value: Double) -> [UInt8] {
        var buf = [UInt8](repeating: 0, count: size * size * 4)
        _ = pixmith_cp_wheel_render(&buf, UInt32(size), UInt32(model.rawValue), value)
        return buf
    }

    static func normalPixels(size: Int) -> [UInt8] {
        var buf = [UInt8](repeating: 0, count: size * size * 4)
        _ = pixmith_cp_normal_render(&buf, UInt32(size))
        return buf
    }

    /// The unit normal at a normal-disc point (x right, y down).
    static func normalFromDisc(x: Double, y: Double) -> [Double] {
        var out = [0.0, 0.0, 0.0]
        pixmith_cp_normal_from_disc(x, y, &out)
        return out
    }

    /// The disc point of the normal a color stores.
    static func normalDiscPoint(_ rgba: UInt32) -> (x: Double, y: Double) {
        var out = [0.0, 0.0]
        pixmith_cp_normal_disc_point(rgba, &out)
        return (out[0], out[1])
    }

    static func normalDecode(_ rgba: UInt32) -> [Double] {
        var out = [0.0, 0.0, 0.0]
        pixmith_cp_normal_decode(rgba, &out)
        return out
    }

    static func normalEncode(_ n: [Double], alpha: Double) -> UInt32 {
        var n = n
        return pixmith_cp_normal_encode(&n, alpha)
    }

    /// `n` with X (0), Y (1) or Z (2) replaced by `v`.
    static func normalSet(_ n: [Double], axis: Int, to v: Double) -> [Double] {
        var n = n
        pixmith_cp_normal_set(&n, UInt32(axis), v)
        return n
    }
}

// MARK: - Selectors and their preferences

/// The ways to pick a color, in menu order.
enum ColorSelector: String, CaseIterable, Identifiable {
    case spectrum, wheel, rgb, hsv, hsl, gray, normal
    var id: String { rawValue }

    var title: String {
        switch self {
        case .spectrum: "Spectrum"
        case .wheel: "Color Wheel"
        case .rgb: "RGB Sliders"
        case .hsv: "HSV Sliders"
        case .hsl: "HSL Sliders"
        case .gray: "Gray"
        case .normal: "Normal Map"
        }
    }
}

/// Which color model the wheel and its bar use (the core's `WheelModel`).
enum WheelModel: Int, CaseIterable {
    case hsv, hsl
    var title: String { self == .hsv ? "HSV" : "HSL" }
}

/// Colors marked on the wheel besides the picked one (the core's `Harmony`).
enum Harmony: Int, CaseIterable {
    case none, complementary, triadic, analogous, splitComplementary, square
    var title: String {
        switch self {
        case .none: "No Harmony"
        case .complementary: "Complementary"
        case .triadic: "Triadic"
        case .analogous: "Analogous"
        case .splitComplementary: "Split Complementary"
        case .square: "Square"
        }
    }
}

/// The picked selector and the wheel's settings, remembered between launches.
final class ColorPickerPrefs: ObservableObject {
    static let shared = ColorPickerPrefs()
    private let defaults: UserDefaults

    @Published var selector: ColorSelector { didSet { defaults.set(selector.rawValue, forKey: "colorPicker.selector") } }
    @Published var wheelModel: WheelModel { didSet { defaults.set(wheelModel.rawValue, forKey: "colorPicker.wheelModel") } }
    @Published var harmony: Harmony { didSet { defaults.set(harmony.rawValue, forKey: "colorPicker.harmony") } }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        selector = defaults.string(forKey: "colorPicker.selector").flatMap(ColorSelector.init(rawValue:)) ?? .spectrum
        wheelModel = WheelModel(rawValue: defaults.integer(forKey: "colorPicker.wheelModel")) ?? .hsv
        harmony = Harmony(rawValue: defaults.integer(forKey: "colorPicker.harmony")) ?? .none
    }
}

// MARK: - The edited color in every space

/// The edited color in every space the selectors use. `hsva` always matches the `0xRRGGBBAA` value; `hsl` is kept beside
/// it so an HSL slider holds its value (an RGB round trip would nudge it) and gray keeps its hue.
struct ColorSpaces: Equatable {
    var hsva: HSVA
    /// Hue (degrees), saturation and lightness (0...1).
    var hsl: [Double]

    init(rgba: UInt32) {
        hsva = HSVA(rgba: rgba)
        let t = ColorMath.hsl(rgba)
        hsl = [t.a, t.b, t.c]
    }

    var rgba: UInt32 { hsva.rgba }

    /// Follows a color that was changed elsewhere (palette, eyedropper, typed hex), keeping hue and saturation where
    /// the new color leaves them undefined.
    mutating func setRGBA(_ rgba: UInt32) {
        hsva = HSVA(rgba: rgba, hint: (hsva.h, hsva.s))
        let t = ColorMath.hsl(rgba, hint: (hsl[0], hsl[1]))
        hsl = [t.a, t.b, t.c]
    }

    mutating func setHSV(_ h: Double, _ s: Double, _ v: Double) {
        hsva = HSVA(h: h, s: s, v: v, a: hsva.a)
        let t = ColorMath.hsl(hsva.rgba, hint: (h, hsl[1]))
        hsl = [t.a, t.b, t.c]
    }

    mutating func setHSL(_ h: Double, _ s: Double, _ l: Double) {
        hsl = [h, s, l]
        hsva = HSVA(rgba: ColorMath.rgba(hslH: h, s: s, l: l, a: hsva.a), hint: (h, hsva.s))
    }

    mutating func setAlpha(_ a: Double) { hsva.a = a }

    /// One channel (0 red, 1 green, 2 blue) as a byte.
    mutating func setChannel(_ index: Int, _ value: Double) {
        var c = [rgba >> 24 & 0xFF, rgba >> 16 & 0xFF, rgba >> 8 & 0xFF].map(Double.init)
        c[index] = min(max(value.rounded(), 0), 255)
        setRGBA(UInt32(c[0]) << 24 | UInt32(c[1]) << 16 | UInt32(c[2]) << 8 | UInt32((hsva.a * 255).rounded()))
    }

    /// Red, green and blue as bytes.
    var channels: [Double] { [rgba >> 24 & 0xFF, rgba >> 16 & 0xFF, rgba >> 8 & 0xFF].map(Double.init) }

    /// The gray level (luma) as a byte.
    var gray: Double { (ColorMath.luma(rgba) * 255).rounded() }

    mutating func setGray(_ value: Double) {
        let g = UInt32(min(max(value.rounded(), 0), 255))
        setRGBA(g << 24 | g << 16 | g << 8 | UInt32((hsva.a * 255).rounded()))
    }

    // MARK: Wheel

    /// Hue, saturation and value / lightness in the wheel's model.
    func wheelTriple(_ model: WheelModel) -> (h: Double, s: Double, v: Double) {
        model == .hsv ? (hsva.h, hsva.s, hsva.v) : (hsl[0], hsl[1], hsl[2])
    }

    mutating func setWheel(_ model: WheelModel, h: Double, s: Double, v: Double) {
        if model == .hsv { setHSV(h, s, v) } else { setHSL(h, s, v) }
    }

    /// A press on the wheel disc at (`x`, `y`) in -1...1, y down: picks hue and saturation, keeps the value.
    mutating func pickOnWheel(_ model: WheelModel, x: Double, y: Double) {
        let t = wheelTriple(model)
        let hit = ColorMath.wheelHit(x: x, y: y, hintHue: t.h)
        setWheel(model, h: hit.hue, s: hit.sat, v: t.v)
    }

    // MARK: Normal map

    /// The normal this color stores.
    var normal: [Double] { ColorMath.normalDecode(rgba) }

    mutating func setNormal(_ n: [Double]) {
        setRGBA(ColorMath.normalEncode(n, alpha: hsva.a))
    }

    /// A press on the normal disc at (`x`, `y`) in -1...1, y down.
    mutating func pickNormal(x: Double, y: Double) {
        setNormal(ColorMath.normalFromDisc(x: x, y: y))
    }
}

// MARK: - Disc images

/// The wheel and normal-disc bitmaps as images, re-rendered only when what they show changes.
enum DiscImage {
    private static var cache: [String: NSImage] = [:]

    static func wheel(points: CGFloat, scale: CGFloat, model: WheelModel, value: Double) -> NSImage {
        let px = max(8, Int((points * scale).rounded()))
        let key = "wheel-\(px)-\(model.rawValue)-\(Int((value * 1024).rounded()))"
        return cached(key, points: points, px: px) { ColorMath.wheelPixels(size: px, model: model, value: value) }
    }

    static func normal(points: CGFloat, scale: CGFloat) -> NSImage {
        let px = max(8, Int((points * scale).rounded()))
        return cached("normal-\(px)", points: points, px: px) { ColorMath.normalPixels(size: px) }
    }

    private static func cached(_ key: String, points: CGFloat, px: Int, _ render: () -> [UInt8]) -> NSImage {
        if let image = cache[key] { return image }
        let data = Data(render())
        guard let provider = CGDataProvider(data: data as CFData),
              let cg = CGImage(width: px, height: px, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: px * 4,
                               space: CGColorSpaceCreateDeviceRGB(),
                               bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue), provider: provider,
                               decode: nil, shouldInterpolate: true, intent: .defaultIntent)
        else { return NSImage(size: NSSize(width: points, height: points)) }
        let image = NSImage(cgImage: cg, size: NSSize(width: points, height: points))
        if cache.count > 24 { cache.removeAll() }
        cache[key] = image
        return image
    }
}

// MARK: - Slider rows

/// A gradient across the track from `at(0)` to `at(1)`, sampled at `cells + 1` stops.
func colorGradient(cells: Int, _ at: @escaping (Double) -> UInt32) -> some View {
    LinearGradient(colors: (0...cells).map { Color(rgba: at(Double($0) / Double(cells))) }, startPoint: .leading,
                   endPoint: .trailing)
}

/// One slider row: a gradient track and a number field with the channel's letter. `value` is in display units.
private struct SliderRow<Track: View>: View {
    let label: String
    let name: String
    @Binding var value: Double
    let range: ClosedRange<Double>
    var unit = ""
    var fieldWidth: CGFloat = 64
    @ViewBuilder var track: Track

    var body: some View {
        HStack(spacing: DS.Space.sm) {
            TrackSlider(value: $value, range: range, label: name) { track }
            DSNumberField(label, value: $value, range: range, step: (range.upperBound - range.lowerBound) / 200, unit: unit,
                          width: fieldWidth, help: name)
        }
    }
}

/// The alpha row of the slider selectors.
private struct AlphaRow: View {
    @Binding var spaces: ColorSpaces
    var changed: () -> Void

    var body: some View {
        SliderRow(label: "A", name: "Alpha", value: Binding(get: { spaces.hsva.a * 100 },
                                                          set: { spaces.setAlpha($0 / 100); changed() }),
                  range: 0...100, unit: "%") { AlphaTrack(rgba: spaces.rgba) }
    }
}

/// A checkerboard under the color from transparent to opaque.
struct AlphaTrack: View {
    let rgba: UInt32
    var body: some View {
        ZStack {
            DSChecker(cell: 4)
            colorGradient(cells: 1) { UInt32(rgba & 0xFFFF_FF00) | UInt32(($0 * 255).rounded()) }
        }
    }
}

// MARK: - Slider selectors

struct RGBSliders: View {
    @Binding var spaces: ColorSpaces
    var changed: () -> Void

    var body: some View {
        VStack(spacing: DS.Space.sm) {
            ForEach(0..<3, id: \.self) { i in
                let name = ["Red", "Green", "Blue"][i]
                let base = spaces.channels
                SliderRow(label: ["R", "G", "B"][i], name: name,
                          value: Binding(get: { spaces.channels[i] }, set: { spaces.setChannel(i, $0); changed() }),
                          range: 0...255) {
                    colorGradient(cells: 1) { t in
                        var c = base
                        c[i] = t * 255
                        return UInt32(c[0]) << 24 | UInt32(c[1]) << 16 | UInt32(c[2]) << 8 | 0xFF
                    }
                }
            }
            AlphaRow(spaces: $spaces, changed: changed)
        }
    }
}

struct HSVSliders: View {
    @Binding var spaces: ColorSpaces
    var changed: () -> Void

    var body: some View {
        let h = spaces.hsva.h, s = spaces.hsva.s, v = spaces.hsva.v
        VStack(spacing: DS.Space.sm) {
            SliderRow(label: "H", name: "Hue", value: Binding(get: { spaces.hsva.h }, set: { spaces.setHSV($0, s, v); changed() }),
                      range: 0...360, unit: "°") { colorGradient(cells: 12) { ColorMath.rgba(h: $0 * 360, s: 1, v: 1) } }
            SliderRow(label: "S", name: "Saturation",
                      value: Binding(get: { spaces.hsva.s * 100 }, set: { spaces.setHSV(h, $0 / 100, v); changed() }),
                      range: 0...100, unit: "%") { colorGradient(cells: 1) { ColorMath.rgba(h: h, s: $0, v: v) } }
            SliderRow(label: "V", name: "Value",
                      value: Binding(get: { spaces.hsva.v * 100 }, set: { spaces.setHSV(h, s, $0 / 100); changed() }),
                      range: 0...100, unit: "%") { colorGradient(cells: 1) { ColorMath.rgba(h: h, s: s, v: $0) } }
            AlphaRow(spaces: $spaces, changed: changed)
        }
    }
}

struct HSLSliders: View {
    @Binding var spaces: ColorSpaces
    var changed: () -> Void

    var body: some View {
        let h = spaces.hsl[0], s = spaces.hsl[1], l = spaces.hsl[2]
        VStack(spacing: DS.Space.sm) {
            SliderRow(label: "H", name: "Hue", value: Binding(get: { spaces.hsl[0] }, set: { spaces.setHSL($0, s, l); changed() }),
                      range: 0...360, unit: "°") { colorGradient(cells: 12) { ColorMath.rgba(hslH: $0 * 360, s: 1, l: 0.5) } }
            SliderRow(label: "S", name: "Saturation",
                      value: Binding(get: { spaces.hsl[1] * 100 }, set: { spaces.setHSL(h, $0 / 100, l); changed() }),
                      range: 0...100, unit: "%") { colorGradient(cells: 1) { ColorMath.rgba(hslH: h, s: $0, l: l) } }
            SliderRow(label: "L", name: "Lightness",
                      value: Binding(get: { spaces.hsl[2] * 100 }, set: { spaces.setHSL(h, s, $0 / 100); changed() }),
                      range: 0...100, unit: "%") { colorGradient(cells: 8) { ColorMath.rgba(hslH: h, s: s, l: $0) } }
            AlphaRow(spaces: $spaces, changed: changed)
        }
    }
}

struct GraySlider: View {
    @Binding var spaces: ColorSpaces
    var changed: () -> Void

    var body: some View {
        VStack(spacing: DS.Space.sm) {
            SliderRow(label: "Gray", name: "Gray level (luma)",
                      value: Binding(get: { spaces.gray }, set: { spaces.setGray($0); changed() }), range: 0...255,
                      fieldWidth: 76) {
                colorGradient(cells: 1) { UInt32(($0 * 255).rounded()) * 0x0101_0100 | 0xFF }
            }
            AlphaRow(spaces: $spaces, changed: changed)
        }
    }
}

// MARK: - Discs

private let discSide: CGFloat = 136

/// Hue/saturation wheel with a value (or lightness) bar, harmony markers, H/S/V (or L) fields and the model and
/// harmony menus.
struct WheelPicker: View {
    @Binding var spaces: ColorSpaces
    @ObservedObject var prefs: ColorPickerPrefs
    var changed: () -> Void
    @Environment(\.displayScale) private var scale

    private func set(h: Double? = nil, s: Double? = nil, v: Double? = nil) {
        let t = spaces.wheelTriple(prefs.wheelModel)
        spaces.setWheel(prefs.wheelModel, h: h ?? t.h, s: s ?? t.s, v: v ?? t.v)
        changed()
    }

    private func color(_ hue: Double, _ sat: Double, _ value: Double) -> UInt32 {
        prefs.wheelModel == .hsv ? ColorMath.rgba(h: hue, s: sat, v: value) : ColorMath.rgba(hslH: hue, s: sat, l: value)
    }

    private func ring(_ radius: CGFloat) -> some View {
        Circle().stroke(Color.white, lineWidth: 2).frame(width: radius * 2, height: radius * 2)
            .background(Circle().stroke(Color.black.opacity(0.4), lineWidth: 1).frame(width: radius * 2 + 2, height: radius * 2 + 2))
    }

    private func wheel(_ t: (h: Double, s: Double, v: Double)) -> some View {
        let half = discSide / 2
        let image = DiscImage.wheel(points: discSide, scale: scale, model: prefs.wheelModel, value: t.v)
        let hues = ColorMath.harmonyHues(t.h, prefs.harmony)
        func point(_ hue: Double) -> CGPoint {
            let p = ColorMath.wheelPoint(hue: hue, sat: t.s)
            return CGPoint(x: half + p.x * half, y: half + p.y * half)
        }
        return ZStack(alignment: .topLeading) {
            Image(nsImage: image).resizable().frame(width: discSide, height: discSide)
            Circle().stroke(Color.black.opacity(0.18), lineWidth: 1).frame(width: discSide, height: discSide)
            ForEach(Array(hues.enumerated()), id: \.offset) { _, hue in
                Path { p in
                    p.move(to: CGPoint(x: half, y: half))
                    p.addLine(to: point(hue))
                }
                .stroke(Color.white.opacity(0.47), lineWidth: 1)
            }
            ForEach(Array(hues.enumerated()), id: \.offset) { _, hue in
                Circle().fill(Color(rgba: color(hue, t.s, t.v))).frame(width: 9, height: 9)
                    .overlay(Circle().stroke(Color.white, lineWidth: 1.5).frame(width: 10, height: 10))
                    .overlay(Circle().stroke(Color.black.opacity(0.35), lineWidth: 1).frame(width: 12, height: 12))
                    .position(point(hue))
            }
            ring(6).position(point(t.h))
        }
        .frame(width: discSide, height: discSide)
        .contentShape(Rectangle())
        .gesture(DragGesture(minimumDistance: 0).onChanged { g in
            spaces.pickOnWheel(prefs.wheelModel, x: (g.location.x - half) / half, y: (g.location.y - half) / half)
            changed()
        })
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Color wheel")
    }

    private func valueBar(_ t: (h: Double, s: Double, v: Double)) -> some View {
        let name = prefs.wheelModel == .hsl ? "Lightness" : "Value"
        return GeometryReader { geo in
            let travel = geo.size.height - 14
            ZStack(alignment: .top) {
                LinearGradient(colors: (0...12).map { Color(rgba: color(t.h, t.s, 1 - Double($0) / 12)) }, startPoint: .top,
                               endPoint: .bottom)
                    .clipShape(Capsule())
                Circle().fill(Color.white).frame(width: 14, height: 14)
                    .overlay(Circle().stroke(Color.black.opacity(0.35), lineWidth: 1))
                    .shadow(color: .black.opacity(0.3), radius: 1.5, y: 0.5)
                    .offset(y: travel * (1 - min(max(t.v, 0), 1)))
            }
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0).onChanged { g in
                let f: Double = Double((g.location.y - 7) / max(travel, 1))
                set(v: 1 - min(max(f, 0), 1))
            })
        }
        .frame(width: 14, height: discSide)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(name)
        .accessibilityValue("\(Int((t.v * 100).rounded())) percent")
        .help(name)
    }

    var body: some View {
        let t = spaces.wheelTriple(prefs.wheelModel)
        let gap = (discSide - 4 * DS.Metric.control) / 3
        VStack(spacing: DS.Space.sm) {
            HStack(alignment: .top, spacing: DS.Space.sm) {
                wheel(t)
                valueBar(t)
                VStack(spacing: gap) {
                    ZStack {
                        DSChecker(cell: 4)
                        Color(rgba: ColorMath.rgba(h: spaces.hsva.h, s: spaces.hsva.s, v: spaces.hsva.v, a: spaces.hsva.a))
                    }
                    .frame(height: DS.Metric.control)
                    .clipShape(RoundedRectangle(cornerRadius: 5))
                    .overlay(RoundedRectangle(cornerRadius: 5).stroke(Color.white.opacity(0.07), lineWidth: 1))
                    DSNumberField("H", value: Binding(get: { t.h }, set: { set(h: $0) }), range: 0...359, unit: "°", width: nil,
                                  help: "Hue")
                    DSNumberField("S", value: Binding(get: { t.s * 100 }, set: { set(s: $0 / 100) }), range: 0...100, step: 0.5,
                                  unit: "%", width: nil, help: "Saturation")
                    DSNumberField(prefs.wheelModel == .hsl ? "L" : "V", value: Binding(get: { t.v * 100 }, set: { set(v: $0 / 100) }),
                                  range: 0...100, step: 0.5, unit: "%", width: nil,
                                  help: prefs.wheelModel == .hsl ? "Lightness" : "Value")
                }
                .frame(maxWidth: .infinity)
            }
            HStack(spacing: DS.Space.sm) {
                DSPopup(selection: $prefs.wheelModel, options: WheelModel.allCases.map { ($0, $0.title) },
                        help: "Color model of the wheel", fill: true)
                DSPopup(selection: $prefs.harmony, options: Harmony.allCases.map { ($0, $0.title) },
                        help: "Mark harmonious colors on the wheel", fill: true)
            }
        }
    }
}

/// The normal-map picker: a hemisphere disc, X / Y / Z fields and a button for the flat normal.
struct NormalPicker: View {
    @Binding var spaces: ColorSpaces
    var changed: () -> Void
    @Environment(\.displayScale) private var scale

    private func setAxis(_ axis: Int, _ v: Double) {
        spaces.setNormal(ColorMath.normalSet(spaces.normal, axis: axis, to: v))
        changed()
    }

    var body: some View {
        let half = discSide / 2
        let n = spaces.normal
        let p = ColorMath.normalDiscPoint(spaces.rgba)
        let gap = (discSide - 4 * DS.Metric.control) / 3
        HStack(alignment: .top, spacing: DS.Space.sm) {
            ZStack(alignment: .topLeading) {
                Image(nsImage: DiscImage.normal(points: discSide, scale: scale)).resizable().frame(width: discSide, height: discSide)
                Circle().stroke(Color.black.opacity(0.18), lineWidth: 1).frame(width: discSide, height: discSide)
                Circle().stroke(Color.white, lineWidth: 2).frame(width: 12, height: 12)
                    .background(Circle().stroke(Color.black.opacity(0.4), lineWidth: 1).frame(width: 14, height: 14))
                    .position(x: half + p.x * half, y: half + p.y * half)
            }
            .frame(width: discSide, height: discSide)
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0).onChanged { g in
                spaces.pickNormal(x: (g.location.x - half) / half, y: (g.location.y - half) / half)
                changed()
            })
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Normal map disc")
            .help("Tangent-space normal: drag to tilt. The center is flat (#8080FF).")
            VStack(spacing: gap) {
                DSNumberField("X", value: Binding(get: { n[0] }, set: { setAxis(0, $0) }), range: -1...1, step: 0.01, digits: 2,
                              width: nil, help: "Normal X (right)")
                DSNumberField("Y", value: Binding(get: { n[1] }, set: { setAxis(1, $0) }), range: -1...1, step: 0.01, digits: 2,
                              width: nil, help: "Normal Y (up)")
                DSNumberField("Z", value: Binding(get: { n[2] }, set: { setAxis(2, $0) }), range: 0...1, step: 0.01, digits: 2,
                              width: nil, help: "Normal Z (out of the surface)")
                Button {
                    spaces.setNormal([0, 0, 1])
                    changed()
                } label: {
                    Text("Flat").font(DS.Font.body).foregroundStyle(DSColor.text)
                        .frame(maxWidth: .infinity).frame(height: DS.Metric.control)
                        .background(RoundedRectangle(cornerRadius: DS.Radius.field).fill(DSColor.field))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("The flat normal, #8080FF")
            }
            .frame(maxWidth: .infinity)
        }
    }
}
