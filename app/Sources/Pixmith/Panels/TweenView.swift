import AppKit
import PixmithCore
import SwiftUI

// Tweening panel (View > Tween), dope sheet, Bezier curve editor and the canvas overlay / handles.
// All logic lives in core/src/tween.rs; this file only drives it.

/// Properties and eases mirror the codes in pixmith_tween.h.
enum TweenProp: UInt32, CaseIterable, Identifiable {
    case posX, posY, rotation, scaleX, scaleY, opacity
    var id: UInt32 { rawValue }
    var title: String {
        switch self {
        case .posX: "Position X"
        case .posY: "Position Y"
        case .rotation: "Rotation (deg)"
        case .scaleX: "Scale X"
        case .scaleY: "Scale Y"
        case .opacity: "Opacity (0-1)"
        }
    }
    var short: String {
        switch self {
        case .posX: "X"
        case .posY: "Y"
        case .rotation: "Rot"
        case .scaleX: "SX"
        case .scaleY: "SY"
        case .opacity: "Op"
        }
    }
}

enum TweenEase: UInt32, CaseIterable, Identifiable {
    case linear, hold, inQuad, outQuad, inOutQuad, inOutCubic, outBack, outBounce, bezier
    var id: UInt32 { rawValue }
    var title: String {
        switch self {
        case .linear: "Linear"
        case .hold: "Hold"
        case .inQuad: "In Quad"
        case .outQuad: "Out Quad"
        case .inOutQuad: "In-Out Quad"
        case .inOutCubic: "In-Out Cubic"
        case .outBack: "Out Back"
        case .outBounce: "Out Bounce"
        case .bezier: "Bezier"
        }
    }
}

struct TweenKeyInfo: Hashable {
    var frame: Int
    var value: Float
    var ease: TweenEase
    var bezier: [Float]
}

/// UI state shared by the panel and the canvas.
final class TweenUI: ObservableObject {
    static let shared = TweenUI()
    @Published var panelOpen = false
    @Published var editOnCanvas = false
    @Published var autoKey = true
    @Published var selProp: TweenProp = .posX
    @Published var selFrame: Int?
}

extension SpriteModel {
    func tweenKeys(_ prop: TweenProp) -> [TweenKeyInfo] {
        let n = pixmith_tween_key_count(doc, UInt32(layer), prop.rawValue)
        var out: [TweenKeyInfo] = []
        var buf = [Float](repeating: 0, count: 7)
        for i in 0..<n where pixmith_tween_key_at(doc, UInt32(layer), prop.rawValue, i, &buf) {
            out.append(TweenKeyInfo(frame: Int(buf[0]), value: buf[1], ease: TweenEase(rawValue: UInt32(buf[2])) ?? .linear,
                                    bezier: Array(buf[3..<7])))
        }
        return out
    }

    func tweenValue(_ prop: TweenProp, at frame: Int) -> Float {
        pixmith_tween_value_at(doc, UInt32(layer), prop.rawValue, UInt32(frame))
    }

    func tweenSetKey(_ prop: TweenProp, frame f: Int, value: Float, ease: TweenEase, bezier: [Float]) {
        if pixmith_tween_set_key(doc, UInt32(layer), prop.rawValue, UInt32(f), value, ease.rawValue,
                              bezier[0], bezier[1], bezier[2], bezier[3]) {
            labelLastStep("Set Tween Key")
            changed()
        }
    }

    func tweenSetEase(_ prop: TweenProp, frame f: Int, ease: TweenEase, bezier: [Float]) {
        if pixmith_tween_set_ease(doc, UInt32(layer), prop.rawValue, UInt32(f), ease.rawValue,
                               bezier[0], bezier[1], bezier[2], bezier[3]) {
            labelLastStep("Change Tween Curve")
            changed()
        }
    }

    func tweenRemoveKey(_ prop: TweenProp, frame f: Int) {
        if pixmith_tween_remove_key(doc, UInt32(layer), prop.rawValue, UInt32(f)) {
            labelLastStep("Remove Tween Key")
            changed()
        }
    }

    func tweenMoveKey(_ prop: TweenProp, from: Int, to: Int) {
        if pixmith_tween_move_key(doc, UInt32(layer), prop.rawValue, UInt32(from), UInt32(to)) {
            labelLastStep("Move Tween Key")
            changed()
        }
    }

    func tweenCopyKeys(from: Int, to: Int) -> Int {
        Int(pixmith_tween_copy_keys(doc, UInt32(layer), UInt32(max(0, from)), UInt32(max(0, to))))
    }

    func tweenPasteKeys(at f: Int) {
        if pixmith_tween_paste_keys(doc, UInt32(layer), UInt32(f)) > 0 {
            labelLastStep("Paste Tween Keys")
            changed()
        }
    }

    /// `mode` 0 all keys, 1 frame range, 2 current frame.
    func tweenBake(mode: UInt32, from: Int, to: Int, newLayer: Bool, keepSource: Bool) {
        let r = pixmith_tween_bake_ex(doc, UInt32(layer), mode, UInt32(max(0, from)), UInt32(max(0, to)), newLayer, keepSource)
        if r >= 0 {
            labelLastStep(keepSource ? "Bake Tween (keep source)" : "Bake Tween")
            changed()
        }
    }

    func tweenBakeAll(keepSource: Bool, newLayer: Bool) {
        if pixmith_tween_bake_all(doc, keepSource, newLayer) > 0 {
            labelLastStep("Bake All Tweens")
            changed()
        }
    }

    var tweenPivot: (Float, Float)? {
        var p = [Float](repeating: 0, count: 2)
        return pixmith_tween_get_pivot(doc, UInt32(layer), &p) ? (p[0], p[1]) : nil
    }

    func tweenSetPivot(_ x: Float, _ y: Float, custom: Bool) {
        if pixmith_tween_set_pivot(doc, UInt32(layer), x, y, custom) { labelLastStep("Set Tween Pivot"); changed() }
    }

    /// Flag 0 RotSprite, 1 snap scale, 2 ignore opacity.
    func tweenFlag(_ flag: UInt32) -> Bool {
        let o = pixmith_tween_options(doc, UInt32(layer))
        return [o & 2 != 0, o & 8 != 0, o & 16 != 0][Int(flag)]
    }

    func tweenSetFlag(_ flag: UInt32, _ on: Bool) {
        if pixmith_tween_set_flag(doc, UInt32(layer), flag, on) { labelLastStep("Tween Option"); changed() }
    }

    /// The tween parent (rig link) of the active layer, if any.
    var tweenParent: Int? {
        let p = pixmith_tween_get_parent(doc, UInt32(layer))
        return p >= 0 ? Int(p) : nil
    }

    /// Layers the active layer can follow: not itself, no descendant, no group it contains or sits in.
    var tweenParentCandidates: [Int] {
        let cur = tweenParent
        return (0..<layerCount).reversed().filter { $0 == cur || pixmith_tween_can_parent(doc, UInt32(layer), UInt32($0)) }
    }

    func tweenSetParent(_ parent: Int?) {
        if pixmith_tween_set_parent(doc, UInt32(layer), Int32(parent ?? -1)) {
            labelLastStep("Set Tween Parent"); changed()
        }
    }

    var tweenWarning: String? {
        guard let p = pixmith_tween_warning_text(doc, UInt32(layer)) else { return nil }
        defer { pixmith_string_free(p) }
        return String(cString: p)
    }

    var tweenRange: ClosedRange<Int>? {
        var r: [UInt32] = [0, 0]
        guard pixmith_tween_range(doc, UInt32(layer), &r) else { return nil }
        return Int(r[0])...Int(r[1])
    }
}

// MARK: - Bezier curve editor

struct BezierEditor: View {
    @Binding var p: [Double]
    var onCommit: () -> Void
    @State private var dragging: Int?

    private let size: CGFloat = 156
    private let yLo = -0.5, yHi = 1.5

    private func toView(_ x: Double, _ y: Double) -> CGPoint {
        CGPoint(x: x * size, y: size * (1 - (y - yLo) / (yHi - yLo)))
    }

    private func fromView(_ pt: CGPoint) -> (Double, Double) {
        let x = min(1, max(0, Double(pt.x / size)))
        let y = yLo + (1 - Double(pt.y / size)) * (yHi - yLo)
        return (x, min(yHi, max(yLo, y)))
    }

    static let presets: [(String, [Double])] = [
        ("Linear", [0, 0, 1, 1]), ("Ease", [0.25, 0.1, 0.25, 1]), ("Ease In", [0.42, 0, 1, 1]),
        ("Ease Out", [0, 0, 0.58, 1]), ("Ease In-Out", [0.42, 0, 0.58, 1]), ("Snap", [0.8, 0, 0.2, 1]),
        ("Back", [0.34, 1.56, 0.64, 1]),
    ]

    var body: some View {
        HStack(alignment: .top, spacing: DS.Space.md) {
            curve
            VStack(alignment: .leading, spacing: DS.Space.sm) {
                Menu {
                    ForEach(BezierEditor.presets, id: \.0) { name, v in
                        Button(name) { p = v; onCommit() }
                    }
                } label: {
                    HStack(spacing: DS.Space.xs) {
                        Text("Presets").font(DS.Font.body)
                        Spacer(minLength: 0)
                        Image(systemName: "chevron.down").font(.system(size: 9, weight: .semibold)).foregroundStyle(DSColor.textTertiary)
                    }
                    .padding(.horizontal, DS.Space.sm)
                    .frame(width: 112, height: DS.Metric.control)
                    .background(RoundedRectangle(cornerRadius: DS.Radius.field).fill(DSColor.field))
                    .contentShape(Rectangle())
                }
                .menuStyle(.button)
                .buttonStyle(.plain)
                .menuIndicator(.hidden)
                .fixedSize()
                .help("Curve presets")
                VStack(alignment: .leading, spacing: 2) {
                    Text(String(format: "%.2f   %.2f", p[0], p[1]))
                    Text(String(format: "%.2f   %.2f", p[2], p[3]))
                }
                .font(DS.Font.mono).foregroundStyle(DSColor.textSecondary)
                DSNote("Drag the two handles to shape the ease.")
            }
        }
    }

    private var curve: some View {
        Canvas { ctx, _ in
            let box = Path(CGRect(x: 0, y: toView(0, 1).y, width: size, height: toView(0, 0).y - toView(0, 1).y))
            ctx.stroke(box, with: .color(DSColor.separator), lineWidth: 1)
            var diag = Path()
            diag.move(to: toView(0, 0))
            diag.addLine(to: toView(1, 1))
            ctx.stroke(diag, with: .color(DSColor.textTertiary.opacity(0.7)), style: StrokeStyle(lineWidth: 1, dash: [3, 3]))
            let a = toView(p[0], p[1]), b = toView(p[2], p[3])
            var handles = Path()
            handles.move(to: toView(0, 0))
            handles.addLine(to: a)
            handles.move(to: toView(1, 1))
            handles.addLine(to: b)
            ctx.stroke(handles, with: .color(DSColor.textSecondary), lineWidth: 1)
            var curve = Path()
            curve.move(to: toView(0, 0))
            for i in 1...48 {
                let u = Double(i) / 48
                let m = 1 - u
                let x = 3 * p[0] * u * m * m + 3 * p[2] * u * u * m + u * u * u
                let y = 3 * p[1] * u * m * m + 3 * p[3] * u * u * m + u * u * u
                curve.addLine(to: toView(x, y))
            }
            ctx.stroke(curve, with: .color(DSColor.accent), lineWidth: 2)
            for pt in [a, b] {
                let r = CGRect(x: pt.x - 5, y: pt.y - 5, width: 10, height: 10)
                ctx.fill(Path(ellipseIn: r), with: .color(.white))
                ctx.stroke(Path(ellipseIn: r), with: .color(DSColor.accent), lineWidth: 2)
            }
        }
        .frame(width: size, height: size)
        .background(RoundedRectangle(cornerRadius: DS.Radius.field).fill(DSColor.recessed))
        .clipShape(RoundedRectangle(cornerRadius: DS.Radius.field))
        .gesture(DragGesture(minimumDistance: 0)
            .onChanged { g in
                if dragging == nil {
                    let a = toView(p[0], p[1]), b = toView(p[2], p[3])
                    let da = hypot(a.x - g.startLocation.x, a.y - g.startLocation.y)
                    let db = hypot(b.x - g.startLocation.x, b.y - g.startLocation.y)
                    dragging = da <= db ? 0 : 1
                }
                let (x, y) = fromView(g.location)
                if dragging == 0 { p[0] = x; p[1] = y } else { p[2] = x; p[3] = y }
            }
            .onEnded { _ in
                dragging = nil
                onCommit()
            })
    }
}

// MARK: - Dope sheet

struct DopeSheet: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject var ui: TweenUI
    @State private var drag: (prop: TweenProp, frame: Int, dx: CGFloat)?

    private let cell: CGFloat = 24
    private let rowHeight: CGFloat = 22
    private let labelWidth: CGFloat = 32

    var body: some View {
        let frames = model.frameCount
        let range = model.tweenRange
        ScrollView(.horizontal) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 0) {
                    Color.clear.frame(width: labelWidth, height: 18)
                    ForEach(0..<frames, id: \.self) { f in
                        Text("\(f + 1)").font(.system(size: 10, weight: f == model.frame ? .semibold : .regular).monospacedDigit())
                            .foregroundStyle(f == model.frame ? DSColor.text : DSColor.textTertiary)
                            .frame(width: cell, height: 18)
                            .contentShape(Rectangle())
                            .onTapGesture { model.frame = f }
                    }
                }
                ForEach(TweenProp.allCases) { prop in
                    row(prop, frames: frames, range: range)
                }
            }
            .padding(.bottom, DS.Space.xs)
        }
    }

    private func row(_ prop: TweenProp, frames: Int, range: ClosedRange<Int>?) -> some View {
        let keys = model.tweenKeys(prop)
        let rowSelected = prop == ui.selProp
        return HStack(spacing: 0) {
            Text(prop.short).font(DS.Font.captionMedium)
                .foregroundStyle(rowSelected ? DSColor.text : DSColor.textTertiary)
                .frame(width: labelWidth, alignment: .leading)
            ZStack(alignment: .leading) {
                RoundedRectangle(cornerRadius: DS.Radius.chip)
                    .fill(rowSelected ? DSColor.selected : DSColor.field)
                    .frame(width: CGFloat(frames) * cell, height: rowHeight)
                Rectangle().fill(DSColor.accent.opacity(0.22))
                    .frame(width: cell, height: rowHeight)
                    .offset(x: CGFloat(model.frame) * cell)
                if let range {
                    Capsule().fill(DSColor.textTertiary.opacity(0.45))
                        .frame(width: CGFloat(range.count) * cell - 6, height: 2)
                        .offset(x: CGFloat(range.lowerBound) * cell + 3, y: 0)
                }
                ForEach(keys, id: \.frame) { k in
                    let moving = drag.map { $0.prop == prop && $0.frame == k.frame } ?? false
                    let selected = ui.selProp == prop && ui.selFrame == k.frame
                    let shift: CGFloat = moving ? (drag?.dx ?? 0) : 0
                    Image(systemName: "diamond.fill")
                        .font(.system(size: selected ? 12 : 10))
                        .foregroundStyle(selected ? DSColor.text : DSColor.accent)
                        .frame(width: cell, height: rowHeight)
                        .contentShape(Rectangle())
                        .offset(x: CGFloat(k.frame) * cell + shift)
                        .gesture(DragGesture(minimumDistance: 3)
                            .onChanged { g in drag = (prop, k.frame, g.translation.width) }
                            .onEnded { g in
                                drag = nil
                                let to = min(frames - 1, max(0, k.frame + Int((g.translation.width / cell).rounded())))
                                if to != k.frame { model.tweenMoveKey(prop, from: k.frame, to: to) }
                                ui.selProp = prop
                                ui.selFrame = to
                            })
                        .onTapGesture {
                            ui.selProp = prop
                            ui.selFrame = k.frame
                            model.frame = k.frame
                        }
                }
            }
            .frame(width: CGFloat(frames) * cell, height: rowHeight)
        }
    }
}

// MARK: - Panel

struct TweenView: View {
    @ObservedObject var model: SpriteModel
    @ObservedObject private var ui = TweenUI.shared
    @State private var valueText = "0"
    @State private var ease: TweenEase = .linear
    @State private var bezier: [Double] = [0.25, 0.1, 0.25, 1]
    @State private var pivotX = ""
    @State private var pivotY = ""
    @State private var copyFrom = "1"
    @State private var copyTo = "1"
    @State private var bakeMode: UInt32 = 0
    @State private var bakeFrom = "1"
    @State private var bakeTo = "1"
    @State private var bakeNewLayer = false
    @State private var bakeKeep = false

    private var bezierFloats: [Float] { bezier.map { Float($0) } }
    private var currentKey: TweenKeyInfo? { model.tweenKeys(ui.selProp).first { $0.frame == model.frame } }

    var body: some View {
        // Reading `revision` makes the panel refresh after every edit.
        let _ = model.revision
        VStack(spacing: 0) {
            DSToolbar {
                Image(systemName: "point.topleft.down.curvedto.point.bottomright.up").font(.system(size: 12)).foregroundStyle(DSColor.accent)
                Text(model.layerName(model.layer)).font(DS.Font.title).lineLimit(1)
                Text("layer \(model.layer + 1) · frame \(model.frame + 1)").font(DS.Font.caption).foregroundStyle(DSColor.textTertiary)
                Spacer(minLength: 0)
            }
            ScrollView {
                VStack(spacing: 0) {
                    keySection
                    InspectorSection("Dope sheet", id: "tween.dope", subtitle: "\(model.frameCount) frames") {
                        VStack(alignment: .leading, spacing: DS.Space.md) {
                            DopeSheet(model: model, ui: ui)
                            copySection
                        }
                    }
                    InspectorSection("Pivot and options", id: "tween.pivot") { pivotAndOptions }
                    InspectorSection("Canvas", id: "tween.canvas") { canvasSection }
                    InspectorSection("Bake", id: "tween.bake") { bakeSection }
                }
            }
        }
        .frame(minWidth: 400, minHeight: 520)
        .background(DSColor.panel)
        .onAppear { loadKey(); loadPivot() }
        .onChange(of: model.frame) { _ in loadKey() }
        .onChange(of: model.layer) { _ in loadKey(); loadPivot() }
        .onChange(of: ui.selProp) { _ in loadKey() }
        .onChange(of: ui.editOnCanvas) { _ in model.revision &+= 1 }
    }

    private var keySection: some View {
        InspectorSection("Key", id: "tween.key", subtitle: ui.selProp.title) {
            VStack(alignment: .leading, spacing: DS.Space.sm) {
                DSFormRow(label: "Property", labelWidth: 84) {
                    DSPopup(selection: $ui.selProp, options: TweenProp.allCases.map { ($0, $0.title) }, help: "Property to key", fill: true)
                }
                DSFormRow(label: "Value", labelWidth: 84) {
                    TextField("0", text: $valueText).dsTextField().frame(width: 96)
                    Spacer(minLength: 0)
                }
                DSFormRow(label: "Ease to next", labelWidth: 84) {
                    DSPopup(selection: $ease, options: TweenEase.allCases.map { ($0, $0.title) }, help: "How the value moves to the next key",
                            fill: true)
                        .onChange(of: ease) { e in
                            if currentKey != nil, e != .bezier || currentKey?.ease != .bezier {
                                model.tweenSetEase(ui.selProp, frame: model.frame, ease: e, bezier: bezierFloats)
                            }
                        }
                }
                if ease == .bezier {
                    BezierEditor(p: $bezier) {
                        if currentKey != nil {
                            model.tweenSetEase(ui.selProp, frame: model.frame, ease: .bezier, bezier: bezierFloats)
                        }
                    }
                    .padding(.vertical, DS.Space.xs)
                }
                HStack(spacing: DS.Space.sm) {
                    Button("Set Key at Current Frame") {
                        model.tweenSetKey(ui.selProp, frame: model.frame, value: Float(valueText) ?? 0, ease: ease, bezier: bezierFloats)
                        ui.selFrame = model.frame
                    }
                    .buttonStyle(DSPrimaryButtonStyle())
                    Button("Delete Key") { model.tweenRemoveKey(ui.selProp, frame: ui.selFrame ?? model.frame) }
                        .buttonStyle(DSSecondaryButtonStyle())
                        .disabled(model.tweenKeys(ui.selProp).isEmpty)
                }
                .padding(.top, DS.Space.xs)
            }
        }
    }

    private var copySection: some View {
        HStack(spacing: DS.Space.sm - 2) {
            Text("Keys").font(DS.Font.body).foregroundStyle(DSColor.textSecondary)
            TextField("", text: $copyFrom).dsTextField().frame(width: 46)
            Text("to").font(DS.Font.body).foregroundStyle(DSColor.textTertiary)
            TextField("", text: $copyTo).dsTextField().frame(width: 46)
            Button("Copy") {
                _ = model.tweenCopyKeys(from: (Int(copyFrom) ?? 1) - 1, to: (Int(copyTo) ?? 1) - 1)
            }
            .buttonStyle(DSBarButtonStyle())
            .help("Copy the keys between these frames")
            Button("Paste at Current Frame") { model.tweenPasteKeys(at: model.frame) }
                .buttonStyle(DSBarButtonStyle())
            Spacer(minLength: 0)
        }
    }

    private var pivotAndOptions: some View {
        VStack(alignment: .leading, spacing: DS.Space.sm) {
            DSFormRow(label: "Pivot", labelWidth: 84) {
                TextField("X", text: $pivotX).dsTextField().frame(width: 60)
                TextField("Y", text: $pivotY).dsTextField().frame(width: 60)
                Button("Apply") {
                    if let x = Float(pivotX), let y = Float(pivotY) { model.tweenSetPivot(x, y, custom: true) }
                }
                .buttonStyle(DSBarButtonStyle())
                Button("Center") { model.tweenSetPivot(0, 0, custom: false); loadPivot() }
                    .buttonStyle(DSBarButtonStyle())
                Spacer(minLength: 0)
            }
            DSFormRow(label: "Parent", labelWidth: 84) {
                Picker("Parent", selection: Binding(get: { model.tweenParent ?? -1 },
                                                    set: { model.tweenSetParent($0 < 0 ? nil : $0) })) {
                    Text("None").tag(-1)
                    ForEach(model.tweenParentCandidates, id: \.self) { Text("\($0 + 1) \u{00B7} \(model.layerName($0))").tag($0) }
                }
                .labelsHidden()
                Spacer(minLength: 0)
            }
            DSNote("Follows the parent's motion; pivot and keys are relative to it. Bake All keeps the rig together.")
            Toggle("RotSprite rotation (off = nearest, faster)", isOn: Binding(get: { model.tweenFlag(0) },
                                                                              set: { model.tweenSetFlag(0, $0) }))
                .toggleStyle(.checkbox).font(DS.Font.body)
            Toggle("Snap scale to integer", isOn: Binding(get: { model.tweenFlag(1) }, set: { model.tweenSetFlag(1, $0) }))
                .toggleStyle(.checkbox).font(DS.Font.body)
            Toggle("Ignore opacity track", isOn: Binding(get: { model.tweenFlag(2) }, set: { model.tweenSetFlag(2, $0) }))
                .toggleStyle(.checkbox).font(DS.Font.body)
            if let w = model.tweenWarning {
                HStack(alignment: .top, spacing: DS.Space.xs + 2) {
                    Image(systemName: "exclamationmark.triangle.fill").font(.system(size: 11)).foregroundStyle(Color.orange)
                    Text(w).font(DS.Font.caption).foregroundStyle(DSColor.textSecondary).fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    private var canvasSection: some View {
        VStack(alignment: .leading, spacing: DS.Space.sm) {
            Toggle("Edit on canvas", isOn: $ui.editOnCanvas).toggleStyle(.checkbox).font(DS.Font.body)
            DSNote("Drag = move, corner = scale, top handle = rotate, orange dot = pivot.")
            Toggle("Auto-key (create keys while dragging)", isOn: $ui.autoKey).toggleStyle(.checkbox).font(DS.Font.body)
                .disabled(!ui.editOnCanvas)
            DSNote("Motion path: dots are frames, large dots are keys.")
        }
    }

    private var bakeSection: some View {
        VStack(alignment: .leading, spacing: DS.Space.sm) {
            DSSegmented(selection: $bakeMode, options: [
                .init(UInt32(0), title: "All keys"), .init(UInt32(1), title: "Selected frames"), .init(UInt32(2), title: "Current frame"),
            ])
            if bakeMode == 1 {
                HStack(spacing: DS.Space.sm - 2) {
                    Text("Frames").font(DS.Font.body).foregroundStyle(DSColor.textSecondary)
                    TextField("", text: $bakeFrom).dsTextField().frame(width: 52)
                    Text("to").font(DS.Font.body).foregroundStyle(DSColor.textTertiary)
                    TextField("", text: $bakeTo).dsTextField().frame(width: 52)
                }
            }
            Toggle("Into a new layer (original is hidden)", isOn: $bakeNewLayer).toggleStyle(.checkbox).font(DS.Font.body)
            Toggle("Keep source and keys (re-bakeable)", isOn: $bakeKeep).toggleStyle(.checkbox).font(DS.Font.body)
            HStack(spacing: DS.Space.sm) {
                Button("Bake") { bake(keep: bakeKeep) }.buttonStyle(DSPrimaryButtonStyle())
                Button("Bake & Keep Source") { bake(keep: true) }.buttonStyle(DSSecondaryButtonStyle())
                Button("Bake All Layers") { model.tweenBakeAll(keepSource: bakeKeep, newLayer: bakeNewLayer) }
                    .buttonStyle(DSSecondaryButtonStyle())
            }
            .padding(.top, DS.Space.xs)
            DSNote("Source = layer cel (group: its composite) at the first key frame. Groups bake into a new layer. Partial ranges keep the keys.")
        }
    }

    private func bake(keep: Bool) {
        model.tweenBake(mode: bakeMode, from: bakeMode == 2 ? model.frame : (Int(bakeFrom) ?? 1) - 1,
                        to: (Int(bakeTo) ?? 1) - 1, newLayer: bakeNewLayer, keepSource: keep)
    }

    private func loadKey() {
        let keys = model.tweenKeys(ui.selProp)
        if let k = keys.first(where: { $0.frame == model.frame }) {
            valueText = String(format: "%g", Double(k.value))
            ease = k.ease
            if k.ease == .bezier { bezier = k.bezier.map { Double($0) } }
            ui.selFrame = k.frame
        } else {
            valueText = String(format: "%g", Double(model.tweenValue(ui.selProp, at: model.frame)))
        }
    }

    private func loadPivot() {
        if let p = model.tweenPivot {
            pivotX = String(format: "%g", Double(p.0))
            pivotY = String(format: "%g", Double(p.1))
        }
    }
}

/// Floating panel opened from View > Tween.
final class TweenController: NSObject, NSWindowDelegate {
    static let shared = TweenController()
    private var panel: NSPanel?
    private weak var model: SpriteModel?

    func toggle(model: SpriteModel) {
        if let panel, panel.isVisible { panel.close(); return }
        self.model = model
        if panel == nil {
            let p = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 440, height: 680),
                            styleMask: [.titled, .closable, .resizable, .utilityWindow],
                            backing: .buffered, defer: false)
            p.title = "Tween"
            p.isFloatingPanel = true
            p.hidesOnDeactivate = false
            p.isReleasedWhenClosed = false
            p.delegate = self
            p.setFrameAutosaveName("PixmithTween")
            if !p.setFrameUsingName("PixmithTween") { p.center() }
            panel = p
        }
        panel?.contentView = NSHostingView(rootView: TweenView(model: model))
        panel?.orderFront(nil)
        TweenUI.shared.panelOpen = true
        model.revision &+= 1
    }

    func windowWillClose(_ notification: Notification) {
        panel?.contentView = NSView()
        TweenUI.shared.panelOpen = false
        TweenUI.shared.editOnCanvas = false
        model?.revision &+= 1
    }
}

/// View menu items plus Tween in one entry: the commands builder takes at most ten children.
struct ViewTweenCommands: Commands {
    @ObservedObject var model: SpriteModel
    var body: some Commands {
        ViewCommands(model: model)
        TweenCommands(model: model)
    }
}

struct TweenCommands: Commands {
    @ObservedObject var model: SpriteModel

    var body: some Commands {
        CommandGroup(after: .toolbar) {
            Button("Tween") { TweenController.shared.toggle(model: model) }
                .keyboardShortcut("t", modifiers: [.command, .option])
        }
    }
}

// MARK: - Canvas overlay and handles

/// Pointer drag in progress on the canvas.
private final class TweenDrag {
    static let shared = TweenDrag()
    enum Kind { case move, rotate, scale, pivot }
    var active = false
    var kind = Kind.move
    var start = CGPoint.zero
    var t = [Float](repeating: 0, count: 8)
    var startAngle = 0.0
    var startDist = 1.0
    var keysAtFrame: Set<UInt32> = []
}

extension SpriteModel {
    /// Transform at the current frame (x, y, rot, sx, sy, opacity, pivot x, y), or an identity one
    /// around the effective pivot when the layer has no keys yet.
    private func tweenCurrent() -> [Float]? {
        var t = [Float](repeating: 0, count: 8)
        if pixmith_tween_transform_at(doc, UInt32(layer), UInt32(frame), &t) { return t }
        guard let p = tweenPivot else { return nil }
        return [0, 0, 0, 1, 1, 1, p.0, p.1]
    }

    private func tweenBounds() -> [Float]? {
        var b = [Float](repeating: 0, count: 4)
        return pixmith_tween_source_bounds(doc, UInt32(layer), &b) ? b : nil
    }

    /// Box corners (TL, TR, BR, BL), the pivot's destination and the rotate handle, in canvas pixels.
    private func tweenGeometry(_ t: [Float], _ b: [Float]) -> (corners: [CGPoint], pivot: CGPoint, rotate: CGPoint) {
        let rad = Double(t[2]) * Double.pi / 180
        let c = cos(rad), s = sin(rad)
        let pv = (x: Double(t[6]), y: Double(t[7]))
        let dest = CGPoint(x: pv.x + Double(t[0]), y: pv.y + Double(t[1]))
        func map(_ x: Float, _ y: Float) -> CGPoint {
            let dx = (Double(x) - pv.x) * Double(t[3]), dy = (Double(y) - pv.y) * Double(t[4])
            return CGPoint(x: dest.x + c * dx - s * dy, y: dest.y + s * dx + c * dy)
        }
        let corners = [map(b[0], b[1]), map(b[2], b[1]), map(b[2], b[3]), map(b[0], b[3])]
        let top = CGPoint(x: (corners[0].x + corners[1].x) / 2, y: (corners[0].y + corners[1].y) / 2)
        let center = CGPoint(x: (corners[0].x + corners[2].x) / 2, y: (corners[0].y + corners[2].y) / 2)
        let len = max(hypot(top.x - center.x, top.y - center.y), 0.001)
        let off = 14 / Double(zoom)
        let rot = CGPoint(x: top.x + (top.x - center.x) / len * off, y: top.y + (top.y - center.y) / len * off)
        return (corners, dest, rot)
    }

    /// Motion path, box and handles of the current layer's tween.
    func drawTweenOverlay(_ ctx: CGContext) {
        let ui = TweenUI.shared
        guard ui.panelOpen || ui.editOnCanvas, let range = tweenRange else { return }
        let z = zoom
        var keyFrames = Set<Int>()
        for p in TweenProp.allCases { for k in tweenKeys(p) { keyFrames.insert(k.frame) } }
        ctx.saveGState()
        var prev: CGPoint?
        var dots: [(CGPoint, Bool, Bool)] = []
        let path = CGMutablePath()
        for f in range {
            var t = [Float](repeating: 0, count: 8)
            guard pixmith_tween_transform_at(doc, UInt32(layer), UInt32(f), &t) else { continue }
            let pt = CGPoint(x: (Double(t[6]) + Double(t[0])) * z, y: (Double(t[7]) + Double(t[1])) * z)
            if prev == nil { path.move(to: pt) } else { path.addLine(to: pt) }
            prev = pt
            dots.append((pt, keyFrames.contains(f), f == frame))
        }
        // The motion path in the accent color with a dark hairline under it, so it reads on any art.
        ctx.setLineJoin(.round)
        ctx.setLineWidth(3)
        ctx.setStrokeColor(NSColor.black.withAlphaComponent(0.35).cgColor)
        ctx.addPath(path)
        ctx.strokePath()
        ctx.setLineWidth(1.5)
        ctx.setStrokeColor(NSColor.controlAccentColor.cgColor)
        ctx.addPath(path)
        ctx.strokePath()
        for (pt, isKey, isCurrent) in dots {
            let r: CGFloat = isKey ? 4.5 : 2.5
            let rect = CGRect(x: pt.x - r, y: pt.y - r, width: r * 2, height: r * 2)
            ctx.setFillColor((isCurrent ? NSColor.white : isKey ? NSColor.controlAccentColor : NSColor.controlAccentColor.withAlphaComponent(0.7)).cgColor)
            ctx.fillEllipse(in: rect)
            if isKey || isCurrent {
                ctx.setLineWidth(1)
                ctx.setStrokeColor(NSColor.black.cgColor)
                ctx.strokeEllipse(in: rect)
            }
        }
        if ui.editOnCanvas, let t = tweenCurrent(), let b = tweenBounds() {
            // Same handle look as the free transform: white and black dashed outline, square corner handles with a
            // black edge, an orange pivot. The rotate handle is a white dot on a stem.
            let g = tweenGeometry(t, b)
            let pts = g.corners.map { CGPoint(x: $0.x * z, y: $0.y * z) }
            let top = CGPoint(x: (pts[0].x + pts[1].x) / 2, y: (pts[0].y + pts[1].y) / 2)
            let rot = CGPoint(x: g.rotate.x * z, y: g.rotate.y * z)
            let outline = CGMutablePath()
            outline.addLines(between: pts)
            outline.closeSubpath()
            outline.move(to: top)
            outline.addLine(to: rot)
            ctx.setLineWidth(1)
            ctx.setStrokeColor(NSColor.white.cgColor)
            ctx.addPath(outline)
            ctx.strokePath()
            ctx.setStrokeColor(NSColor.black.cgColor)
            ctx.setLineDash(phase: 0, lengths: [4, 4])
            ctx.addPath(outline)
            ctx.strokePath()
            ctx.setLineDash(phase: 0, lengths: [])
            for p in pts {
                let r = CGRect(x: p.x - 4, y: p.y - 4, width: 8, height: 8)
                ctx.setFillColor(NSColor.white.cgColor)
                ctx.fill(r)
                ctx.setStrokeColor(NSColor.black.cgColor)
                ctx.stroke(r)
            }
            let rr = CGRect(x: rot.x - 4, y: rot.y - 4, width: 8, height: 8)
            ctx.setFillColor(NSColor.white.cgColor)
            ctx.fillEllipse(in: rr)
            ctx.setStrokeColor(NSColor.black.cgColor)
            ctx.strokeEllipse(in: rr)
            let pv = CGPoint(x: g.pivot.x * z, y: g.pivot.y * z)
            let pr = CGRect(x: pv.x - 4, y: pv.y - 4, width: 8, height: 8)
            ctx.setFillColor(NSColor.systemOrange.cgColor)
            ctx.fillEllipse(in: pr)
            ctx.setStrokeColor(NSColor.black.cgColor)
            ctx.strokeEllipse(in: pr)
        }
        ctx.restoreGState()
    }

    var tweenDragging: Bool { TweenDrag.shared.active }

    private func canvasPoint(_ e: NSEvent, in view: NSView) -> CGPoint {
        let p = view.convert(e.locationInWindow, from: nil)
        return CGPoint(x: p.x / zoom, y: p.y / zoom)
    }

    /// Starts a handle drag when "Edit on canvas" is on. Returns whether the click was consumed.
    func tweenMouseDown(_ e: NSEvent, in view: NSView) -> Bool {
        let ui = TweenUI.shared
        guard ui.editOnCanvas, let t = tweenCurrent(), let b = tweenBounds() else { return false }
        let d = TweenDrag.shared
        let p = canvasPoint(e, in: view)
        let g = tweenGeometry(t, b)
        let tol = 8 / Double(zoom)
        func near(_ q: CGPoint) -> Bool { hypot(q.x - p.x, q.y - p.y) <= tol }
        d.t = t
        d.start = p
        d.keysAtFrame = []
        for prop in TweenProp.allCases where tweenKeys(prop).contains(where: { $0.frame == frame }) {
            d.keysAtFrame.insert(prop.rawValue)
        }
        if near(g.pivot) {
            d.kind = .pivot
        } else if near(g.rotate) {
            d.kind = .rotate
            d.startAngle = atan2(Double(p.y - g.pivot.y), Double(p.x - g.pivot.x))
        } else if let c = g.corners.first(where: near) {
            d.kind = .scale
            d.startDist = max(hypot(Double(c.x - g.pivot.x), Double(c.y - g.pivot.y)), 0.001)
            d.start = c
        } else {
            d.kind = .move
        }
        d.active = true
        pixmith_begin_transaction(doc)
        return true
    }

    func tweenMouseDragged(_ e: NSEvent, in view: NSView) {
        let d = TweenDrag.shared
        guard d.active else { return }
        let ui = TweenUI.shared
        let p = canvasPoint(e, in: view)
        var values: [Float] = [d.t[0], d.t[1], d.t[2], d.t[3], d.t[4], d.t[5]]
        var mask: UInt32 = 0
        let pivotDest = CGPoint(x: Double(d.t[6]) + Double(d.t[0]), y: Double(d.t[7]) + Double(d.t[1]))
        switch d.kind {
        case .move:
            values[0] = d.t[0] + Float(p.x - d.start.x)
            values[1] = d.t[1] + Float(p.y - d.start.y)
            mask = 0b11
        case .rotate:
            var delta = (atan2(Double(p.y - pivotDest.y), Double(p.x - pivotDest.x)) - d.startAngle) * 180 / Double.pi
            var angle = Double(d.t[2]) + delta
            if e.modifierFlags.contains(.shift) { angle = (angle / 15).rounded() * 15 }
            delta = angle
            values[2] = Float(delta)
            mask = 0b100
        case .scale:
            let dist = hypot(Double(p.x - pivotDest.x), Double(p.y - pivotDest.y))
            let f = Float(dist / d.startDist)
            values[3] = d.t[3] * f
            values[4] = d.t[4] * f
            mask = 0b11000
        case .pivot:
            let x = Float(((Double(p.x) - Double(d.t[0])) * 2).rounded() / 2)
            let y = Float(((Double(p.y) - Double(d.t[1])) * 2).rounded() / 2)
            if pixmith_tween_set_pivot(doc, UInt32(layer), x, y, true) { changed() }
            return
        }
        if !ui.autoKey {
            var allowed: UInt32 = 0
            for k in d.keysAtFrame { allowed |= 1 << k }
            mask &= allowed
        }
        guard mask != 0 else { return }
        if pixmith_tween_set_keys_at(doc, UInt32(layer), UInt32(frame), mask, values, 0) { changed() }
    }

    func tweenMouseUp() {
        let d = TweenDrag.shared
        guard d.active else { return }
        d.active = false
        pixmith_end_transaction(doc)
        labelLastStep("Tween Canvas Edit")
        changed()
    }
}
