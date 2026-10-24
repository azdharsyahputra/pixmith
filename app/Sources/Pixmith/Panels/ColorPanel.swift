import AppKit
import SwiftUI

/// Which of the two drawing colors the Color section edits.
final class ColorTarget: ObservableObject {
    static let shared = ColorTarget()
    @Published var secondary = false

    /// Selects a color for editing and makes sure the Color section is open.
    func edit(secondary: Bool) {
        self.secondary = secondary
        UserDefaults.standard.set(false, forKey: "panel.color.collapsed")
    }
}

/// Hue (degrees), saturation, value and alpha, all but hue in 0...1. The math is the core's `colorpick` module.
struct HSVA: Equatable {
    var h: Double = 0
    var s: Double = 0
    var v: Double = 0
    var a: Double = 1

    /// From `0xRRGGBBAA`. `hue` is kept when the color is gray (hue is undefined there) so dragging stays stable.
    init(rgba: UInt32, hue: Double? = nil) {
        self.init(rgba: rgba, hint: hue.map { ($0, 0) })
    }

    /// Like `init(rgba:hue:)`, with a (hue, saturation) hint for what black and gray leave undefined.
    init(rgba: UInt32, hint: (Double, Double)?) {
        let t = ColorMath.hsv(rgba, hint: hint.map { (hue: $0.0, sat: $0.1) })
        (h, s, v) = (t.a, t.b, t.c)
        a = Double(rgba & 0xFF) / 255
    }

    init(h: Double, s: Double, v: Double, a: Double) {
        (self.h, self.s, self.v, self.a) = (h, s, v, a)
    }

    var rgba: UInt32 { ColorMath.rgba(h: h, s: s, v: v, a: a) }
}

/// A horizontal track with a draggable thumb; `background` paints the track.
struct TrackSlider<Background: View>: View {
    @Binding var value: Double
    let range: ClosedRange<Double>
    let label: String
    @ViewBuilder var background: Background

    var body: some View {
        GeometryReader { geo in
            let f = (value - range.lowerBound) / (range.upperBound - range.lowerBound)
            ZStack(alignment: .leading) {
                background.clipShape(Capsule())
                Circle().fill(Color.white).frame(width: 14, height: 14)
                    .overlay(Circle().stroke(Color.black.opacity(0.35), lineWidth: 1))
                    .shadow(color: .black.opacity(0.3), radius: 1.5, y: 0.5)
                    .offset(x: (geo.size.width - 14) * min(max(f, 0), 1))
            }
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0).onChanged { g in
                let t = min(max((g.location.x - 7) / max(geo.size.width - 14, 1), 0), 1)
                value = range.lowerBound + t * (range.upperBound - range.lowerBound)
            })
        }
        .frame(height: 14)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
        .accessibilityValue("\(Int(value))")
        .accessibilityAdjustableAction { dir in
            let d = (range.upperBound - range.lowerBound) / 20
            value = min(max(value + (dir == .increment ? d : -d), range.lowerBound), range.upperBound)
        }
    }
}

/// Saturation (x) and value (y) of the current hue.
private struct SVPlane: View {
    @Binding var spaces: ColorSpaces
    var changed: () -> Void
    private var hsva: HSVA { spaces.hsva }

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .topLeading) {
                Color(hue: hsva.h / 360, saturation: 1, brightness: 1)
                LinearGradient(colors: [.white, .white.opacity(0)], startPoint: .leading, endPoint: .trailing)
                LinearGradient(colors: [.black.opacity(0), .black], startPoint: .top, endPoint: .bottom)
                Circle().stroke(Color.white, lineWidth: 2).frame(width: 12, height: 12)
                    .background(Circle().stroke(Color.black.opacity(0.4), lineWidth: 1).frame(width: 14, height: 14))
                    .offset(x: hsva.s * geo.size.width - 6, y: (1 - hsva.v) * geo.size.height - 6)
            }
            .clipShape(RoundedRectangle(cornerRadius: DS.Radius.control))
            .overlay(RoundedRectangle(cornerRadius: DS.Radius.control).stroke(Color.black.opacity(0.2), lineWidth: 1))
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0).onChanged { g in
                let fx: Double = Double(g.location.x / max(geo.size.width, 1))
                let fy: Double = Double(g.location.y / max(geo.size.height, 1))
                let s: Double = min(max(fx, 0), 1)
                let v: Double = 1 - min(max(fy, 0), 1)
                spaces.setHSV(hsva.h, s, v)
                changed()
            })
        }
        .frame(height: 96)
        .accessibilityLabel("Saturation and brightness")
    }
}

/// The primary and secondary color chips, overlapped, with a swap button. Click one to edit it in the Color section.
struct ColorChips: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject var opts: DrawingOptions
    @ObservedObject private var target = ColorTarget.shared

    init(model: SpriteModel) {
        self.model = model
        opts = model.opts
    }

    private func chip(_ rgba: UInt32, ring: Bool) -> some View {
        Swatch(color: Color(rgba: rgba), size: 22, radius: 5)
            .overlay(RoundedRectangle(cornerRadius: 5).stroke(ring ? DSColor.accent : DSColor.separator, lineWidth: ring ? 2 : 1)
                .padding(ring ? -1 : -1.5))
            .overlay(RoundedRectangle(cornerRadius: 5).stroke(DSColor.panel, lineWidth: 1.5).padding(-1.5).opacity(ring ? 0 : 1))
    }

    var body: some View {
        VStack(spacing: DS.Space.xs) {
            ZStack(alignment: .topLeading) {
                Button { target.edit(secondary: true) } label: { chip(opts.secondary, ring: target.secondary) }
                    .buttonStyle(.plain)
                    .offset(x: 14, y: 14)
                    .help("Secondary color (right click). Click to edit it.")
                Button { target.edit(secondary: false) } label: { chip(model.color, ring: !target.secondary) }
                    .buttonStyle(.plain)
                    .help("Primary color (left click). Click to edit it.")
            }
            .frame(width: 36, height: 36, alignment: .topLeading)
            IconButton("arrow.left.arrow.right", help: "Swap colors", shortcut: "X", compact: true, size: 24, iconSize: 11) {
                model.swapColors()
            }
        }
    }
}

/// Inspector section: a selector menu in the header (Spectrum, Wheel, RGB, HSV and HSL sliders, Gray, Normal map), the
/// picked selector, and the hex field. Edits the primary or the secondary color, whichever is selected in the header
/// (or on the tool rail).
struct ColorSection: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject var opts: DrawingOptions
    @ObservedObject private var target = ColorTarget.shared
    @ObservedObject private var prefs = ColorPickerPrefs.shared
    @State private var spaces = ColorSpaces(rgba: 0x000000FF)
    @State private var hex = ""
    @FocusState private var hexFocused: Bool

    init(model: SpriteModel) {
        self.model = model
        opts = model.opts
    }

    private var current: UInt32 { target.secondary ? opts.secondary : model.color }

    private func apply() {
        let c = spaces.rgba
        if target.secondary { opts.secondary = c } else { model.color = c }
    }

    private func pull() {
        let c = current
        if c != spaces.rgba { spaces.setRGBA(c) }
        if !hexFocused { hex = Self.hexString(c) }
    }

    private static func hexString(_ c: UInt32) -> String { String(format: "%06X", c >> 8) }

    private func commitHex() {
        var t = hex.trimmingCharacters(in: CharacterSet(charactersIn: "# ")).uppercased()
        if t.count == 3 { t = t.map { "\($0)\($0)" }.joined() }
        guard t.count == 6 || t.count == 8, let v = UInt32(t, radix: 16) else { hex = Self.hexString(current); return }
        spaces.setRGBA(t.count == 6 ? v << 8 | UInt32((spaces.hsva.a * 255).rounded()) : v)
        apply()
        hex = Self.hexString(current)
    }

    private func targetButton(_ name: String, rgba: UInt32, secondary: Bool) -> some View {
        Button { target.secondary = secondary } label: {
            Swatch(color: Color(rgba: rgba), size: 14, selected: target.secondary == secondary, radius: 4)
                .frame(width: DS.Metric.control, height: DS.Metric.control)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Edit the \(name) color")
        .accessibilityLabel("Edit \(name) color")
    }

    private var selectorMenu: some View {
        DSMenuButton(symbol: "paintpalette", help: "Color selector: \(prefs.selector.title)") {
            ForEach(ColorSelector.allCases) { s in
                Button {
                    prefs.selector = s
                } label: {
                    if s == prefs.selector { Label(s.title, systemImage: "checkmark") } else { Text(s.title) }
                }
            }
        }
    }

    /// A number field bound to a component of the spectrum's HSV.
    private func hsvNumber(_ label: String, scale: Double, range: ClosedRange<Double>, unit: String, help: String,
                           get: @escaping (HSVA) -> Double, set: @escaping (inout ColorSpaces, Double) -> Void) -> some View {
        DSNumberField(label, value: Binding(get: { get(spaces.hsva) * scale },
                                            set: { set(&spaces, $0 / scale); apply() }),
                      range: range, step: scale == 1 ? 1 : 0.5, unit: unit, width: nil, help: help)
    }

    private var alphaSlider: some View {
        TrackSlider(value: Binding(get: { spaces.hsva.a }, set: { spaces.setAlpha($0); apply() }), range: 0...1, label: "Alpha") {
            AlphaTrack(rgba: spaces.rgba)
        }
    }

    private func hexRow(alphaField: Bool) -> some View {
        HStack(spacing: 6) {
            HStack(spacing: 6) {
                Swatch(color: Color(rgba: current), size: 12, radius: 3)
                TextField("", text: $hex)
                    .textFieldStyle(.plain).font(DS.Font.numberLarge).focused($hexFocused)
                    .onSubmit(commitHex)
                    .onChange(of: hexFocused) { if !$0 { commitHex() } }
            }
            .padding(.horizontal, DS.Space.sm)
            .frame(maxWidth: .infinity)
            .frame(height: DS.Metric.control)
            .dsFieldBackground(focused: hexFocused)
            .help("Hex color: RRGGBB")
            if alphaField {
                DSNumberField("A", value: Binding(get: { spaces.hsva.a * 100 }, set: { spaces.setAlpha($0 / 100); apply() }),
                              range: 0...100, step: 0.5, unit: "%", width: 70, help: "Alpha. Drag the label to scrub.")
            }
        }
    }

    @ViewBuilder private var selectorBody: some View {
        switch prefs.selector {
        case .spectrum:
            SVPlane(spaces: $spaces, changed: apply)
            TrackSlider(value: Binding(get: { spaces.hsva.h }, set: { spaces.setHSV($0, spaces.hsva.s, spaces.hsva.v); apply() }),
                        range: 0...360, label: "Hue") {
                LinearGradient(colors: stride(from: 0.0, through: 1.0, by: 1.0 / 6).map {
                    Color(hue: $0, saturation: 1, brightness: 1)
                }, startPoint: .leading, endPoint: .trailing)
            }
            alphaSlider
            hexRow(alphaField: true)
            HStack(spacing: 6) {
                hsvNumber("H", scale: 1, range: 0...359, unit: "°", help: "Hue", get: { $0.h }) {
                    $0.setHSV($1, $0.hsva.s, $0.hsva.v)
                }
                hsvNumber("S", scale: 100, range: 0...100, unit: "%", help: "Saturation", get: { $0.s }) {
                    $0.setHSV($0.hsva.h, $1, $0.hsva.v)
                }
                hsvNumber("V", scale: 100, range: 0...100, unit: "%", help: "Value", get: { $0.v }) {
                    $0.setHSV($0.hsva.h, $0.hsva.s, $1)
                }
            }
        case .wheel:
            WheelPicker(spaces: $spaces, prefs: prefs, changed: apply)
            alphaSlider
            hexRow(alphaField: true)
        case .normal:
            NormalPicker(spaces: $spaces, changed: apply)
            alphaSlider
            hexRow(alphaField: true)
        case .rgb:
            RGBSliders(spaces: $spaces, changed: apply)
            hexRow(alphaField: false)
        case .hsv:
            HSVSliders(spaces: $spaces, changed: apply)
            hexRow(alphaField: false)
        case .hsl:
            HSLSliders(spaces: $spaces, changed: apply)
            hexRow(alphaField: false)
        case .gray:
            GraySlider(spaces: $spaces, changed: apply)
            hexRow(alphaField: false)
        }
    }

    var body: some View {
        InspectorSection("Color", id: "color", subtitle: target.secondary ? "Secondary" : "Primary") {
            HStack(spacing: 0) {
                selectorMenu
                targetButton("primary", rgba: model.color, secondary: false)
                targetButton("secondary", rgba: opts.secondary, secondary: true)
            }
        } content: {
            VStack(alignment: .leading, spacing: DS.Space.sm) {
                selectorBody
            }
        }
        .onAppear(perform: pull)
        .onChange(of: current) { _ in pull() }
        .onChange(of: target.secondary) { _ in pull() }
    }
}
