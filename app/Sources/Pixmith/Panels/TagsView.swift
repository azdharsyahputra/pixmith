import PixmithCore
import SwiftUI

struct TagInfo {
    var name: String
    var from: Int
    var to: Int
    var direction: Int
    var repeatCount: Int
    /// 0xRRGGBB
    var color: UInt32
}

enum TagDirection: Int, CaseIterable {
    case forward, reverse, pingPong, pingPongReverse

    var title: String {
        switch self {
        case .forward: "Forward"
        case .reverse: "Reverse"
        case .pingPong: "Ping-pong"
        case .pingPongReverse: "Ping-pong reverse"
        }
    }
}

/// Geometry shared with the frame strip so tag bars line up with the frames below them.
enum Timeline {
    static let cellWidth: CGFloat = 32
    static let gap: CGFloat = 0
    static let barHeight: CGFloat = 14
    /// Vertical space between stacked tag rows.
    static let rowGap: CGFloat = 2
    /// Height of the frame number row.
    static let numberRow: CGFloat = 20
}

extension SpriteModel {
    var tagCount: Int { Int(pixmith_tag_count(doc)) }

    func tag(_ i: Int) -> TagInfo? {
        var t = PixmithTag()
        guard pixmith_tag_get(doc, UInt32(i), &t), let p = pixmith_tag_name(doc, UInt32(i)) else { return nil }
        defer { pixmith_string_free(p) }
        return TagInfo(name: String(cString: p), from: Int(t.from), to: Int(t.to), direction: Int(t.direction),
                       repeatCount: Int(t.repeat), color: t.color)
    }

    private func raw(_ t: TagInfo) -> PixmithTag {
        PixmithTag(from: UInt32(t.from), to: UInt32(t.to), direction: UInt8(clamping: t.direction),
                repeat: UInt16(clamping: t.repeatCount), color: t.color & 0xFFFFFF)
    }

    func addTag(_ t: TagInfo) {
        if t.name.withCString({ pixmith_tag_add(doc, $0, raw(t)) }) >= 0 { labelLastStep("Add Tag"); changed() }
    }

    func updateTag(_ i: Int, _ t: TagInfo) {
        if t.name.withCString({ pixmith_tag_update(doc, UInt32(i), $0, raw(t)) }) { labelLastStep("Edit Tag"); changed() }
    }

    func removeTag(_ i: Int) {
        if pixmith_tag_remove(doc, UInt32(i)) { labelLastStep("Delete Tag"); changed() }
    }

    /// Default values for a new tag over the current frame.
    func newTagDraft() -> TagInfo {
        let colors: [UInt32] = [0x3B82F6, 0xEF4444, 0x22C55E, 0xEAB308, 0xA855F7, 0xF97316]
        return TagInfo(name: "Tag \(tagCount + 1)", from: frame, to: frame, direction: 0, repeatCount: 0,
                       color: colors[tagCount % colors.count])
    }
}

/// Greedy row assignment for the tag bars: a tag goes into the first row whose last tag ends before it starts.
struct TagLayout {
    var items: [(index: Int, tag: TagInfo, row: Int)] = []
    var rowCount = 0

    init(model: SpriteModel) {
        let tags = (0..<model.tagCount).compactMap { i in model.tag(i).map { (i, $0) } }
        var rowEnds: [Int] = []
        for (i, t) in tags.sorted(by: { $0.1.from < $1.1.from }) {
            if let r = rowEnds.firstIndex(where: { $0 < t.from }) {
                rowEnds[r] = t.to
                items.append((i, t, r))
            } else {
                rowEnds.append(t.to)
                items.append((i, t, rowEnds.count - 1))
            }
        }
        rowCount = rowEnds.count
    }

    /// Height of the tag area above the frame numbers; always at least one row so the header does not jump.
    var height: CGFloat { CGFloat(max(rowCount, 1)) * (Timeline.barHeight + Timeline.rowGap) + 4 }
}

/// Colored bars above the frame numbers, one per tag; overlapping tags stack into rows.
struct TagsBar: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        let layout = TagLayout(model: model)
        let pitch = Timeline.cellWidth + Timeline.gap
        ZStack(alignment: .topLeading) {
            Color.clear
            ForEach(layout.items, id: \.index) { item in
                let t = item.tag
                TagBar(tag: t, selected: model.frame >= t.from && model.frame <= t.to)
                    .frame(width: CGFloat(t.to - t.from + 1) * pitch - 2, height: Timeline.barHeight)
                    .offset(x: CGFloat(t.from) * pitch + 1, y: 4 + CGFloat(item.row) * (Timeline.barHeight + Timeline.rowGap))
                    .onTapGesture(count: 2) { model.toolSheet = .tag(item.index) }
                    .onTapGesture { model.stop(); model.frame = t.from }
                    .contextMenu {
                        Button("Edit Tag…") { model.toolSheet = .tag(item.index) }
                        Button("Delete Tag") { model.removeTag(item.index) }
                    }
                    .help("\(t.name) (frames \(t.from + 1)–\(t.to + 1)). Double-click to edit.")
            }
        }
        .frame(width: CGFloat(model.frameCount) * pitch, height: layout.height, alignment: .topLeading)
    }
}

private struct TagBar: View {
    let tag: TagInfo
    let selected: Bool
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let base = Color(cgColor: (tag.color << 8 | 0xFF).cgColor)
        ZStack(alignment: .leading) {
            RoundedRectangle(cornerRadius: DS.Radius.chip).fill(base.opacity(selected ? 0.42 : 0.28))
            Text(tag.name)
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(scheme == .dark ? base.mix(with: .white, by: 0.6) : base.mix(with: .black, by: 0.45))
                .lineLimit(1)
                .padding(.horizontal, 6)
        }
        .contentShape(Rectangle())
    }
}

extension Color {
    /// Blends toward `other` by `amount` (0...1) in sRGB.
    func mix(with other: Color, by amount: Double) -> Color {
        let a = NSColor(self).usingColorSpace(.sRGB) ?? .gray, b = NSColor(other).usingColorSpace(.sRGB) ?? .white
        let t = CGFloat(amount)
        return Color(.sRGB, red: Double(a.redComponent + (b.redComponent - a.redComponent) * t),
                     green: Double(a.greenComponent + (b.greenComponent - a.greenComponent) * t),
                     blue: Double(a.blueComponent + (b.blueComponent - a.blueComponent) * t))
    }
}

/// Creates a tag (index nil) or edits an existing one.
struct TagSheet: View {
    @ObservedObject var model: SpriteModel
    let index: Int?
    @Environment(\.dismiss) private var dismiss
    @State private var tag = TagInfo(name: "", from: 0, to: 0, direction: 0, repeatCount: 0, color: 0x3B82F6)

    var body: some View {
        let last = model.frameCount
        DSDialog(title: index == nil ? "New Tag" : "Edit Tag", primary: index == nil ? "Create" : "Save",
                 destructive: index.map { i in ("Delete", { model.removeTag(i); dismiss() }) }, width: 360,
                 onPrimary: {
                     tag.to = max(tag.from, tag.to)
                     if let index { model.updateTag(index, tag) } else { model.addTag(tag) }
                     dismiss()
                 }, onCancel: { dismiss() }) {
            DSFormRow(label: "Name") { TextField("Name", text: $tag.name).dsTextField() }
            DSFormRow(label: "Color") {
                ColorPicker("Color", selection: Binding(
                    get: { Color(cgColor: (tag.color << 8 | 0xFF).cgColor) },
                    set: { tag.color = UInt32($0.cgColor ?? .black) >> 8 }))
                    .labelsHidden()
                Spacer(minLength: 0)
            }
            DSFormRow(label: "Frames") {
                DSNumberField("From", value: Binding(get: { tag.from + 1 }, set: { tag.from = $0 - 1 }),
                              range: 1...max(1, last), width: 96)
                DSNumberField("To", value: Binding(get: { tag.to + 1 }, set: { tag.to = $0 - 1 }),
                              range: 1...max(1, last), width: 96)
            }
            DSFormRow(label: "Direction") {
                DSPopup(selection: $tag.direction, options: TagDirection.allCases.map { ($0.rawValue, $0.title) }, fill: true)
            }
            DSFormRow(label: "Repeat") {
                DSNumberField("×", value: $tag.repeatCount, range: 0...255, width: 96, help: "0 repeats forever")
                Text(tag.repeatCount == 0 ? "infinite" : "times").font(DS.Font.caption).foregroundStyle(DSColor.textTertiary)
            }
        }
        .onAppear { tag = index.flatMap { model.tag($0) } ?? model.newTagDraft() }
    }
}
