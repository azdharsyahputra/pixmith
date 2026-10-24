import AppKit
import SwiftUI

// MARK: - Tokens
//
// Pixmith's visual language: a calm, native-feeling pro tool. Neutral surfaces, one accent (the system accent), hairline
// separators, 4/8pt spacing, 6pt control radius and 8pt panel radius. Everything adapts to Light and Dark.

enum DS {
    /// Spacing on a 4/8pt grid.
    enum Space {
        static let xxs: CGFloat = 2
        static let xs: CGFloat = 4
        static let sm: CGFloat = 8
        static let md: CGFloat = 12
        static let lg: CGFloat = 16
        static let xl: CGFloat = 24
    }

    enum Radius {
        static let chip: CGFloat = 4
        /// Inputs, segmented controls and popups.
        static let field: CGFloat = 5
        /// Icon buttons and other controls.
        static let control: CGFloat = 6
        static let panel: CGFloat = 8
    }

    enum Metric {
        /// Height of fields, popups and small (24pt) icon buttons.
        static let control: CGFloat = 24
        /// Standard icon button and tool button: the 28pt hit target.
        static let iconButton: CGFloat = 28
        static let toolButton: CGFloat = 28
        static let toolRail: CGFloat = 48
        static let inspectorWidth: CGFloat = 272
        /// Title bar and context bar.
        static let barHeight: CGFloat = 40
        static let timelineHeight: CGFloat = 252
        static let timelineHeader: CGFloat = 36
        static let timelineRow: CGFloat = 24
        static let statusHeight: CGFloat = 22
        static let sectionHeader: CGFloat = 36
        /// Horizontal inset of inspector content and section titles.
        static let inset: CGFloat = 16
    }

    /// SF system fonts. Dense labels are 11pt, body 12-13pt; numbers use tabular digits.
    enum Font {
        static let caption = SwiftUI.Font.system(size: 11)
        static let captionMedium = SwiftUI.Font.system(size: 11, weight: .medium)
        static let body = SwiftUI.Font.system(size: 12)
        static let bodyLarge = SwiftUI.Font.system(size: 13)
        static let section = SwiftUI.Font.system(size: 11, weight: .semibold)
        static let title = SwiftUI.Font.system(size: 13, weight: .semibold)
        static let number = SwiftUI.Font.system(size: 11).monospacedDigit()
        static let numberLarge = SwiftUI.Font.system(size: 12).monospacedDigit()
        static let mono = SwiftUI.Font.system(size: 11, design: .monospaced)
    }

    enum Anim {
        /// Collapsing, hover and selection changes: smooth but subtle.
        static let quick = Animation.easeInOut(duration: 0.15)
    }

    /// "Title (K)" for tooltips.
    static func tip(_ title: String, _ shortcut: String? = nil) -> String {
        guard let shortcut, !shortcut.isEmpty, shortcut != "none" else { return title }
        return "\(title) (\(shortcut))"
    }
}

/// Semantic colors, adaptive to the effective appearance.
enum DSColor {
    private static func ns(_ v: UInt32) -> NSColor {
        NSColor(srgbRed: CGFloat(v >> 24 & 0xFF) / 255, green: CGFloat(v >> 16 & 0xFF) / 255,
                blue: CGFloat(v >> 8 & 0xFF) / 255, alpha: CGFloat(v & 0xFF) / 255)
    }

    /// An adaptive color. A color theme (`ThemeStore`, from an extension) can override `token` per appearance.
    private static func dynamic(_ token: String, light: UInt32, dark: UInt32) -> NSColor {
        NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            if let v = ThemeStore.override(token, dark: isDark) { return ns(v) }
            return isDark ? ns(dark) : ns(light)
        }
    }

    /// A system color that a theme token can replace.
    private static func themed(_ token: String, fallback: NSColor) -> NSColor {
        NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            return ThemeStore.override(token, dark: isDark).map(ns) ?? fallback
        }
    }

    // NSColors (for AppKit drawing) -------------------------------------------------------------------------------
    // Dark values follow docs/design/mockup.html exactly; light values are the same hierarchy on a light ground.

    /// The area around the sprite.
    static let workspaceNS = dynamic("workspace", light: 0xE9E9EBFF, dark: 0x1B1B1DFF)
    static let panelNS = dynamic("panel", light: 0xF6F6F7FF, dark: 0x262628FF)
    /// A step lighter than the panel: the active document tab.
    static let panel2NS = dynamic("panel2", light: 0xFFFFFFFF, dark: 0x2E2E31FF)
    static let fieldNS = dynamic("field", light: 0xE8E8EBFF, dark: 0x333336FF)
    static let hoverNS = dynamic("hover", light: 0xDEDEE2FF, dark: 0x38383CFF)
    static let pressedNS = dynamic("pressed", light: 0xD3D3D8FF, dark: 0x424247FF)
    static let separatorNS = dynamic("separator", light: 0xCFCFD4FF, dark: 0x3A3A3EFF)
    static let separatorSoftNS = dynamic("separatorSoft", light: 0xE0E0E4FF, dark: 0x303033FF)
    static let elevatedNS = dynamic("elevated", light: 0xFFFFFFFF, dark: 0x2E2E31FF)
    static let canvasShadowNS = dynamic("canvasShadow", light: 0x00000040, dark: 0x00000059)
    static let textNS = dynamic("text", light: 0x1D1D1FFF, dark: 0xECECEEFF)
    static let text2NS = dynamic("textSecondary", light: 0x65656BFF, dark: 0xA3A3A8FF)
    static let text3NS = dynamic("textTertiary", light: 0x9A9AA0FF, dark: 0x75757BFF)

    // SwiftUI colors ----------------------------------------------------------------------------------------------

    static let workspace = Color(nsColor: workspaceNS)
    /// Background of the bars, the tool rail, the inspector and the timeline.
    static let panel = Color(nsColor: panelNS)
    static let panel2 = Color(nsColor: panel2NS)
    /// Slightly recessed areas inside a panel.
    static let recessed = Color(nsColor: dynamic("recessed", light: 0x0000000A, dark: 0x00000033))
    /// Popovers and menus.
    static let elevated = Color(nsColor: elevatedNS)
    /// Lines between regions (title bar, timeline top).
    static let separator = Color(nsColor: separatorNS)
    /// Quieter lines between sections and rows.
    static let separatorSoft = Color(nsColor: separatorSoftNS)
    /// Resting fill of inputs, segmented controls and popups.
    static let field = Color(nsColor: fieldNS)
    static let fieldBorder = Color(nsColor: separatorNS)
    static let hover = Color(nsColor: hoverNS)
    static let pressed = Color(nsColor: pressedNS)
    static let accentNS = themed("accent", fallback: .controlAccentColor)
    static let accent = Color(nsColor: accentNS)
    /// Selected fills: the accent at 18%.
    static let selected = accent.opacity(0.18)
    static let selectedHover = accent.opacity(0.26)
    /// Outlines of selected things: the accent at 55%.
    static let accentLine = accent.opacity(0.55)
    static let destructive = Color(nsColor: themed("destructive", fallback: .systemRed))
    /// Faint frame-column guides in the timeline.
    static let guide = Color(nsColor: dynamic("guide", light: 0x00000009, dark: 0xFFFFFF09))
    /// Status chips floating over the canvas.
    static let overlay = Color(nsColor: dynamic("overlay", light: 0xF6F6F7E6, dark: 0x262628E6))

    static let text = Color(nsColor: textNS)
    static let textSecondary = Color(nsColor: text2NS)
    static let textTertiary = Color(nsColor: text3NS)
}

// MARK: - Separators and surfaces

/// One-pixel line in the separator color.
struct DSDivider: View {
    enum Axis { case horizontal, vertical }
    var axis: Axis = .horizontal
    var body: some View {
        Rectangle().fill(DSColor.separator)
            .frame(width: axis == .vertical ? 1 : nil, height: axis == .horizontal ? 1 : nil)
    }
}

/// A subtle vertical divider between groups inside a bar (20pt tall, 6pt margins).
struct DSGroupDivider: View {
    var body: some View { DSDivider(axis: .vertical).frame(height: 20).padding(.horizontal, DS.Space.xs) }
}

/// The short horizontal rule between tool groups in the rail.
struct DSRailDivider: View {
    var body: some View { DSDivider().frame(width: 20).padding(.vertical, DS.Space.xs + 2) }
}

/// Rounded surface used by panels and cards.
struct Panel<Content: View>: View {
    var padding: CGFloat = DS.Space.md
    @ViewBuilder var content: Content
    var body: some View {
        content
            .padding(padding)
            .background(DSColor.panel)
            .clipShape(RoundedRectangle(cornerRadius: DS.Radius.panel))
            .overlay(RoundedRectangle(cornerRadius: DS.Radius.panel).stroke(DSColor.separator, lineWidth: 1))
    }
}

/// Container for a row of tools: panel background, fixed height, a hairline on the chosen edge.
struct DSToolbar<Content: View>: View {
    enum Edge { case bottom, top, none }
    var edge: Edge = .bottom
    var height: CGFloat = DS.Metric.barHeight
    @ViewBuilder var content: Content

    var body: some View {
        HStack(spacing: DS.Space.xs) { content }
            .padding(.horizontal, DS.Space.md)
            .frame(maxWidth: .infinity, minHeight: height, maxHeight: height, alignment: .leading)
            .background(DSColor.panel)
            .overlay(alignment: edge == .top ? .top : .bottom) {
                if edge != .none { Rectangle().fill(DSColor.separatorSoft).frame(height: 1) }
            }
    }
}

extension View {
    /// Field chrome: resting fill, hairline border, accent border while focused.
    func dsFieldBackground(focused: Bool = false, radius: CGFloat = DS.Radius.field) -> some View {
        background(RoundedRectangle(cornerRadius: radius).fill(DSColor.field))
            .overlay(RoundedRectangle(cornerRadius: radius)
                .stroke(focused ? DSColor.accentLine : Color.clear, lineWidth: 1.5))
    }

    /// Background of a list row (layers, slices, history): selected fill, hover fill, otherwise clear.
    func dsRow(selected: Bool, hovering: Bool = false) -> some View {
        background(selected ? DSColor.selected : hovering ? DSColor.hover : Color.clear)
    }

    /// A single-line text field in the Pixmith style.
    func dsTextField() -> some View { modifier(DSTextFieldStyle()) }
}

private struct DSTextFieldStyle: ViewModifier {
    @FocusState private var focused: Bool
    func body(content: Content) -> some View {
        content
            .textFieldStyle(.plain)
            .font(DS.Font.body)
            .focused($focused)
            .padding(.horizontal, DS.Space.sm)
            .frame(height: DS.Metric.control)
            .dsFieldBackground(focused: focused)
    }
}

// MARK: - Buttons

/// Icon-only button style: hover, pressed and selected states. `solid` fills the selected state with the accent color
/// (used by the tool buttons); otherwise selection is a tinted fill.
struct DSIconButtonStyle: ButtonStyle {
    var selected = false
    var solid = false
    var destructive = false
    var radius: CGFloat = DS.Radius.control

    func makeBody(configuration: Configuration) -> some View {
        Inner(configuration: configuration, style: self)
    }

    private struct Inner: View {
        let configuration: ButtonStyleConfiguration
        let style: DSIconButtonStyle
        @State private var hover = false
        @Environment(\.isEnabled) private var enabled

        private var fill: Color {
            if style.selected {
                if style.solid { return configuration.isPressed ? DSColor.accent.opacity(0.8) : DSColor.accent }
                return configuration.isPressed || hover ? DSColor.selectedHover : DSColor.selected
            }
            if configuration.isPressed { return DSColor.pressed }
            return hover && enabled ? DSColor.hover : .clear
        }

        private var foreground: Color {
            if style.selected { return style.solid ? .white : DSColor.accent }
            if style.destructive { return hover ? DSColor.destructive : DSColor.textSecondary }
            return hover ? DSColor.text : DSColor.textSecondary
        }

        var body: some View {
            configuration.label
                .foregroundStyle(foreground)
                .background(RoundedRectangle(cornerRadius: style.radius).fill(fill))
                .contentShape(RoundedRectangle(cornerRadius: style.radius))
                .opacity(enabled ? 1 : 0.35)
                .onHover { hover = $0 }
                .animation(DS.Anim.quick, value: hover)
        }
    }
}

/// Fixed-size SF Symbol button with hover / pressed / selected states, a tooltip with the shortcut and an optional badge.
struct IconButton: View {
    let symbol: String
    var help: String
    var shortcut: String?
    var selected = false
    var badge: String?
    var size: CGFloat = DS.Metric.iconButton
    var iconSize: CGFloat = 14
    var destructive = false
    let action: () -> Void

    /// `size` 28 / `iconSize` 14 is the standard button; header actions pass `compact: true` (24 / 13).
    init(_ symbol: String, help: String, shortcut: String? = nil, selected: Bool = false, badge: String? = nil,
         compact: Bool = false, size: CGFloat? = nil, iconSize: CGFloat? = nil, destructive: Bool = false,
         action: @escaping () -> Void) {
        self.symbol = symbol
        self.help = help
        self.shortcut = shortcut
        self.selected = selected
        self.badge = badge
        self.size = size ?? (compact ? DS.Metric.control : DS.Metric.iconButton)
        self.iconSize = iconSize ?? (compact ? 13 : 14)
        self.destructive = destructive
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: iconSize, weight: .medium))
                .frame(width: size, height: size)
                .overlay(alignment: .topTrailing) {
                    if let badge {
                        Text(badge).font(.system(size: 8, weight: .bold)).foregroundStyle(.white)
                            .padding(.horizontal, 3).background(Capsule().fill(DSColor.accent))
                            .offset(x: 2, y: 1)
                    }
                }
        }
        .buttonStyle(DSIconButtonStyle(selected: selected, destructive: destructive))
        .help(DS.tip(help, shortcut))
        .accessibilityLabel(help)
    }
}

/// A tool of the left rail: 28pt, accent-tinted fill and accent glyph when active.
struct ToolButton: View {
    let symbol: String
    let title: String
    var shortcut: String?
    var active: Bool
    var size: CGFloat = DS.Metric.toolButton
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: .regular))
                .frame(width: size, height: size)
        }
        .buttonStyle(DSIconButtonStyle(selected: active))
        .help(DS.tip(title, shortcut))
        .accessibilityLabel(title)
        .accessibilityAddTraits(active ? .isSelected : [])
    }
}

/// Accent-filled button. In dialogs it is the default action; in the title bar it carries a symbol (Export).
struct DSPrimaryButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        Inner(configuration: configuration)
    }

    private struct Inner: View {
        let configuration: ButtonStyleConfiguration
        @Environment(\.isEnabled) private var enabled
        @State private var hover = false
        var body: some View {
            configuration.label
                .font(DS.Font.bodyLarge.weight(.medium))
                .foregroundStyle(.white)
                .padding(.horizontal, DS.Space.md)
                .frame(minWidth: 64, minHeight: 28)
                .background(RoundedRectangle(cornerRadius: DS.Radius.control)
                    .fill(DSColor.accent.opacity(configuration.isPressed ? 0.75 : hover ? 0.92 : 1)))
                .opacity(enabled ? 1 : 0.4)
                .onHover { hover = $0 }
        }
    }
}

struct DSSecondaryButtonStyle: ButtonStyle {
    var destructive = false
    func makeBody(configuration: Configuration) -> some View {
        Inner(configuration: configuration, destructive: destructive)
    }

    private struct Inner: View {
        let configuration: ButtonStyleConfiguration
        let destructive: Bool
        @Environment(\.isEnabled) private var enabled
        @State private var hover = false
        var body: some View {
            configuration.label
                .font(DS.Font.bodyLarge)
                .foregroundStyle(destructive ? DSColor.destructive : DSColor.text)
                .padding(.horizontal, DS.Space.md)
                .frame(minWidth: 64, minHeight: 28)
                .background(RoundedRectangle(cornerRadius: DS.Radius.control)
                    .fill(configuration.isPressed ? DSColor.pressed : hover ? DSColor.hover : DSColor.field))
                .opacity(enabled ? 1 : 0.4)
                .onHover { hover = $0 }
        }
    }
}

/// Compact text button for bars ("Apply", "Reset").
struct DSBarButtonStyle: ButtonStyle {
    var prominent = false
    func makeBody(configuration: Configuration) -> some View {
        Inner(configuration: configuration, prominent: prominent)
    }

    private struct Inner: View {
        let configuration: ButtonStyleConfiguration
        let prominent: Bool
        @Environment(\.isEnabled) private var enabled
        @State private var hover = false
        var body: some View {
            configuration.label
                .font(DS.Font.captionMedium)
                .foregroundStyle(prominent ? Color.white : DSColor.text)
                .padding(.horizontal, DS.Space.sm + 2)
                .frame(minHeight: DS.Metric.control)
                .background(RoundedRectangle(cornerRadius: DS.Radius.control).fill(
                    prominent ? DSColor.accent.opacity(configuration.isPressed ? 0.75 : hover ? 0.92 : 1)
                        : (configuration.isPressed ? DSColor.pressed : hover ? DSColor.hover : DSColor.field)))
                .opacity(enabled ? 1 : 0.4)
                .onHover { hover = $0 }
        }
    }
}

// MARK: - Toggle chip, segmented control, popup

/// A pill toggle for tool options ("Filled", "Pixel-perfect"); optional leading symbol.
struct DSChip: View {
    let title: String?
    var symbol: String?
    var help: String
    @Binding var isOn: Bool

    init(_ title: String? = nil, symbol: String? = nil, help: String = "", isOn: Binding<Bool>) {
        self.title = title
        self.symbol = symbol
        self.help = help.isEmpty ? (title ?? "") : help
        _isOn = isOn
    }

    var body: some View {
        Button { isOn.toggle() } label: {
            HStack(spacing: 5) {
                if let symbol { Image(systemName: symbol).font(.system(size: 14, weight: .medium)) }
                if let title { Text(title).font(DS.Font.captionMedium) }
            }
            .padding(.horizontal, title == nil ? 0 : DS.Space.sm + 2)
            .frame(width: title == nil ? DS.Metric.iconButton : nil, height: title == nil ? DS.Metric.iconButton : DS.Metric.control + 2)
            .background(RoundedRectangle(cornerRadius: DS.Radius.control).fill(title != nil && !isOn ? DSColor.field : .clear))
        }
        .buttonStyle(DSIconButtonStyle(selected: isOn))
        .help(help)
        .accessibilityLabel(help)
        .accessibilityAddTraits(isOn ? .isSelected : [])
    }
}

/// Segmented control: a recessed track with a raised selection pill.
struct DSSegmented<Value: Hashable>: View {
    struct Option {
        var value: Value
        var title: String?
        var symbol: String?
        var help: String
        init(_ value: Value, title: String? = nil, symbol: String? = nil, help: String = "") {
            self.value = value
            self.title = title
            self.symbol = symbol
            self.help = help.isEmpty ? (title ?? "") : help
        }
    }

    @Binding var selection: Value
    let options: [Option]
    var segmentWidth: CGFloat?
    @Namespace private var pill

    init(selection: Binding<Value>, options: [Option], segmentWidth: CGFloat? = nil) {
        _selection = selection
        self.options = options
        self.segmentWidth = segmentWidth
    }

    var body: some View {
        HStack(spacing: 2) {
            ForEach(options.indices, id: \.self) { i in
                let o = options[i]
                let on = o.value == selection
                Button { withAnimation(DS.Anim.quick) { selection = o.value } } label: {
                    HStack(spacing: 4) {
                        if let s = o.symbol { Image(systemName: s).font(.system(size: 11, weight: .medium)) }
                        if let t = o.title { Text(t).font(DS.Font.captionMedium).lineLimit(1) }
                    }
                    .foregroundStyle(on ? DSColor.text : DSColor.textTertiary)
                    .padding(.horizontal, o.title == nil ? 0 : DS.Space.sm)
                    .frame(width: segmentWidth ?? (o.title == nil ? 22 : nil), height: DS.Metric.control - 4)
                    .background {
                        if on {
                            RoundedRectangle(cornerRadius: DS.Radius.chip)
                                .fill(DSColor.hover)
                                .matchedGeometryEffect(id: "pill", in: pill)
                        }
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help(o.help)
                .accessibilityLabel(o.help)
                .accessibilityAddTraits(on ? .isSelected : [])
            }
        }
        .padding(2)
        .background(RoundedRectangle(cornerRadius: DS.Radius.field).fill(DSColor.field))
    }
}

/// Dropdown in the field style.
struct DSPopup<Value: Hashable>: View {
    @Binding var selection: Value
    let options: [(value: Value, title: String)]
    var width: CGFloat?
    var help: String = ""
    /// Stretch to the available width instead of hugging the title.
    var fill = false

    init(selection: Binding<Value>, options: [(value: Value, title: String)], width: CGFloat? = nil, help: String = "",
         fill: Bool = false) {
        _selection = selection
        self.options = options
        self.width = width
        self.help = help
        self.fill = fill
    }

    var body: some View {
        Menu {
            ForEach(options.indices, id: \.self) { i in
                let o = options[i]
                Button {
                    selection = o.value
                } label: {
                    if o.value == selection { Label(o.title, systemImage: "checkmark") } else { Text(o.title) }
                }
            }
        } label: {
            HStack(spacing: DS.Space.xs) {
                Text(options.first { $0.value == selection }?.title ?? "")
                    .font(DS.Font.body).lineLimit(1)
                Spacer(minLength: 0)
                Image(systemName: "chevron.down").font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(DSColor.textTertiary)
            }
            .padding(.horizontal, DS.Space.sm)
            .frame(width: width, height: DS.Metric.control)
            .frame(maxWidth: fill ? .infinity : nil)
            .background(RoundedRectangle(cornerRadius: DS.Radius.field).fill(DSColor.field))
            .contentShape(Rectangle())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize(horizontal: !fill, vertical: true)
        .help(help)
    }
}

// MARK: - Numeric controls

/// A number field with a label and optional unit. Drag the label to scrub the value (hold Shift for 10x), or type.
struct DSNumberField: View {
    let label: String
    @Binding var value: Double
    var range: ClosedRange<Double> = 0...100
    /// Value change per point of horizontal drag.
    var step: Double = 1
    var unit = ""
    var digits = 0
    var width: CGFloat? = 80
    var help = ""
    var onEditing: (Bool) -> Void = { _ in }
    /// When true a scrub shows the live value but writes the binding only when the drag ends (one undo step).
    var deferScrub = false

    @State private var live: Double?
    @State private var scrubStart: Double?
    @State private var cursorPushed = false
    @FocusState private var focused: Bool

    init(_ label: String, value: Binding<Double>, range: ClosedRange<Double> = 0...100, step: Double = 1,
         unit: String = "", digits: Int = 0, width: CGFloat? = 80, help: String = "", deferScrub: Bool = false,
         onEditing: @escaping (Bool) -> Void = { _ in }) {
        self.deferScrub = deferScrub
        self.label = label
        _value = value
        self.range = range
        self.step = step
        self.unit = unit
        self.digits = digits
        self.width = width
        self.help = help
        self.onEditing = onEditing
    }

    init(_ label: String, value: Binding<Int>, range: ClosedRange<Int> = 0...100, unit: String = "",
         width: CGFloat? = 80, help: String = "", deferScrub: Bool = false, onEditing: @escaping (Bool) -> Void = { _ in }) {
        self.init(label, value: Binding(get: { Double(value.wrappedValue) }, set: { value.wrappedValue = Int($0.rounded()) }),
                  range: Double(range.lowerBound)...Double(range.upperBound), unit: unit, width: width, help: help,
                  deferScrub: deferScrub, onEditing: onEditing)
    }

    private func clamp(_ v: Double) -> Double {
        let r = digits == 0 ? v.rounded() : (v * pow(10, Double(digits))).rounded() / pow(10, Double(digits))
        return min(max(r, range.lowerBound), range.upperBound)
    }

    var body: some View {
        HStack(spacing: DS.Space.xs) {
            Text(label)
                .font(DS.Font.captionMedium)
                .foregroundStyle(scrubStart != nil ? DSColor.accent : DSColor.textTertiary)
                .padding(.vertical, 4)
                .contentShape(Rectangle())
                .onHover { inside in
                    if inside, !cursorPushed { NSCursor.resizeLeftRight.push(); cursorPushed = true }
                    if !inside, cursorPushed, scrubStart == nil { NSCursor.pop(); cursorPushed = false }
                }
                .gesture(DragGesture(minimumDistance: 1)
                    .onChanged { g in
                        if scrubStart == nil { scrubStart = value; onEditing(true) }
                        let fast = NSEvent.modifierFlags.contains(.shift) ? 10.0 : 1.0
                        let v = clamp((scrubStart ?? value) + Double(g.translation.width) * step * fast)
                        if deferScrub { live = v } else { value = v }
                    }
                    .onEnded { _ in
                        if let l = live { value = l; live = nil }
                        scrubStart = nil
                        onEditing(false)
                        if cursorPushed { NSCursor.pop(); cursorPushed = false }
                    })
            TextField("", value: Binding(get: { live ?? value }, set: { value = clamp($0) }),
                      format: .number.precision(.fractionLength(0...digits)))
                .textFieldStyle(.plain)
                .font(DS.Font.numberLarge)
                .foregroundStyle(DSColor.text)
                .multilineTextAlignment(.trailing)
                .focused($focused)
            if !unit.isEmpty {
                Text(unit).font(DS.Font.caption).foregroundStyle(DSColor.textTertiary)
            }
        }
        .padding(.horizontal, DS.Space.sm)
        .frame(width: width, height: DS.Metric.control)
        .dsFieldBackground(focused: focused)
        .help(help.isEmpty ? "Drag the label \(label) to scrub" : help)
        .accessibilityElement(children: .combine)
        .accessibilityValue("\(Int(value)) \(unit)")
        .accessibilityAdjustableAction { dir in
            value = clamp(value + (dir == .increment ? 1 : -1) * max(step, digits == 0 ? 1 : step))
        }
    }
}

/// A slider that lives inside a field: label left, value right, the fill shows the position. Drag anywhere.
struct DSSlider: View {
    let label: String
    @Binding var value: Double
    var range: ClosedRange<Double>
    var unit = ""
    var format: (Double) -> String = { String(Int($0.rounded())) }
    var onEditing: (Bool) -> Void = { _ in }
    @State private var dragging = false

    init(_ label: String, value: Binding<Double>, range: ClosedRange<Double>, unit: String = "",
         format: @escaping (Double) -> String = { String(Int($0.rounded())) },
         onEditing: @escaping (Bool) -> Void = { _ in }) {
        self.label = label
        _value = value
        self.range = range
        self.unit = unit
        self.format = format
        self.onEditing = onEditing
    }

    var body: some View {
        GeometryReader { geo in
            let frac = (min(max(value, range.lowerBound), range.upperBound) - range.lowerBound) / (range.upperBound - range.lowerBound)
            ZStack(alignment: .leading) {
                RoundedRectangle(cornerRadius: DS.Radius.field).fill(DSColor.field)
                // Ranges that cross zero fill from the zero point, so +/- adjustments read at a glance.
                let zero = range.lowerBound < 0 && range.upperBound > 0
                    ? (0 - range.lowerBound) / (range.upperBound - range.lowerBound) : 0
                Rectangle().fill(DSColor.accent.opacity(dragging ? 0.34 : 0.22))
                    .frame(width: max(0, geo.size.width * abs(frac - zero)))
                    .offset(x: geo.size.width * min(frac, zero))
                    .clipShape(RoundedRectangle(cornerRadius: DS.Radius.field))
                HStack {
                    Text(label).font(DS.Font.captionMedium).foregroundStyle(DSColor.textTertiary)
                    Spacer(minLength: 0)
                    Text(format(value) + unit).font(DS.Font.number).foregroundStyle(DSColor.text)
                }
                .padding(.horizontal, DS.Space.sm)
            }
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0)
                .onChanged { g in
                    if !dragging { dragging = true; onEditing(true) }
                    let f = min(max(g.location.x / max(geo.size.width, 1), 0), 1)
                    value = range.lowerBound + f * (range.upperBound - range.lowerBound)
                }
                .onEnded { _ in dragging = false; onEditing(false) })
        }
        .frame(height: DS.Metric.control)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
        .accessibilityValue(format(value) + unit)
        .accessibilityAdjustableAction { dir in
            let d = (range.upperBound - range.lowerBound) / 20
            value = min(max(value + (dir == .increment ? d : -d), range.lowerBound), range.upperBound)
        }
    }
}

// MARK: - Swatches, key hints

/// Two-tone checkerboard for showing transparency.
struct DSChecker: View {
    var cell: CGFloat = 4
    var body: some View {
        Canvas { ctx, size in
            ctx.fill(Path(CGRect(origin: .zero, size: size)), with: .color(Color(white: 0.92)))
            var path = Path()
            let cols = Int(ceil(size.width / cell)), rows = Int(ceil(size.height / cell))
            for r in 0..<rows { for c in 0..<cols where (r + c) % 2 == 1 {
                path.addRect(CGRect(x: CGFloat(c) * cell, y: CGFloat(r) * cell, width: cell, height: cell))
            } }
            ctx.fill(path, with: .color(Color(white: 0.72)))
        }
    }
}

/// A color chip: shows transparency behind the color, with an optional selection ring.
struct Swatch: View {
    let color: Color
    var size: CGFloat = 20
    var selected = false
    var radius: CGFloat = 4

    var body: some View {
        ZStack {
            DSChecker(cell: max(2, size / 5))
            color
        }
        .frame(width: size, height: size)
        .clipShape(RoundedRectangle(cornerRadius: radius))
        .overlay(RoundedRectangle(cornerRadius: radius).stroke(Color.white.opacity(0.07), lineWidth: 1))
        .overlay {
            if selected {
                RoundedRectangle(cornerRadius: radius + 1.5).stroke(DSColor.accent, lineWidth: 2).padding(-2.5)
            }
        }
    }
}

/// A shortcut shown as a little keycap.
struct KeyHint: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View {
        Text(text)
            .font(.system(size: 10, weight: .medium, design: .rounded))
            .foregroundStyle(DSColor.textSecondary)
            .padding(.horizontal, 5)
            .frame(minWidth: 16, minHeight: 16)
            .background(RoundedRectangle(cornerRadius: DS.Radius.chip).fill(DSColor.field))
    }
}

/// A short hint: keycaps followed by a description.
struct HintLabel: View {
    var keys: [String] = []
    let text: String
    var body: some View {
        HStack(spacing: DS.Space.xs) {
            ForEach(keys, id: \.self) { KeyHint($0) }
            Text(text).font(DS.Font.caption).foregroundStyle(DSColor.textSecondary).lineLimit(1)
        }
    }
}

// MARK: - Inspector section

/// A collapsible inspector section: header with chevron, title and trailing actions. The collapsed state is remembered
/// in UserDefaults under `panel.<id>.collapsed`.
struct InspectorSection<Content: View, Trailing: View>: View {
    let title: String
    /// Quiet text after the title ("32", "Outline · frame 2").
    var subtitle: String?
    @AppStorage private var collapsed: Bool
    let trailing: Trailing
    let content: Content
    var padded = true

    init(_ title: String, id: String, subtitle: String? = nil, padded: Bool = true,
         @ViewBuilder trailing: () -> Trailing, @ViewBuilder content: () -> Content) {
        self.title = title
        self.subtitle = subtitle
        _collapsed = AppStorage(wrappedValue: false, "panel.\(id).collapsed")
        self.padded = padded
        self.trailing = trailing()
        self.content = content()
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: DS.Space.xs) {
                Button {
                    withAnimation(DS.Anim.quick) { collapsed.toggle() }
                } label: {
                    HStack(spacing: DS.Space.xs) {
                        Image(systemName: "chevron.right")
                            .font(.system(size: 8, weight: .bold))
                            .foregroundStyle(DSColor.textTertiary)
                            .rotationEffect(.degrees(collapsed ? 0 : 90))
                            .frame(width: 12)
                        Text(title).font(DS.Font.section).foregroundStyle(DSColor.text)
                        if let subtitle {
                            Text(subtitle).font(DS.Font.caption).foregroundStyle(DSColor.textTertiary).lineLimit(1)
                        }
                        Spacer(minLength: 0)
                    }
                    .frame(minHeight: DS.Metric.sectionHeader)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help(collapsed ? "Expand \(title)" : "Collapse \(title)")
                .accessibilityLabel("\(title), \(collapsed ? "collapsed" : "expanded")")
                trailing
            }
            .padding(.leading, DS.Metric.inset)
            .padding(.trailing, DS.Space.sm)
            if !collapsed {
                content
                    .padding(.horizontal, padded ? DS.Metric.inset : 0)
                    .padding(.bottom, 14)
                    .transition(.opacity)
            }
            Rectangle().fill(DSColor.separatorSoft).frame(height: 1)
        }
    }
}

extension InspectorSection where Trailing == EmptyView {
    init(_ title: String, id: String, subtitle: String? = nil, padded: Bool = true, @ViewBuilder content: () -> Content) {
        self.init(title, id: id, subtitle: subtitle, padded: padded, trailing: { EmptyView() }, content: content)
    }
}

// MARK: - Dialogs

/// Standard layout of sheets: title, content, then Cancel and a primary button. Return triggers the primary button,
/// Escape cancels.
struct DSDialog<Content: View>: View {
    let title: String
    var subtitle: String?
    var primary: String
    var primaryEnabled = true
    var showsCancel = true
    var cancelTitle = "Cancel"
    var destructive: (title: String, action: () -> Void)?
    var width: CGFloat = 340
    let onPrimary: () -> Void
    let onCancel: () -> Void
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: DS.Space.lg) {
            VStack(alignment: .leading, spacing: DS.Space.xs) {
                Text(title).font(.system(size: 15, weight: .semibold))
                if let subtitle { Text(subtitle).font(DS.Font.body).foregroundStyle(DSColor.textSecondary) }
            }
            VStack(alignment: .leading, spacing: DS.Space.md) { content }
            HStack(spacing: DS.Space.sm) {
                if let destructive {
                    Button(destructive.title, action: destructive.action).buttonStyle(DSSecondaryButtonStyle(destructive: true))
                }
                Spacer(minLength: 0)
                if showsCancel {
                    Button(cancelTitle, action: onCancel).buttonStyle(DSSecondaryButtonStyle()).keyboardShortcut(.cancelAction)
                }
                Button(primary, action: onPrimary).buttonStyle(DSPrimaryButtonStyle())
                    .keyboardShortcut(.defaultAction).disabled(!primaryEnabled)
            }
        }
        .padding(DS.Space.xl - 4)
        .frame(width: width)
        .background(DSColor.panel)
    }
}

/// A label-left, control-right row with the label column aligned across rows.
struct DSFormRow<Content: View>: View {
    let label: String
    var labelWidth: CGFloat = 92
    @ViewBuilder var content: Content
    var body: some View {
        HStack(spacing: DS.Space.sm) {
            Text(label).font(DS.Font.body).foregroundStyle(DSColor.textSecondary)
                .frame(width: labelWidth, alignment: .leading)
            content
        }
    }
}

/// A caption under a control.
struct DSNote: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View {
        Text(text).font(DS.Font.caption).foregroundStyle(DSColor.textSecondary).fixedSize(horizontal: false, vertical: true)
    }
}

// MARK: - Color helpers

extension Color {
    /// Builds a color from `0xRRGGBBAA`.
    init(rgba: UInt32) { self.init(cgColor: rgba.cgColor) }
}

// MARK: - Document tabs, overlay chips, info rows

/// A document tab for the title bar: 28pt tall, the active tab sits on the lifted panel color, a dot marks unsaved
/// changes. The tab bar view composes these.
struct DSTab: View {
    let title: String
    var active = false
    var dirty = false
    var onSelect: () -> Void = {}
    var onClose: (() -> Void)?
    @State private var hover = false

    var body: some View {
        HStack(spacing: DS.Space.sm) {
            Text(title).font(DS.Font.bodyLarge).lineLimit(1).truncationMode(.middle)
                .foregroundStyle(active ? DSColor.text : DSColor.textSecondary)
            if let onClose, hover || active && !dirty {
                IconButton("xmark", help: "Close", compact: true, size: 18, iconSize: 9, action: onClose)
            } else if dirty {
                Circle().fill(DSColor.textSecondary).frame(width: 6, height: 6)
            }
        }
        .padding(.leading, 12).padding(.trailing, 10)
        .frame(height: DS.Metric.iconButton)
        .background(RoundedRectangle(cornerRadius: DS.Radius.control)
            .fill(active ? DSColor.panel2 : hover ? DSColor.hover.opacity(0.5) : Color.clear))
        .contentShape(Rectangle())
        .onTapGesture(perform: onSelect)
        .onHover { hover = $0 }
        .accessibilityAddTraits(active ? .isSelected : [])
    }
}

/// A small status chip that floats over the canvas (size, cursor position).
struct DSOverlayChip: View {
    var symbol: String?
    let text: String
    var body: some View {
        HStack(spacing: 6) {
            if let symbol { Image(systemName: symbol).font(.system(size: 10)).foregroundStyle(DSColor.textTertiary) }
            Text(text).font(DS.Font.number).foregroundStyle(symbol == nil ? DSColor.textTertiary : DSColor.text)
        }
        .padding(.horizontal, DS.Space.sm)
        .frame(height: 22)
        .background(RoundedRectangle(cornerRadius: DS.Radius.field).fill(DSColor.overlay))
    }
}

/// Label / value rows for read-only info (the Sprite section): quiet labels, primary values.
struct DSInfoGrid: View {
    let rows: [(label: String, value: String)]
    var body: some View {
        Grid(alignment: .leading, horizontalSpacing: DS.Space.lg, verticalSpacing: DS.Space.sm) {
            ForEach(rows.indices, id: \.self) { i in
                GridRow {
                    Text(rows[i].label).font(DS.Font.caption).foregroundStyle(DSColor.textTertiary)
                    Text(rows[i].value).font(DS.Font.body.monospacedDigit()).foregroundStyle(DSColor.text)
                }
            }
        }
    }
}

/// A read-only value shown in field chrome (the hex swatch field, "Linked").
struct DSValueField<Leading: View>: View {
    let text: String
    var tertiary = false
    @ViewBuilder var leading: Leading
    var body: some View {
        HStack(spacing: 6) {
            leading
            Text(text).font(DS.Font.numberLarge).foregroundStyle(tertiary ? DSColor.textSecondary : DSColor.text)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, DS.Space.sm)
        .frame(height: DS.Metric.control)
        .background(RoundedRectangle(cornerRadius: DS.Radius.field).fill(DSColor.field))
    }
}

/// An icon-only menu button (the "more" menus of section headers) with the same hover states as `IconButton`.
struct DSMenuButton<Content: View>: View {
    var symbol = "ellipsis"
    let help: String
    var compact = true
    @ViewBuilder var content: Content

    var body: some View {
        let size = compact ? DS.Metric.control : DS.Metric.iconButton
        Menu {
            content
        } label: {
            Image(systemName: symbol).font(.system(size: compact ? 13 : 14, weight: .medium))
                .frame(width: size, height: size)
        }
        .menuStyle(.button)
        .buttonStyle(DSIconButtonStyle())
        .menuIndicator(.hidden)
        .fixedSize()
        .help(help)
        .accessibilityLabel(help)
    }
}
