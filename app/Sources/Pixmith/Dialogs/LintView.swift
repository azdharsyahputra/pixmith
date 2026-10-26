import PixmithCore
import SwiftUI

/// FX > Lint Sprite…: a read-only report of what is wrong with the sprite (paper halo, holes, detached specks,
/// edge pixels without an outline, stray and semi-transparent pixels, colors outside the palette), per cel.
/// The report comes from the core (`pixmith_sprite_lint`) as text; see `pixmith_filters.h` for its format.

struct LintCategory: Identifiable, Equatable {
    var id: String { name }
    let name: String
    let count: Int
    var items: [(x: Int, y: Int, size: Int)] = []

    static func == (a: LintCategory, b: LintCategory) -> Bool {
        a.name == b.name && a.count == b.count && a.items.count == b.items.count
    }

    var title: String {
        switch name {
        case "light_edge": "Light pixels on the outline ring"
        case "holes": "Enclosed holes"
        case "specks": "Detached specks"
        case "no_outline": "Edge pixels with no outline"
        case "strays": "Stray pixels"
        case "semi_transparent": "Semi-transparent pixels"
        case "off_palette": "Colors not in the palette"
        default: name
        }
    }

    var help: String {
        switch name {
        case "light_edge": "Light, grayish pixels in the two outermost rings (paper halo)."
        case "holes": "Transparent areas closed off by the sprite. Listed: first pixel and size."
        case "specks": "Small pieces apart from the body. Listed: first pixel and size."
        case "no_outline": "Outer pixels that are not outline color and have no outline pixel next to them."
        case "strays": "Single pixels that differ from all of their neighbors."
        case "semi_transparent": "Pixels that are neither fully opaque nor fully transparent."
        case "off_palette": "Pixels whose color is missing from the palette."
        default: ""
        }
    }
}

struct LintCel: Identifiable {
    var id: String { "\(layer).\(frame)" }
    let layer: Int
    let frame: Int
    var categories: [LintCategory] = []
    var total: Int { categories.reduce(0) { $0 + $1.count } }
}

/// Parses the core's report: `cel <layer> <frame>`, then per category `<name> <count>` and `x y size` lines.
func parseLintReport(_ text: String) -> [LintCel] {
    var cels: [LintCel] = []
    for line in text.split(separator: "\n") {
        let w = line.split(separator: " ")
        if w.count == 3, w[0] == "cel", let l = Int(w[1]), let f = Int(w[2]) {
            cels.append(LintCel(layer: l, frame: f))
        } else if w.count == 2, !cels.isEmpty, let n = Int(w[1]) {
            cels[cels.count - 1].categories.append(LintCategory(name: String(w[0]), count: n))
        } else if w.count == 3, !cels.isEmpty, let x = Int(w[0]), let y = Int(w[1]), let s = Int(w[2]),
                  !cels[cels.count - 1].categories.isEmpty {
            let c = cels.count - 1
            cels[c].categories[cels[c].categories.count - 1].items.append((x, y, s))
        }
    }
    return cels
}

struct LintSheet: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var state = FilterState.shared
    @State private var target = FilterTarget.cel
    @State private var speck = 4
    @State private var checkPalette = false
    @State private var report: [LintCel] = []

    private var total: Int { report.reduce(0) { $0 + $1.total } }

    var body: some View {
        DSDialog(title: "Lint Sprite", subtitle: "Reports problems without changing anything.", primary: "Close",
                 showsCancel: false, width: 420, onPrimary: { state.active = nil }, onCancel: { state.active = nil }) {
            DSFormRow(label: "Check") {
                DSPopup(selection: $target, options: FilterTarget.allCases.map { ($0, $0.title) }, fill: true)
            }
            ValueSlider(label: "Speck size (px)", value: $speck, range: 2...32)
            Toggle("Report colors that are not in the palette", isOn: $checkPalette).toggleStyle(.checkbox)
            DSDivider()
            Text(total == 0 ? "No problems found." : "\(total) findings.").foregroundStyle(DSColor.textSecondary)
            ScrollView {
                VStack(alignment: .leading, spacing: DS.Space.sm) {
                    ForEach(report.filter { $0.total > 0 }) { cel in
                        HStack {
                            Text("Layer \(cel.layer + 1), frame \(cel.frame + 1)").fontWeight(.semibold)
                            Button("Go to cel") {
                                model.layer = cel.layer
                                model.frame = cel.frame
                            }
                            .buttonStyle(.link)
                        }
                        ForEach(cel.categories.filter { $0.count > 0 }) { cat in
                            DisclosureGroup("\(cat.title) (\(cat.count))") {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(cat.help).foregroundStyle(DSColor.textSecondary)
                                    ForEach(Array(cat.items.enumerated()), id: \.offset) { _, it in
                                        Text(it.size > 1 ? "x \(it.x), y \(it.y) (\(it.size) px)" : "x \(it.x), y \(it.y)")
                                            .font(.system(.body, design: .monospaced))
                                    }
                                    if cat.count > cat.items.count {
                                        Text("… and \(cat.count - cat.items.count) more").foregroundStyle(DSColor.textSecondary)
                                    }
                                }
                            }
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: 320)
        }
        .onAppear {
            model.stop()
            refresh()
        }
        .onChange(of: target) { _ in refresh() }
        .onChange(of: speck) { _ in refresh() }
        .onChange(of: checkPalette) { _ in refresh() }
    }

    private func refresh() {
        let params: [Int32] = [Int32(speck), 64, checkPalette ? 1 : 0]
        let text = params.withUnsafeBufferPointer { p -> String in
            guard let c = pixmith_sprite_lint(model.doc, target.rawValue, UInt32(model.layer), UInt32(model.frame),
                                              p.baseAddress, p.count) else { return "" }
            defer { pixmith_string_free(c) }
            return String(cString: c)
        }
        report = parseLintReport(text)
    }
}
