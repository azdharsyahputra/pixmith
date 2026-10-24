import AppKit
import SwiftUI

// Pieces shared by the Home screen, the project view and the editor's project switcher.

extension DSColor {
    /// Ground of the Home screen and the project view: white in Light so the field-colored controls and the
    /// panel-colored cards read against it, the workspace color in Dark.
    static let browse = Color(nsColor: NSColor(name: nil) { appearance in
        appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            ? NSColor(srgbRed: 0x1B / 255, green: 0x1B / 255, blue: 0x1D / 255, alpha: 1) : .white
    })
}

/// A search box in the field style: magnifier, plain text field, clear button. Return and Escape give the focus back
/// to the window (so arrow keys work in the list below); Escape also clears the text.
struct DSSearchField: View {
    @Binding var text: String
    var prompt = "Search"
    var width: CGFloat? = 200
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass").font(.system(size: 11, weight: .medium)).foregroundStyle(DSColor.textTertiary)
            TextField(prompt, text: $text)
                .textFieldStyle(.plain)
                .font(DS.Font.body)
                .focused($focused)
                .onSubmit { focused = false }
                .onExitCommand { text = ""; focused = false }
            if !text.isEmpty {
                Button { text = "" } label: {
                    Image(systemName: "xmark.circle.fill").font(.system(size: 11)).foregroundStyle(DSColor.textTertiary)
                }
                .buttonStyle(.plain)
                .help("Clear search")
            }
        }
        .padding(.horizontal, DS.Space.sm)
        .frame(width: width, height: DS.Metric.control)
        .dsFieldBackground(focused: focused)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(prompt)
    }
}

/// The transparency checkerboard behind a thumbnail: quiet in both appearances, like the canvas's own.
struct DSCheckerBackdrop: View {
    var cell: CGFloat = 8
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let dark = scheme == .dark
        let a = Color(rgba: dark ? 0x2C2C2FFF : 0xF4F4F6FF), b = Color(rgba: dark ? 0x353538FF : 0xE6E6EAFF)
        Canvas { ctx, size in
            ctx.fill(Path(CGRect(origin: .zero, size: size)), with: .color(a))
            var path = Path()
            let cols = Int(ceil(size.width / cell)), rows = Int(ceil(size.height / cell))
            for r in 0..<rows {
                for c in 0..<cols where (r + c) % 2 == 1 {
                    path.addRect(CGRect(x: CGFloat(c) * cell, y: CGFloat(r) * cell, width: cell, height: cell))
                }
            }
            ctx.fill(path, with: .color(b))
        }
    }
}

/// A short message pill at the bottom of a screen, with an optional action ("Undo").
struct DSToast: View {
    let message: String
    var actionTitle: String?
    var action: (() -> Void)?

    var body: some View {
        HStack(spacing: DS.Space.md) {
            Text(message).font(DS.Font.bodyLarge).foregroundStyle(DSColor.text).lineLimit(1)
            if let actionTitle, let action {
                Button(actionTitle, action: action)
                    .buttonStyle(.plain)
                    .font(DS.Font.bodyLarge.weight(.semibold))
                    .foregroundStyle(DSColor.accent)
            }
        }
        .padding(.horizontal, DS.Space.lg)
        .frame(height: 36)
        .background(RoundedRectangle(cornerRadius: DS.Radius.panel).fill(DSColor.elevated))
        .overlay(RoundedRectangle(cornerRadius: DS.Radius.panel).stroke(DSColor.separator, lineWidth: 1))
        .shadow(color: .black.opacity(0.25), radius: 12, y: 4)
    }
}

/// A preview of a document file on a checkerboard, loaded in the background. Small sprites are enlarged by whole
/// factors so their pixels stay square; large ones are reduced to fit.
struct CanvasThumb: View {
    let url: URL
    let key: String
    var padding: CGFloat = 12
    var cell: CGFloat = 8
    @State private var image: CGImage?
    @State private var failed = false

    var body: some View {
        ZStack {
            DSCheckerBackdrop(cell: cell)
            if let image {
                GeometryReader { geo in
                    let w = CGFloat(image.width), h = CGFloat(image.height)
                    let fit = min((geo.size.width - padding * 2) / w, (geo.size.height - padding * 2) / h)
                    let scale = fit >= 1 ? fit.rounded(.down) : max(fit, 0.01)
                    Image(decorative: image, scale: 1)
                        .resizable()
                        .interpolation(.none)
                        .frame(width: w * scale, height: h * scale)
                        .position(x: geo.size.width / 2, y: geo.size.height / 2)
                }
            } else if failed {
                Image(systemName: "exclamationmark.triangle").font(.system(size: 16)).foregroundStyle(DSColor.textTertiary)
            }
        }
        .task(id: key) {
            failed = false
            if let hit = ThumbnailLoader.shared.cached(key) {
                image = hit
                failed = hit == nil
                return
            }
            image = nil
            let loaded = await ThumbnailLoader.shared.load(url, key: key)
            guard !Task.isCancelled else { return }
            image = loaded
            failed = loaded == nil
        }
    }
}

/// The surface of a card: panel fill, hairline border, accent outline when selected.
struct CardSurface: ViewModifier {
    var selected = false
    var hovering = false
    var dropEdge: Edge.Set?

    func body(content: Content) -> some View {
        content
            .background(DSColor.panel)
            .clipShape(RoundedRectangle(cornerRadius: DS.Radius.panel))
            .overlay(RoundedRectangle(cornerRadius: DS.Radius.panel)
                .stroke(selected ? DSColor.accent : hovering ? DSColor.separator : DSColor.separatorSoft, lineWidth: selected ? 2 : 1))
            .overlay(alignment: dropEdge == .trailing ? .trailing : .leading) {
                if dropEdge != nil {
                    RoundedRectangle(cornerRadius: 1.5).fill(DSColor.accent).frame(width: 3).padding(.vertical, 4).offset(x: dropEdge == .trailing ? 9 : -9)
                }
            }
    }
}

extension View {
    func cardSurface(selected: Bool = false, hovering: Bool = false, dropEdge: Edge.Set? = nil) -> some View {
        modifier(CardSurface(selected: selected, hovering: hovering, dropEdge: dropEdge))
    }
}

enum Ago {
    private static let formatter: RelativeDateTimeFormatter = {
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .short
        f.dateTimeStyle = .numeric
        return f
    }()

    /// "Just now", "5 min. ago", "2 days ago".
    static func text(_ date: Date, now: Date = Date()) -> String {
        if date == .distantPast { return "–" }
        if now.timeIntervalSince(date) < 45 { return "Just now" }
        return formatter.localizedString(for: date, relativeTo: now)
    }
}
