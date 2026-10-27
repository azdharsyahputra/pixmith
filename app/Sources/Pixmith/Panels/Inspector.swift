import SwiftUI

/// The right-hand inspector: Color, Palette, the selected Layer's properties, the Cel, Tileset and Slices when they are
/// relevant, and the Sprite info. Layers themselves live in the timeline.
struct SidePanel: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        ScrollView {
            VStack(spacing: 0) {
                DockedPreviewSection(model: model, state: PreviewController.shared.state)
                ColorSection(model: model)
                PaletteEditor(model: model)
                LayerSection(model: model)
                CelSection(model: model)
                TilemapPanel(model: model)
                SlicesSection(model: model)
                SpriteSection(model: model)
            }
        }
        .frame(width: DS.Metric.inspectorWidth)
        .background(DSColor.panel)
        .overlay(alignment: .leading) { Rectangle().fill(DSColor.separatorSoft).frame(width: 1) }
        .tilemapSheets(model: model)
    }
}

// MARK: Layer

/// Name, visibility, blend mode and opacity of the selected layer.
struct LayerSection: View {
    @ObservedObject var model: SpriteModel
    @State private var name = ""
    @FocusState private var nameFocused: Bool

    private var layer: Int { model.layer }

    private func commitName() {
        let n = name.trimmingCharacters(in: .whitespaces)
        if !n.isEmpty, n != model.layerName(layer) { model.rename(layer, to: n) } else { name = model.layerName(layer) }
    }

    private var blend: Binding<Int> {
        Binding(get: { model.blendMode(layer) }, set: {
            model.setLayerProps(layer, name: model.layerName(layer), opacity: model.rawOpacity(layer), blend: $0,
                                visible: model.isVisible(layer))
        })
    }

    private var opacity: Binding<Double> {
        Binding(get: { model.layerOpacity / 255 * 100 }, set: { model.setOpacityDuringDrag($0 / 100 * 255) })
    }

    var body: some View {
        let visible = model.isVisible(layer)
        InspectorSection("Layer", id: "layer") {
            IconButton(visible ? "eye" : "eye.slash", help: visible ? "Hide layer" : "Show layer", compact: true) {
                model.setVisible(layer, !visible)
            }
            IconButton("slider.horizontal.3", help: "Layer properties", compact: true) { model.toolSheet = .layer(layer) }
        } content: {
            VStack(spacing: DS.Space.sm - 2) {
                TextField("Layer name", text: $name)
                    .dsTextField()
                    .focused($nameFocused)
                    .onSubmit(commitName)
                    .onChange(of: nameFocused) { if !$0 { commitName() } }
                HStack(spacing: DS.Space.sm - 2) {
                    DSPopup(selection: blend, options: BlendMode.allCases.map { ($0.rawValue, $0.title) }, help: "Blend mode",
                            fill: true)
                    DSNumberField("Opacity", value: opacity, range: 0...100, unit: "%", width: 118,
                                  help: "Layer opacity. Drag the label to scrub.", onEditing: { model.opacityDragChanged($0) })
                }
            }
        }
        .onAppear { name = model.layerName(layer) }
        .onChange(of: model.layer) { _ in name = model.layerName(model.layer) }
        .onChange(of: model.revision) { _ in if !nameFocused { name = model.layerName(model.layer) } }
    }
}

// MARK: Cel

/// Opacity and link state of the selected cel.
struct CelSection: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        let opacity = model.celOpacity
        InspectorSection("Cel", id: "cel", subtitle: "\(model.layerName(model.layer)) · frame \(model.frame + 1)") {
            HStack(spacing: DS.Space.sm - 2) {
                DSNumberField("Opacity", value: Binding(
                    get: { Double(opacity ?? 255) / 255 * 100 },
                    set: { model.setCelOpacity(Int(($0 / 100 * 255).rounded())) }),
                    range: 0...100, unit: "%", width: 128, help: "Opacity of this cel. Drag the label to scrub.",
                    deferScrub: true)
                    .disabled(opacity == nil)
                Button { model.toolSheet = .cel } label: {
                    HStack(spacing: 6) {
                        Image(systemName: model.isCelLinked ? "link" : "link.badge.plus").font(.system(size: 12))
                        Text(model.isCelLinked ? "Linked" : "Link…").font(DS.Font.body)
                    }
                    .foregroundStyle(model.isCelLinked ? DSColor.accent : DSColor.textSecondary)
                    .padding(.horizontal, DS.Space.sm)
                    .frame(maxWidth: .infinity, minHeight: DS.Metric.control, maxHeight: DS.Metric.control)
                    .background(RoundedRectangle(cornerRadius: DS.Radius.field).fill(DSColor.field))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Link cels across frames, or unlink this one")
            }
        }
    }
}

// MARK: Slices

/// The sprite's slices; shown only when there are some or the slice tool is active.
struct SlicesSection: View {
    @ObservedObject var model: SpriteModel

    var body: some View {
        if model.sliceCount > 0 || model.tool == .slice {
            InspectorSection("Slices", id: "slices", subtitle: "\(model.sliceCount)") {
                IconButton("slider.horizontal.3", help: "Slice properties", compact: true) { model.showSliceSheet() }
                    .disabled(!model.hasSelectedSlice)
                IconButton("trash", help: "Delete slice", compact: true, destructive: true) { model.deleteSelectedSlice() }
                    .disabled(!model.hasSelectedSlice)
            } content: {
                if model.sliceCount == 0 {
                    Text("Drag on the canvas with the slice tool to create one.")
                        .font(DS.Font.caption).foregroundStyle(DSColor.textTertiary)
                } else {
                    VStack(spacing: 2) {
                        ForEach(0..<model.sliceCount, id: \.self) { i in sliceRow(i) }
                    }
                    .padding(.horizontal, -DS.Space.sm)
                }
            }
        }
    }

    private func sliceRow(_ i: Int) -> some View {
        let selected = model.selectedSliceIndex == i
        let info = model.sliceInfo(i)
        return HStack(spacing: DS.Space.sm) {
            Image(systemName: "crop").font(.system(size: 11)).foregroundStyle(DSColor.textTertiary)
            Text(model.sliceName(i)).font(DS.Font.body).lineLimit(1)
                .foregroundStyle(selected ? DSColor.text : DSColor.textSecondary)
            Spacer(minLength: 0)
            if let info {
                Text("\(info.w)×\(info.h)").font(DS.Font.number).foregroundStyle(DSColor.textTertiary)
            }
        }
        .padding(.horizontal, DS.Space.sm)
        .frame(height: DS.Metric.control + 2)
        .background(RoundedRectangle(cornerRadius: DS.Radius.control).fill(selected ? DSColor.selected : .clear))
        .contentShape(Rectangle())
        .onTapGesture(count: 2) { model.selectSlice(i); model.showSliceSheet(i) }
        .onTapGesture { model.selectSlice(i) }
        .contextMenu {
            Button("Properties…") { model.selectSlice(i); model.showSliceSheet(i) }
            Button("Delete") { model.selectSlice(i); model.deleteSelectedSlice() }
        }
    }
}

// MARK: Sprite

/// Read-only facts about the sprite, plus the color mode and the grid toggle.
struct SpriteSection: View {
    @ObservedObject var model: SpriteModel

    private var totalMs: Int { (0..<model.frameCount).reduce(0) { $0 + model.duration($1) } }

    var body: some View {
        InspectorSection("Sprite", id: "sprite") {
            Grid(alignment: .leading, horizontalSpacing: DS.Space.lg, verticalSpacing: DS.Space.sm) {
                row("Size") { value("\(model.width) × \(model.height) px") }
                row("Color mode") {
                    DSPopup(selection: Binding(get: { model.colorMode }, set: { model.convertColorMode(to: $0) }),
                            options: ColorMode.allCases.map { ($0, $0.title) }, help: "Convert the sprite's color mode")
                }
                row("Frames") {
                    value("\(model.frameCount) · \(totalMs) ms" + (model.tagCount > 0 ? " · \(model.tagCount) tag\(model.tagCount == 1 ? "" : "s")" : ""))
                }
            }
        }
    }

    private func value(_ s: String) -> some View {
        Text(s).font(DS.Font.body.monospacedDigit()).foregroundStyle(DSColor.text)
    }

    private func row<C: View>(_ label: String, @ViewBuilder _ content: () -> C) -> some View {
        GridRow {
            Text(label).font(DS.Font.caption).foregroundStyle(DSColor.textTertiary)
                .gridColumnAlignment(.leading)
            content().frame(maxWidth: .infinity, minHeight: DS.Metric.control, alignment: .leading)
        }
    }
}
