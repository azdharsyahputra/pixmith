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

/// Hue (degrees), saturation, value and alpha, all but hue in 0...1.
struct HSVA: Equatable {
    var h: Double = 0
    var s: Double = 0
    var v: Double = 0
    var a: Double = 1

    /// From `0xRRGGBBAA`. `hue` is kept when the color is gray (hue is undefined there) so dragging stays stable.
    init(rgba: UInt32, hue: Double? = nil) {
        let r = Double(rgba >> 24 & 0xFF) / 255, g = Double(rgba >> 16 & 0xFF) / 255
        let b = Double(rgba >> 8 & 0xFF) / 255
        a = Double(rgba & 0xFF) / 255
        let mx = max(r, g, b), mn = min(r, g, b), d = mx - mn
        v = mx
        s = mx == 0 ? 0 : d / mx
        if d == 0 {
            h = hue ?? 0
        } else if mx == r {
            h = 60 * ((g - b) / d).truncatingRemainder(dividingBy: 6)
        } else if mx == g {
            h = 60 * ((b - r) / d + 2)
        } else {
            h = 60 * ((r - g) / d + 4)
        }
        if h < 0 { h += 360 }
        if s == 0 || v == 0, let hue { h = hue }
    }

    init(h: Double, s: Double, v: Double, a: Double) {
        (self.h, self.s, self.v, self.a) = (h, s, v, a)
    }

    var rgba: UInt32 {
        let c = v * s
        let hp = h.truncatingRemainder(dividingBy: 360) / 60
        let x = c * (1 - abs(hp.truncatingRemainder(dividingBy: 2) - 1))
        let (r1, g1, b1): (Double, Double, Double)
        switch Int(hp) {
        case 0: (r1, g1, b1) = (c, x, 0)
        case 1: (r1, g1, b1) = (x, c, 0)
        case 2: (r1, g1, b1) = (0, c, x)
        case 3: (r1, g1, b1) = (0, x, c)
        case 4: (r1, g1, b1) = (x, 0, c)
        default: (r1, g1, b1) = (c, 0, x)
        }
        let m = v - c
        func byte(_ f: Double) -> UInt32 { UInt32(min(max((f * 255).rounded(), 0), 255)) }
        return byte(r1 + m) << 24 | byte(g1 + m) << 16 | byte(b1 + m) << 8 | byte(a)
    }
}

/// A horizontal track with a draggable thumb; `background` paints the track.
private struct TrackSlider<Background: View>: View {
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
    @Binding var hsva: HSVA
    var changed: () -> Void

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
                hsva.s = min(max(g.location.x / max(geo.size.width, 1), 0), 1)
                hsva.v = 1 - min(max(g.location.y / max(geo.size.height, 1), 0), 1)
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

/// Inspector section: saturation-value plane, hue and alpha sliders, hex, alpha and HSV fields. Edits the primary or
/// the secondary color, whichever is selected in the header (or on the tool rail).
struct ColorSection: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject var opts: DrawingOptions
    @ObservedObject private var target = ColorTarget.shared
    @State private var hsva = HSVA(rgba: 0x000000FF)
    @State private var hex = ""
    @FocusState private var hexFocused: Bool

    init(model: SpriteModel) {
        self.model = model
        opts = model.opts
    }

    private var current: UInt32 { target.secondary ? opts.secondary : model.color }

    private func apply() {
        let c = hsva.rgba
        if target.secondary { opts.secondary = c } else { model.color = c }
    }

    private func pull() {
        let c = current
        if c != hsva.rgba { hsva = HSVA(rgba: c, hue: hsva.h) }
        if !hexFocused { hex = Self.hexString(c) }
    }

    private static func hexString(_ c: UInt32) -> String { String(format: "%06X", c >> 8) }

    private func commitHex() {
        var t = hex.trimmingCharacters(in: CharacterSet(charactersIn: "# ")).uppercased()
        if t.count == 3 { t = t.map { "\($0)\($0)" }.joined() }
        guard t.count == 6 || t.count == 8, let v = UInt32(t, radix: 16) else { hex = Self.hexString(current); return }
        let rgba = t.count == 6 ? v << 8 | UInt32((hsva.a * 255).rounded()) : v
        hsva = HSVA(rgba: rgba, hue: hsva.h)
        apply()
        hex = Self.hexString(current)
    }

    private func number(_ label: String, _ keyPath: WritableKeyPath<HSVA, Double>, scale: Double, range: ClosedRange<Double>,
                        unit: String, help: String) -> some View {
        DSNumberField(label, value: Binding(
            get: { hsva[keyPath: keyPath] * scale },
            set: { hsva[keyPath: keyPath] = $0 / scale; apply() }),
            range: range, step: scale == 1 ? 1 : 0.5, unit: unit, width: nil, help: help)
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

    var body: some View {
        InspectorSection("Color", id: "color", subtitle: target.secondary ? "Secondary" : "Primary") {
            HStack(spacing: 0) {
                targetButton("primary", rgba: model.color, secondary: false)
                targetButton("secondary", rgba: opts.secondary, secondary: true)
            }
        } content: {
            VStack(alignment: .leading, spacing: DS.Space.sm) {
                SVPlane(hsva: $hsva, changed: apply)
                TrackSlider(value: Binding(get: { hsva.h }, set: { hsva.h = $0; apply() }), range: 0...360, label: "Hue") {
                    LinearGradient(colors: stride(from: 0.0, through: 1.0, by: 1.0 / 6).map {
                        Color(hue: $0, saturation: 1, brightness: 1)
                    }, startPoint: .leading, endPoint: .trailing)
                }
                TrackSlider(value: Binding(get: { hsva.a }, set: { hsva.a = $0; apply() }), range: 0...1, label: "Alpha") {
                    ZStack {
                        DSChecker(cell: 4)
                        LinearGradient(colors: [Color(rgba: HSVA(h: hsva.h, s: hsva.s, v: hsva.v, a: 0).rgba),
                                                Color(rgba: HSVA(h: hsva.h, s: hsva.s, v: hsva.v, a: 1).rgba)],
                                       startPoint: .leading, endPoint: .trailing)
                    }
                }
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
                    number("A", \.a, scale: 100, range: 0...100, unit: "%", help: "Alpha. Drag the label to scrub.")
                        .frame(width: 70)
                }
                HStack(spacing: 6) {
                    number("H", \.h, scale: 1, range: 0...359, unit: "°", help: "Hue")
                    number("S", \.s, scale: 100, range: 0...100, unit: "%", help: "Saturation")
                    number("V", \.v, scale: 100, range: 0...100, unit: "%", help: "Value")
                }
            }
        }
        .onAppear(perform: pull)
        .onChange(of: current) { _ in pull() }
        .onChange(of: target.secondary) { _ in pull() }
    }
}
