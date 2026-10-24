import PixmithCore
import SwiftUI

/// Modal editors opened from the layers panel and the timeline.
enum ToolSheet: Identifiable {
    case layer(Int)
    case tag(Int?)
    case cel
    case sprite

    var id: String {
        switch self {
        case .layer(let i): "layer\(i)"
        case .tag(let i): "tag\(i.map(String.init) ?? "new")"
        case .cel: "cel"
        case .sprite: "sprite"
        }
    }
}

enum BlendMode: Int, CaseIterable {
    case normal, multiply, screen, overlay, darken, lighten, colorDodge, colorBurn, hardLight, softLight
    case difference, exclusion, hue, saturation, color, luminosity, addition, subtract, divide

    var title: String {
        switch self {
        case .colorDodge: "Color Dodge"
        case .colorBurn: "Color Burn"
        case .hardLight: "Hard Light"
        case .softLight: "Soft Light"
        default: "\(self)".capitalized
        }
    }
}

extension SpriteModel {
    // MARK: Layers

    func blendMode(_ i: Int) -> Int { Int(pixmith_layer_blend_mode(doc, UInt32(i))) }
    func canMergeDown(_ i: Int) -> Bool { pixmith_layer_can_merge_down(doc, UInt32(i)) }

    /// Selects the drawable layer at or above `i` after a structural change.
    private func finish(_ result: Int32, _ name: String) {
        guard result >= 0 else { return }
        labelLastStep(name)
        layer = (Int(result)..<layerCount).first { !isGroup($0) } ?? layer
        clampSelection()
        changed()
    }

    func moveLayer(_ i: Int, up: Bool) { finish(pixmith_layer_move(doc, UInt32(i), up), "Move Layer") }
    func duplicateLayer(_ i: Int) { finish(pixmith_layer_duplicate(doc, UInt32(i)), "Duplicate Layer") }
    func mergeLayerDown(_ i: Int) { finish(pixmith_layer_merge_down(doc, UInt32(i)), "Merge Layer Down") }
    func wrapInGroup(_ i: Int) { finish(pixmith_layer_group(doc, UInt32(i)), "Group Layer") }
    func moveOutOfGroup(_ i: Int) { finish(pixmith_layer_out_of_group(doc, UInt32(i)), "Move Out of Group") }
    func moveLayer(_ i: Int, into group: Int) { finish(pixmith_layer_into_group(doc, UInt32(i), UInt32(group)), "Move Into Group") }

    func addGroup() {
        if pixmith_layer_add_group(doc, UInt32(layer)) >= 0 { labelLastStep("New Group"); changed() }
    }

    /// Groups a layer can be moved into: any group outside its own subtree.
    func groupTargets(for i: Int) -> [Int] {
        var end = i + 1
        while end < layerCount, level(end) > level(i) { end += 1 }
        return (0..<layerCount).filter { isGroup($0) && !(i..<end).contains($0) }
    }

    func setLayerProps(_ i: Int, name: String, opacity: Int, blend: Int, visible: Bool) {
        let ok = name.withCString {
            pixmith_layer_set_props(doc, UInt32(i), $0, UInt8(clamping: opacity), UInt16(clamping: blend), visible)
        }
        if ok { labelLastStep("Layer Properties"); changed() }
    }

    func rawOpacity(_ i: Int) -> Int { Int(pixmith_layer_opacity(doc, UInt32(i))) }

    // MARK: Frames and cels

    func moveFrame(by delta: Int) {
        stop()
        let to = frame + delta
        guard to >= 0, to < frameCount, pixmith_frame_move(doc, UInt32(frame), UInt32(to)) else { return }
        labelLastStep("Move Frame")
        frame = to
        changed()
    }

    /// Opacity of the current layer's cel at `frame`, or nil if it is empty.
    var celOpacity: Int? {
        let v = pixmith_cel_opacity(doc, UInt32(layer), UInt32(frame))
        return v < 0 ? nil : Int(v)
    }

    var isCelLinked: Bool { pixmith_cel_is_linked(doc, UInt32(layer), UInt32(frame)) }

    func setCelOpacity(_ v: Int) {
        if pixmith_cel_set_opacity(doc, UInt32(layer), UInt32(frame), UInt8(clamping: v)) { labelLastStep("Cel Opacity"); changed() }
    }

    func linkCels(from a: Int, to b: Int) {
        if pixmith_cel_link(doc, UInt32(layer), UInt32(a), UInt32(b)) { labelLastStep("Link Cels"); changed() }
    }

    func unlinkCel() {
        if pixmith_cel_unlink(doc, UInt32(layer), UInt32(frame)) { labelLastStep("Unlink Cel"); changed() }
    }

    // MARK: Playback

    /// Moves to the next frame, looping inside the current tag and honoring its direction and repeat count.
    func advance() {
        frame = nextPlayFrame(after: frame, cursor: &playCursor)
    }

    func nextPlayFrame(after f: Int, cursor: inout PlaybackCursor) -> Int {
        Int(pixmith_play_next(doc, UInt32(f), &cursor.raw, loopTags))
    }

    /// Frame range of the tag containing the current frame, if tags are being looped.
    var activeTagRange: ClosedRange<Int>? {
        guard loopTags else { return nil }
        for i in 0..<tagCount {
            if let t = tag(i), t.from <= t.to, (t.from...t.to).contains(frame) { return t.from...t.to }
        }
        return nil
    }

    // MARK: Layer opacity

    /// The side panel's opacity slider: one undo step per drag.
    func setOpacityDuringDrag(_ v: Double) {
        let drag = OpacityDrag.shared
        if !drag.active || !drag.checkpointed {
            pixmith_begin_stroke(doc)
            labelLastStep("Layer Opacity")
            drag.checkpointed = drag.active
        }
        layerOpacity = v
    }

    func opacityDragChanged(_ active: Bool) {
        OpacityDrag.shared.active = active
        OpacityDrag.shared.checkpointed = false
    }
}

struct PlaybackCursor {
    var raw = PixmithPlayState(reversed: false, passes: 0, tag: -1)
}

final class OpacityDrag {
    static let shared = OpacityDrag()
    var active = false
    var checkpointed = false
}

// MARK: Views

/// Context menu entries for one row of the layers list.
struct LayerMenuItems: View {
    @ObservedObject var model: SpriteModel
    let index: Int

    var body: some View {
        Divider()
        Button("Properties…") { model.toolSheet = .layer(index) }
        Button("Move Up") { model.moveLayer(index, up: true) }
        Button("Move Down") { model.moveLayer(index, up: false) }
        Button("Duplicate") { model.duplicateLayer(index) }
        Button("Merge Down") { model.mergeLayerDown(index) }.disabled(!model.canMergeDown(index))
        Divider()
        Button("New Group") { model.layer = index; model.addGroup() }
        Button("Group Layer") { model.wrapInGroup(index) }
        Menu("Move Into Group") {
            ForEach(model.groupTargets(for: index), id: \.self) { g in
                Button(model.layerName(g)) { model.moveLayer(index, into: g) }
            }
        }
        .disabled(model.groupTargets(for: index).isEmpty)
        Button("Move Out of Group") { model.moveOutOfGroup(index) }.disabled(model.level(index) == 0)
    }
}

struct LayerPropertiesSheet: View {
    @ObservedObject var model: SpriteModel
    let layer: Int
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var opacity = 255.0
    @State private var blend = 0
    @State private var visible = true
    @State private var reference = false
    @State private var locked = false

    var body: some View {
        DSDialog(title: model.isGroup(layer) ? "Group Properties" : "Layer Properties", primary: "OK", width: 340, onPrimary: {
            model.inTransaction {
                model.setLayerProps(layer, name: name, opacity: Int(opacity.rounded()), blend: blend, visible: visible)
                if !model.isGroup(layer) {
                    if reference != model.isReference(layer) { model.setReference(layer, reference) }
                    if reference, locked != model.isLocked(layer) { model.setLocked(layer, locked) }
                }
            }
            dismiss()
        }, onCancel: { dismiss() }) {
            DSFormRow(label: "Name") { TextField("Name", text: $name).dsTextField() }
            DSFormRow(label: "Visible") {
                Toggle("Visible", isOn: $visible).labelsHidden().toggleStyle(.switch).controlSize(.small)
            }
            DSFormRow(label: "Opacity") {
                DSSlider("", value: $opacity, range: 0...255, format: { "\(Int(($0 / 255 * 100).rounded()))%" })
            }
            DSFormRow(label: "Blend mode") {
                DSPopup(selection: $blend, options: BlendMode.allCases.map { ($0.rawValue, $0.title) }, fill: true)
            }
            .disabled(model.isGroup(layer))
            if !model.isGroup(layer) {
                DSFormRow(label: "Reference") {
                    Toggle("Reference layer", isOn: $reference).labelsHidden().toggleStyle(.switch).controlSize(.small)
                }
                if reference {
                    DSFormRow(label: "Locked") {
                        Toggle("Locked", isOn: $locked).labelsHidden().toggleStyle(.switch).controlSize(.small)
                    }
                    DSNote("A reference layer is shown in the editor but never exported. While it is locked, drawing on it is refused.")
                }
            }
        }
        .onAppear {
            name = model.layerName(layer)
            opacity = Double(model.rawOpacity(layer))
            blend = model.blendMode(layer)
            visible = model.isVisible(layer)
            reference = model.isReference(layer)
            locked = model.isLocked(layer)
        }
        .onChange(of: reference) { on in if on, !model.isReference(layer) { locked = true } }
    }
}

/// Opacity of the current cel and linking of cels across frames.
struct CelSheet: View {
    @ObservedObject var model: SpriteModel
    @Environment(\.dismiss) private var dismiss
    @State private var opacity = 255.0
    @State private var zIndex = 0
    @State private var from = 1
    @State private var to = 2

    var body: some View {
        let hasCel = model.celOpacity != nil
        DSDialog(title: "Cel Properties", subtitle: "\(model.layerName(model.layer)) · frame \(model.frame + 1)",
                 primary: "Done", showsCancel: false, width: 380, onPrimary: { dismiss() }, onCancel: { dismiss() }) {
            HStack(spacing: DS.Space.sm) {
                DSSlider("Opacity", value: $opacity, range: 0...255, format: { "\(Int(($0 / 255 * 100).rounded()))%" })
                Button("Apply") { model.setCelOpacity(Int(opacity.rounded())) }.buttonStyle(DSSecondaryButtonStyle())
            }
            .disabled(!hasCel)
            HStack(spacing: DS.Space.sm) {
                DSNumberField("Z-index", value: $zIndex, range: -32768...32767, unit: "", width: 140,
                              help: "Draws this cel as if its layer were that many places higher (or lower)")
                Button("Apply") { model.setCelZIndex(zIndex) }.buttonStyle(DSSecondaryButtonStyle())
                Spacer(minLength: 0)
            }
            .disabled(!hasCel)
            DSNote("Cels draw in order of layer position plus z-index; equal positions keep the lower layer first.")
            DSDivider()
            DSNote("Linked cels share their image, opacity and z-index: editing one edits all.")
            HStack(spacing: DS.Space.sm) {
                DSNumberField("From", value: $from, range: 1...max(1, model.frameCount), unit: "", width: 110)
                DSNumberField("To", value: $to, range: 1...max(1, model.frameCount), unit: "", width: 110)
                Spacer(minLength: 0)
            }
            HStack(spacing: DS.Space.sm) {
                Button("Link Cels") { model.linkCels(from: from - 1, to: to - 1); dismiss() }
                    .buttonStyle(DSSecondaryButtonStyle())
                    .disabled(from >= to)
                    .help("Make the cels in this frame range copies of the cel in the first frame")
                Button("Unlink This Cel") { model.unlinkCel(); dismiss() }
                    .buttonStyle(DSSecondaryButtonStyle())
                    .disabled(!model.isCelLinked)
            }
        }
        .onAppear {
            opacity = Double(model.celOpacity ?? 255)
            zIndex = model.celZIndex ?? 0
            from = model.frame + 1
            to = min(model.frame + 2, model.frameCount)
        }
    }
}

struct ToolSheetView: View {
    @ObservedObject var model: SpriteModel
    let sheet: ToolSheet

    var body: some View {
        switch sheet {
        case .layer(let i): LayerPropertiesSheet(model: model, layer: i)
        case .tag(let i): TagSheet(model: model, index: i)
        case .cel: CelSheet(model: model)
        case .sprite: SpritePropertiesSheet(model: model)
        }
    }
}

struct LayerCommands: Commands {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var sheet = SheetState.shared

    var body: some Commands {
        CommandMenu("Layer") {
            Group {
            Button("Layer Properties…") { model.toolSheet = .layer(model.layer) }
            Divider()
            Button("Move Layer Up") { model.moveLayer(model.layer, up: true) }
            Button("Move Layer Down") { model.moveLayer(model.layer, up: false) }
            Button("Duplicate Layer") { model.duplicateLayer(model.layer) }
            Button("Merge Layer Down") { model.mergeLayerDown(model.layer) }
            Button("New Group") { model.addGroup() }
            ReferenceLayerMenuItems(model: model)
            Divider()
            Button("Move Frame Left") { model.moveFrame(by: -1) }
            Button("Move Frame Right") { model.moveFrame(by: 1) }
            Button("Cel Properties & Links…") { model.toolSheet = .cel }
            Button("New Tag…") { model.toolSheet = .tag(nil) }
            }
            .disabled(sheet.isOpen)
        }
    }
}
